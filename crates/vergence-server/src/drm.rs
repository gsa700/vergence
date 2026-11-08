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
use vergence_profile::scaling::{Distance, derive};

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
    scale: f64,
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
        .insert_source(LibinputInputBackend::new(libinput), |event, _, state| state.process_input_event(event))?;

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

    // udev keeps the device list; the monitors come from a connector scan.
    let _udev = UdevBackend::new(&seat)?;
    scan(state);
    let crtcs: Vec<crtc::Handle> = state.backend.as_ref().map(|b| b.surfaces.keys().copied().collect()).unwrap_or_default();
    for c in crtcs {
        render(state, c);
    }
    Ok(())
}

/// The connected monitors: an output each, at its EDID's scale, laid out left to right.
fn scan(state: &mut Server) {
    let Some(b) = state.backend.as_mut() else { return };
    let events = match b.scanner.scan_connectors(b.manager.device()) {
        Ok(e) => e,
        Err(e) => return tracing::warn!("connector scan: {e:?}"),
    };
    for event in events {
        if let DrmScanEvent::Connected { connector, crtc: Some(crtc) } = event {
            connect(state, connector, crtc);
        }
    }
    lay_out(state);
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
    let derived = derive((w as u32, h as u32), mm, Distance::Desktop, 1.0);
    // In 120ths, as wp-fractional-scale-v1 tells it to apps: the output at exactly the
    // scale the apps draw at, so their buffers are shown 1:1, not resampled (the ASUS's
    // 1.0703 drawn at 128/120 and shown at 1.0703 was soft, 2026-11-07).
    let scale = crate::scale_in_120ths(derived.exact);
    let label = parsed.as_ref().and_then(|e| e.name.clone()).unwrap_or_else(|| name.clone());
    tracing::info!("{name}: {label:?} {w}x{h} {:?} mm -> scale {:.4} ({}/120, exactly {:.4})", mm, scale, (scale * 120.0).round(), derived.exact);

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
    let _global = output.create_global::<Server>(&dh);
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
        Err(e) => return tracing::warn!("{name}: cannot initialise: {e:?}"),
    };
    b.surfaces.insert(crtc, Surface { output, drm_output, connector: name, scale });
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

/// Left to right, side by side in logical space, tops aligned.
fn lay_out(state: &mut Server) {
    let Some(b) = state.backend.as_ref() else { return };
    let order = std::env::var("VERGENCE_LEFT_TO_RIGHT").unwrap_or_else(|_| "HDMI-A-2,HDMI-A-1".into());
    let rank = |c: &str| order.split(',').position(|o| o.trim() == c).unwrap_or(usize::MAX);
    let mut list: Vec<&Surface> = b.surfaces.values().collect();
    list.sort_by_key(|s| (rank(&s.connector), s.connector.clone()));
    let mut x = 0.0_f64;
    let mut placed = Vec::new();
    for s in list {
        let Some(mode) = s.output.current_mode() else { continue };
        s.output.change_current_state(None, None, None, Some((x.round() as i32, 0).into()));
        placed.push((s.output.clone(), (x.round() as i32, 0)));
        tracing::info!("{} at x={}, {:.0} points wide", s.connector, x.round(), mode.size.w as f64 / s.scale);
        x += mode.size.w as f64 / s.scale;
    }
    for (o, p) in placed {
        state.space.map_output(&o, p);
    }
}

fn render(state: &mut Server, crtc: crtc::Handle) {
    // The backend is taken out of the state for the frame, so the scene can be
    // built from the state with the backend's renderer.
    let Some(mut backend) = state.backend.take() else { return };
    render_with(state, &mut backend, crtc);
    state.backend = Some(backend);
}

fn render_with(state: &mut Server, b: &mut Backend, crtc: crtc::Handle) {
    let pointer_at = state.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default();
    let Some(s) = b.surfaces.get_mut(&crtc) else { return };
    let output = s.output.clone();
    let Some(geo) = state.space.output_geometry(&output) else { return };
    let scale = smithay::utils::Scale::from(s.scale);

    let mut elements: Vec<OutputElements> = Vec::new();
    if geo.to_f64().contains(pointer_at) {
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
        match capture(&mut b.renderer, &elements, size, s.scale) {
            Ok(rgba) => crate::control::save_png(s.connector.clone(), size.w as u32, size.h as u32, rgba),
            Err(e) => tracing::warn!("shot {}: {e}", s.connector),
        }
    }

    match s.drm_output.render_frame(&mut b.renderer, &elements, [0.1, 0.1, 0.1, 1.0], FrameFlags::DEFAULT) {
        Ok(r) if !r.is_empty => {
            if let Err(e) = s.drm_output.queue_frame(()) {
                tracing::warn!("queue frame: {e:?}");
            }
        }
        Ok(_) => {
            // Nothing changed: look again in a frame's time (no VBlank comes without a frame).
            let _ = state.loop_handle.insert_source(Timer::from_duration(Duration::from_millis(16)), move |_, _, st| {
                render(st, crtc);
                TimeoutAction::Drop
            });
        }
        Err(e) => tracing::warn!("render: {e:?}"),
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
) -> Result<Vec<u8>, String> {
    use smithay::backend::renderer::{Bind, ExportMem, Offscreen, damage::OutputDamageTracker, gles::GlesTexture};
    let buf = smithay::utils::Size::<i32, smithay::utils::Buffer>::from((size.w, size.h));
    let mut tex: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, buf).map_err(|e| e.to_string())?;
    let mut fb = renderer.bind(&mut tex).map_err(|e| e.to_string())?;
    let mut damage = OutputDamageTracker::new(size, scale, Transform::Normal);
    damage.render_output(renderer, &mut fb, 0, elements, [0.1, 0.1, 0.1, 1.0]).map_err(|e| format!("{e:?}"))?;
    let mapping = renderer
        .copy_framebuffer(&fb, smithay::utils::Rectangle::from_size(buf), Fourcc::Abgr8888)
        .map_err(|e| e.to_string())?;
    let bytes = renderer.map_texture(&mapping).map_err(|e| e.to_string())?;
    Ok(bytes.to_vec())
}

impl Server {
    fn shm_formats_from(&mut self, renderer: &GlesRenderer) {
        self.shm_state.update_formats(renderer.shm_formats());
        let _ = renderer.dmabuf_formats();
    }
}

#[allow(dead_code)]
fn _unused(_: &Path) {}
