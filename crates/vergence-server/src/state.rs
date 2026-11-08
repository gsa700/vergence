// Based on Smithay's smallvil example (MIT; see LICENSE-smithay.txt), adapted for Vergence.
use std::{ffi::OsString, sync::Arc};

use smithay::{
    desktop::{PopupManager, Space, Window},
    input::{Seat, SeatState},
    reexports::{
        calloop::{EventLoop, Interest, LoopSignal, Mode, PostAction, generic::Generic},
        wayland_server::{
            Display, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Logical, Point},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::XdgShellState,
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};

/// Where a virtual output sits in the nested window, for mapping the pointer (1c).
pub struct OutputRegion {
    /// Its part of the host window, in window pixels.
    pub window: smithay::utils::Rectangle<f64, smithay::utils::Physical>,
    /// Its left edge in the server's logical space.
    pub logical_x: f64,
    pub scale: f64,
}

pub struct Server {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,

    pub space: Space<Window>,
    pub loop_signal: LoopSignal,

    // Smithay State
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Server>,
    pub data_device_state: DataDeviceState,
    pub viewporter_state: smithay::wayland::viewporter::ViewporterState,
    pub fractional_scale_manager_state: smithay::wayland::fractional_scale::FractionalScaleManagerState,
    pub cursor_shape_manager_state: smithay::wayland::cursor_shape::CursorShapeManagerState,
    pub xwayland_shell_state: smithay::wayland::xwayland_shell::XWaylandShellState,
    /// Xwayland's window manager, once Xwayland is ready.
    pub xwm: Option<smithay::xwayland::X11Wm>,
    pub xdisplay: Option<u32>,
    pub xdg_decoration_state: smithay::wayland::shell::xdg::decoration::XdgDecorationState,
    /// Title bars: per-window ids for damage tracking (decor.rs).
    pub decor: std::collections::HashMap<Window, crate::decor::Decor>,
    /// Where maximised windows go back to.
    pub restore: std::collections::HashMap<Window, smithay::utils::Rectangle<i32, Logical>>,
    /// The last press on a title bar, for double-click to maximise.
    pub last_title_press: Option<(Window, std::time::Instant)>,
    /// Monitors (connector names) still to capture for the control socket's `shot`.
    pub shot_pending: Vec<String>,
    /// The desktop font for title text (None: titles without text).
    pub title_font: Option<crate::text::TextFont>,
    /// Wayland windows already placed (place.rs: on their first sized commit).
    pub placed: std::collections::HashSet<Window>,
    /// When X11 windows were mapped (a move soon after is the app restoring its place).
    /// ... and the size the app asked for then (before fitting).
    pub mapped_at: std::collections::HashMap<Window, (std::time::Instant, smithay::utils::Size<i32, Logical>)>,
    /// linux-dmabuf (DRM backend): apps' GPU buffers.
    /// Where windows were, per monitor setup (memory.rs).
    pub memory: crate::memory::Memory,
    /// The wallpaper and its desktop.toml watch (wallpaper.rs).
    pub wallpaper: Option<crate::wallpaper::Wallpaper>,
    /// The desktop menu while it is open (menu.rs), and the button whose release
    /// it swallows.
    pub menu: Option<crate::menu::OpenMenu>,
    pub menu_swallow: Option<u32>,
    pub dmabuf: Option<(smithay::wayland::dmabuf::DmabufState, smithay::wayland::dmabuf::DmabufGlobal)>,
    /// The process exit status (control.rs: 75 = restart me).
    pub exit_code: i32,
    /// This server made the control socket (and removes it on exit).
    pub control_owned: bool,
    /// The session bus (bus.rs).
    pub bus: crate::bus::Bus,
    /// The D-Bus edge (dbus.rs).
    pub dbus: crate::dbus::DBus,
    /// The desktop settings (settings.rs).
    pub settings: crate::settings::SettingsState,
    pub popups: PopupManager,

    pub seat: Seat<Self>,
    /// The nested window's outputs, left to right (1c).
    pub regions: Vec<OutputRegion>,
    pub window_px: smithay::utils::Size<i32, smithay::utils::Physical>,
    /// Stage 2: the DRM backend and the libseat session (None when nested).
    pub backend: Option<crate::drm::Backend>,
    pub session: Option<smithay::backend::session::libseat::LibSeatSession>,
    pub loop_handle: smithay::reexports::calloop::LoopHandle<'static, Server>,
    /// The pointer image: themed shapes at each output's scale, or a client's surface.
    pub pointer_image: crate::cursor::Pointer,
}

impl Server {
    pub fn new(event_loop: &mut EventLoop<'static, Self>, display: Display<Self>) -> Self {
        let start_time = std::time::Instant::now();

        let dh = display.handle();

        // Here we initialize implementations of some wayland protocols
        // Some of them require us to implement traits on the Server state,
        // you can find those implementations in the `crate::handlers` module

        // Initialize protocols needed for displaying windows
        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let popups = PopupManager::default();

        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);

        // Data device is responsible for clipboard and drag-and-drop
        let data_device_state = DataDeviceState::new::<Self>(&dh);

        // Vergence (server milestone 1b): apps draw natively at a fractional scale.
        // wp-fractional-scale-v1 tells each surface the scale to draw at; wp-viewporter
        // lets it hand over a buffer of that many pixels for its logical size.
        let viewporter_state = smithay::wayland::viewporter::ViewporterState::new::<Self>(&dh);
        let fractional_scale_manager_state =
            smithay::wayland::fractional_scale::FractionalScaleManagerState::new::<Self>(&dh);
        // Apps name the pointer shape they want (wp-cursor-shape-v1); the server draws it
        // from the theme at each monitor's scale, so it is the same size everywhere.
        let cursor_shape_manager_state = smithay::wayland::cursor_shape::CursorShapeManagerState::new::<Self>(&dh);
        // X11 apps: Xwayland ties its X windows to Wayland surfaces with xwayland-shell-v1.
        let xwayland_shell_state = smithay::wayland::xwayland_shell::XWaylandShellState::new::<Self>(&dh);
        // Title bars by the server for apps that ask (decor.rs).
        let xdg_decoration_state = smithay::wayland::shell::xdg::decoration::XdgDecorationState::new::<Self>(&dh);

        // A seat is a group of keyboards, pointer and touch devices.
        // A seat typically has a pointer and maintains a keyboard focus and a pointer focus.
        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "winit");

        // Notify clients that we have a keyboard, for the sake of the example we assume that keyboard is always present.
        // You may want to track keyboard hot-plug in real compositor.
        seat.add_keyboard(Default::default(), 200, 25).unwrap();

        // Notify clients that we have a pointer (mouse)
        // Here we assume that there is always pointer plugged in
        seat.add_pointer();

        // A space represents a two-dimensional plane. Windows and Outputs can be mapped onto it.
        //
        // Windows get a position and stacking order through mapping.
        // Outputs become views of a part of the Space and can be rendered via Space::render_output.
        let space = Space::default();

        // Setup a wayland socket that will be used to accept clients
        let socket_name = Self::init_wayland_listener(display, event_loop);

        // Get the loop signal, used to stop the event loop
        let loop_signal = event_loop.get_signal();

        Self {
            start_time,
            display_handle: dh,

            space,
            loop_signal,
            socket_name,

            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            viewporter_state,
            fractional_scale_manager_state,
            cursor_shape_manager_state,
            xwayland_shell_state,
            xwm: None,
            xdisplay: None,
            xdg_decoration_state,
            decor: Default::default(),
            restore: Default::default(),
            last_title_press: None,
            shot_pending: Vec::new(),
            title_font: crate::text::TextFont::load(),
            placed: Default::default(),
            mapped_at: Default::default(),
            dmabuf: None,
            memory: crate::memory::Memory::load(),
            wallpaper: None,
            menu: None,
            menu_swallow: None,
            exit_code: 0,
            control_owned: false,
            bus: Default::default(),
            dbus: Default::default(),
            settings: Default::default(),
            popups,
            seat,
            regions: Vec::new(),
            window_px: (1, 1).into(),
            backend: None,
            session: None,
            loop_handle: event_loop.handle(),
            pointer_image: crate::cursor::Pointer::load(),
        }
    }

    fn init_wayland_listener(display: Display<Server>, event_loop: &mut EventLoop<Self>) -> OsString {
        // Creates a new listening socket, automatically choosing the next available `wayland` socket name.
        let listening_socket = ListeningSocketSource::new_auto().unwrap();

        // Get the name of the listening socket.
        // Clients will connect to this socket.
        let socket_name = listening_socket.socket_name().to_os_string();

        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                // Inside the callback, you should insert the client into the display.
                //
                // You may also associate some data with the client when inserting the client.
                state
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                    .unwrap();
            })
            .expect("Failed to init the wayland event source.");

        // You also need to add the display itself to the event loop, so that client events will be processed by wayland-server.
        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // Safety: we don't drop the display
                    unsafe {
                        display.get_mut().dispatch_clients(state).unwrap();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .unwrap();

        socket_name
    }

    /// The app surface under the pointer, if any (title bars and resize bands cover
    /// what is behind them); off a surface, the pointer shape for what it is over.
    pub fn surface_under(&mut self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        use crate::decor::{Hit, edge_icon};
        use smithay::input::pointer::{CursorImageStatus, CursorIcon};
        match self.hit(pos) {
            Hit::Surface(_, s, p) => return Some((s, p)),
            Hit::Edge(_, e) => self.pointer_image.status = CursorImageStatus::Named(edge_icon(e)),
            _ => self.pointer_image.status = CursorImageStatus::Named(CursorIcon::Default),
        }
        None
    }
}

impl Server {
    /// Rootless Xwayland for X11 apps. When it is ready: its window manager, the
    /// X default pointer, DISPLAY for children, then `ready` (starts the first app).
    pub fn start_xwayland(&mut self, ready: impl FnOnce() + 'static) {
        use smithay::xwayland::{X11Wm, XWayland, XWaylandEvent};
        // FreeBSD has no abstract sockets: the /tmp/.X11-unix socket only.
        let spawned = XWayland::spawn(
            &self.display_handle,
            None,
            std::iter::empty::<(String, String)>(),
            std::iter::empty::<String>(),
            cfg!(target_os = "linux"),
            std::process::Stdio::null(),
            std::process::Stdio::inherit(),
            |_| (),
        );
        let (xwayland, client) = match spawned {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!("Xwayland not started: {e}");
                return ready();
            }
        };
        let dh = self.display_handle.clone();
        let mut ready = Some(ready);
        let res = self.loop_handle.insert_source(xwayland, move |event, _, state| match event {
            XWaylandEvent::Ready { x11_socket, display_number } => {
                // One scale for all X clients: the densest monitor's (or a number from
                // VERGENCE_XWAYLAND_SCALE), so X apps draw sharp there and are scaled
                // down elsewhere (xsettings.rs).
                let densest = state.space.outputs().map(|o| o.current_scale().fractional_scale()).fold(1.0_f64, f64::max);
                let scale = match std::env::var("VERGENCE_XWAYLAND_SCALE").ok().as_deref() {
                    None | Some("max") => densest,
                    Some(v) => v.parse::<f64>().ok().filter(|s| *s > 0.0).unwrap_or(densest),
                };
                {
                    use smithay::wayland::compositor::CompositorHandler;
                    state.client_compositor_state(&client).set_client_scale(scale);
                }
                // Xwayland bound the outputs before it had a client scale; an empty
                // state change re-sends them in its pixels (14": 3840 wide, not 1170).
                for o in state.space.outputs() {
                    o.change_current_state(None, None, None, None);
                }
                match X11Wm::start_wm(state.loop_handle.clone(), &dh, x11_socket, client.clone()) {
                    Ok(mut wm) => {
                        let px = (state.pointer_image.size_points() as f64 * scale).round() as u32;
                        if let Some((pixels, w, h, hx, hy)) = state.pointer_image.x_default(px)
                            && let Err(e) = wm.set_cursor(&pixels, (w, h).into(), (hx, hy).into())
                        {
                            tracing::warn!("Xwayland default pointer: {e}");
                        }
                        state.xwm = Some(wm);
                    }
                    Err(e) => tracing::warn!("Xwayland window manager: {e}"),
                }
                state.xdisplay = Some(display_number);
                crate::xsettings::publish(display_number, scale, state.pointer_image.size_points(), &state.pointer_image.theme_name);
                unsafe {
                    std::env::set_var("DISPLAY", format!(":{display_number}"));
                    // Avalonia takes its scale from Xft.dpi; its env factors MULTIPLY
                    // that (AVALONIA_GLOBAL_SCALE_FACTOR=3.28 gave 10.77, 2026-11-07).
                    std::env::remove_var("AVALONIA_GLOBAL_SCALE_FACTOR");
                    std::env::remove_var("AVALONIA_SCREEN_SCALE_FACTORS");
                }
                tracing::info!("Xwayland ready on DISPLAY=:{display_number}, X client scale {scale}");
                if let Some(r) = ready.take() {
                    r();
                }
            }
            XWaylandEvent::Error => {
                tracing::warn!("Xwayland failed to start");
                if let Some(r) = ready.take() {
                    r();
                }
            }
        });
        if let Err(e) = res {
            tracing::warn!("Xwayland event source: {e}");
        }
    }
}

/// Data associated with a wayland client that connects to Server.
/// One instance of this type per client.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
