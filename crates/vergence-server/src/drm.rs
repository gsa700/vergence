// Based on Smithay's anvil udev backend (MIT; see LICENSE-smithay.txt), cut down for
// Vergence: server milestone 1, stage 2 (the real monitors).
//
// Every GPU with connectors, every connected monitor, libinput, a libseat session
// (seatd on FreeBSD). The seat's primary GPU draws every screen; a screen on another
// GPU (Hubble's HDMI, on the NVIDIA card) is drawn by the primary into a linear
// buffer allocated on the GPU that shows it (GPUs milestone 1, step 3: "drawn here,
// shown there"; both GPUs render and import linear XRGB, gpus.rs).
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
            element::{memory::MemoryRenderBufferRenderElement, surface::WaylandSurfaceRenderElement},
            gles::GlesRenderer,
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{UdevBackend, all_gpus, primary_gpu},
    },
    output::{Mode as WlMode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        calloop::{
            EventLoop,
            timer::{TimeoutAction, Timer},
        },
        drm::control::{Device as ControlDevice, ModeTypeFlags, connector, crtc, property},
        input::Libinput,
        rustix::fs::OFlags,
    },
    utils::{DeviceFd, Logical, Point, Rectangle, Transform},
    wayland::fractional_scale::with_fractional_scale,
};
use smithay_drm_extras::drm_scanner::{DrmScanEvent, DrmScanner};

use crate::Server;

smithay::backend::renderer::element::render_elements! {
    /// A screen on another GPU: the frame the primary drew, as a texture there.
    pub RelayElements<=GlesRenderer>;
    Texture = smithay::backend::renderer::element::texture::TextureRenderElement<smithay::backend::renderer::gles::GlesTexture>,
}

smithay::backend::renderer::element::render_elements! {
    pub OutputElements<=GlesRenderer>;
    Scene = crate::decor::SceneElement,
    PointerTheme = MemoryRenderBufferRenderElement<GlesRenderer>,
    PointerSurface = WaylandSurfaceRenderElement<GlesRenderer>,
    /// Through the magnifier (zoom.rs).
    Zoomed = crate::zoom::Zoomed,
}

/// A screen's elements, front first: the pointer (unless a finger pointed
/// last, touch.rs) and the scene.
fn screen_elements(
    state: &mut Server,
    renderer: &mut GlesRenderer,
    output: &Output,
    geo: Rectangle<i32, Logical>,
    scale: f64,
    pointer_at: Point<f64, Logical>,
) -> Vec<crate::zoom::Inner> {
    let mut elements = Vec::new();
    // Behind Vergence Remote's curtain the desk's monitors draw no pointer: it
    // is the remote person's, moving, and the desk would see where they work.
    let curtained = state.lock.curtain && !state.remote.drawing;
    if geo.to_f64().contains(pointer_at) && !state.touch.fingers_last && !curtained {
        let at = (pointer_at - geo.loc.to_f64()).to_physical(smithay::utils::Scale::from(scale));
        let time = state.start_time.elapsed();
        for d in state.pointer_image.elements(renderer, at, scale, time) {
            elements.push(match d {
                crate::cursor::Drawn::Theme(e) => crate::zoom::Inner::PointerTheme(e),
                crate::cursor::Drawn::Surface(e) => crate::zoom::Inner::PointerSurface(e),
            });
        }
    }
    // Super+Shift+4's picker, over everything but the pointer, and the last
    // screenshot's thumbnail (capture.rs).
    elements.extend(state.capture_pick_element(renderer, geo, scale).map(crate::zoom::Inner::Scene));
    elements.extend(state.shot_element(renderer, geo, scale).map(crate::zoom::Inner::Scene));
    elements.extend(state.bar_element(renderer, geo, scale).map(crate::zoom::Inner::Scene));
    elements.extend(state.scene(renderer, output, scale).into_iter().map(crate::zoom::Inner::Scene));
    elements
}

type Manager = DrmOutputManager<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;
type Out = DrmOutput<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

/// A screen: which GPU shows it (an index into Backend::devices) and its CRTC
/// there (CRTC handles are per GPU).
type Key = (usize, crtc::Handle);

/// A GPU that shows screens: its outputs and its connector scanner.
struct Device {
    manager: Manager,
    scanner: DrmScanner,
    /// A GPU other than the primary: its own renderer, which copies the frame
    /// the primary drew into its own scan-out buffer (nouveau scans out of its
    /// video memory only, and the Intel GPU cannot draw there: EGL BAD_ALLOC).
    renderer: Option<GlesRenderer>,
    /// Its device node's path, for the log.
    path: String,
}

struct Surface {
    output: Output,
    /// How its frames get out: to a monitor's connector, or (headless
    /// milestone 1) nowhere but memory.
    present: Present,
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
    /// On another GPU: the buffers the primary draws into (system memory,
    /// linear), and the hidden scale-1 output the copy is shown through.
    relay: Option<Relay>,
    /// The monitor's modes (each once) and the one running, for the bus.
    modes: Vec<vergence_protocol::bus::ModeInfo>,
    mode: u32,
    /// When it was connected, in order: screens a setup has no places for
    /// are laid out in this order, so a new one goes on the right.
    since: u64,
}

/// How a screen's frames get out (headless milestone 1, step 1).
enum Present {
    /// A monitor: Smithay's DRM output on its CRTC, and its connector (to set
    /// it up again in another mode).
    Drm { out: Out, handle: connector::Handle },
    /// A screen in memory (`vctl screen add`, headless milestone 1, step 2):
    /// drawn into an offscreen texture of the primary's renderer (no GBM, so
    /// a machine with no GPU has them too), at its refresh by a timer.
    Memory(MemoryScreen),
}

struct MemoryScreen {
    tex: smithay::backend::renderer::gles::GlesTexture,
    damage: smithay::backend::renderer::damage::OutputDamageTracker,
    /// A frame's time, standing in for the monitor's VBlank.
    interval: Duration,
}

/// The "GPU" of screens in memory, in their Key (no CRTC: a number of ours).
const MEMORY: usize = usize::MAX;
static MEMORY_SCREENS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

impl Surface {
    /// The monitor's DRM output; None for a screen that is not a monitor.
    fn drm(&mut self) -> Option<&mut Out> {
        match &mut self.present {
            Present::Drm { out, .. } => Some(out),
            Present::Memory(_) => None,
        }
    }

    fn handle(&self) -> Option<connector::Handle> {
        match &self.present {
            Present::Drm { handle, .. } => Some(*handle),
            Present::Memory(_) => None,
        }
    }

    /// The frame queued last is on the screen (its VBlank, or the watchdog).
    fn frame_submitted(&mut self) {
        if let Some(out) = self.drm() {
            let _ = out.frame_submitted();
        }
    }

    /// Draw it in full next time.
    fn reset_buffers(&mut self) {
        if let Some(out) = self.drm() {
            out.reset_buffers();
        }
    }
}

/// The order screens were connected in (Surface::since).
static CONNECTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// "Drawn here, shown there" (GPUs milestone 1, step 3): the primary draws
/// the screen into one linear buffer in its own memory, and the showing GPU
/// takes it in as a texture and copies **only what changed** into its own
/// scan-out buffers (its compositor's damage tracking, fed the primary's
/// damage). A full 4K copy takes nouveau 60 to 77 ms (reading system memory
/// over PCIe), so copying whole frames could not keep up and the screen
/// flipped to half-copied ones. The two steps take
/// turns on the one buffer: the primary draws, the showing GPU copies once the
/// primary's fence has signaled (checked, never waited on), and finishes its
/// copy before the flip and before the primary draws again.
struct Relay {
    buf: RelayBuf,
    /// The buffer as a texture on the showing GPU, with the damage of each
    /// draw recorded on it (imported at the first copy).
    shown: Option<
        smithay::backend::renderer::element::texture::TextureRenderBuffer<
            smithay::backend::renderer::gles::GlesTexture,
        >,
    >,
    /// Calls to the damage tracker so far, drawn or not (its history has one
    /// entry for each; the buffer's age is counted in them).
    frame: usize,
    /// A frame is drawn and not copied yet.
    pending: bool,
    /// The area the last draw changed, in pixels (for the log).
    last_damage_px: i64,
    damage: smithay::backend::renderer::damage::OutputDamageTracker,
    /// The output the showing GPU's frame goes through: the screen's mode at scale 1.
    _plain: Output,
}

struct RelayBuf {
    dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
    /// The call it was last drawn in (0: never).
    drawn: usize,
    /// Its drawing's fence, until copied.
    sync: Option<smithay::backend::renderer::sync::SyncPoint>,
}

/// What a relay turn did.
enum Relayed {
    /// A frame went to the screen (a VBlank follows).
    Shown,
    /// Nothing changed.
    Nothing,
    /// A frame is drawn but its fence has not signaled: look again soon.
    Waiting,
}

pub struct Backend {
    renderer: GlesRenderer,
    /// The primary GPU's buffers, for the frames of screens on other GPUs
    /// (None headless: no monitors, so no relays).
    gbm: Option<GbmDevice<DrmDeviceFd>>,
    /// `devices[0]` is the primary GPU, the one that draws.
    devices: Vec<Device>,
    surfaces: HashMap<Key, Surface>,
    /// The seat is another console's (a VT switch away, the login screen):
    /// nothing is drawn until it comes back (drawing anyway failed ten frames
    /// a second, 10,000 log lines, while the login screen had the seat).
    paused: bool,
    /// Monitors that came back while the screens were off (energy.rs), set up by
    /// screens_on. Setting one up turns it on, and a monitor that drops off in
    /// standby (some do) then woke and dropped again.
    waiting: Vec<(usize, connector::Handle, crtc::Handle)>,
    /// Tells the main loop that a thread has woken another GPU and probed its
    /// connectors (screens_on), so its scan is quick.
    probed: smithay::reexports::calloop::channel::Sender<usize>,
    /// Remote views (remote milestone 1, step 1): screens drawn again at a
    /// viewer's size and scale, for Vergence Remote.
    remotes: Vec<RemoteView>,
    /// Page flips refused in a row (any screen): past FLIPS_REFUSED, the
    /// displays are set up again (reset_displays).
    refused: u32,
}

/// Page flips refused in a row before the displays are set up again.
const FLIPS_REFUSED: u32 = 5;

pub fn run(event_loop: &mut EventLoop<Server>, state: &mut Server) -> Result<(), Box<dyn std::error::Error>> {
    let (session, notifier) = LibSeatSession::new()?;
    let seat = session.seat();
    tracing::info!("seat {seat}");

    let path = primary_gpu(&seat)?.or_else(|| all_gpus(&seat).ok()?.into_iter().next()).ok_or("no GPU")?;
    let node = DrmNode::from_path(&path)?;
    tracing::info!("GPU {} ({:?}, render node {:?})", path.display(), node.ty(), node.node_with_type(NodeType::Render));

    let mut session_handle = session.clone();
    let fd = session_handle.open(&path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true)?;
    let gbm = GbmDevice::new(fd)?;

    let display = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new(&display)?;
    let mut renderer = unsafe { GlesRenderer::new(context)? };
    let name = gl_renderer(&mut renderer);
    tracing::info!("gpu: {} draws with {name}", path.display());
    let render_formats: FormatSet = renderer.egl_context().dmabuf_render_formats().iter().copied().collect();
    // Every GPU on the seat, probed and logged (GPUs milestone 1, step 1).
    let formats = crate::gpus::formats_of(renderer.egl_context());
    state.gpus = crate::gpus::probe(&mut session_handle, &seat, &path, drm.device_fd(), &formats);
    state.shm_formats_from(&renderer);

    // GPU buffers from apps (linux-dmabuf v4): Xwayland's glamor and GL apps draw on
    // the GPU and hand their buffers over without a copy. Without it Xwayland fell
    // back to software and AlbumWall managed ~4 frames a second at 3.28.
    // The feedback names the render node: Mesa opens the device by its number.
    let render_node = node.node_with_type(NodeType::Render).and_then(|r| r.ok()).unwrap_or(node);
    match smithay::wayland::dmabuf::DmabufFeedbackBuilder::new(render_node.dev_id(), renderer.dmabuf_formats()).build()
    {
        Ok(feedback) => {
            let mut dmabuf_state = smithay::wayland::dmabuf::DmabufState::new();
            let global = dmabuf_state.create_global_with_default_feedback::<Server>(&state.display_handle, &feedback);
            state.dmabuf = Some((dmabuf_state, global));
            tracing::info!("linux-dmabuf on {render_node:?}");
        }
        Err(e) => tracing::warn!("linux-dmabuf: {e:?}"),
    }

    let primary_gbm = gbm.clone();
    let manager = DrmOutputManager::new(
        drm,
        GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT),
        GbmFramebufferExporter::new(gbm.clone(), Some(node).into()),
        Some(gbm),
        [Fourcc::Abgr8888, Fourcc::Argb8888],
        render_formats.clone(),
    );

    let mut devices =
        vec![Device { manager, scanner: DrmScanner::new(), renderer: None, path: path.display().to_string() }];
    // The other GPUs with connectors: the primary draws their screens into
    // linear buffers in its own memory, and each copies the frame into its
    // own scan-out buffer with its own renderer (step 3). Their VBlank sources below.
    let mut notifiers = Vec::new();
    let others: Vec<String> =
        state.gpus.iter().filter(|g| !g.primary && !g.connectors.is_empty()).map(|g| g.path.clone()).collect();
    for other in others {
        let opened = (|| -> Result<(Device, smithay::backend::drm::DrmDeviceNotifier), Box<dyn std::error::Error>> {
            let p = Path::new(&other);
            let fd = session_handle.open(p, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)?;
            let fd = DrmDeviceFd::new(DeviceFd::from(fd));
            let (drm, notifier) = DrmDevice::new(fd.clone(), true)?;
            let gbm = GbmDevice::new(fd)?;
            let node = DrmNode::from_path(p)?;
            let display = unsafe { EGLDisplay::new(gbm.clone())? };
            let context = EGLContext::new(&display)?;
            let mut copier = unsafe { GlesRenderer::new(context)? };
            // Which driver really draws there: Mesa falls back to software
            // (llvmpipe) when the GPU's own GL driver is missing. On an NVIDIA
            // card Mesa 26 runs nouveau's GL through Zink, which needs NVK, and
            // without it the "GPU copy" of a screen's frames was the CPU's
            // (40 to 77 ms a 4K frame).
            let name = gl_renderer(&mut copier);
            if is_software(&name) {
                tracing::warn!(
                    "gpu: {other}: its GL is {name}, software on the CPU: screens on it will be slow. Its own GL driver is missing (nouveau: Mesa's Vulkan driver NVK and the Vulkan loader, for Zink)"
                );
            } else {
                tracing::info!("gpu: {other} copies with {name}");
            }
            let formats: FormatSet = copier.egl_context().dmabuf_render_formats().iter().copied().collect();
            let manager = DrmOutputManager::new(
                drm,
                GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT),
                GbmFramebufferExporter::new(gbm.clone(), Some(node).into()),
                Some(gbm),
                [Fourcc::Xrgb8888, Fourcc::Argb8888],
                formats,
            );
            Ok((Device { manager, scanner: DrmScanner::new(), renderer: Some(copier), path: other.clone() }, notifier))
        })();
        match opened {
            Ok((dev, notifier)) => {
                tracing::info!(
                    "gpu: {other} shows screens: drawn by {}, copied there by its own renderer",
                    path.display()
                );
                notifiers.push((devices.len(), notifier));
                devices.push(dev);
            }
            Err(e) => tracing::warn!("gpu: {other}: cannot show screens on it: {e}"),
        }
    }
    // The other GPUs' connectors after a thread woke them (screens_on): scanned
    // here, now quickly, as the card is awake.
    let (probed, probed_rx) = smithay::reexports::calloop::channel::channel::<usize>();
    event_loop.handle().insert_source(probed_rx, |event, _, state| {
        if let smithay::reexports::calloop::channel::Event::Msg(gpu) = event {
            let anchors = state.window_anchors();
            if scan_gpu(state, gpu, false) {
                setup_changed(state, anchors);
            }
        }
    })?;
    state.backend = Some(Backend {
        renderer,
        gbm: Some(primary_gbm),
        devices,
        surfaces: HashMap::new(),
        paused: false,
        waiting: Vec::new(),
        probed,
        remotes: Vec::new(),
        refused: 0,
    });
    state.session = Some(session.clone());

    // Input from every device on the seat.
    let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput.udev_assign_seat(&seat).map_err(|_| "libinput: cannot assign seat")?;
    let libinput_for_session = libinput.clone();
    event_loop.handle().insert_source(LibinputInputBackend::new(libinput), |event, _, state| {
        // Mice and touchpads are kept, to be given their settings (pointer.rs).
        match &event {
            smithay::backend::input::InputEvent::DeviceAdded { device } => state.pointer_device_added(device.clone()),
            smithay::backend::input::InputEvent::DeviceRemoved { device } => state.pointer_device_removed(device),
            _ => {}
        }
        state.process_input_event(event)
    })?;

    // VBlank: the frame is on screen; draw the next one. One source per GPU.
    for (gpu, notifier) in std::iter::once((0usize, drm_notifier)).chain(notifiers) {
        event_loop.handle().insert_source(notifier, move |event, _, state| match event {
            DrmEvent::VBlank(crtc) => {
                let key = (gpu, crtc);
                let mut shown = None;
                if let Some(b) = state.backend.as_mut()
                    && let Some(s) = b.surfaces.get_mut(&key)
                {
                    s.frame_submitted();
                    shown = Some((s.connector.clone(), b.surfaces.len()));
                }
                // The lock's frame is on this screen now (lock.rs, before sleep).
                if let Some((name, screens)) = shown {
                    state.lock_frame_shown(&name, screens);
                }
                render(state, key);
            }
            DrmEvent::Error(e) => tracing::error!("drm (gpu {gpu}): {e:?}"),
        })?;
    }

    watchdog(event_loop)?;

    // Console switches.
    let mut libinput = libinput_for_session;
    event_loop.handle().insert_source(notifier, move |event, _, state| match event {
        SessionEvent::PauseSession => {
            tracing::info!("session paused");
            libinput.suspend();
            if let Some(b) = state.backend.as_mut() {
                b.paused = true;
                for d in &mut b.devices {
                    d.manager.pause();
                }
            }
        }
        SessionEvent::ActivateSession => {
            tracing::info!("session active");
            // Away past the idle time: locked before anything is drawn (lock.rs).
            state.lock_if_idle();
            let _ = libinput.resume();
            let crtcs: Vec<Key> = state
                .backend
                .as_mut()
                .map(|b| {
                    b.paused = false;
                    for d in &mut b.devices {
                        let _ = d.manager.lock().activate(false);
                    }
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
    // the server blocked; polling every 2 s froze dragged windows). The
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
    let crtcs: Vec<Key> = state.backend.as_ref().map(|b| b.surfaces.keys().copied().collect()).unwrap_or_default();
    for c in crtcs {
        render(state, c);
    }
    Ok(())
}

/// The connected monitors: an output each, at its EDID's scale, laid out left to right.
/// At start-up every monitor at once; afterwards (hotplug) a new monitor is set up a
/// second later from a fresh forced probe: FreeBSD's first look after a hotplug can
/// see only the kernel's fallback modes (800x600).
fn scan(state: &mut Server, startup: bool) {
    let count = state.backend.as_ref().map_or(0, |b| b.devices.len());
    let anchors = state.window_anchors();
    let mut changed = false;
    for gpu in 0..count {
        changed |= scan_gpu(state, gpu, startup);
    }
    if changed {
        setup_changed(state, anchors);
    }
}

/// One GPU's connectors; true if screens came or went at once.
fn scan_gpu(state: &mut Server, gpu: usize, startup: bool) -> bool {
    let Some(b) = state.backend.as_mut() else { return false };
    let d = &mut b.devices[gpu];
    let events = match d.scanner.scan_connectors(d.manager.device()) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("connector scan ({}): {e:?}", d.path);
            return false;
        }
    };
    let mut changed = false;
    for event in events {
        match event {
            DrmScanEvent::Connected { connector, crtc: Some(crtc) } if startup => {
                connect(state, gpu, connector, crtc);
                changed = true;
            }
            DrmScanEvent::Connected { connector, crtc: Some(crtc) } => {
                let handle = connector.handle();
                // While the screens are off, a monitor in standby may come and go every
                // few seconds (one did every 10 s, all night): debug only.
                if state.energy.screens_off {
                    tracing::debug!(
                        "hotplug: {}-{} connected",
                        connector.interface().as_str(),
                        connector.interface_id()
                    );
                } else {
                    tracing::info!(
                        "hotplug: {}-{} connected; setting up in 1 s",
                        connector.interface().as_str(),
                        connector.interface_id()
                    );
                }
                let _ =
                    state.loop_handle.insert_source(Timer::from_duration(Duration::from_secs(1)), move |_, _, st| {
                        if st.energy.screens_off {
                            if let Some(b) = st.backend.as_mut() {
                                if !b.waiting.contains(&(gpu, handle, crtc)) {
                                    tracing::info!("hotplug: screens are off; setting it up when they come on");
                                    b.waiting.push((gpu, handle, crtc));
                                }
                            }
                            return TimeoutAction::Drop;
                        }
                        let anchors = st.window_anchors();
                        if connect_if_there(st, gpu, handle, crtc) {
                            setup_changed(st, anchors);
                            render(st, (gpu, crtc));
                        }
                        TimeoutAction::Drop
                    });
            }
            DrmScanEvent::Disconnected { connector, crtc: Some(crtc) } => {
                if state.energy.screens_off {
                    tracing::debug!(
                        "hotplug: {}-{} disconnected",
                        connector.interface().as_str(),
                        connector.interface_id()
                    );
                } else {
                    tracing::info!(
                        "hotplug: {}-{} disconnected",
                        connector.interface().as_str(),
                        connector.interface_id()
                    );
                }
                disconnect(state, (gpu, crtc));
                changed = true;
            }
            DrmScanEvent::Changed { connector, crtc: Some(crtc) } => {
                // New modes on a monitor that stayed connected (a late probe): set it up
                // again, but only if the mode we would pick differs from the one running.
                // (The scanner reports the ASUS as changed on every scan, mode list
                // unchanged in substance; reconnecting each time blanked it.)
                let want = connector
                    .modes()
                    .iter()
                    .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
                    .or(connector.modes().first())
                    .map(|m| m.size());
                let running = state
                    .backend
                    .as_ref()
                    .and_then(|b| b.surfaces.get(&(gpu, crtc)))
                    .and_then(|s| s.output.current_mode())
                    .map(|m| (m.size.w as u16, m.size.h as u16));
                if want.is_none() || want == running {
                    continue;
                }
                tracing::info!(
                    "hotplug: {}-{} modes changed ({running:?} -> {want:?})",
                    connector.interface().as_str(),
                    connector.interface_id()
                );
                disconnect(state, (gpu, crtc));
                changed = true;
                if state.energy.screens_off {
                    if let Some(b) = state.backend.as_mut() {
                        b.waiting.push((gpu, connector.handle(), crtc));
                    }
                    continue;
                }
                connect(state, gpu, connector, crtc);
                render(state, (gpu, crtc));
            }
            _ => {}
        }
    }
    changed
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
mod spec_tests {
    #[test]
    fn software_renderers_are_known() {
        assert!(super::is_software("llvmpipe (LLVM 22.1.8, 256 bits)"));
        assert!(!super::is_software("Mesa Intel(R) UHD Graphics (CML GT2)"));
        assert!(!super::is_software("zink Vulkan 1.4(NVIDIA GeForce GTX 1650 (NVK TU117))"));
    }

    #[test]
    fn devd_drm_hotplug() {
        assert!(super::is_drm_hotplug("!system=DRM subsystem=CONNECTOR type=HOTPLUG cdev=dri/card0 connector=115"));
        assert!(!super::is_drm_hotplug("!system=USB subsystem=DEVICE type=ATTACH"));
        assert!(!super::is_drm_hotplug("+uhid0 at bus=0"));
    }
}

/// The monitor at `crtc` is gone: its output leaves the desktop and the clients.
/// Set up a monitor the scanner reported, if it is still connected.
fn connect_if_there(state: &mut Server, gpu: usize, handle: connector::Handle, crtc: crtc::Handle) -> bool {
    let info =
        state.backend.as_ref().and_then(|b| b.devices.get(gpu)?.manager.device().get_connector(handle, true).ok());
    match info.filter(|i| i.state() == connector::State::Connected) {
        Some(info) => {
            connect(state, gpu, info, crtc);
            true
        }
        None => false,
    }
}

fn disconnect(state: &mut Server, key: Key) {
    let Some(b) = state.backend.as_mut() else { return };
    let Some(s) = b.surfaces.remove(&key) else { return };
    state.space.unmap_output(&s.output);
    state.display_handle.remove_global::<Server>(s.global);
    tracing::info!("{} removed", s.connector);
    state.bus_outputs_changed();
}

/// After monitors came or went: lay them out, find the setup, put windows back.
fn setup_changed(
    state: &mut Server,
    anchors: Vec<(crate::win::Win, String, smithay::utils::Point<i32, smithay::utils::Logical>)>,
) {
    // The setup first: the arrangement is kept per setup.
    let edids: Vec<Vec<u8>> =
        state.backend.as_ref().map(|b| b.surfaces.values().map(|s| s.edid.clone()).collect()).unwrap_or_default();
    state.set_setup(&edids);
    // A setup with a screen in memory is a test's: never saved (memory.rs).
    if state.backend.as_ref().is_some_and(|b| b.surfaces.keys().any(|k| k.0 == MEMORY)) {
        state.memory.mark_test_setup();
    }
    lay_out(state);
    state.rearrange_windows(anchors);
}

/// desktop.toml's `[displays]` changed (the settings app): work every scale out
/// again and lay the screens out again; windows go with their screens.
pub fn displays_changed(state: &mut Server) {
    let mut cfg = crate::displays::load();
    // Test scales (vctl scale) over what is chosen, never saved.
    if let Some(b) = state.backend.as_ref() {
        for s in b.surfaces.values() {
            if let Some(v) = state.scale_test.get(&s.connector) {
                cfg.set_scale(&s.id, *v);
            }
        }
    }
    let anchors = state.window_anchors();
    // A screen whose chosen mode is not the one running (or that lost its
    // choice) is set up again in it (settings milestone 2).
    let redo: Vec<(Key, connector::Handle)> = state
        .backend
        .as_ref()
        .map(|b| {
            b.surfaces
                .iter()
                .filter(|(_, s)| {
                    let running = s.modes.get(s.mode as usize).map(|m| (m.width, m.height, m.refresh_mhz));
                    let preferred = s
                        .modes
                        .iter()
                        .find(|m| m.preferred)
                        .or(s.modes.first())
                        .map(|m| (m.width, m.height, m.refresh_mhz));
                    let want = cfg
                        .mode_for(&s.id)
                        .filter(|w| {
                            s.modes.iter().any(|m| {
                                m.width == w.0 && m.height == w.1 && (m.refresh_mhz as i64 - w.2 as i64).abs() <= 50
                            })
                        })
                        .or(preferred);
                    match (want, running) {
                        (Some(w), Some(r)) => !(w.0 == r.0 && w.1 == r.1 && (w.2 as i64 - r.2 as i64).abs() <= 50),
                        _ => false,
                    }
                })
                .filter_map(|(k, s)| Some((*k, s.handle()?)))
                .collect()
        })
        .unwrap_or_default();
    if !redo.is_empty() {
        for (key, handle) in &redo {
            tracing::info!("displays: a new mode chosen; setting the screen up again");
            disconnect(state, *key);
            connect_if_there(state, key.0, *handle, key.1);
        }
        setup_changed(state, anchors.clone());
        for (key, _) in &redo {
            render(state, *key);
        }
    }
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

fn connect(state: &mut Server, gpu: usize, conn: connector::Info, crtc: crtc::Handle) {
    let dh = state.display_handle.clone();
    let Some(b) = state.backend.as_mut() else { return };
    let mut name = format!("{}-{}", conn.interface().as_str(), conn.interface_id());
    // Two GPUs may both have an HDMI-A-1: the other GPU's gets its card's name too.
    if gpu > 0 && b.surfaces.values().any(|s| s.connector == name) {
        name = format!("{name}@{}", b.devices[gpu].path.rsplit('/').next().unwrap_or("gpu"));
    }
    // The monitor's identity first: a mode chosen for it in the settings app
    // (desktop.toml, by its EDID id) decides the mode; else the preferred one.
    let edid = edid_of(b.devices[gpu].manager.device(), conn.handle());
    // The same monitor on another of its inputs keeps its identity (monitors.rs).
    let others: Vec<Vec<u8>> = b.surfaces.values().map(|s| s.edid.clone()).collect();
    let identity =
        edid.as_deref().map(|e| crate::monitors::identity(e, &others)).unwrap_or_else(|| name.clone().into_bytes());
    let id = vergence_profile::monitor_id(&identity);
    let preferred = conn.modes().iter().position(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED)).unwrap_or(0);
    let wanted = crate::displays::load().mode_for(&id);
    // Only a mode the monitor itself offers: never one taken on trust.
    let chosen = wanted.and_then(|(ww, wh, wr)| {
        conn.modes().iter().position(|m| {
            let i = mode_info(m);
            i.width == ww && i.height == wh && (i.refresh_mhz as i64 - wr as i64).abs() <= 50
        })
    });
    if let (Some((ww, wh, wr)), None) = (wanted, chosen) {
        tracing::warn!(
            "{name}: {ww}x{wh} at {:.2} Hz is not one of its modes; its preferred mode instead",
            wr as f64 / 1000.0
        );
    }
    let mode_id = chosen.unwrap_or(preferred);
    let Some(&drm_mode) = conn.modes().get(mode_id) else { return };
    // Every mode the monitor offers, each once, for the settings app (bus v9).
    let mut modes: Vec<vergence_protocol::bus::ModeInfo> = Vec::new();
    for m in conn.modes() {
        let info = mode_info(m);
        if !modes.iter().any(|x| (x.width, x.height, x.refresh_mhz) == (info.width, info.height, info.refresh_mhz)) {
            modes.push(info);
        }
    }
    let running = mode_info(&drm_mode);
    let mode_index = modes
        .iter()
        .position(|x| (x.width, x.height, x.refresh_mhz) == (running.width, running.height, running.refresh_mhz))
        .map_or(u32::MAX, |i| i as u32);
    let wl_mode = WlMode::from(drm_mode);
    let (w, h) = drm_mode.size();

    // The panel's size from its EDID (the connector's mm are only the base block's).
    let parsed = edid.as_deref().and_then(|e| vergence_edid::Edid::parse(e).ok());
    let mm =
        parsed.as_ref().and_then(|e| e.physical_size_mm()).map(|(a, c)| (a as u32, c as u32)).or_else(|| conn.size());
    // Calculated from the EDID, the screen's viewing distance and the size
    // preference, or set by hand (displays.rs). In 120ths, as wp-fractional-scale-v1
    // tells it to apps: the output at exactly the scale the apps draw at, so their
    // buffers are shown 1:1, not resampled (the ASUS's 1.0703 drawn at 128/120 and
    // shown at 1.0703 was soft).
    let scales = crate::displays::load().scale_for(&id, &name, (w as u32, h as u32), mm);
    let (scale, how) = (scales.shown, scales.how.clone());
    let label = parsed.as_ref().and_then(|e| e.name.clone()).unwrap_or_else(|| name.clone());
    tracing::info!(
        "{name}: {label:?} {w}x{h} at {} Hz {:?} mm -> scale {:.4} ({}/120), {how}",
        drm_mode.vrefresh(),
        mm,
        scale,
        (scale * 120.0).round()
    );
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

    let planes = b.devices[gpu].manager.device().planes(&crtc).ok();
    let mut relay = None;
    let initialized = if gpu == 0 {
        b.devices[0].manager.lock().initialize_output::<_, OutputElements>(
            crtc,
            drm_mode,
            &[conn.handle()],
            &output,
            planes,
            &mut b.renderer,
            &DrmOutputRenderElements::default(),
        )
    } else {
        // The showing GPU's frame goes through a hidden output at scale 1, so the
        // copy is pixel for pixel; the real output keeps the screen's own scale.
        let plain = Output::new(
            format!("{name} (copy)"),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Vergence".into(),
                model: String::new(),
                serial_number: String::new(),
            },
        );
        plain.change_current_state(Some(wl_mode), Some(Transform::Normal), Some(Scale::Integer(1)), None);
        let Some(gbm) = b.gbm.as_ref() else { return };
        let buf = match relay_buffer(gbm, w as u32, h as u32) {
            Ok(v) => v,
            Err(e) => return tracing::warn!("{name}: no buffer to draw it in: {e}"),
        };
        let size = smithay::utils::Size::<i32, smithay::utils::Physical>::from((w as i32, h as i32));
        let damage = smithay::backend::renderer::damage::OutputDamageTracker::new(size, scale, Transform::Normal);
        let dev = &mut b.devices[gpu];
        let Some(copier) = dev.renderer.as_mut() else { return };
        let r = dev.manager.lock().initialize_output::<_, RelayElements>(
            crtc,
            drm_mode,
            &[conn.handle()],
            &plain,
            planes,
            copier,
            &DrmOutputRenderElements::default(),
        );
        relay = Some(Relay { buf, shown: None, frame: 0, pending: false, last_damage_px: 0, damage, _plain: plain });
        r
    };
    let drm_output = match initialized {
        Ok(o) => o,
        Err(e) => return tracing::warn!("{name}: cannot initialize: {e:?}"),
    };
    if gpu > 0 {
        tracing::info!("{name}: shown by {}, drawn by {} and copied there", b.devices[gpu].path, b.devices[0].path);
    }
    b.surfaces.insert(
        (gpu, crtc),
        Surface {
            output,
            present: Present::Drm { out: drm_output, handle: conn.handle() },
            connector: name,
            last: std::time::Instant::now(),
            scale,
            id,
            px: (w as u32, h as u32),
            mm,
            edid: identity,
            global,
            relay,
            modes,
            mode: mode_index,
            since: CONNECTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        },
    );
}

/// The GL renderer's name ("Mesa Intel(R) UHD Graphics (CML GT2)", "llvmpipe ...").
fn gl_renderer(r: &mut GlesRenderer) -> String {
    use smithay::backend::renderer::gles::ffi;
    r.with_context(|gl| unsafe {
        let p = gl.GetString(ffi::RENDERER);
        if p.is_null() {
            String::from("an unknown renderer")
        } else {
            std::ffi::CStr::from_ptr(p as *const std::ffi::c_char).to_string_lossy().into_owned()
        }
    })
    .unwrap_or_else(|_| "an unknown renderer".into())
}

/// Whether a GL renderer is Mesa's software rendering on the CPU.
fn is_software(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["llvmpipe", "softpipe", "swrast", "software"].iter().any(|s| n.contains(s))
}

/// A DRM mode as the bus says it: pixels, refresh in millihertz (from the
/// pixel clock and the totals, so 59.94 and 60 stay apart), preferred or not.
fn mode_info(m: &smithay::reexports::drm::control::Mode) -> vergence_protocol::bus::ModeInfo {
    let (w, h) = m.size();
    let (_, _, htotal) = m.hsync();
    let (_, _, vtotal) = m.vsync();
    let total = htotal as u64 * vtotal as u64;
    let refresh_mhz = if total > 0 { (m.clock() as u64 * 1_000_000 / total) as u32 } else { m.vrefresh() * 1000 };
    vergence_protocol::bus::ModeInfo {
        width: w as u32,
        height: h as u32,
        refresh_mhz,
        preferred: m.mode_type().contains(ModeTypeFlags::PREFERRED),
    }
}

/// A linear XRGB8888 buffer in the primary GPU's memory, the size of a screen
/// on another GPU (the format both GPUs render and import: gpus.rs).
fn relay_buffer(gbm: &GbmDevice<DrmDeviceFd>, w: u32, h: u32) -> Result<RelayBuf, String> {
    use smithay::backend::allocator::{Allocator, Modifier, dmabuf::AsDmabuf};
    let mut alloc = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING);
    let bo = alloc.create_buffer(w, h, Fourcc::Xrgb8888, &[Modifier::Linear]).map_err(|e| format!("{e:?}"))?;
    let dmabuf = bo.export().map_err(|e| format!("{e:?}"))?;
    Ok(RelayBuf { dmabuf, drawn: 0, sync: None })
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
    // Screens the arrangement has no place for go on the right in the order
    // they were connected: a screen plugged in joins at the right and moves
    // nothing (a fixed connector order put a newly plugged screen on the left
    // and pushed the others over).
    // VERGENCE_LEFT_TO_RIGHT="DP-1,HDMI-A-1" still puts connectors first by hand.
    let order = std::env::var("VERGENCE_LEFT_TO_RIGHT").unwrap_or_default();
    let rank = |c: &str| order.split(',').position(|o| !o.trim().is_empty() && o.trim() == c).unwrap_or(usize::MAX);
    let mut list: Vec<&Surface> = b.surfaces.values().filter(|s| s.output.current_mode().is_some()).collect();
    list.sort_by_key(|s| (rank(&s.connector), s.since));
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
    if crate::greeter() {
        state.greeter_start_screen();
    }
}

fn render(state: &mut Server, crtc: Key) {
    // The backend is taken out of the state for the frame, so the scene can be
    // built from the state with the backend's renderer.
    if state.backend.as_ref().is_some_and(|b| b.paused) {
        return;
    }
    let Some(mut backend) = state.backend.take() else { return };
    let t = std::time::Instant::now();
    render_with(state, &mut backend, crtc);
    let ms = t.elapsed().as_millis();
    if ms > 20 {
        let name = backend.surfaces.get(&crtc).map(|s| s.connector.as_str()).unwrap_or("?");
        tracing::warn!("slow: frame {ms} ms ({name})");
    }
    // A frame that blocks for seconds was waiting on a GPU hang (the kernel resets
    // the engine after ~7 s). The reset leaves this GL context drawing nothing but
    // black (on screen) or white (offscreen) for good, while every call succeeds
    // (seen on an Intel UHD 630), and Smithay's context is not robust, so it
    // cannot ask. Start over with a new context; textures are cached per context
    // and upload again.
    if ms > 2000 {
        tracing::warn!("GPU hang suspected (frame blocked {ms} ms)");
        renew_renderer(&mut backend);
        state.scenes_renew();
        // And the displays set up again from scratch: after three hangs in a
        // row on kepler (UHD 630, FreeBSD, 2026-11-27 14:08) the new context
        // drew and its page flips were taken, the server saw nothing wrong for
        // six minutes, and the monitor kept showing the frame before the hang,
        // pointer and all, until the session was ended. Only a modeset (as a
        // console switch does) brings such a display back, and nothing tells
        // the server it is needed, so it is done after every hang.
        reset_displays(&mut backend);
    }
    state.backend = Some(backend);
}

impl Server {
    /// The control socket's `renew-gpu`: what a GPU hang does to the drawing, on purpose
    /// (tests of renew_renderer: every window, the frost, the corners and the screen
    /// itself drawn again on a new context, none of the old one's textures kept).
    pub fn renew_gpu(&mut self) -> String {
        let Some(mut b) = self.backend.take() else { return "renew-gpu: only with --drm".into() };
        renew_renderer(&mut b);
        self.backend = Some(b);
        self.scenes_renew();
        "renew-gpu: drawing with a new GL context".into()
    }

    /// The control socket's `reset-displays`: the displays set up again from
    /// scratch, as after a GPU hang (tests of reset_displays: each screen goes
    /// dark for a moment and comes back drawing).
    pub fn reset_displays_now(&mut self) -> String {
        let Some(mut b) = self.backend.take() else { return "reset-displays: only with --drm".into() };
        let crtcs: Vec<Key> = b.surfaces.keys().copied().filter(|k| k.0 != MEMORY).collect();
        reset_displays(&mut b);
        self.backend = Some(b);
        for c in crtcs {
            render(self, c);
        }
        "reset-displays: every display set up again".into()
    }
}

/// Set every display up again from scratch, as coming back from another
/// console does, but with every connector and plane turned off first, so the
/// next frame of each screen is a full modeset (Smithay's activate(true)),
/// not a page flip onto whatever state the kernel kept: what brings back a
/// monitor left showing an old frame after a GPU reset. The screens go dark
/// for a moment.
fn reset_displays(b: &mut Backend) {
    for d in &mut b.devices {
        d.manager.pause();
        if let Err(e) = d.manager.lock().activate(true) {
            tracing::error!("displays: {} could not be set up again: {e:?}", d.path);
        }
    }
    b.redraw_all();
    tracing::warn!("displays: set up again from scratch (modeset)");
}

/// Replace the renderer with one on a new EGL context, and draw every monitor in full.
/// What the backend itself made on the old context is made again here; the caches
/// elsewhere each notice the new context themselves (backdrop.rs Ctx).
fn renew_renderer(b: &mut Backend) {
    let display = b.renderer.egl_context().display().clone();
    let context = match EGLContext::new(&display) {
        Ok(c) => c,
        Err(e) => return tracing::error!("GPU hang: cannot make a new EGL context: {e:?}"),
    };
    match unsafe { GlesRenderer::new(context) } {
        Ok(r) => {
            b.renderer = r;
            // Remote views make their texture again at their next frame.
            for r in &mut b.remotes {
                (r.tex, r.damage) = (None, None);
            }
            // Screens kept in memory (virtual screens) draw into a texture of the renderer's.
            use smithay::backend::renderer::Offscreen;
            for s in b.surfaces.values_mut() {
                let Present::Memory(m) = &mut s.present else { continue };
                match b.renderer.create_buffer(Fourcc::Abgr8888, (s.px.0 as i32, s.px.1 as i32).into()) {
                    Ok(t) => {
                        m.tex = t;
                        m.damage = smithay::backend::renderer::damage::OutputDamageTracker::from_output(&s.output);
                    }
                    Err(e) => tracing::error!("GPU renewed: {} has no texture to draw in: {e}", s.connector),
                }
            }
            b.redraw_all();
            tracing::warn!("GPU renewed: drawing with a new GL context");
        }
        Err(e) => tracing::error!("GPU hang: cannot make a new renderer: {e:?}"),
    }
}

fn render_with(state: &mut Server, b: &mut Backend, crtc: Key) {
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
    let scale = s.scale;
    // The magnifier follows the pointer that moved (zoom.rs).
    state.zoom_pointer(pointer_at);
    let front = screen_elements(state, &mut b.renderer, &output, geo, scale, pointer_at);
    let elements: Vec<OutputElements> = match state.zoom_here(geo) {
        None => front
            .into_iter()
            .map(|e| match e {
                crate::zoom::Inner::Scene(e) => OutputElements::Scene(e),
                crate::zoom::Inner::PointerTheme(e) => OutputElements::PointerTheme(e),
                crate::zoom::Inner::PointerSurface(e) => OutputElements::PointerSurface(e),
            })
            .collect(),
        Some(lens) => {
            let back = if lens {
                screen_elements(state, &mut b.renderer, &output, geo, scale, pointer_at)
            } else {
                Vec::new()
            };
            let ground = state.background_color(&s.connector);
            state.zoom_frame(front, back, geo, scale, ground).into_iter().map(OutputElements::Zoomed).collect()
        }
    };

    // The control socket's `shot`: this monitor's frame, rendered once more
    // offscreen; for a screen on another GPU, the very buffer copied to it last
    // (what is on the screen, glitches and all).
    if let Some(i) = state.shot_pending.iter().position(|n| *n == s.connector) {
        state.shot_pending.remove(i);
        let size = output.current_mode().map(|m| m.size).unwrap_or_default();
        let shown =
            s.relay.as_mut().filter(|r| r.buf.drawn > 0).map(|r| read_back(&mut b.renderer, &mut r.buf.dmabuf, size));
        match shown
            .unwrap_or_else(|| capture(&mut b.renderer, &elements, size, s.scale, state.background_color(&s.connector)))
        {
            Ok(rgba) => crate::control::save_png(s.connector.clone(), size.w as u32, size.h as u32, rgba),
            Err(e) => tracing::warn!("shot {}: {e}", s.connector),
        }
    }

    // Screenshots asked for (capture.rs).
    run_captures(state, &mut b.renderer, pointer_at);

    // The control socket's `picture`: windows' own pictures, alone, at scale 1.
    for (w, name) in std::mem::take(&mut state.picture_pending) {
        match crate::decor::picture_elements(&mut b.renderer, &w) {
            Some((els, size)) => {
                let els: Vec<OutputElements> = els.into_iter().map(OutputElements::Scene).collect();
                match capture(&mut b.renderer, &els, size, 1.0, [0.0, 0.0, 0.0, 0.0]) {
                    Ok(rgba) => crate::control::save_png(name, size.w as u32, size.h as u32, rgba),
                    Err(e) => tracing::warn!("picture {name}: {e}"),
                }
            }
            None => tracing::warn!("picture {name}: the window has no picture now"),
        }
    }

    // The control socket's `thumb`: windows' pictures as the pickers show them.
    for (w, side, name) in std::mem::take(&mut state.thumb_pending) {
        match state.thumb(&mut b.renderer, &w, (side, side).into()) {
            Some((mut texture, px)) => match read_texture(&mut b.renderer, &mut texture, px) {
                Ok(rgba) => crate::control::save_png(name, px.w as u32, px.h as u32, rgba),
                Err(e) => tracing::warn!("thumb {name}: {e}"),
            },
            None => tracing::warn!("thumb {name}: the window has nothing to show"),
        }
    }
    state.thumbs_prune();

    let clear = state.background_color(&s.connector);
    // Remote views of this screen: drawn again at each viewer's scale.
    if b.remotes.iter().any(|r| r.source == s.connector) {
        remote_frames(state, &mut b.renderer, &mut b.remotes, &output, &s.connector, geo, pointer_at, clear);
    }
    let outcome = if crtc.0 == MEMORY {
        memory_frame(&mut b.renderer, s, &elements, clear)
    } else if crtc.0 == 0 {
        match s.drm() {
            Some(out) => out
                .render_frame(&mut b.renderer, &elements, clear, FrameFlags::DEFAULT)
                .map(|r| if r.is_empty { Relayed::Nothing } else { Relayed::Shown })
                .map_err(|e| format!("{e:?}")),
            None => Err("not a monitor".to_string()),
        }
    } else {
        relay_frame(&mut b.renderer, b.devices[crtc.0].renderer.as_mut(), s, &elements, clear)
    };
    // A relay turn that only looked whether the copy may start is no frame:
    // windows are not asked for their next one then (asking on every 3 ms look
    // had glmark2 drawing ~110 frames a second for a 60 Hz screen).
    let paced = !matches!(outcome, Ok(Relayed::Waiting));
    match outcome {
        Ok(Relayed::Waiting) => {
            // The primary is still drawing it: look again in 3 ms, not a frame.
            let _ = state.loop_handle.insert_source(Timer::from_duration(Duration::from_millis(3)), move |_, _, st| {
                render(st, crtc);
                TimeoutAction::Drop
            });
        }
        Ok(Relayed::Shown) if crtc.0 == MEMORY => {
            // In memory it is "on the screen" at once; the next frame comes
            // after a frame's time, as a VBlank would.
            let name = s.connector.clone();
            let interval = match &s.present {
                Present::Memory(m) => m.interval,
                Present::Drm { .. } => Duration::from_millis(16),
            };
            let screens = b.surfaces.len();
            state.lock_frame_queued(name.clone());
            state.lock_frame_shown(&name, screens);
            let _ = state.loop_handle.insert_source(Timer::from_duration(interval), move |_, _, st| {
                render(st, crtc);
                TimeoutAction::Drop
            });
        }
        Ok(Relayed::Shown) => {
            let queued = s.drm().map_or(Ok(()), |out| out.queue_frame(()).map_err(|e| format!("{e:?}")));
            if queued.is_ok() {
                let name = s.connector.clone();
                state.lock_frame_queued(name);
                b.refused = 0;
            }
            if let Err(e) = queued {
                tracing::warn!("queue frame: {e:?}");
                // Refused again and again (EBUSY after a GPU reset, a CRTC
                // the kernel left in a state of its own): set up from scratch.
                b.refused += 1;
                if b.refused >= FLIPS_REFUSED {
                    b.refused = 0;
                    reset_displays(b);
                }
                let _ = state.loop_handle.insert_source(
                    Timer::from_duration(Duration::from_millis(100)),
                    move |_, _, st| {
                        render(st, crtc);
                        TimeoutAction::Drop
                    },
                );
            }
        }
        Ok(Relayed::Nothing) => {
            // Nothing changed: look again in a frame's time (no VBlank comes without a frame).
            let _ =
                state.loop_handle.insert_source(Timer::from_duration(Duration::from_millis(16)), move |_, _, st| {
                    render(st, crtc);
                    TimeoutAction::Drop
                });
        }
        Err(e) => {
            // Try again shortly: without a frame no VBlank comes, and drawing
            // would stop for good.
            tracing::warn!("render {}: {e}", s.connector);
            let _ =
                state.loop_handle.insert_source(Timer::from_duration(Duration::from_millis(100)), move |_, _, st| {
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
            if *o == output && paced {
                window.send_frame(o, start, Some(Duration::ZERO), |_, _| Some(o.clone()));
            }
        }
    }
    // A client's own cursor surface (animated ones wait for this).
    if let smithay::input::pointer::CursorImageStatus::Surface(cs) = &state.pointer_image.status
        && geo.to_f64().contains(pointer_at)
        && paced
    {
        smithay::desktop::utils::send_frames_surface_tree(cs, &output, start, Some(Duration::ZERO), |_, _| {
            Some(output.clone())
        });
    }
    // Native windows on this monitor that wait for a frame, and scale changes (native.rs).
    if paced {
        state.native_frames(&output);
    }
    state.space.refresh();
    state.popups.cleanup();
    let _ = state.display_handle.flush_clients();
}

/// A screen on another GPU, one turn: a frame drawn and not yet copied is
/// copied once its fence has signaled, only its damage; otherwise the
/// primary draws any change and records its damage for the copy.
fn relay_frame(
    primary: &mut GlesRenderer,
    copier: Option<&mut GlesRenderer>,
    s: &mut Surface,
    elements: &[OutputElements],
    clear: [f32; 4],
) -> Result<Relayed, String> {
    use smithay::backend::renderer::{
        Bind, ImportDma,
        element::{
            Kind,
            texture::{TextureRenderBuffer, TextureRenderElement},
        },
    };
    use smithay::utils::{Buffer, Rectangle};
    let copier = copier.ok_or("no renderer on the showing GPU")?;
    let relay = s.relay.as_mut().ok_or("no relay buffer")?;
    let t0 = std::time::Instant::now();
    if !relay.pending {
        // Draw any change; its damage goes to the copy. The one buffer always
        // holds the newest frame (the last call drew into it, or nothing had
        // changed), so its age is 1. (Counting calls since it was drawn ran past
        // the few frames the tracker keeps, and every frame was drawn and
        // copied whole: 8.3 million pixels, 40 to 65 ms on nouveau.)
        let age = if relay.buf.drawn == 0 { 0 } else { 1 };
        let drawn = {
            let mut fb = primary.bind(&mut relay.buf.dmabuf).map_err(|e| format!("bind: {e}"))?;
            let r = relay
                .damage
                .render_output(primary, &mut fb, age, elements, clear)
                .map_err(|e| format!("draw: {e:?}"))?;
            r.damage.map(|d| {
                (
                    d.iter()
                        .map(|r| Rectangle::<i32, Buffer>::new((r.loc.x, r.loc.y).into(), (r.size.w, r.size.h).into()))
                        .collect::<Vec<_>>(),
                    r.sync.clone(),
                )
            })
        };
        relay.frame += 1;
        let Some((rects, sync)) = drawn else { return Ok(Relayed::Nothing) };
        relay.buf.drawn = relay.frame;
        relay.buf.sync = Some(sync);
        relay.pending = true;
        relay.last_damage_px = rects.iter().map(|r| r.size.w as i64 * r.size.h as i64).sum();
        if let Some(shown) = relay.shown.as_mut() {
            let _ = shown.render().draw(|_| Ok::<_, ()>(rects));
        }
    }
    if !relay.buf.sync.as_ref().is_none_or(|sy| sy.is_reached()) {
        return Ok(Relayed::Waiting);
    }
    // Copy what changed onto the showing GPU's scan-out buffer.
    let t1 = std::time::Instant::now();
    relay.pending = false;
    relay.buf.sync = None;
    if relay.shown.is_none() {
        let tex =
            copier.import_dmabuf(&relay.buf.dmabuf, None).map_err(|e| format!("import on the showing GPU: {e}"))?;
        relay.shown = Some(TextureRenderBuffer::from_texture(copier, tex, 1, Transform::Normal, None));
    }
    let shown = relay.shown.as_ref().ok_or("no texture")?;
    let el = TextureRenderElement::from_texture_render_buffer((0.0, 0.0), shown, None, None, None, Kind::Unspecified);
    let els = [RelayElements::Texture(el)];
    // The field itself, not drm(): `relay` holds a borrow of the surface.
    let out = match &mut s.present {
        Present::Drm { out, .. } => out,
        Present::Memory(_) => return Err("copy: not a monitor".into()),
    };
    let empty =
        out.render_frame(copier, &els, clear, FrameFlags::DEFAULT).map_err(|e| format!("copy: {e:?}"))?.is_empty;
    let damage_px = relay.last_damage_px;
    let tr = std::time::Instant::now();
    // The copy must be finished before the screen flips to it and before the
    // primary draws into the buffer again: nouveau's EGL gives no fence the
    // kernel can wait on.
    let _ = copier.with_context(|gl| unsafe { gl.Finish() });
    let t2 = std::time::Instant::now();
    if std::env::var_os("VERGENCE_RELAY_TIMES").is_some() || (t2 - t0).as_millis() > 20 {
        tracing::info!(
            "relay {}: draw {:.1} ms, copy submitted {:.1} ms, finished {:.1} ms, {} px copied",
            s.connector,
            (t1 - t0).as_secs_f64() * 1e3,
            (tr - t1).as_secs_f64() * 1e3,
            (t2 - tr).as_secs_f64() * 1e3,
            damage_px
        );
    }
    Ok(if empty { Relayed::Nothing } else { Relayed::Shown })
}

/// A relay buffer's pixels as the screen got them (R, G, B, A bytes, top row first).
fn read_back(
    renderer: &mut GlesRenderer,
    dmabuf: &mut smithay::backend::allocator::dmabuf::Dmabuf,
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
) -> Result<Vec<u8>, String> {
    use smithay::backend::renderer::{Bind, ExportMem};
    let buf = smithay::utils::Size::<i32, smithay::utils::Buffer>::from((size.w, size.h));
    let fb = renderer.bind(dmabuf).map_err(|e| e.to_string())?;
    let mapping = renderer
        .copy_framebuffer(&fb, smithay::utils::Rectangle::from_size(buf), Fourcc::Abgr8888)
        .map_err(|e| e.to_string())?;
    drop(fb);
    let bytes = renderer.map_texture(&mapping).map_err(|e| e.to_string())?;
    Ok(bytes.to_vec())
}

/// Render `elements` into an offscreen buffer the size of the monitor and read it back
/// (R, G, B, A bytes, top row first).
/// `--headless` (headless milestone 1, step 3): no seat, no DRM master, no
/// input devices; the GPU through EGL's device platform (its render node,
/// any user's), or Mesa's software device on a machine with none (canary);
/// every screen in memory (`--screen WxH MMxMM[@HZ]`, again and again, or
/// `vctl screen add`). Input comes only from vctl. Run it with its own HOME
/// and runtime directory (tools/vergence-session --headless): it is a whole
/// session, with its own settings, window memory and session restore.
pub fn run_headless(
    event_loop: &mut EventLoop<Server>,
    state: &mut Server,
    screens: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    use smithay::backend::egl::EGLDevice;
    // The GPU: VERGENCE_HEADLESS_GPU (a render node's path) if given, else the
    // lowest-numbered render node (renderD128, the main GPU as a rule: Hubble's
    // Intel before its NVIDIA, which zink drew on at first), else Mesa's
    // software device (canary, no GPU).
    let mut all: Vec<(Option<smithay::backend::drm::DrmNode>, EGLDevice)> =
        EGLDevice::enumerate()?.map(|d| (d.try_get_render_node().ok().flatten(), d)).collect();
    all.sort_by_key(|(n, d)| (d.is_software(), n.map_or(u64::MAX, |n| n.minor() as u64)));
    let wanted =
        std::env::var("VERGENCE_HEADLESS_GPU").ok().and_then(|p| smithay::backend::drm::DrmNode::from_path(p).ok());
    let pick = match wanted {
        Some(w) => all
            .iter()
            .find(|(n, _)| n.is_some_and(|n| n.dev_id() == w.dev_id()))
            .ok_or("VERGENCE_HEADLESS_GPU: no such render node")?,
        None => all.first().ok_or("no EGL device, not even a software one")?,
    };
    let device = pick.1.clone();
    let node = device.try_get_render_node().ok().flatten();
    let display = unsafe { EGLDisplay::new(device)? };
    let context = EGLContext::new(&display)?;
    let mut renderer = unsafe { GlesRenderer::new(context)? };
    let name = gl_renderer(&mut renderer);
    tracing::info!(
        "headless: drawing with {name} ({})",
        node.map_or("no render node: software".to_string(), |n| format!("{n:?}"))
    );
    state.shm_formats_from(&renderer);
    if let Some(node) = node {
        match smithay::wayland::dmabuf::DmabufFeedbackBuilder::new(node.dev_id(), renderer.dmabuf_formats()).build() {
            Ok(feedback) => {
                let mut dmabuf_state = smithay::wayland::dmabuf::DmabufState::new();
                let global =
                    dmabuf_state.create_global_with_default_feedback::<Server>(&state.display_handle, &feedback);
                state.dmabuf = Some((dmabuf_state, global));
            }
            Err(e) => tracing::warn!("linux-dmabuf: {e:?}"),
        }
    }
    let (probed, _probed_rx) = smithay::reexports::calloop::channel::channel::<usize>();
    state.backend = Some(Backend {
        renderer,
        gbm: None,
        devices: Vec::new(),
        surfaces: HashMap::new(),
        paused: false,
        waiting: Vec::new(),
        probed,
        remotes: Vec::new(),
        refused: 0,
    });
    state.session = None;
    watchdog(event_loop)?;
    for spec in screens {
        // `scale=S` sets the screen's scale outright (a session with no
        // monitors, at its viewer's scale).
        let scale = spec.split(' ').find_map(|w| w.strip_prefix("scale=")).and_then(|v| v.parse::<f64>().ok());
        let rest: String = spec.split(' ').filter(|w| !w.starts_with("scale=")).collect::<Vec<_>>().join(" ");
        let parsed = parse_screen_spec(&rest)
            .ok_or_else(|| format!("--screen {spec}: WxH MMxMM[@HZ] [scale=S], as 2560x1440 597x336@60"))?;
        let (px, mm, hz) = parsed;
        let edid = fake_screen_edid(px, mm, &format!("HL{}x{}", px.0, px.1));
        if let Some(s) = scale.filter(|s| (0.5..=4.0).contains(s)) {
            // As a test scale is: for this run, not saved in the settings.
            let next = MEMORY_SCREENS.load(std::sync::atomic::Ordering::Relaxed);
            state.scale_test.insert(format!("MEM-{next}"), s);
        }
        add_memory_screen(state, edid, px, hz)?;
    }
    Ok(())
}

/// A screen in memory: its mode in pixels, its size in mm, its rate in mHz.
pub type ScreenSpec = ((u32, u32), (u32, u32), u32);

/// "2560x1440 597x336@60": a screen's mode, its size in mm, and its rate
/// (60 Hz unless given); the rate may follow either part.
pub fn parse_screen_spec(spec: &str) -> Option<ScreenSpec> {
    let pair = |v: &str| -> Option<(u32, u32)> {
        let (a, b) = v.split_once('x')?;
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    };
    let mut hz = 60_000;
    let mut parts = Vec::new();
    for word in spec.split([' ', ',']).filter(|w| !w.is_empty()) {
        let size = match word.split_once('@') {
            Some((size, rate)) => {
                hz = (rate.parse::<f64>().ok()? * 1000.0).round() as u32;
                size
            }
            None => word,
        };
        parts.push(pair(size)?);
    }
    match parts.as_slice() {
        [px, mm] => Some((*px, *mm, hz)),
        _ => None,
    }
}

/// A fake monitor's EDID for a screen in memory: the same mode, size and
/// name give the same monitor (window memory per setup).
pub fn fake_screen_edid(px: (u32, u32), mm: (u32, u32), name: &str) -> Vec<u8> {
    let serial =
        (px.0.wrapping_mul(31).wrapping_add(px.1)).wrapping_mul(2654435761).wrapping_add(mm.0 * 7 + mm.1) % 100_000_000;
    vergence_edid::fake::build(&vergence_edid::fake::Spec {
        name,
        product: 0x9a00,
        serial,
        timing: vergence_edid::fake::FHD_60,
        size_mm: (mm.0.min(4095) as u16, mm.1.min(4095) as u16),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_specs() {
        assert_eq!(parse_screen_spec("2560x1440 597x336"), Some(((2560, 1440), (597, 336), 60_000)));
        assert_eq!(parse_screen_spec("3840x2160@30 310x174"), Some(((3840, 2160), (310, 174), 30_000)));
        assert_eq!(parse_screen_spec("1920x1080,531x299@59.94"), Some(((1920, 1080), (531, 299), 59_940)));
        assert_eq!(parse_screen_spec("1920x1080"), None);
        assert_eq!(fake_screen_edid((2560, 1440), (597, 336), "A"), fake_screen_edid((2560, 1440), (597, 336), "A"));
        assert_ne!(fake_screen_edid((2560, 1440), (597, 336), "A"), fake_screen_edid((1920, 1080), (597, 336), "A"));
    }
}

fn watchdog(event_loop: &mut EventLoop<Server>) -> Result<(), Box<dyn std::error::Error>> {
    // THE WATCHDOG (without it the whole desktop froze, pointer and all, with
    // the server idle and nothing logged): drawing is driven by VBlank events,
    // so a lost one, or a frame that ended without arranging the next, stops a
    // monitor for good. Any monitor not drawn for 2 s while the session is
    // active is drawn again, and it is logged.
    event_loop.handle().insert_source(Timer::from_duration(Duration::from_secs(1)), |_, _, state| {
        let stalled: Vec<(Key, String, u128)> = state
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
        for (key, name, ms) in stalled {
            tracing::warn!("watchdog: {name} not drawn for {ms} ms (VBlank lost?); drawing it again");
            if let Some(b) = state.backend.as_mut()
                && let Some(s) = b.surfaces.get_mut(&key)
            {
                s.frame_submitted();
                s.reset_buffers();
            }
            render(state, key);
        }
        TimeoutAction::ToDuration(Duration::from_secs(1))
    })?;
    Ok(())
}

/// A screen in memory, one frame: drawn into its texture (what changed).
fn memory_frame(
    renderer: &mut GlesRenderer,
    s: &mut Surface,
    elements: &[OutputElements],
    clear: [f32; 4],
) -> Result<Relayed, String> {
    use smithay::backend::renderer::Bind;
    let Present::Memory(m) = &mut s.present else { return Err("not in memory".into()) };
    let mut fb = renderer.bind(&mut m.tex).map_err(|e| e.to_string())?;
    // One texture, kept: what was drawn last is still in it (age 1).
    let r = m.damage.render_output(renderer, &mut fb, 1, elements, clear).map_err(|e| format!("{e:?}"))?;
    Ok(if r.damage.is_some_and(|d| !d.is_empty()) { Relayed::Shown } else { Relayed::Nothing })
}

/// A screen in memory beside the monitors (`vctl screen add`): `edid` its
/// identity (a fake's, vergence_edid::fake), `px` its mode, `refresh_mhz` its
/// rate. Laid out, scaled and remembered like a monitor, except that its
/// setups are never saved (memory.rs). Its name, `MEM-<n>`.
pub fn add_memory_screen(
    state: &mut Server,
    edid: Vec<u8>,
    px: (u32, u32),
    refresh_mhz: u32,
) -> Result<String, String> {
    use smithay::backend::renderer::Offscreen;
    if !(1..=8192).contains(&px.0) || !(1..=8192).contains(&px.1) || !(1_000..=500_000).contains(&refresh_mhz) {
        return Err("a screen from 1x1 to 8192x8192 pixels, 1 to 500 Hz".into());
    }
    let n = MEMORY_SCREENS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = format!("MEM-{n}");
    let crtc: crtc::Handle = smithay::reexports::drm::control::from_u32(n).ok_or("no number for it")?;
    let dh = state.display_handle.clone();
    let Some(b) = state.backend.as_mut() else { return Err("no backend".into()) };
    let others: Vec<Vec<u8>> = b.surfaces.values().map(|s| s.edid.clone()).collect();
    let identity = crate::monitors::identity(&edid, &others);
    let id = vergence_profile::monitor_id(&identity);
    let parsed = vergence_edid::Edid::parse(&edid).ok();
    let mm = parsed.as_ref().and_then(|e| e.physical_size_mm()).map(|(a, c)| (a as u32, c as u32));
    let label = parsed.as_ref().and_then(|e| e.name.clone()).unwrap_or_else(|| name.clone());
    let mut cfg = crate::displays::load();
    if let Some(v) = state.scale_test.get(&name) {
        cfg.set_scale(&id, *v);
    }
    let scales = cfg.scale_for(&id, &name, px, mm);
    let scale = scales.shown;
    tracing::info!(
        "{name}: in memory, {label:?} {}x{} at {:.2} Hz {mm:?} mm -> scale {scale:.4}, {}",
        px.0,
        px.1,
        refresh_mhz as f64 / 1000.0,
        scales.how
    );
    state.scale_notes.insert(name.clone(), scales);
    let Some(b) = state.backend.as_mut() else { return Err("no backend".into()) };
    let wl_mode = WlMode { size: (px.0 as i32, px.1 as i32).into(), refresh: refresh_mhz as i32 };
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
    output.user_data().insert_if_missing(|| crate::memory::MonitorId(id.clone()));
    output.set_preferred(wl_mode);
    output.change_current_state(Some(wl_mode), Some(Transform::Normal), Some(Scale::Fractional(scale)), None);
    let tex = b
        .renderer
        .create_buffer(Fourcc::Abgr8888, (px.0 as i32, px.1 as i32).into())
        .map_err(|e| format!("{name}: no texture to draw it in: {e}"))?;
    // From the output, so a scale changed later (vctl scale, the settings app) is followed.
    let damage = smithay::backend::renderer::damage::OutputDamageTracker::from_output(&output);
    let interval = Duration::from_micros(1_000_000_000 / refresh_mhz as u64);
    let modes = vec![vergence_protocol::bus::ModeInfo { width: px.0, height: px.1, refresh_mhz, preferred: true }];
    b.surfaces.insert(
        (MEMORY, crtc),
        Surface {
            output,
            present: Present::Memory(MemoryScreen { tex, damage, interval }),
            connector: name.clone(),
            last: std::time::Instant::now(),
            scale,
            id,
            px,
            mm,
            edid: identity,
            global,
            relay: None,
            modes,
            mode: 0,
            since: CONNECTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        },
    );
    let anchors = state.window_anchors();
    setup_changed(state, anchors);
    state.bus_outputs_changed();
    render(state, (MEMORY, crtc));
    Ok(name)
}

/// A screen in memory given a new size, as a monitor's mode changed (a
/// session with no monitors following its viewer's window, remote milestone
/// 1, step 6): its scale kept, windows laid out again.
pub fn resize_memory_screen(state: &mut Server, name: &str, px: (u32, u32)) -> Result<(), String> {
    use smithay::backend::renderer::Offscreen;
    if !(1..=8192).contains(&px.0) || !(1..=8192).contains(&px.1) {
        return Err("a screen from 1x1 to 8192x8192 pixels".into());
    }
    let anchors = state.window_anchors();
    let Some(b) = state.backend.as_mut() else { return Err("no backend".into()) };
    let Some((key, s)) = b.surfaces.iter_mut().find(|(k, s)| k.0 == MEMORY && s.connector == name) else {
        return Err(format!("no screen {name} in memory"));
    };
    let key = *key;
    if s.px == px {
        return Ok(());
    }
    let Present::Memory(m) = &mut s.present else { return Err(format!("{name} is not in memory")) };
    let refresh = (1_000_000_000 / m.interval.as_micros().max(1)) as i32;
    let tex = b
        .renderer
        .create_buffer(Fourcc::Abgr8888, (px.0 as i32, px.1 as i32).into())
        .map_err(|e| format!("{name}: no texture to draw it in: {e}"))?;
    let wl_mode = WlMode { size: (px.0 as i32, px.1 as i32).into(), refresh };
    s.output.set_preferred(wl_mode);
    s.output.change_current_state(Some(wl_mode), None, None, None);
    m.tex = tex;
    m.damage = smithay::backend::renderer::damage::OutputDamageTracker::from_output(&s.output);
    tracing::info!("{name}: now {}x{} (was {}x{})", px.0, px.1, s.px.0, s.px.1);
    s.px = px;
    s.modes = vec![vergence_protocol::bus::ModeInfo {
        width: px.0,
        height: px.1,
        refresh_mhz: refresh as u32,
        preferred: true,
    }];
    s.mode = 0;
    setup_changed(state, anchors);
    state.bus_outputs_changed();
    render(state, key);
    Ok(())
}

/// Take a screen in memory away, as a monitor unplugged.
pub fn remove_memory_screen(state: &mut Server, name: &str) -> Result<(), String> {
    let key = state
        .backend
        .as_ref()
        .and_then(|b| b.surfaces.iter().find(|(k, s)| k.0 == MEMORY && s.connector == name).map(|(k, _)| *k))
        .ok_or_else(|| format!("no screen {name} in memory"))?;
    let anchors = state.window_anchors();
    disconnect(state, key);
    state.scale_notes.remove(name);
    state.scale_test.remove(name);
    setup_changed(state, anchors);
    Ok(())
}

/// The screens in memory, by name.
pub fn memory_screens(state: &Server) -> Vec<String> {
    let mut v: Vec<String> = state
        .backend
        .as_ref()
        .map(|b| b.surfaces.iter().filter(|(k, _)| k.0 == MEMORY).map(|(_, s)| s.connector.clone()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// A texture's pixels, RGBA.
fn read_texture(
    renderer: &mut GlesRenderer,
    tex: &mut smithay::backend::renderer::gles::GlesTexture,
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
) -> Result<Vec<u8>, String> {
    use smithay::backend::renderer::{Bind, ExportMem};
    let buf = smithay::utils::Size::<i32, smithay::utils::Buffer>::from((size.w, size.h));
    let fb = renderer.bind(tex).map_err(|e| e.to_string())?;
    let mapping = renderer
        .copy_framebuffer(&fb, smithay::utils::Rectangle::from_size(buf), Fourcc::Abgr8888)
        .map_err(|e| e.to_string())?;
    let bytes = renderer.map_texture(&mapping).map_err(|e| e.to_string())?;
    Ok(bytes.to_vec())
}

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

fn to_output(e: crate::zoom::Inner) -> OutputElements {
    match e {
        crate::zoom::Inner::Scene(e) => OutputElements::Scene(e),
        crate::zoom::Inner::PointerTheme(e) => OutputElements::PointerTheme(e),
        crate::zoom::Inner::PointerSurface(e) => OutputElements::PointerSurface(e),
    }
}

/// A screen as it shows (unmagnified), the pointer at `pointer_at` (far
/// away to leave it out): its size in pixels, its scale, its pixels.
fn screen_pixels(
    state: &mut Server,
    renderer: &mut GlesRenderer,
    output: &Output,
    pointer_at: Point<f64, Logical>,
) -> Result<(usize, usize, f64, Vec<u8>), String> {
    let geo = state.space.output_geometry(output).ok_or("a screen without a place")?;
    let scale = output.current_scale().fractional_scale();
    let size = output.current_mode().map(|m| m.size).ok_or("a screen without a mode")?;
    // Behind Vergence Remote's curtain a screenshot is the remote person's
    // (theirs are the only hands on the session): the session, as their view
    // draws it, not the desk's lock (Dave, 2026-11-23).
    state.remote.drawing = state.lock.curtain;
    let elements: Vec<OutputElements> =
        screen_elements(state, renderer, output, geo, scale, pointer_at).into_iter().map(to_output).collect();
    state.remote.drawing = false;
    let clear = state.background_color(&output.name());
    let rgba = capture(renderer, &elements, size, scale, clear)?;
    Ok((size.w as usize, size.h as usize, scale, rgba))
}

/// A picture's width and height in pixels and its RGBA pixels.
type Pixels = (usize, usize, Vec<u8>);

/// Screenshots asked for (capture.rs), taken from what the screens show now.
fn run_captures(state: &mut Server, renderer: &mut GlesRenderer, pointer_at: Point<f64, Logical>) {
    use crate::capture::{Taken, What};
    if state.capture_pending.is_empty() {
        return;
    }
    let asked = std::mem::take(&mut state.capture_pending);
    state.capturing = true;
    let outputs: Vec<Output> = state.space.outputs().cloned().collect();
    let geo_of = |state: &Server, o: &Output| state.space.output_geometry(o).unwrap_or_default();
    for (req, paths) in asked {
        let p_at = if req.pointer { pointer_at } else { Point::from((-1.0e9, -1.0e9)) };
        let mut taken: Vec<Result<Pixels, String>> = Vec::new();
        match &req.what {
            What::Screen(name) => {
                let o = match name {
                    Some(n) => outputs.iter().find(|o| o.name() == *n),
                    None => outputs.iter().find(|o| geo_of(state, o).to_f64().contains(pointer_at)).or(outputs.first()),
                };
                taken.push(match o {
                    Some(o) => screen_pixels(state, renderer, o, p_at).map(|(w, h, _, px)| (w, h, px)),
                    None => Err("no such screen".into()),
                });
            }
            What::Screens => {
                for o in &outputs {
                    taken.push(screen_pixels(state, renderer, o, p_at).map(|(w, h, _, px)| (w, h, px)));
                }
            }
            What::Area(r) => {
                // Each screen's part as it shows, at the densest scale touched.
                let touched: Vec<&Output> = outputs.iter().filter(|o| geo_of(state, o).overlaps(*r)).collect();
                let d = touched.iter().map(|o| o.current_scale().fractional_scale()).fold(0.0f64, f64::max);
                if touched.is_empty() || r.size.w <= 0 || r.size.h <= 0 {
                    taken.push(Err("the area is on no screen".into()));
                } else {
                    let (cw, ch) = ((r.size.w as f64 * d).round() as usize, (r.size.h as f64 * d).round() as usize);
                    let mut canvas = vec![0u8; cw * ch * 4];
                    let mut failed = None;
                    for o in touched {
                        let geo = geo_of(state, o);
                        let Some(inter) = geo.intersection(*r) else { continue };
                        match screen_pixels(state, renderer, o, p_at) {
                            Ok((sw, sh, s, px)) => {
                                let x0 = (((inter.loc.x - geo.loc.x) as f64) * s).round() as usize;
                                let y0 = (((inter.loc.y - geo.loc.y) as f64) * s).round() as usize;
                                let w = ((inter.size.w as f64 * s).round() as usize).min(sw.saturating_sub(x0));
                                let h = ((inter.size.h as f64 * s).round() as usize).min(sh.saturating_sub(y0));
                                let mut part = Vec::with_capacity(w * h * 4);
                                for y in y0..y0 + h {
                                    part.extend_from_slice(&px[(y * sw + x0) * 4..(y * sw + x0 + w) * 4]);
                                }
                                let at = ((inter.loc.x - r.loc.x) as f64 * d).round() as i64;
                                let bt = ((inter.loc.y - r.loc.y) as f64 * d).round() as i64;
                                crate::capture::paste(&mut canvas, cw, ch, &part, w, h, at, bt, d / s);
                            }
                            Err(e) => failed = Some(e),
                        }
                    }
                    taken.push(match failed {
                        Some(e) => Err(e),
                        None => Ok((cw, ch, canvas)),
                    });
                }
            }
            What::Window(w) => taken.push(window_pixels(state, renderer, &outputs, w, req.shadow)),
        }
        // For a program that asked through the portal (portal_shot.rs).
        if let crate::capture::Dest::Portal(a) = &req.dest {
            let first = match taken.into_iter().next() {
                Some(Ok(p)) => Some(p),
                Some(Err(e)) => {
                    tracing::warn!("portal: the capture for {} failed: {e}", a.name);
                    None
                }
                None => None,
            };
            state.portal_shot_done(a, first);
            continue;
        }
        if matches!(req.dest, crate::capture::Dest::Clipboard) {
            match taken.into_iter().next() {
                // Made PNG on a thread, put on the clipboard on the main loop.
                Some(Ok((w, h, rgba))) => {
                    let (tx, rx) = smithay::reexports::calloop::channel::channel::<(Vec<u8>, Pixels, f64)>();
                    let _ = state.loop_handle.insert_source(rx, |ev, _, st| {
                        if let smithay::reexports::calloop::channel::Event::Msg((png, (tw, th, thumb), s)) = ev {
                            tracing::info!("screenshot: to the clipboard ({} KB)", png.len() >> 10);
                            st.server_copy_picture(png);
                            st.shot_show(None, tw, th, thumb, s);
                        }
                    });
                    let s = state.shot_screen().map_or(1.0, |(_, s)| s);
                    std::thread::spawn(move || match crate::capture::png_bytes(w as u32, h as u32, &rgba) {
                        Ok(png) => {
                            let thumb = crate::capture::thumb_pixels(w, h, &rgba, s);
                            let _ = tx.send((png, thumb, s));
                        }
                        Err(e) => tracing::warn!("screenshot to the clipboard: {e}"),
                    });
                }
                Some(Err(e)) => tracing::warn!("screenshot to the clipboard: {e}"),
                None => {}
            }
            continue;
        }
        // The first file's thumbnail floats once it is written (vctl's own
        // files, --to, have none).
        let s = state.shot_screen().map_or(1.0, |(_, s)| s);
        let thumbs = matches!(req.dest, crate::capture::Dest::Folder);
        for (i, (t, path)) in taken.into_iter().zip(paths).enumerate() {
            match t {
                Ok((w, h, rgba)) => {
                    let thumb = (thumbs && i == 0).then(|| crate::capture::thumb_pixels(w, h, &rgba, s));
                    let (tx, rx) = smithay::reexports::calloop::channel::channel::<(std::path::PathBuf, Pixels)>();
                    let _ = state.loop_handle.insert_source(rx, move |ev, _, st| {
                        if let smithay::reexports::calloop::channel::Event::Msg((p, (tw, th, px))) = ev {
                            st.shot_show(Some(p), tw, th, px, s);
                        }
                    });
                    crate::capture::save(Taken { w: w as u32, h: h as u32, rgba, path }, move |r| match r {
                        Ok(p) => {
                            tracing::info!("screenshot: {}", p.display());
                            if let Some(t) = thumb {
                                let _ = tx.send((p, t));
                            }
                        }
                        Err(e) => tracing::warn!("screenshot not saved: {e}"),
                    })
                }
                Err(e) => tracing::warn!("screenshot {}: {e}", path.display()),
            }
        }
    }
    state.capturing = false;
}

/// A window alone (its bar, border, corners and content) on a clear
/// ground, at the densest scale of the screens it is on; a soft shadow
/// around it if asked.
fn window_pixels(
    state: &mut Server,
    renderer: &mut GlesRenderer,
    outputs: &[Output],
    w: &crate::win::Win,
    shadow: bool,
) -> Result<Pixels, String> {
    use crate::{decor::SceneElement, overview::WinParts};
    use smithay::backend::renderer::element::{
        Element,
        utils::{Relocate, RelocateRenderElement, RescaleRenderElement},
    };
    let wgeo = state.space.element_geometry(w).ok_or("the window is not on a screen")?;
    let on: Vec<(&Output, i32)> = outputs
        .iter()
        .filter_map(|o| {
            state.space.output_geometry(o).and_then(|g| g.intersection(wgeo)).map(|i| (o, i.size.w * i.size.h))
        })
        .collect();
    let main = on.iter().max_by_key(|(_, a)| *a).map(|(o, _)| (*o).clone()).ok_or("the window is on no screen")?;
    let d = on.iter().map(|(o, _)| o.current_scale().fractional_scale()).fold(1.0f64, f64::max);
    state.capture_only = Some(w.clone());
    let els = state.scene(renderer, &main, d);
    state.capture_only = None;
    let k = smithay::utils::Scale::from(d);
    let bx = els
        .iter()
        .map(|e| e.geometry(k))
        .reduce(|a, b| a.merge(b))
        .filter(|b| b.size.w > 0 && b.size.h > 0)
        .ok_or("the window has nothing drawn")?;
    let placed: Vec<OutputElements> = els
        .into_iter()
        .filter_map(|e| {
            let part = match e {
                SceneElement::Surface(x) => WinParts::Surface(x),
                SceneElement::Solid(x) => WinParts::Solid(x),
                SceneElement::Memory(x) => WinParts::Memory(x),
                SceneElement::Texture(x) => WinParts::Texture(x),
                SceneElement::Shaded(x) => WinParts::Shaded(x),
                SceneElement::Backdrop(x) => WinParts::Backdrop(x),
                _ => return None,
            };
            Some(OutputElements::Scene(SceneElement::Placed(RelocateRenderElement::from_element(
                RescaleRenderElement::from_element(part, bx.loc, 1.0),
                Point::from((0, 0)) - bx.loc,
                Relocate::Relative,
            ))))
        })
        .collect();
    let rgba = capture(renderer, &placed, bx.size, d, [0.0, 0.0, 0.0, 0.0])?;
    let (cw, ch) = (bx.size.w as usize, bx.size.h as usize);
    Ok(if shadow { crate::capture::with_shadow(cw, ch, &rgba, (24.0 * d).round() as usize) } else { (cw, ch, rgba) })
}

/// Energy (energy.rs): every monitor off. The CRTC is disabled (DPMS off) and its
/// planes cleared; the monitor shows nothing and the GPU idles until screens_on.
pub fn screens_off(state: &mut Server) {
    let Some(b) = state.backend.as_mut() else { return };
    for s in b.surfaces.values_mut() {
        let name = s.connector.clone();
        if let Some(out) = s.drm()
            && let Err(e) = out.with_compositor(|c| c.clear())
        {
            tracing::warn!("{name}: cannot turn off: {e:?}");
        }
    }
}

/// Energy (energy.rs): every monitor back on, each with a full frame (the first
/// queued frame after a clear enables the CRTC again).
pub fn screens_on(state: &mut Server) {
    // The other GPUs' connectors, probed again: with the screens off such a GPU
    // has nothing to do and sleeps, and a sleeping GPU sees no cable come or go
    // (an NVIDIA card kept a monitor "connected" after it was unplugged with
    // the screens off). A forced probe wakes it, which takes
    // about half a second (the server stalled 0.45 s doing it here), so a thread
    // probes and the main loop scans once the card is awake.
    if let Some(b) = state.backend.as_ref() {
        for gpu in 1..b.devices.len() {
            let fd = b.devices[gpu].manager.device().device_fd().clone();
            let tx = b.probed.clone();
            let spawned = std::thread::Builder::new().name("gpu-probe".into()).spawn(move || {
                if let Ok(res) = fd.resource_handles() {
                    for c in res.connectors() {
                        let _ = fd.get_connector(*c, true);
                    }
                }
                let _ = tx.send(gpu);
            });
            if let Err(e) = spawned {
                tracing::warn!("gpu: cannot probe {} on waking: {e}", b.devices[gpu].path);
            }
        }
    }
    let waiting = state.backend.as_mut().map(|b| std::mem::take(&mut b.waiting)).unwrap_or_default();
    if !waiting.is_empty() {
        let anchors = state.window_anchors();
        let mut changed = false;
        for (gpu, handle, crtc) in waiting {
            let up = state.backend.as_ref().is_some_and(|b| b.surfaces.contains_key(&(gpu, crtc)));
            if !up {
                changed |= connect_if_there(state, gpu, handle, crtc);
            }
        }
        if changed {
            setup_changed(state, anchors);
        }
    }
    let crtcs: Vec<Key> = state
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
    /// A screen's modes and the index of the one running (u32::MAX: unknown).
    pub fn modes_of(&self, connector: &str) -> (Vec<vergence_protocol::bus::ModeInfo>, u32) {
        self.surfaces
            .values()
            .find(|s| s.connector == connector)
            .map_or((Vec::new(), u32::MAX), |s| (s.modes.clone(), s.mode))
    }

    /// The device path of the GPU that shows (or, `drawn`, draws) a screen.
    pub fn gpu_path(&self, connector: &str, drawn: bool) -> Option<String> {
        let (key, _) = self.surfaces.iter().find(|(_, s)| s.connector == connector)?;
        let gpu = if drawn { 0 } else { key.0 };
        self.devices.get(gpu).map(|d| d.path.clone())
    }

    /// Draw every monitor in full on its next frame (wallpaper or settings changed).
    pub fn redraw_all(&mut self) {
        for s in self.surfaces.values_mut() {
            s.reset_buffers();
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

// ---------------------------------------------------------------------------
// Remote views (remote milestone 1, step 1; docs/remote-milestone-1.md, "Frames,
// not display lists"). A remote view is a screen drawn again at a viewer's size
// and scale, each time the screen itself is drawn: its elements built afresh at
// the viewer's scale (so text is rendered at the viewer's resolution, and native
// windows are drawn again from their scenes at it, as for a window across two
// screens), into a texture of the view's own, with a damage tracker of its own.
// What changed is kept until the front end pulls it (the pull is the flow
// control: a slow viewer gets fewer, larger updates, never a queue). A view is
// no output: no wl_output, no place in the layout, never in window memory.

/// The most changed rectangles kept before they become one (the whole view).
const REMOTE_RECTS: usize = 64;

struct RemoteView {
    id: u32,
    /// Who is viewing, as the Dockworker's tile names them ("Control"), and since when.
    who: String,
    since: std::time::Instant,
    /// The screen it shows (its connector name).
    source: String,
    /// The viewer's size in pixels: the view fits the screen into it.
    want: (u32, u32),
    /// The scale it is drawn at, and its size in pixels (the screen's
    /// logical size times the scale); the texture is made at the first frame
    /// and again when either changes.
    scale: f64,
    px: smithay::utils::Size<i32, smithay::utils::Physical>,
    tex: Option<smithay::backend::renderer::gles::GlesTexture>,
    damage: Option<smithay::backend::renderer::damage::OutputDamageTracker>,
    /// Changed since the last pull; `whole` when everything is to be sent.
    pending: Vec<Rectangle<i32, smithay::utils::Physical>>,
    whole: bool,
    frames: u64,
    /// What the viewer has (RGBA, the view's size): a pull sends only what
    /// differs from it, so damage that changed no pixel (the dock's tiles
    /// redrawn every frame, 2026-11-23) costs nothing on the network.
    sent: Vec<u8>,
}

static REMOTE_IDS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// The scale that fits a screen of logical size `geo` into `want` pixels.
fn remote_fit(geo: smithay::utils::Size<i32, Logical>, want: (u32, u32)) -> f64 {
    let (gw, gh) = (geo.w.max(1) as f64, geo.h.max(1) as f64);
    (want.0 as f64 / gw).min(want.1 as f64 / gh).clamp(0.25, 8.0)
}

/// The views of the screen `connector` drawn again, each at its scale.
#[allow(clippy::too_many_arguments)]
fn remote_frames(
    state: &mut Server,
    renderer: &mut GlesRenderer,
    remotes: &mut [RemoteView],
    output: &Output,
    connector: &str,
    geo: Rectangle<i32, Logical>,
    pointer_at: Point<f64, Logical>,
    clear: [f32; 4],
) {
    use smithay::backend::renderer::{Bind, Offscreen};
    for r in remotes.iter_mut().filter(|r| r.source == connector) {
        let scale = remote_fit(geo.size, r.want);
        let px = smithay::utils::Size::<i32, smithay::utils::Physical>::from((
            (geo.size.w as f64 * scale).round() as i32,
            (geo.size.h as f64 * scale).round() as i32,
        ));
        if r.tex.is_none() || px != r.px || (scale - r.scale).abs() > 1e-9 {
            let buf = smithay::utils::Size::<i32, smithay::utils::Buffer>::from((px.w, px.h));
            match renderer.create_buffer(Fourcc::Abgr8888, buf) {
                Ok(t) => r.tex = Some(t),
                Err(e) => {
                    tracing::warn!("remote view {}: no texture: {e}", r.id);
                    continue;
                }
            }
            r.damage = Some(smithay::backend::renderer::damage::OutputDamageTracker::new(px, scale, Transform::Normal));
            r.scale = scale;
            r.px = px;
            r.whole = true;
            r.pending.clear();
            r.sent = Vec::new();
        }
        // Behind the curtain a remote view draws the session (decor.rs).
        state.remote.drawing = true;
        let elements: Vec<OutputElements> =
            screen_elements(state, renderer, output, geo, scale, pointer_at).into_iter().map(to_output).collect();
        state.remote.drawing = false;
        let (Some(tex), Some(damage)) = (r.tex.as_mut(), r.damage.as_mut()) else { continue };
        let mut fb = match renderer.bind(tex) {
            Ok(fb) => fb,
            Err(e) => {
                tracing::warn!("remote view {}: {e}", r.id);
                continue;
            }
        };
        match damage.render_output(renderer, &mut fb, 1, &elements, clear) {
            Ok(res) => {
                if let Some(d) = res.damage
                    && !d.is_empty()
                {
                    r.frames += 1;
                    if !r.whole {
                        for rect in d.iter() {
                            remote_add(&mut r.pending, *rect);
                        }
                        let area: i64 = r.pending.iter().map(|p| p.size.w as i64 * p.size.h as i64).sum();
                        if r.pending.len() > REMOTE_RECTS || area * 2 > r.px.w as i64 * r.px.h as i64 {
                            r.whole = true;
                            r.pending.clear();
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("remote view {}: {e:?}", r.id),
        }
    }
}

/// A changed rectangle added to those kept: nothing if one already covers
/// it; joined with any it overlaps (again, until none overlap), so a region
/// that changes every frame stays one rectangle, not one a frame.
fn remote_add(
    pending: &mut Vec<Rectangle<i32, smithay::utils::Physical>>,
    rect: Rectangle<i32, smithay::utils::Physical>,
) {
    if rect.size.w <= 0 || rect.size.h <= 0 || pending.iter().any(|p| p.contains_rect(rect)) {
        return;
    }
    let mut rect = rect;
    loop {
        let before = pending.len();
        pending.retain(|p| {
            if p.overlaps(rect) || rect.contains_rect(*p) {
                rect = rect.merge(*p);
                false
            } else {
                true
            }
        });
        if pending.len() == before {
            break;
        }
    }
    pending.push(rect);
}

/// The part of `rect` (its pixels `px`) that differs from what was sent,
/// trimmed to the rows and columns that changed, and `sent` brought up to
/// date; None when nothing in it changed.
fn remote_changed(
    sent: &mut [u8],
    stride: usize,
    rect: Rectangle<i32, smithay::utils::Physical>,
    px: &[u8],
) -> Option<RemoteRect> {
    let (x0, y0, w, h) = (rect.loc.x as usize, rect.loc.y as usize, rect.size.w as usize, rect.size.h as usize);
    let row_of = |y: usize| -> std::ops::Range<usize> {
        let at = (y0 + y) * stride + x0 * 4;
        at..at + w * 4
    };
    // The rows that changed, then the columns within them.
    let changed_row = |y: usize| sent[row_of(y)] != px[y * w * 4..(y + 1) * w * 4];
    let top = (0..h).find(|&y| changed_row(y))?;
    let bottom = (0..h).rev().find(|&y| changed_row(y)).unwrap_or(top);
    let mut left = w;
    let mut right = 0;
    for y in top..=bottom {
        let old = &sent[row_of(y)];
        let new = &px[y * w * 4..(y + 1) * w * 4];
        if let Some(l) = (0..w).find(|&x| old[x * 4..x * 4 + 4] != new[x * 4..x * 4 + 4]) {
            left = left.min(l);
            let r = (0..w).rev().find(|&x| old[x * 4..x * 4 + 4] != new[x * 4..x * 4 + 4]).unwrap_or(l);
            right = right.max(r);
        }
    }
    let (cw, ch) = (right + 1 - left, bottom + 1 - top);
    let mut part = Vec::with_capacity(cw * ch * 4);
    for y in top..=bottom {
        let src = &px[y * w * 4 + left * 4..y * w * 4 + (right + 1) * 4];
        part.extend_from_slice(src);
        let at = (y0 + y) * stride + (x0 + left) * 4;
        sent[at..at + cw * 4].copy_from_slice(src);
    }
    Some(((x0 + left) as u32, (y0 + top) as u32, cw as u32, ch as u32, part))
}

/// What a pull hands the front end: the view's size, and each changed
/// rectangle (x, y, width, height in the view's pixels) with its RGBA pixels.
pub struct RemoteFrame {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub rects: Vec<RemoteRect>,
}

/// A changed rectangle: x, y, width, height in the view's pixels, and its RGBA pixels.
pub type RemoteRect = (u32, u32, u32, u32, Vec<u8>);

/// A remote view of the screen `source` for a viewer of `want` pixels, for
/// `who` (named on the Dockworker's tile); its id.
pub fn remote_start(state: &mut Server, source: &str, want: (u32, u32), who: &str) -> Result<u32, String> {
    // A whole picture must fit one message uncompressed on the local leg
    // (vergence-remote's protocol caps messages at 128 MiB): 25 megapixels,
    // a 6K screen; larger viewers get a view that fits.
    if !(16..=16384).contains(&want.0) || !(16..=16384).contains(&want.1) {
        return Err("a viewer from 16x16 to 16384x16384 pixels".into());
    }
    let want = if want.0 as u64 * want.1 as u64 > 25_000_000 {
        let k = (25_000_000.0 / (want.0 as f64 * want.1 as f64)).sqrt();
        ((want.0 as f64 * k) as u32, (want.1 as f64 * k) as u32)
    } else {
        want
    };
    let Some(b) = state.backend.as_mut() else { return Err("no backend".into()) };
    let Some((key, _)) = b.surfaces.iter().find(|(_, s)| s.connector == source) else {
        return Err(format!("no screen named {source}"));
    };
    let key = *key;
    let id = REMOTE_IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let who: String = who.chars().filter(|c| !c.is_control()).take(40).collect();
    let who = if who.trim().is_empty() { "A remote viewer".to_string() } else { who.trim().to_string() };
    b.remotes.push(RemoteView {
        id,
        who,
        since: std::time::Instant::now(),
        source: source.to_string(),
        want,
        scale: 0.0,
        px: (0, 0).into(),
        tex: None,
        damage: None,
        pending: Vec::new(),
        whole: true,
        frames: 0,
        sent: Vec::new(),
    });
    tracing::info!("remote view {id}: {source} for a viewer of {}x{}", want.0, want.1);
    let first = b.remotes.len() == 1;
    remote_tile(state);
    render(state, key);
    if first {
        remote_clock(state);
    }
    Ok(id)
}

/// While there are remote views, every 16 ms: a view whose screen is not
/// being drawn (its monitor off, energy.rs, or blanked; or nothing changed
/// on it for a while) is drawn by itself, so the viewer never freezes with
/// the desk. Drawing a view alone presents nothing on the monitor. Stops
/// with the last view; not while the seat is another console's.
fn remote_clock(state: &mut Server) {
    let _ = state.loop_handle.insert_source(Timer::from_duration(Duration::from_millis(16)), |_, _, st| {
        let Some(b) = st.backend.as_ref() else { return TimeoutAction::Drop };
        if b.remotes.is_empty() {
            return TimeoutAction::Drop;
        }
        if !b.paused {
            let off = st.energy.screens_off;
            let idle: Vec<Key> = b
                .surfaces
                .iter()
                .filter(|(_, s)| b.remotes.iter().any(|r| r.source == s.connector))
                .filter(|(_, s)| off || s.last.elapsed() > Duration::from_millis(48))
                .map(|(k, _)| *k)
                .collect();
            for key in idle {
                remote_alone(st, key);
            }
            // A session with no monitors: its screen to the viewer's window,
            // once the window has kept a size for a moment (not at every step
            // of a drag).
            st.remote_follow_window();
            // Frames to the viewers that may take one (remote.rs).
            st.remote_service();
        }
        TimeoutAction::ToDuration(Duration::from_millis(16))
    });
}

/// The remote views of screen `key` drawn without the screen itself.
fn remote_alone(state: &mut Server, key: Key) {
    let Some(mut b) = state.backend.take() else { return };
    if let Some(s) = b.surfaces.get(&key) {
        let output = s.output.clone();
        let connector = s.connector.clone();
        if let Some(geo) = state.space.output_geometry(&output) {
            let pointer_at = state.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default();
            let clear = state.background_color(&connector);
            remote_frames(state, &mut b.renderer, &mut b.remotes, &output, &connector, geo, pointer_at, clear);
        }
    }
    state.backend = Some(b);
}

/// The view `id` resized for a viewer of `want` pixels.
pub fn remote_resize(state: &mut Server, id: u32, want: (u32, u32)) -> bool {
    let Some(b) = state.backend.as_mut() else { return false };
    match b.remotes.iter_mut().find(|r| r.id == id) {
        Some(r) => {
            r.want = want;
            true
        }
        None => false,
    }
}

pub fn remote_stop(state: &mut Server, id: u32) -> bool {
    let Some(b) = state.backend.as_mut() else { return false };
    let before = b.remotes.len();
    b.remotes.retain(|r| r.id != id);
    let stopped = b.remotes.len() != before;
    if stopped {
        tracing::info!("remote view {id}: stopped");
        remote_tile(state);
        // A viewer whose view this was is told (remote.rs).
        state.remote_service();
    }
    stopped
}

/// Every remote view ended (the Dockworker tile's End, from whoever is at
/// the desk).
pub fn remote_end_all(state: &mut Server) {
    let Some(b) = state.backend.as_mut() else { return };
    if b.remotes.is_empty() {
        return;
    }
    let n = b.remotes.len();
    b.remotes.clear();
    tracing::info!("remote views: {n} ended from the desk");
    remote_tile(state);
    // Their viewers told now: the views' clock stops with the last view.
    state.remote_service();
}

/// The Dockworker's tile while anyone views this computer (principle 8: who
/// is watching, and the desk can end it): who, which screens, since when,
/// and End. Taken down when the last view stops. Kept up to date by the
/// dock's tick.
pub fn remote_tile(state: &mut Server) {
    use vergence_protocol::tile::{Card, Control, ControlKind, Tile, TileState};
    let views: Vec<(String, String, u64)> = state
        .backend
        .as_ref()
        .map(|b| b.remotes.iter().map(|r| (r.who.clone(), r.source.clone(), r.since.elapsed().as_secs())).collect())
        .unwrap_or_default();
    let Some((who, _, secs)) = views.first().cloned() else {
        state.tile_remove(crate::tiles::Owner::Desktop, "remote");
        return;
    };
    let mut screens: Vec<String> = views.iter().map(|v| v.1.clone()).collect();
    screens.dedup();
    let mins = secs / 60;
    let long = if mins == 0 {
        "just now".to_string()
    } else if mins < 60 {
        format!("{mins} min")
    } else {
        format!("{} h {} min", mins / 60, mins % 60)
    };
    let others = views.iter().filter(|v| v.0 != who).count();
    let title = if others > 0 {
        format!("{who} and {others} more are viewing this computer")
    } else {
        format!("{who} is viewing this computer")
    };
    state.tile_set(
        crate::tiles::Owner::Desktop,
        Tile {
            id: "remote".into(),
            value: who.chars().take(12).collect(),
            label: "viewing".into(),
            state: TileState::Warning,
            group: "System".into(),
            icon: String::new(),
            badge: None,
            card: Card {
                title,
                big: String::new(),
                rows: vec![("Screens".into(), screens.join(", ")), ("Since".into(), long)],
                chart: Vec::new(),
                controls: vec![Control { id: "end".into(), label: "End".into(), kind: ControlKind::Button }],
                note: "Vergence Remote, over SSH. End stops it at once.".into(),
                picture: None,
            },
            page: String::new(),
        },
    );
}

/// A place in view `id` (its pixels) as a place on its screen (logical),
/// kept on the screen.
pub fn remote_to_logical(state: &Server, id: u32, x: f32, y: f32) -> Option<Point<f64, Logical>> {
    let b = state.backend.as_ref()?;
    let r = b.remotes.iter().find(|r| r.id == id)?;
    if r.scale <= 0.0 {
        return None;
    }
    let output = state.space.outputs().find(|o| o.name() == r.source)?;
    let geo = state.space.output_geometry(output)?;
    let lx = (x as f64 / r.scale).clamp(0.0, (geo.size.w - 1).max(0) as f64);
    let ly = (y as f64 / r.scale).clamp(0.0, (geo.size.h - 1).max(0) as f64);
    Some(Point::from((geo.loc.x as f64 + lx, geo.loc.y as f64 + ly)))
}

/// The remote views' ids.
pub fn remote_ids(state: &Server) -> Vec<u32> {
    state.backend.as_ref().map(|b| b.remotes.iter().map(|r| r.id).collect()).unwrap_or_default()
}

/// How many remote views there are.
pub fn remote_count(state: &Server) -> usize {
    state.backend.as_ref().map_or(0, |b| b.remotes.len())
}

/// Each view: its id, screen, size, scale and frames drawn.
pub fn remote_list(state: &Server) -> Vec<String> {
    state
        .backend
        .as_ref()
        .map(|b| {
            b.remotes
                .iter()
                .map(|r| {
                    format!("{} {} {}x{} scale {:.3} frames {}", r.id, r.source, r.px.w, r.px.h, r.scale, r.frames)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What changed in view `id` since the last pull, read back from its
/// texture; None for no such view, an empty list when nothing changed.
pub fn remote_pull(state: &mut Server, id: u32) -> Option<Result<RemoteFrame, String>> {
    let b = state.backend.as_mut()?;
    let r = b.remotes.iter_mut().find(|r| r.id == id)?;
    let Some(tex) = r.tex.as_mut() else {
        return Some(Ok(RemoteFrame { width: 0, height: 0, scale: r.scale, rects: Vec::new() }));
    };
    let rects: Vec<Rectangle<i32, smithay::utils::Physical>> =
        if r.whole { vec![Rectangle::from_size(r.px)] } else { std::mem::take(&mut r.pending) };
    r.whole = false;
    r.pending.clear();
    let bounds = Rectangle::<i32, smithay::utils::Physical>::from_size(r.px);
    let stride = r.px.w as usize * 4;
    if r.sent.len() != stride * r.px.h as usize {
        // Nothing sent yet at this size: everything differs.
        r.sent = vec![0; stride * r.px.h as usize];
    }
    let mut out = Vec::new();
    for rect in rects {
        let Some(rect) = rect.intersection(bounds) else { continue };
        if rect.size.w <= 0 || rect.size.h <= 0 {
            continue;
        }
        let px = match read_region(&mut b.renderer, tex, rect) {
            Ok(px) => px,
            Err(e) => return Some(Err(e)),
        };
        if let Some(changed) = remote_changed(&mut r.sent, stride, rect, &px) {
            out.push(changed);
        }
    }
    Some(Ok(RemoteFrame { width: r.px.w as u32, height: r.px.h as u32, scale: r.scale, rects: out }))
}

/// The view `id`'s whole picture (for the tests: a pull's rectangles must
/// match it).
pub fn remote_picture(state: &mut Server, id: u32) -> Option<Result<Pixels32, String>> {
    let b = state.backend.as_mut()?;
    let r = b.remotes.iter_mut().find(|r| r.id == id)?;
    let px = r.px;
    let tex = r.tex.as_mut()?;
    Some(read_texture(&mut b.renderer, tex, px).map(|v| (px.w as u32, px.h as u32, v)))
}

/// A picture's width and height and its RGBA pixels.
pub type Pixels32 = (u32, u32, Vec<u8>);

/// A rectangle of a texture's pixels, RGBA, rows from the top.
fn read_region(
    renderer: &mut GlesRenderer,
    tex: &mut smithay::backend::renderer::gles::GlesTexture,
    rect: Rectangle<i32, smithay::utils::Physical>,
) -> Result<Vec<u8>, String> {
    use smithay::backend::renderer::{Bind, ExportMem};
    let region = Rectangle::<i32, smithay::utils::Buffer>::new(
        (rect.loc.x, rect.loc.y).into(),
        (rect.size.w, rect.size.h).into(),
    );
    let fb = renderer.bind(tex).map_err(|e| e.to_string())?;
    let mapping = renderer.copy_framebuffer(&fb, region, Fourcc::Abgr8888).map_err(|e| e.to_string())?;
    let bytes = renderer.map_texture(&mapping).map_err(|e| e.to_string())?;
    Ok(bytes.to_vec())
}
