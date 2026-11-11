// Based on Smithay's anvil udev backend (MIT; see LICENSE-smithay.txt), cut down for
// Vergence: server milestone 1, stage 2 (the real monitors).
//
// One GPU, every connected monitor, libinput, a libseat session (seatd on FreeBSD).
// Each monitor's scale comes from its own EDID (read from DRM, parsed by
// vergence-edid, derived by vergence-profile), so every app draws natively at each
// panel's calculated scale (Mode B, decision 2026-11-07). Left to right as
// VERGENCE_LEFT_TO_RIGHT says (connector names), else Hubble's desk: the 14" 4K
// (HDMI-A-2) left of the ASUS (HDMI-A-1). Ctrl+Alt+Backspace quits; Ctrl+Alt+Fn
// switches consoles (input.rs).

use std::{collections::HashMap, path::Path, time::Duration};

use smithay::{
    backend::{
        allocator::{
            Fourcc,
            format::FormatSet,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, NodeType,
            compositor::FrameFlags,
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
        },
        egl::{EGLContext, EGLDisplay},
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            ImportDma, ImportMemWl,
            element::{
                memory::MemoryRenderBufferRenderElement,
                surface::WaylandSurfaceRenderElement,
            },
            gles::GlesRenderer,
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{UdevBackend, all_gpus, primary_gpu},
    },
    output::{Mode as WlMode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        calloop::{EventLoop, timer::{TimeoutAction, Timer}},
        drm::control::{Device as ControlDevice, ModeTypeFlags, connector, crtc, property},
        input::Libinput,
        rustix::fs::OFlags,
    },
    utils::{DeviceFd, Transform},
    wayland::fractional_scale::with_fractional_scale,
};
use smithay_drm_extras::drm_scanner::{DrmScanEvent, DrmScanner};


use crate::Server;

smithay::backend::renderer::element::render_elements! {
    pub OutputElements<=GlesRenderer>;
    Scene = crate::decor::SceneElement,
    PointerTheme = MemoryRenderBufferRenderElement<GlesRenderer>,
    PointerSurface = WaylandSurfaceRenderElement<GlesRenderer>,
}

type Manager = DrmOutputManager<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;
type Out = DrmOutput<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

struct Surface {
    output: Output,
    drm_output: Out,
    connector: String,
    /// When this monitor was last drawn (the watchdog, below).
    last: std::time::Instant,
    scale: f64,
    /// For working the scale out again (displays.rs): the monitor's id, its
    /// mode in pixels and its size in millimeters.
    id: String,
    px: (u32, u32),
    mm: Option<(u32, u32)>,
    /// The monitor's EDID (or its connector name when it has none): its identity.
    edid: Vec<u8>,
    /// Its wl_output global, removed when the monitor goes.
    global: smithay::reexports::wayland_server::backend::GlobalId,
}

pub struct Backend {
    renderer: GlesRenderer,
    manager: Manager,
    scanner: DrmScanner,
    surfaces: HashMap<crtc::Handle, Surface>,
}

pub fn run(event_loop: &mut EventLoop<Server>, state: &mut Server) -> Result<(), Box<dyn std::error::Error>> {
    let (session, notifier) = LibSeatSession::new()?;
    let seat = session.seat();
    tracing::info!("seat {seat}");

    let path = primary_gpu(&seat)?
        .or_else(|| all_gpus(&seat).ok()?.into_iter().next())
        .ok_or("no GPU")?;
    let node = DrmNode::from_path(&path)?;
    tracing::info!("GPU {} ({:?}, render node {:?})", path.display(), node.ty(), node.node_with_type(NodeType::Render));

    let mut session_handle = session.clone();
    let fd = session_handle.open(&path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true)?;
    let gbm = GbmDevice::new(fd)?;

    let display = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new(&display)?;
    let renderer = unsafe { GlesRenderer::new(context)? };
    let render_formats: FormatSet = renderer.egl_context().dmabuf_render_formats().iter().copied().collect();
    state.shm_formats_from(&renderer);

    // GPU buffers from apps (linux-dmabuf v4): Xwayland's glamor and GL apps draw on
    // the GPU and hand their buffers over without a copy. Without it Xwayland fell
    // back to software and AlbumWall managed ~4 frames a second at 3.28 (2026-11-07).
    // The feedback names the render node: Mesa opens the device by its number.
    let render_node = node.node_with_type(NodeType::Render).and_then(|r| r.ok()).unwrap_or(node);
    match smithay::wayland::dmabuf::DmabufFeedbackBuilder::new(render_node.dev_id(), renderer.dmabuf_formats()).build() {
        Ok(feedback) => {
            let mut dmabuf_state = smithay::wayland::dmabuf::DmabufState::new();
            let global = dmabuf_state.create_global_with_default_feedback::<Server>(&state.display_handle, &feedback);
            state.dmabuf = Some((dmabuf_state, global));
            tracing::info!("linux-dmabuf on {render_node:?}");
        }
        Err(e) => tracing::warn!("linux-dmabuf: {e:?}"),
    }

    let manager = DrmOutputManager::new(
        drm,
        GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT),
        GbmFramebufferExporter::new(gbm.clone(), Some(node).into()),
        Some(gbm),
        [Fourcc::Abgr8888, Fourcc::Argb8888],
        render_formats,
    );

    state.backend = Some(Backend {
        renderer,
        manager,
        scanner: DrmScanner::new(),
        surfaces: HashMap::new(),
    });
    state.session = Some(session.clone());

    // Input from every device on the seat.
    let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput.udev_assign_seat(&seat).map_err(|_| "libinput: cannot assign seat")?;
    let libinput_for_session = libinput.clone();
    event_loop
        .handle()
        .insert_source(LibinputInputBackend::new(libinput), |event, _, state| {
            // Mice and touchpads are kept, to be given their settings (pointer.rs).
            match &event {
                smithay::backend::input::InputEvent::DeviceAdded { device } => state.pointer_device_added(device.clone()),
                smithay::backend::input::InputEvent::DeviceRemoved { device } => state.pointer_device_removed(device),
                _ => {}
            }
            state.process_input_event(event)
        })?;

    // VBlank: the frame is on screen; draw the next one.
    event_loop.handle().insert_source(drm_notifier, |event, _, state| match event {
        DrmEvent::VBlank(crtc) => {
            if let Some(b) = state.backend.as_mut()
                && let Some(s) = b.surfaces.get_mut(&crtc)
            {
                let _ = s.drm_output.frame_submitted();
            }
            render(state, crtc);
        }
        DrmEvent::Error(e) => tracing::error!("drm: {e:?}"),
    })?;

    // THE WATCHDOG (2026-11-08: the whole desktop froze, pointer and all, with
    // the server idle and nothing logged): drawing is driven by VBlank events,
    // so a lost one, or a frame that ended without arranging the next, stops a
    // monitor for good. Any monitor not drawn for 2 s while the session is
    // active is drawn again, and it is logged.
    event_loop.handle().insert_source(Timer::from_duration(Duration::from_secs(1)), |_, _, state| {
        let stalled: Vec<(crtc::Handle, String, u128)> = state
            .backend
            .as_ref()
            .filter(|_| state.session.as_ref().is_none_or(|s| s.is_active()) && !state.energy.screens_off)
            .map(|b| {
                b.surfaces
                    .iter()
                    .filter(|(_, s)| s.last.elapsed() > Duration::from_secs(2))
                    .map(|(c, s)| (*c, s.connector.clone(), s.last.elapsed().as_millis()))
                    .collect()
            })
            .unwrap_or_default();
        for (crtc, name, ms) in stalled {
            tracing::warn!("watchdog: {name} not drawn for {ms} ms (VBlank lost?); drawing it again");
            if let Some(b) = state.backend.as_mut()
                && let Some(s) = b.surfaces.get_mut(&crtc)
            {
                let _ = s.drm_output.frame_submitted();
                s.drm_output.reset_buffers();
            }
            render(state, crtc);
        }
        TimeoutAction::ToDuration(Duration::from_secs(1))
    })?;

    // Console switches.
    let mut libinput = libinput_for_session;
    event_loop.handle().insert_source(notifier, move |event, _, state| match event {
        SessionEvent::PauseSession => {
            tracing::info!("session paused");
            libinput.suspend();
            if let Some(b) = state.backend.as_mut() {
                b.manager.pause();
            }
        }
        SessionEvent::ActivateSession => {
            tracing::info!("session active");
            let _ = libinput.resume();
            let crtcs: Vec<crtc::Handle> = state
                .backend
                .as_mut()
                .map(|b| {
                    let _ = b.manager.lock().activate(false);
                    b.surfaces.keys().copied().collect()
                })
                .unwrap_or_default();
            for c in crtcs {
                render(state, c);
            }
        }
    })?;

    // The monitors: scanned now, then again whenever udev says the GPU changed, and
    // every 2 s as well, in case FreeBSD's udev emulation does not pass on DRM
    // hotplug events (devd's "DRM HOTPLUG").
    let udev = UdevBackend::new(&seat)?;
    event_loop.handle().insert_source(udev, |event, _, state| {
        if let smithay::backend::udev::UdevEvent::Changed { .. } = event {
            scan(state, false);
        }
    })?;
    // FreeBSD: libudev-devd passes input devices, not DRM hotplug, and every way of
    // asking DRM for connector state probes the monitors (EDID over DDC: 327 ms with
    // the server blocked; polling every 2 s froze dragged windows, 2026-11-07). The
    // kernel tells devd, so a thread reads devd's pipe (as vergd does) and the
    // scan, with its probe, runs only when a monitor really came or went.
    #[cfg(target_os = "freebsd")]
    {
        let (tx, rx) = smithay::reexports::calloop::channel::channel::<()>();
        std::thread::spawn(move || devd_hotplug(tx));
        event_loop.handle().insert_source(rx, |event, _, state| {
            if let smithay::reexports::calloop::channel::Event::Msg(()) = event
                && state.session.as_ref().is_some_and(|s| s.is_active())
            {
                scan(state, false);
            }
        })?;
    }
    scan(state, true);
    let crtcs: Vec<crtc::Handle> = state.backend.as_ref().map(|b| b.surfaces.keys().copied().collect()).unwrap_or_default();
    for c in crtcs {
        render(state, c);
    }
    Ok(())
}

/// The connected monitors: an output each, at its EDID's scale, laid out left to right.
/// At start-up every monitor at once; afterwards (hotplug) a new monitor is set up a
/// second later from a fresh forced probe: FreeBSD's first look after a hotplug can
/// see only the kernel's fallback modes (800x600; vergd, 2026-11-07).
fn scan(state: &mut Server, startup: bool) {
    let Some(b) = state.backend.as_mut() else { return };
    let events = match b.scanner.scan_connectors(b.manager.device()) {
        Ok(e) => e,
        Err(e) => return tracing::warn!("connector scan: {e:?}"),
    };
    let anchors = state.window_anchors();
    let mut changed = false;
    for event in events {
        match event {
            DrmScanEvent::Connected { connector, crtc: Some(crtc) } if startup => {
                connect(state, connector, crtc);
                changed = true;
            }
            DrmScanEvent::Connected { connector, crtc: Some(crtc) } => {
                let handle = connector.handle();
                tracing::info!("hotplug: {}-{} connected; setting up in 1 s", connector.interface().as_str(), connector.interface_id());
                let _ = state.loop_handle.insert_source(Timer::from_duration(Duration::from_secs(1)), move |_, _, st| {
                    let info = st.backend.as_ref().and_then(|b| b.manager.device().get_connector(handle, true).ok());
                    if let Some(info) = info.filter(|i| i.state() == connector::State::Connected) {
                        let anchors = st.window_anchors();
                        connect(st, info, crtc);
                        setup_changed(st, anchors);
                        render(st, crtc);
                    }
                    TimeoutAction::Drop
                });
            }
            DrmScanEvent::Disconnected { connector, crtc: Some(crtc) } => {
                tracing::info!("hotplug: {}-{} disconnected", connector.interface().as_str(), connector.interface_id());
                disconnect(state, crtc);
                changed = true;
            }
            DrmScanEvent::Changed { connector, crtc: Some(crtc) } => {
                // New modes on a monitor that stayed connected (a late probe): set it up
                // again, but only if the mode we would pick differs from the one running.
                // (The scanner reports the ASUS as changed on every scan, mode list
                // unchanged in substance; reconnecting each time blanked it, 2026-11-07.)
                let want = connector
                    .modes()
                    .iter()
                    .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
                    .or(connector.modes().first())
                    .map(|m| m.size());
                let running = state
                    .backend
                    .as_ref()
                    .and_then(|b| b.surfaces.get(&crtc))
                    .and_then(|s| s.output.current_mode())
                    .map(|m| (m.size.w as u16, m.size.h as u16));
                if want.is_none() || want == running {
                    continue;
                }
                tracing::info!("hotplug: {}-{} modes changed ({running:?} -> {want:?})", connector.interface().as_str(), connector.interface_id());
                disconnect(state, crtc);
                connect(state, connector, crtc);
                changed = true;
                render(state, crtc);
            }
            _ => {}
        }
    }
    if changed {
        setup_changed(state, anchors);
    }
}

/// devd's DRM connector hotplug events (FreeBSD), one message each, for as long as
/// devd runs: `!system=DRM subsystem=CONNECTOR type=HOTPLUG cdev=dri/card0 ...`.
#[cfg(target_os = "freebsd")]
fn devd_hotplug(tx: smithay::reexports::calloop::channel::Sender<()>) {
    use std::io::BufRead;
    const PIPE: &str = "/var/run/devd.pipe";
    loop {
        match std::os::unix::net::UnixStream::connect(PIPE) {
            Ok(stream) => {
                for line in std::io::BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    if is_drm_hotplug(&line) {
                        tracing::info!("devd: {line}");
                        if tx.send(()).is_err() {
                            return;
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("{PIPE}: {e}"),
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

#[cfg_attr(not(target_os = "freebsd"), allow(dead_code))]
fn is_drm_hotplug(line: &str) -> bool {
    line.strip_prefix('!').is_some_and(|rest| {
        let mut w = rest.split_whitespace();
        rest.contains("system=DRM") && w.any(|x| x == "type=HOTPLUG")
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn devd_drm_hotplug() {
        assert!(super::is_drm_hotplug("!system=DRM subsystem=CONNECTOR type=HOTPLUG cdev=dri/card0 connector=115"));
        assert!(!super::is_drm_hotplug("!system=USB subsystem=DEVICE type=ATTACH"));
        assert!(!super::is_drm_hotplug("+uhid0 at bus=0"));
    }
}

/// The monitor at `crtc` is gone: its output leaves the desktop and the clients.
fn disconnect(state: &mut Server, crtc: crtc::Handle) {
    let Some(b) = state.backend.as_mut() else { return };
    let Some(s) = b.surfaces.remove(&crtc) else { return };
    state.space.unmap_output(&s.output);
    state.display_handle.remove_global::<Server>(s.global);
    tracing::info!("{} removed", s.connector);
    state.bus_outputs_changed();
}

/// After monitors came or went: lay them out, find the setup, put windows back.
fn setup_changed(state: &mut Server, anchors: Vec<(smithay::desktop::Window, String, smithay::utils::Point<i32, smithay::utils::Logical>)>) {
    // The setup first: the arrangement is kept per setup.
    let edids: Vec<Vec<u8>> = state.backend.as_ref().map(|b| b.surfaces.values().map(|s| s.edid.clone()).collect()).unwrap_or_default();
    state.set_setup(&edids);
    lay_out(state);
    state.rearrange_windows(anchors);
}

/// desktop.toml's [displays] changed (the settings app): work every scale out
/// again and lay the screens out again; windows go with their screens.
pub fn displays_changed(state: &mut Server) {
    let cfg = crate::displays::load();
    let anchors = state.window_anchors();
    let mut notes = Vec::new();
    let Some(b) = state.backend.as_mut() else { return };
    for s in b.surfaces.values_mut() {
        let scales = cfg.scale_for(&s.id, &s.connector, s.px, s.mm);
        if scales.shown != s.scale {
            tracing::info!("{}: scale {:.4} -> {:.4}, {}", s.connector, s.scale, scales.shown, scales.how);
            s.scale = scales.shown;
            s.output.change_current_state(None, None, Some(Scale::Fractional(scales.shown)), None);
        }
        notes.push((s.connector.clone(), scales));
    }
    b.redraw_all();
    state.scale_notes.extend(notes);
    lay_out(state);
    state.rearrange_windows(anchors);
}

fn connect(state: &mut Server, conn: connector::Info, crtc: crtc::Handle) {
    let dh = state.display_handle.clone();
    let Some(b) = state.backend.as_mut() else { return };
    let name = format!("{}-{}", conn.interface().as_str(), conn.interface_id());
    let mode_id = conn.modes().iter().position(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED)).unwrap_or(0);
    let Some(&drm_mode) = conn.modes().get(mode_id) else { return };
    let wl_mode = WlMode::from(drm_mode);
    let (w, h) = drm_mode.size();

    // The panel's size from its EDID (the connector's mm are only the base block's).
    let edid = edid_of(b.manager.device(), conn.handle());
    let parsed = edid.as_deref().and_then(|e| vergence_edid::Edid::parse(e).ok());
    let mm = parsed.as_ref().and_then(|e| e.physical_size_mm()).map(|(a, c)| (a as u32, c as u32)).or_else(|| conn.size());
    let identity = edid.clone().unwrap_or_else(|| name.clone().into_bytes());
    let id = vergence_profile::monitor_id(&identity);
    // Calculated from the EDID, the screen's viewing distance and the size
    // preference, or set by hand (displays.rs). In 120ths, as wp-fractional-scale-v1
    // tells it to apps: the output at exactly the scale the apps draw at, so their
    // buffers are shown 1:1, not resampled (the ASUS's 1.0703 drawn at 128/120 and
    // shown at 1.0703 was soft, 2026-11-07).
    let scales = crate::displays::load().scale_for(&id, &name, (w as u32, h as u32), mm);
    let (scale, how) = (scales.shown, scales.how.clone());
    let label = parsed.as_ref().and_then(|e| e.name.clone()).unwrap_or_else(|| name.clone());
    tracing::info!("{name}: {label:?} {w}x{h} at {} Hz {:?} mm -> scale {:.4} ({}/120), {how}", drm_mode.vrefresh(), mm, scale, (scale * 120.0).round());
    state.scale_notes.insert(name.clone(), scales);
    let Some(b) = state.backend.as_mut() else { return };

    let output = Output::new(
        name.clone(),
        PhysicalProperties {
            size: mm.map(|(a, c)| (a as i32, c as i32)).unwrap_or((0, 0)).into(),
            subpixel: Subpixel::Unknown,
            make: "Vergence".into(),
            model: label,
            serial_number: String::new(),
        },
    );
    let global = output.create_global::<Server>(&dh);
    output.user_data().insert_if_missing(|| crate::memory::MonitorId(vergence_profile::monitor_id(&identity)));
    output.set_preferred(wl_mode);
    output.change_current_state(Some(wl_mode), Some(Transform::Normal), Some(Scale::Fractional(scale)), None);

    let planes = b.manager.device().planes(&crtc).ok();
    let drm_output = match b.manager.lock().initialize_output::<_, OutputElements>(
        crtc,
        drm_mode,
        &[conn.handle()],
        &output,
        planes,
        &mut b.renderer,
        &DrmOutputRenderElements::default(),
    ) {
        Ok(o) => o,
        Err(e) => return tracing::warn!("{name}: cannot initialize: {e:?}"),
    };
    b.surfaces.insert(crtc, Surface { output, drm_output, connector: name, last: std::time::Instant::now(), scale, id, px: (w as u32, h as u32), mm, edid: identity, global });
}

/// The EDID blob DRM keeps for a connector.
fn edid_of(dev: &DrmDevice, conn: connector::Handle) -> Option<Vec<u8>> {
    let props = dev.get_properties(conn).ok()?;
    let (handles, values) = props.as_props_and_values();
    handles.iter().zip(values).find_map(|(h, v)| {
        let info = dev.get_property(*h).ok()?;
        if info.name().to_str() != Ok("EDID") {
            return None;
        }
        match info.value_type().convert_value(*v) {
            property::Value::Blob(id) => dev.get_property_blob(id).ok(),
            _ => None,
        }
    })
}

/// Where the arrangement for this setup puts each screen (displays.rs), else
/// left to right in connector order, tops aligned.
fn lay_out(state: &mut Server) {
    let Some(b) = state.backend.as_ref() else { return };
    let order = std::env::var("VERGENCE_LEFT_TO_RIGHT").unwrap_or_else(|_| "HDMI-A-2,HDMI-A-1".into());
    let rank = |c: &str| order.split(',').position(|o| o.trim() == c).unwrap_or(usize::MAX);
    let mut list: Vec<&Surface> = b.surfaces.values().filter(|s| s.output.current_mode().is_some()).collect();
    list.sort_by_key(|s| (rank(&s.connector), s.connector.clone()));
    let sizes: Vec<(String, i32, i32)> = list
        .iter()
        .map(|s| {
            let m = s.output.current_mode().unwrap().size;
            (s.id.clone(), (m.w as f64 / s.scale).round() as i32, (m.h as f64 / s.scale).round() as i32)
        })
        .collect();
    let places = crate::displays::lay_out(&crate::displays::load(), &state.memory.setup, &sizes);
    let mut placed = Vec::new();
    for ((s, (x, y)), (_, w, _)) in list.iter().zip(places).zip(&sizes) {
        s.output.change_current_state(None, None, None, Some((x, y).into()));
        placed.push((s.output.clone(), (x, y)));
        tracing::info!("{} at {x},{y}, {w} points wide", s.connector);
    }
    for (o, p) in placed {
        state.space.map_output(&o, p);
    }
    state.bus_outputs_changed();
}

fn render(state: &mut Server, crtc: crtc::Handle) {
    // The backend is taken out of the state for the frame, so the scene can be
    // built from the state with the backend's renderer.
    let Some(mut backend) = state.backend.take() else { return };
    let t = std::time::Instant::now();
    render_with(state, &mut backend, crtc);
    let ms = t.elapsed().as_millis();
    if ms > 20 {
        tracing::warn!("slow: frame {ms} ms");
    }
    // A frame that blocks for seconds was waiting on a GPU hang (the kernel resets
    // the engine after ~7 s). The reset leaves this GL context drawing nothing but
    // black (on screen) or white (offscreen) for good, while every call succeeds
    // (Hubble, UHD 630, 2026-11-08), and Smithay's context is not robust, so it
    // cannot ask. Start over with a new context; textures are cached per context
    // and upload again.
    if ms > 2000 {
        renew_renderer(&mut backend);
    }
    state.backend = Some(backend);
}

/// Replace the renderer with one on a new EGL context, and draw every monitor in full.
fn renew_renderer(b: &mut Backend) {
    let display = b.renderer.egl_context().display().clone();
    let context = match EGLContext::new(&display) {
        Ok(c) => c,
        Err(e) => return tracing::error!("GPU hang: cannot make a new EGL context: {e:?}"),
    };
    match unsafe { GlesRenderer::new(context) } {
        Ok(r) => {
            b.renderer = r;
            b.redraw_all();
            tracing::warn!("GPU hang suspected (frame blocked); drawing with a new GL context");
        }
        Err(e) => tracing::error!("GPU hang: cannot make a new renderer: {e:?}"),
    }
}

fn render_with(state: &mut Server, b: &mut Backend, crtc: crtc::Handle) {
    // Screens off (energy.rs): nothing is drawn, so no VBlank comes and the
    // redraw chain stops; screens_on starts it again.
    if state.energy.screens_off {
        return;
    }
    let pointer_at = state.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default();
    let Some(s) = b.surfaces.get_mut(&crtc) else { return };
    s.last = std::time::Instant::now();
    let output = s.output.clone();
    let Some(geo) = state.space.output_geometry(&output) else { return };
    let scale = smithay::utils::Scale::from(s.scale);

    let mut elements: Vec<OutputElements> = Vec::new();
    // No pointer is drawn while a finger was the last to point (touch.rs).
    if geo.to_f64().contains(pointer_at) && !state.touch.fingers_last {
        let at = (pointer_at - geo.loc.to_f64()).to_physical(scale);
        let time = state.start_time.elapsed();
        for d in state.pointer_image.elements(&mut b.renderer, at, s.scale, time) {
            elements.push(match d {
                crate::cursor::Drawn::Theme(e) => OutputElements::PointerTheme(e),
                crate::cursor::Drawn::Surface(e) => OutputElements::PointerSurface(e),
            });
        }
    }
    elements.extend(state.scene(&mut b.renderer, &output, s.scale).into_iter().map(OutputElements::Scene));

    // The control socket's `shot`: this monitor's frame, rendered once more offscreen.
    if let Some(i) = state.shot_pending.iter().position(|n| *n == s.connector) {
        state.shot_pending.remove(i);
        let size = output.current_mode().map(|m| m.size).unwrap_or_default();
        match capture(&mut b.renderer, &elements, size, s.scale, state.background_color(&s.connector)) {
            Ok(rgba) => crate::control::save_png(s.connector.clone(), size.w as u32, size.h as u32, rgba),
            Err(e) => tracing::warn!("shot {}: {e}", s.connector),
        }
    }

    let clear = state.background_color(&s.connector);
    match s.drm_output.render_frame(&mut b.renderer, &elements, clear, FrameFlags::DEFAULT) {
        Ok(r) if !r.is_empty => {
            if let Err(e) = s.drm_output.queue_frame(()) {
                tracing::warn!("queue frame: {e:?}");
                let _ = state.loop_handle.insert_source(Timer::from_duration(Duration::from_millis(100)), move |_, _, st| {
                    render(st, crtc);
                    TimeoutAction::Drop
                });
            }
        }
        Ok(_) => {
            // Nothing changed: look again in a frame's time (no VBlank comes without a frame).
            let _ = state.loop_handle.insert_source(Timer::from_duration(Duration::from_millis(16)), move |_, _, st| {
                render(st, crtc);
                TimeoutAction::Drop
            });
        }
        Err(e) => {
            // Try again shortly: without a frame no VBlank comes, and drawing
            // would stop for good.
            tracing::warn!("render: {e:?}");
            let _ = state.loop_handle.insert_source(Timer::from_duration(Duration::from_millis(100)), move |_, _, st| {
                render(st, crtc);
                TimeoutAction::Drop
            });
        }
    }

    // Each window draws at the scale of the output most of it is on; frame callbacks.
    let start = state.start_time.elapsed();
    let outputs: Vec<(Output, f64)> = b.surfaces.values().map(|s| (s.output.clone(), s.scale)).collect();
    for window in state.space.elements() {
        let Some(wgeo) = state.space.element_geometry(window) else { continue };
        let main = outputs
            .iter()
            .filter_map(|(o, sc)| {
                let og = state.space.output_geometry(o)?;
                let i = og.intersection(wgeo)?;
                Some((i.size.w as i64 * i.size.h as i64, o, *sc))
            })
            .max_by_key(|(a, _, _)| *a);
        if let Some((_, o, sc)) = main {
            window.with_surfaces(|_, states| with_fractional_scale(states, |fs| fs.set_preferred_scale(sc)));
            if *o == output {
                window.send_frame(o, start, Some(Duration::ZERO), |_, _| Some(o.clone()));
            }
        }
    }
    // A client's own cursor surface (animated ones wait for this).
    if let smithay::input::pointer::CursorImageStatus::Surface(cs) = &state.pointer_image.status
        && geo.to_f64().contains(pointer_at)
    {
        smithay::desktop::utils::send_frames_surface_tree(cs, &output, start, Some(Duration::ZERO), |_, _| {
            Some(output.clone())
        });
    }
    state.space.refresh();
    state.popups.cleanup();
    let _ = state.display_handle.flush_clients();
}

/// Render `elements` into an offscreen buffer the size of the monitor and read it back
/// (R, G, B, A bytes, top row first).
fn capture(
    renderer: &mut GlesRenderer,
    elements: &[OutputElements],
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
    scale: f64,
    clear: [f32; 4],
) -> Result<Vec<u8>, String> {
    use smithay::backend::renderer::{Bind, ExportMem, Offscreen, damage::OutputDamageTracker, gles::GlesTexture};
    let buf = smithay::utils::Size::<i32, smithay::utils::Buffer>::from((size.w, size.h));
    let mut tex: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, buf).map_err(|e| e.to_string())?;
    let mut fb = renderer.bind(&mut tex).map_err(|e| e.to_string())?;
    let mut damage = OutputDamageTracker::new(size, scale, Transform::Normal);
    damage.render_output(renderer, &mut fb, 0, elements, clear).map_err(|e| format!("{e:?}"))?;
    let mapping = renderer
        .copy_framebuffer(&fb, smithay::utils::Rectangle::from_size(buf), Fourcc::Abgr8888)
        .map_err(|e| e.to_string())?;
    let bytes = renderer.map_texture(&mapping).map_err(|e| e.to_string())?;
    Ok(bytes.to_vec())
}

/// Energy (energy.rs): every monitor off. The CRTC is disabled (DPMS off) and its
/// planes cleared; the monitor shows nothing and the GPU idles until screens_on.
pub fn screens_off(state: &mut Server) {
    let Some(b) = state.backend.as_mut() else { return };
    for s in b.surfaces.values_mut() {
        if let Err(e) = s.drm_output.with_compositor(|c| c.clear()) {
            tracing::warn!("{}: cannot turn off: {e:?}", s.connector);
        }
    }
}

/// Energy (energy.rs): every monitor back on, each with a full frame (the first
/// queued frame after a clear enables the CRTC again).
pub fn screens_on(state: &mut Server) {
    let crtcs: Vec<crtc::Handle> = state
        .backend
        .as_mut()
        .map(|b| {
            b.redraw_all();
            b.surfaces.keys().copied().collect()
        })
        .unwrap_or_default();
    for c in crtcs {
        render(state, c);
    }
}

impl Backend {
    /// Draw every monitor in full on its next frame (wallpaper or settings changed).
    pub fn redraw_all(&mut self) {
        for s in self.surfaces.values_mut() {
            s.drm_output.reset_buffers();
        }
    }

    /// Can the renderer use this app buffer? (linux-dmabuf import check)
    pub fn import_dmabuf(&mut self, dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf) -> bool {
        self.renderer.import_dmabuf(dmabuf, None).is_ok()
    }
}

impl Server {
    fn shm_formats_from(&mut self, renderer: &GlesRenderer) {
        self.shm_state.update_formats(renderer.shm_formats());
        let _ = renderer.dmabuf_formats();
    }
}

#[allow(dead_code)]
fn _unused(_: &Path) {}
