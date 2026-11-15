// The pointer through the seat (seat milestone 1, step 2; docs/seat-milestone-1.md).
//
// The seat (crates/vergence-seat) decides who gets each pointer event; this
// file tells it what is under the pointer and carries out what it decides,
// after it has decided:
//
// - What is under a point is a `Target`: the open menu or apps popup above
//   everything, then a notification card, then the dock and the windows
//   (decor.rs's hit test, front to back), else the bare desktop. Windows
//   and surfaces are given numbers here for the seat, which knows no
//   Smithay types.
// - A Wayland or X11 surface hears the pointer through Smithay's pointer,
//   called with the focus the seat chose. A native window hears it over the
//   bus (native.rs). The server's own parts act on presses here.
// - Moving and resizing a window are ours (they replaced smallvil's grabs):
//   a maximized window being moved leaves maximized once it is really
//   dragged (DRAG points), a resize keeps the window's own minimum and
//   maximum size, and a Wayland window resized from its top or left edge
//   is kept anchored at the opposite edge as its new size arrives.
// - A Wayland drag and drop is Smithay's grab while it lasts (the seat's
//   Edge grab): its events go to Smithay as they came.

use std::collections::HashMap;

use smithay::{
    backend::input::{ButtonState, InputTime},
    input::pointer::{AxisFrame, ButtonEvent, CursorIcon, CursorImageStatus, MotionEvent},
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{Resource, backend::ObjectId, protocol::wl_surface::WlSurface},
    },
    utils::{IsAlive, Logical, Point, Rectangle, SERIAL_COUNTER, Size},
};
use vergence_protocol::window::WinEvent;
use vergence_seat::{Edges, Out, Part, Target, WinId};

use crate::{decor::Hit, state::Server, win::Win};

bitflags::bitflags! {
    /// The edges a resize moves (the same bits as the seat's `Edges`, the
    /// native protocol's and xdg_toplevel's).
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct ResizeEdge: u32 {
        const TOP = 1;
        const BOTTOM = 2;
        const LEFT = 4;
        const RIGHT = 8;
        const TOP_LEFT = 5;
        const BOTTOM_LEFT = 6;
        const TOP_RIGHT = 9;
        const BOTTOM_RIGHT = 10;
    }
}

impl From<xdg_toplevel::ResizeEdge> for ResizeEdge {
    fn from(e: xdg_toplevel::ResizeEdge) -> Self {
        ResizeEdge::from_bits_truncate(e as u32)
    }
}

impl From<ResizeEdge> for Edges {
    fn from(e: ResizeEdge) -> Self {
        Edges(e.bits() as u8)
    }
}

impl From<Edges> for ResizeEdge {
    fn from(e: Edges) -> Self {
        ResizeEdge::from_bits_truncate(e.0 as u32)
    }
}

/// How far the pointer moves a maximized window's title bar before the
/// window leaves maximized: a press alone does not (a double-click there
/// would restore it and maximize it again).
const DRAG: f64 = 6.0;

/// A move or resize in progress.
enum Running {
    Move { window: Win, from: Point<f64, Logical>, loc: Point<i32, Logical>, unmaximize_on_drag: bool },
    Resize { window: Win, edges: ResizeEdge, from: Point<f64, Logical>, initial: Rectangle<i32, Logical>, size: Size<i32, Logical> },
}

/// A press on a tab (bar milestone 1, step 7): the window, the tab's place in
/// the row, where the press was; past vergence-bar's DRAG_START it is a drag,
/// and where the tab would land if let go now.
struct TabHold {
    window: Win,
    index: usize,
    from: Point<f64, Logical>,
    drag: Option<vergence_bar::TabDrag>,
    landing: Option<vergence_bar::Landing>,
}

/// What came in, for the outputs that pass it on as it came.
pub enum Came {
    Motion,
    Button { button: u32, state: ButtonState },
    Axis { frame: AxisFrame, source: u8, dx: f64, dy: f64, v120x: i32, v120y: i32 },
    /// Something the server decided by itself (a program's request, a window gone).
    Nothing,
}

#[derive(Default)]
pub struct Dispatch {
    pub seat: vergence_seat::Seat,
    next: u64,
    pub(crate) wins: HashMap<WinId, Win>,
    ids: HashMap<Win, WinId>,
    /// Surfaces by number: the surface and where its origin was when last hit.
    surfaces: HashMap<u64, (WlSurface, Point<f64, Logical>)>,
    surface_ids: HashMap<ObjectId, u64>,
    running: Option<Running>,
    /// Buttons Smithay's pointer has seen go down and not up.
    smithay_held: Vec<u32>,
    /// A left press on a tab of a native window's bar, and the drag it may become.
    tab_hold: Option<TabHold>,
    /// A window being moved by its only tab (as a browser's): let go over
    /// another window's bar of the same program, the tab joins it.
    tab_move: Option<(Win, usize)>,
    /// A Wayland window resized from its top or left: kept anchored as its
    /// commits arrive, until the one after the resize ended.
    anchored: Option<(Win, ResizeEdge, Rectangle<i32, Logical>, bool)>,
    /// Our own xkb state (keys.rs), from Smithay's keymap before the first key.
    pub keys: Option<crate::keys::Keys>,
    /// The native window that has the keyboard (Smithay's focus is none then).
    pub keyboard_native: Option<Win>,
    /// Alt+Tab's switching in progress (switch.rs).
    pub switching: Option<crate::switch::Switching>,
    /// When a key was last typed into a window (opened.rs: a window that opens
    /// after it does not take the keyboard from that window).
    pub typed_at: Option<std::time::Instant>,
}

impl Dispatch {
    /// The window the seat calls `id`.
    pub fn window(&self, id: WinId) -> Option<Win> {
        self.wins.get(&id).cloned()
    }
}

impl Server {
    /// A window is being moved by the pointer (snap.rs shows its aim only then).
    pub fn is_moving(&self) -> bool {
        matches!(self.input.running, Some(Running::Move { .. }))
    }

    /// The seat's number for a window.
    pub fn win_id(&mut self, w: &Win) -> WinId {
        if let Some(id) = self.input.ids.get(w) {
            return *id;
        }
        self.input.next += 1;
        let id = self.input.next;
        self.input.ids.insert(w.clone(), id);
        self.input.wins.insert(id, w.clone());
        id
    }

    fn surface_id(&mut self, s: &WlSurface, origin: Point<f64, Logical>) -> u64 {
        let id = match self.input.surface_ids.get(&s.id()) {
            Some(id) => *id,
            None => {
                self.input.next += 1;
                self.input.surface_ids.insert(s.id(), self.input.next);
                self.input.next
            }
        };
        self.input.surfaces.insert(id, (s.clone(), origin));
        id
    }

    /// Before each event: the seat knows which menu is open, and forgets
    /// windows and surfaces that have gone.
    pub fn dispatch_sync(&mut self) {
        // A menu open over the launcher (an application's menu there) comes
        // first: a press on it is the menu's, not one outside the launcher.
        let menu = if self.app_menu_is_open() {
            Some(vergence_seat::Menu::AppMenu)
        } else if self.menu_is_open() {
            Some(vergence_seat::Menu::Menu)
        } else if self.launcher_is_open() {
            Some(vergence_seat::Menu::Popup)
        } else {
            None
        };
        match menu {
            Some(m) => self.input.seat.menu_opened(m),
            None => self.input.seat.menu_closed(),
        }
        let gone: Vec<WinId> = self.input.wins.iter().filter(|(_, w)| !w.alive()).map(|(id, _)| *id).collect();
        for id in gone {
            if let Some(w) = self.input.wins.remove(&id) {
                self.input.ids.remove(&w);
            }
            let out = self.input.seat.window_gone(id);
            let at = self.pointer_location();
            self.apply(out, at, InputTime::now(), &Came::Nothing);
        }
        let dead: Vec<u64> = self.input.surfaces.iter().filter(|(_, (s, _))| !s.alive()).map(|(id, _)| *id).collect();
        for id in dead {
            if let Some((s, _)) = self.input.surfaces.remove(&id) {
                self.input.surface_ids.remove(&s.id());
            }
        }
        if self.input.running.as_ref().is_some_and(|r| !match r {
            Running::Move { window, .. } | Running::Resize { window, .. } => window.alive(),
        }) {
            self.input.running = None;
        }
    }

    pub fn pointer_location(&self) -> Point<f64, Logical> {
        self.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default()
    }

    /// What is under `pos`, for the seat; and the pointer's picture there.
    pub fn target_at(&mut self, pos: Point<f64, Logical>) -> Option<Target> {
        if self.app_menu_contains(pos) {
            return Some(Target::Server(Part::AppList));
        }
        if self.menu_contains(pos) {
            return Some(Target::Server(Part::Menu));
        }
        if self.launcher_contains(pos) {
            return Some(Target::Server(Part::Popup));
        }
        if self.switcher_contains(pos) {
            return Some(Target::Server(Part::Switcher));
        }
        // Anything of a window a modal dialog blocks is the dialog's (dialogs.rs).
        let hit_window = match self.hit(pos) {
            Hit::Surface(w, ..) | Hit::Native(w) | Hit::Title(w) | Hit::Close(w) | Hit::Maximize(w) | Hit::Minimize(w) | Hit::Edge(w, _) | Hit::Tab(w, _) | Hit::CloseTab(w, _) | Hit::Plus(w) | Hit::Tool(w, _) | Hit::AppMenu(w) => Some(w),
            _ => None,
        };
        if let Some(modal) = hit_window.and_then(|w| self.modal_of(&w)) {
            return Some(Target::Server(Part::Blocked(self.win_id(&modal))));
        }
        if self.notify_at(pos) {
            return Some(Target::Server(Part::Note));
        }
        Some(match self.hit(pos) {
            Hit::Surface(w, s, origin) => Target::Surface(self.win_id(&w), self.surface_id(&s, origin)),
            Hit::Native(w) => Target::Native(self.win_id(&w)),
            Hit::Title(w) => Target::Server(Part::Title(self.win_id(&w))),
            Hit::Close(w) => Target::Server(Part::Close(self.win_id(&w))),
            Hit::Maximize(w) => Target::Server(Part::Maximize(self.win_id(&w))),
            Hit::Minimize(w) => Target::Server(Part::Minimize(self.win_id(&w))),
            Hit::Edge(w, e) => Target::Server(Part::Edge(self.win_id(&w), e.into())),
            Hit::Tab(w, i) => Target::Server(Part::Tab(self.win_id(&w), i as u32)),
            Hit::CloseTab(w, i) => Target::Server(Part::CloseTab(self.win_id(&w), i as u32)),
            Hit::Plus(w) => Target::Server(Part::Plus(self.win_id(&w))),
            Hit::Tool(w, i) => Target::Server(Part::Tool(self.win_id(&w), i as u32)),
            Hit::AppMenu(w) => Target::Server(Part::AppMenu(self.win_id(&w))),
            Hit::Shelf => Target::Server(Part::Dock),
            Hit::Nothing => Target::Server(Part::Desktop),
        })
    }

    /// The surface a seat number names, with its origin when last hit.
    pub(crate) fn surface_of(&self, sid: u64) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.input.surfaces.get(&sid).cloned()
    }

    /// The pointer's picture for what it is on: a window edge's arrows, a
    /// native window's own shape; a Wayland surface sets its own.
    fn pointer_picture(&mut self, under: Option<Target>) {
        let icon = match under {
            Some(Target::Surface(..)) => return,
            Some(Target::Server(Part::Edge(_, e))) => crate::decor::edge_icon(e.into()),
            Some(Target::Native(id)) => self.input.wins.get(&id).and_then(|w| w.native()).map(|n| n.cursor()).unwrap_or(CursorIcon::Default),
            _ => CursorIcon::Default,
        };
        self.pointer_image.status = CursorImageStatus::Named(icon);
    }

    /// The pointer at `pos` (logical): hover, focus, picture, and motion to
    /// what has it. The mouse, a pointing finger and `vctl pointer` come here.
    pub fn pointer_to(&mut self, pos: Point<f64, Logical>, time: InputTime) {
        self.dispatch_sync();
        self.tab_hold_check();
        self.native_drag_check();
        self.pointer_hover(pos);
        let under = self.target_at(pos);
        self.pointer_follow(pos, time, under);
        let out = self.input.seat.motion(under);
        self.apply(out, pos, time, &Came::Motion);
        // A native drag follows the pointer beside the seat (drag.rs).
        if self.native_drag.is_some() {
            self.native_drag_motion(pos);
        }
    }

    /// Hover: each of the server's parts looks for itself.
    pub(crate) fn pointer_hover(&mut self, pos: Point<f64, Logical>) {
        self.menu_hover(pos);
        self.app_menu_hover(pos);
        self.notify_hover(pos);
        self.shelf_hover(pos);
        self.launcher_hover(pos);
    }

    /// The pointer's picture, and Smithay's pointer where the pointer is.
    pub(crate) fn pointer_follow(&mut self, pos: Point<f64, Logical>, time: InputTime, under: Option<Target>) {
        let (grab, held, focus) = (self.input.seat.grab(), self.input.seat.buttons_held(), self.input.seat.pointer_focus());
        if grab.is_none() && !held {
            self.pointer_picture(under);
        }
        // Smithay's pointer follows every motion (the pointer is drawn where it
        // is). When the seat sends this motion to a surface, or Smithay's drag
        // has it, that is the call; otherwise its focus is none, which a
        // surface hears as leave only if it had it (the seat's Leave says so first).
        let to_surface = match grab {
            Some(vergence_seat::Grab::Edge) => true,
            Some(_) => false,
            None => matches!(focus, Some(Target::Surface(..))) || (!held && matches!(under, Some(Target::Surface(..)))),
        };
        if !to_surface && let Some(p) = self.seat.get_pointer() {
            p.motion(self, None, &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time });
            p.frame(self);
        }
    }

    /// A button pressed or released. The mouse, a pointing finger and `vctl
    /// press`/`release` come here.
    pub fn handle_button(&mut self, button: u32, state: ButtonState, time: InputTime) {
        self.dispatch_sync();
        self.tab_hold_check();
        let pos = self.pointer_location();
        // The left button let go during a native drag: dropped (drag.rs); the
        // seat still hears the release, so the source's press ends too.
        if state == ButtonState::Released && button == vergence_seat::BTN_LEFT && self.native_drag.is_some() {
            self.native_drag_drop(pos);
        }
        let under = self.target_at(pos);
        // A right press on the desktop's icons' window is the desktop's menu, as on the wallpaper.
        if state == ButtonState::Pressed
            && button == vergence_seat::BTN_RIGHT
            && let Some(Target::Native(id)) = under
            && self.input.wins.get(&id).is_some_and(|w| self.is_desktop(w))
        {
            self.open_menu(pos);
            return;
        }
        let mods = self.seat.get_keyboard().map(|k| k.modifier_state()).map(|m| vergence_seat::keyboard::Mods { ctrl: m.ctrl, alt: m.alt, shift: m.shift, logo: m.logo }).unwrap_or_default();
        let now = self.start_time.elapsed().as_millis() as u64;
        let out = self.input.seat.button(button, state == ButtonState::Pressed, under, mods, now);
        self.apply(out, pos, time, &Came::Button { button, state });
    }

    /// A wheel or a touchpad's scroll.
    pub fn pointer_axis(&mut self, frame: AxisFrame, source: u8, dx: f64, dy: f64, v120x: i32, v120y: i32, time: InputTime) {
        self.dispatch_sync();
        let pos = self.pointer_location();
        let under = self.target_at(pos);
        let out = self.input.seat.axis(under);
        self.apply(out, pos, time, &Came::Axis { frame, source, dx, dy, v120x, v120y });
    }

    /// A program asked to be moved (`edges` None) or resized with the press
    /// it was given: granted by the seat only while a button is held on it.
    pub fn grab_request(&mut self, window: &Win, edges: Option<ResizeEdge>) -> bool {
        self.dispatch_sync();
        let id = self.win_id(window);
        let out = self.input.seat.grab_from_press(id, edges.map(Edges::from));
        if out.is_empty() {
            return false;
        }
        let pos = self.pointer_location();
        self.apply(out, pos, InputTime::now(), &Came::Nothing);
        true
    }

    /// A Wayland program began a drag and drop: Smithay's grab has the pointer.
    pub fn drag_began(&mut self) {
        let out = self.input.seat.edge_begin();
        // Smithay's drag grab tells the surfaces itself; nothing to apply but our own.
        for o in out {
            if let Out::Leave(Target::Native(id)) = o
                && let Some(w) = self.input.wins.get(&id).cloned()
            {
                self.native_leave(&w);
            }
        }
        self.input.smithay_held.clear();
    }

    /// Carry out what the seat decided, in order.
    pub(crate) fn apply(&mut self, out: Vec<Out>, pos: Point<f64, Logical>, time: InputTime, came: &Came) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        for o in out {
            match o {
                Out::Motion(Target::Server(Part::Tab(..))) if self.input.tab_hold.is_some() => self.tab_motion(pos),
                Out::Enter(t) | Out::Motion(t) => match t {
                    Target::Surface(_, sid) => {
                        let focus = self.input.surfaces.get(&sid).cloned();
                        pointer.motion(self, focus, &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time });
                        pointer.frame(self);
                    }
                    Target::Native(id) => {
                        if let Some(w) = self.input.wins.get(&id).cloned() {
                            if matches!(o, Out::Enter(_)) {
                                self.native_enter(&w, pos);
                            } else {
                                self.native_motion(&w, pos, time.millis());
                            }
                        }
                    }
                    Target::Server(_) => {}
                },
                Out::Leave(t) => match t {
                    Target::Surface(..) => {
                        pointer.motion(self, None, &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time });
                        pointer.frame(self);
                    }
                    Target::Native(id) => {
                        if let Some(w) = self.input.wins.get(&id).cloned() {
                            self.native_leave(&w);
                        }
                    }
                    Target::Server(_) => {}
                },
                Out::Press(t, b) | Out::Release(t, b) => {
                    let pressed = matches!(o, Out::Press(..));
                    self.deliver_button(t, b, pressed, pos, time);
                }
                Out::Axis(t) => {
                    let Came::Axis { frame, source, dx, dy, v120x, v120y } = came else { continue };
                    match t {
                        Target::Surface(..) => {
                            pointer.axis(self, frame.clone());
                            pointer.frame(self);
                        }
                        Target::Native(id) => {
                            if let Some(w) = self.input.wins.get(&id).cloned() {
                                self.native_axis(&w, time.millis(), *source, *dx, *dy, *v120x, *v120y);
                            }
                        }
                        Target::Server(Part::Popup) => self.launcher_scroll(*dy),
                        Target::Server(_) => {}
                    }
                }
                Out::Focus(id) => {
                    if let Some(w) = self.input.wins.get(&id).cloned() {
                        self.focus_window(&w, SERIAL_COUNTER.next_serial());
                    }
                }
                Out::Unfocus => self.unfocus_all(),
                Out::SwitchCancel => self.input.switching = None,
                Out::WindowMenu { window, on_button } => {
                    if let Some(w) = self.input.wins.get(&window).cloned() {
                        self.open_window_menu(&w, on_button);
                        self.redraw_all();
                    }
                }
                Out::Maximize(id) => {
                    if let Some(w) = self.input.wins.get(&id).cloned() {
                        self.toggle_maximize(&w);
                    }
                }
                Out::Close(id) => {
                    if let Some(w) = self.input.wins.get(&id).cloned() {
                        self.close_window(&w);
                    }
                }
                Out::Minimize(id) => {
                    if let Some(w) = self.input.wins.get(&id).cloned() {
                        self.minimize(&w);
                    }
                }
                Out::MoveBegin(id) => {
                    if let Some(w) = self.input.wins.get(&id).cloned() {
                        self.begin_move(w, pos, time);
                    }
                }
                Out::ResizeBegin(id, edges) => {
                    if let Some(w) = self.input.wins.get(&id).cloned() {
                        let edges = edges.map(ResizeEdge::from).unwrap_or_else(|| self.nearest_edges(&w, pos));
                        self.begin_resize(w, edges, pos, time);
                    }
                }
                Out::GrabMotion => self.grab_motion(pos),
                Out::GrabEnd(_) => self.grab_end(time),
                Out::AppMenu(id) => {
                    if let Some(w) = self.input.wins.get(&id).cloned() {
                        self.app_menu_toggle(&w, false);
                    }
                }
                Out::CloseMenu => {
                    self.app_menu_close();
                    self.close_menu();
                    self.launcher_close();
                }
                Out::ToEdge => self.to_edge(pos, time, came),
                // Touch's own (touch.rs) are carried out there.
                _ => {}
            }
        }
    }

    /// A press or release, to whatever the seat said.
    fn deliver_button(&mut self, t: Target, button: u32, pressed: bool, pos: Point<f64, Logical>, time: InputTime) {
        let serial = SERIAL_COUNTER.next_serial();
        // A tab held: the left button's release ends it, wherever the seat sends it.
        if !pressed && button == vergence_seat::BTN_LEFT && self.input.tab_hold.is_some() && !matches!(t, Target::Server(Part::Tab(..))) {
            tracing::info!("tab: let go over {t:?}, not the tab's bar");
            self.tab_release(pos);
            return;
        }
        match t {
            Target::Surface(..) => {
                let Some(pointer) = self.seat.get_pointer() else { return };
                if pressed {
                    self.input.smithay_held.push(button);
                } else {
                    self.input.smithay_held.retain(|b| *b != button);
                }
                let state = if pressed { ButtonState::Pressed } else { ButtonState::Released };
                pointer.button(self, &ButtonEvent { button, state, serial, time });
                pointer.frame(self);
            }
            Target::Native(id) => {
                if let Some(w) = self.input.wins.get(&id).cloned() {
                    self.native_button(&w, button, pressed, serial.into(), time.millis());
                }
            }
            Target::Server(Part::Menu) if pressed => self.menu_press(pos),
            Target::Server(Part::AppList) if pressed => self.app_menu_press(pos, button),
            Target::Server(Part::Popup) if pressed && button == vergence_seat::BTN_RIGHT => self.launcher_menu(pos),
            Target::Server(Part::Popup) if pressed => self.launcher_press(pos),
            Target::Server(Part::Popup) => self.launcher_release(pos),
            Target::Server(Part::Note) if pressed => self.notify_press(pos),
            // The dock: a left press is a click or the start of a drag, let go where
            // it ends; a right press is an icon's menu.
            Target::Server(Part::Dock) if pressed && button == vergence_seat::BTN_LEFT => self.shelf_press(pos, button),
            Target::Server(Part::Dock) if pressed && button == vergence_seat::BTN_RIGHT => self.shelf_menu(pos),
            Target::Server(Part::Dock) if !pressed && self.shelf.held.is_some_and(|(b, _)| b == button) => self.shelf_release(pos),
            Target::Server(Part::Desktop) if pressed && button == vergence_seat::BTN_RIGHT => self.open_menu(pos),
            Target::Server(Part::Switcher) if pressed && button == vergence_seat::BTN_LEFT => self.switch_click(pos),
            // A program's own bar button (window styles milestone 1): the program hears it.
            Target::Server(Part::Tool(id, i)) if pressed && button == vergence_seat::BTN_LEFT => {
                if let Some(Win::Native(n)) = self.input.wins.get(&id).cloned()
                    && let Some(tool) = self.bar_tool_id(&Win::Native(n.clone()), i as usize)
                {
                    self.bus_send(n.client, vergence_protocol::bus::ServerMsg::Window(WinEvent::BarButton { window: n.id, id: tool }));
                }
            }
            Target::Server(Part::Blocked(id)) if pressed => {
                if let Some(d) = self.input.wins.get(&id).cloned() {
                    self.flash_dialog(&d);
                }
            }
            // A native window's tabs (bar milestone 1): the program hears what was done.
            Target::Server(Part::Tab(id, i)) if pressed && button == vergence_seat::BTN_LEFT => {
                self.tab_event(id, i, |window, tab| WinEvent::TabChosen { window, tab });
                if let Some(w) = self.input.wins.get(&id).cloned() {
                    self.input.tab_hold = Some(TabHold { window: w, index: i as usize, from: pos, drag: None, landing: None });
                }
            }
            Target::Server(Part::Tab(_, _)) if !pressed && button == vergence_seat::BTN_LEFT => self.tab_release(pos),
            Target::Server(Part::Tab(id, i)) if pressed && button == vergence_seat::BTN_RIGHT => {
                if let Some(w) = self.input.wins.get(&id).cloned()
                    && let (Some((bar, layout)), Some(tab)) = (self.bar_of(&w), self.tab_id(&w, i as usize))
                    && let Some(r) = layout.tabs.get(i as usize)
                {
                    let slot = Rectangle::new((bar.loc.x + r.x as i32, bar.loc.y + r.y as i32).into(), (r.w as i32, r.h as i32).into());
                    self.open_tab_menu(&w, tab, slot);
                }
            }
            Target::Server(Part::Tab(id, i)) if pressed && button == vergence_seat::BTN_MIDDLE => self.tab_event(id, i, |window, tab| WinEvent::TabClose { window, tab }),
            Target::Server(Part::CloseTab(id, i)) if pressed && button == vergence_seat::BTN_LEFT => self.tab_event(id, i, |window, tab| WinEvent::TabClose { window, tab }),
            Target::Server(Part::Plus(id)) if pressed && button == vergence_seat::BTN_LEFT => {
                if let Some(Win::Native(n)) = self.input.wins.get(&id).cloned() {
                    self.bus_send(n.client, vergence_protocol::bus::ServerMsg::Window(WinEvent::TabNew { window: n.id }));
                }
            }
            Target::Server(_) => {}
        }
    }

    /// The pointer moved with a tab held: past DRAG_START it is a drag, and
    /// the bar shows where the tab would land (decor.rs, tab_preview).
    fn tab_motion(&mut self, pos: Point<f64, Logical>) {
        let Some(hold) = self.input.tab_hold.as_ref() else { return };
        let Some((bar, layout)) = self.bar_of(&hold.window) else { return };
        let (lx, ly) = ((pos.x - bar.loc.x as f64) as f32, (pos.y - bar.loc.y as f64) as f32);
        let moved = (pos.x - hold.from.x).hypot(pos.y - hold.from.y) as f32;
        // A window's only tab moves the window (Dave, 2026-11-12).
        if hold.drag.is_none() && moved >= vergence_bar::DRAG_START && layout.tabs.len() == 1 {
            let Some(hold) = self.input.tab_hold.take() else { return };
            if self.grab_request(&hold.window, None) {
                // From the press, not from here: the window keeps up with the pointer.
                if let Some(Running::Move { from, .. }) = self.input.running.as_mut() {
                    *from = hold.from;
                }
                self.input.tab_move = Some((hold.window, hold.index));
                self.grab_motion(pos);
            }
            return;
        }
        let hold = self.input.tab_hold.as_mut().unwrap();
        if hold.drag.is_none() && moved >= vergence_bar::DRAG_START {
            let start_x = (hold.from.x - bar.loc.x as f64) as f32;
            hold.drag = Some(vergence_bar::TabDrag::new(&layout, hold.index, start_x));
            self.pointer_image.status = CursorImageStatus::Named(CursorIcon::Grabbing);
        }
        if let Some(drag) = hold.drag {
            hold.landing = Some(drag.landing(&layout, bar.size.w as f32, lx, ly));
            // Every step: the ghost follows the pointer, the line goes where it would land.
            self.redraw_all();
        }
    }

    /// A tab held with no button held any more: the seat let go of the press
    /// without a release reaching the tab (a window gone, a drag and drop,
    /// the screens off). The hold ends, said in the log, and the bar is drawn
    /// as it is (Techbench, 2026-11-12: a bar stuck mid-drag).
    fn tab_hold_check(&mut self) {
        if self.input.tab_hold.is_some() && !self.input.seat.buttons_held() {
            let hold = self.input.tab_hold.take().unwrap();
            tracing::warn!("tab: a hold on tab {} ended with no release (dragged: {})", hold.index, hold.drag.is_some());
            self.pointer_image.status = CursorImageStatus::default_named();
            self.redraw_all();
        }
    }

    /// Where the line goes that shows a dragged tab's landing: the window
    /// whose bar shows it, and before which of its tabs (the count: after
    /// the last). In its own row, or another window's of the same program
    /// under the pointer, also while a window is moved by its only tab.
    pub fn tab_mark(&self) -> Option<(Win, usize)> {
        let pos = self.pointer_location();
        if let Some(hold) = self.input.tab_hold.as_ref().filter(|h| h.drag.is_some()) {
            return match hold.landing? {
                vergence_bar::Landing::At(to) => vergence_bar::gap_of(hold.index, to).map(|g| (hold.window.clone(), g)),
                vergence_bar::Landing::Outside => self.join_target(&hold.window, pos, false).ok().map(|(w, _, to)| (w, to as usize)),
            };
        }
        let (w, _) = self.input.tab_move.as_ref()?;
        matches!(self.input.running, Some(Running::Move { .. })).then_some(())?;
        self.join_target(w, pos, true).ok().map(|(w, _, to)| (w, to as usize))
    }

    /// The ghost of a tab being dragged: its window, which tab, and where its
    /// top left goes (under the pointer where it was taken hold of).
    pub fn tab_ghost(&self) -> Option<(Win, usize, Point<f64, Logical>)> {
        let hold = self.input.tab_hold.as_ref()?;
        let drag = hold.drag?;
        let (bar, layout) = self.bar_of(&hold.window)?;
        let tab = layout.tabs.get(hold.index)?;
        let pos = self.pointer_location();
        let dy = hold.from.y - (bar.loc.y as f64 + tab.y as f64);
        Some((hold.window.clone(), hold.index, Point::from((pos.x - drag.grab() as f64, pos.y - dy))))
    }

    /// Where a tab being dragged in window `w` would go: its place, and its
    /// new place in the row (None: out of the window).
    pub fn tab_preview(&self, w: &Win) -> Option<(usize, Option<usize>)> {
        let hold = self.input.tab_hold.as_ref().filter(|h| h.window == *w && h.drag.is_some())?;
        match hold.landing? {
            vergence_bar::Landing::At(to) => Some((hold.index, Some(to))),
            vergence_bar::Landing::Outside => Some((hold.index, None)),
        }
    }

    /// The tab is let go: to its new place in the row; onto another window
    /// of the same program's bar, joining it; anywhere else, a window of its
    /// own, where it was dropped.
    fn tab_release(&mut self, pos: Point<f64, Logical>) {
        let Some(hold) = self.input.tab_hold.take() else { return };
        self.redraw_all();
        if hold.drag.is_some() {
            self.pointer_image.status = CursorImageStatus::default_named();
        }
        let (Some(_), Win::Native(n)) = (hold.drag, &hold.window) else { return };
        let Some(tab) = self.tab_id(&hold.window, hold.index) else {
            tracing::info!("tab: dropped, but the window has no tab {} any more", hold.index);
            return;
        };
        let (client, window) = (n.client, n.id);
        let event = match hold.landing {
            Some(vergence_bar::Landing::At(to)) if to != hold.index => WinEvent::TabMoved { window, tab, to: to as u32 },
            Some(vergence_bar::Landing::At(_)) | None => {
                tracing::info!("tab: dropped at {:.0},{:.0} in its own place", pos.x, pos.y);
                return;
            }
            Some(vergence_bar::Landing::Outside) => match self.join_target(&hold.window, pos, false) {
                Ok((_, into, to)) => WinEvent::TabJoin { window, tab, into, to },
                Err(why) => {
                    tracing::info!("tab: dropped at {:.0},{:.0} out of its window, {why}: a window of its own", pos.x, pos.y);
                    WinEvent::TabOut { window, tab, place: self.drop_place(pos, n.size()) }
                }
            },
        };
        tracing::info!("tab: {event:?}");
        self.bus_send(client, vergence_protocol::bus::ServerMsg::Window(event));
    }

    /// The window of the same program whose bar is under `pos`, and the place
    /// in its row the pointer is at: where a tab dropped there joins. `from`,
    /// the tab's window, is looked through when it is the one being moved
    /// (`moving`); otherwise it hides what is below it like any other.
    /// Err says why not, for the log.
    fn join_target(&self, from: &Win, pos: Point<f64, Logical>, moving: bool) -> Result<(Win, u32, u32), String> {
        let Win::Native(n) = from else { return Err("not a native window".into()) };
        let at = pos.to_i32_round::<i32>();
        for w in self.space.elements().rev().filter(|w| !moving || *w != from) {
            let on_bar = self.bar_of(w).filter(|(bar, _)| bar.contains(at));
            if let Win::Native(m) = w
                && m.client == n.client
                && w != from
                && let Some((bar, layout)) = on_bar.as_ref()
            {
                let x = (pos.x - bar.loc.x as f64) as f32;
                return Ok((w.clone(), m.id, layout.tabs.iter().filter(|r| r.x + r.w / 2.0 < x).count() as u32));
            }
            // Any other window over the spot (its bar or its content) hides what is below.
            if on_bar.is_some() || self.space.element_geometry(w).is_some_and(|g| g.contains(at)) {
                let what = match w {
                    _ if w == from => "its own window".to_string(),
                    Win::Native(m) if m.client == n.client => format!("the content of native window {}, not its bar", m.id),
                    Win::Native(m) => format!("a window of another program (native window {}, client {})", m.id, m.client),
                    _ => "a Wayland or X11 window".to_string(),
                };
                return Err(format!("over {what}"));
            }
        }
        Err("over the desktop".into())
    }

    /// Where a window for a tab dropped at `pos` goes: its bar under the
    /// pointer, slid onto the monitor's work area as far as `size` (the
    /// content's, taken from the window the tab left) needs, since a place
    /// partly off the monitor would be refused and the window centered.
    fn drop_place(&self, pos: Point<f64, Logical>, size: Size<i32, Logical>) -> vergence_protocol::window::Place {
        let at = pos.to_i32_round::<i32>();
        let Some(out) = self.space.outputs().find(|o| self.space.output_geometry(o).is_some_and(|g| g.contains(at))) else {
            return vergence_protocol::window::Place::Anywhere;
        };
        let (Some(id), Some(g)) = (crate::memory::monitor_id_of(out), self.space.output_geometry(out)) else {
            return vergence_protocol::window::Place::Anywhere;
        };
        let area = crate::place::content_area(self.work_area(g), self.bar_h_in(g));
        let slide = |want: f64, lo: i32, room: i32, side: i32| want.min((lo + room - side) as f64).max(lo as f64);
        let x = slide(pos.x - 120.0, area.loc.x, area.size.w, size.w) - g.loc.x as f64;
        let y = slide(pos.y + crate::decor::TITLE_H as f64 / 2.0, area.loc.y, area.size.h, size.h) - g.loc.y as f64;
        vergence_protocol::window::Place::Monitor { id, x, y }
    }

    /// A tab menu's entry was chosen (menu.rs).
    pub fn tab_act(&mut self, w: &Win, tab: u32, act: crate::menu::TabAct) {
        use crate::menu::TabAct;
        let Win::Native(n) = w else { return };
        let window = n.id;
        let event = match act {
            TabAct::Close => WinEvent::TabClose { window, tab },
            TabAct::CloseOthers => WinEvent::TabCloseOthers { window, tab },
            TabAct::NewWindow => {
                // A little down and right of the window, on its monitor.
                let at = self.space.element_location(w).map(|p| p.to_f64() + Point::from((60.0, 60.0))).unwrap_or_default();
                WinEvent::TabOut { window, tab, place: self.drop_place(at + Point::from((120.0, -(crate::decor::TITLE_H as f64) / 2.0)), n.size()) }
            }
            TabAct::Program(entry) => WinEvent::TabMenu { window, tab, entry },
        };
        self.bus_send(n.client, vergence_protocol::bus::ServerMsg::Window(event));
    }

    /// Tell a native window what was done with its tab at `index`.
    fn tab_event(&mut self, id: WinId, index: u32, event: impl FnOnce(u32, u32) -> WinEvent) {
        let Some(w) = self.input.wins.get(&id).cloned() else { return };
        let (Win::Native(n), Some(tab)) = (&w, self.tab_id(&w, index as usize)) else { return };
        self.bus_send(n.client, vergence_protocol::bus::ServerMsg::Window(event(n.id, tab)));
    }

    /// The drag and drop's event, to Smithay as it came; when its grab has
    /// ended, the pointer is the seat's again.
    fn to_edge(&mut self, pos: Point<f64, Logical>, time: InputTime, came: &Came) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let serial = SERIAL_COUNTER.next_serial();
        match came {
            Came::Motion => {
                let focus = match self.hit(pos) {
                    Hit::Surface(_, s, origin) => Some((s, origin)),
                    _ => None,
                };
                pointer.motion(self, focus, &MotionEvent { location: pos, serial, time });
            }
            Came::Button { button, state } => pointer.button(self, &ButtonEvent { button: *button, state: *state, serial, time }),
            Came::Axis { frame, .. } => pointer.axis(self, frame.clone()),
            Came::Nothing => {}
        }
        pointer.frame(self);
        if !pointer.is_grabbed() {
            let under = self.target_at(pos);
            let out = self.input.seat.edge_end(under);
            self.apply(out, pos, time, &Came::Nothing);
        }
    }

    /// Smithay's pointer lets go of the surface it was on (a move or resize
    /// has the pointer now): its click grab ends and the surface hears leave.
    fn smithay_let_go(&mut self, pos: Point<f64, Logical>, time: InputTime) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        if pointer.is_grabbed() {
            pointer.unset_grab(self, SERIAL_COUNTER.next_serial(), time);
        }
        pointer.motion(self, None, &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time });
        pointer.frame(self);
    }

    fn begin_move(&mut self, window: Win, pos: Point<f64, Logical>, time: InputTime) {
        self.smithay_let_go(pos, time);
        let Some(loc) = self.space.element_location(&window) else { return };
        let unmaximize_on_drag = self.restore.contains_key(&window) || self.snap.snapped.contains_key(&window);
        self.snap.ctrl_seen = false;
        self.input.running = Some(Running::Move { window, from: pos, loc, unmaximize_on_drag });
    }

    fn begin_resize(&mut self, window: Win, edges: ResizeEdge, pos: Point<f64, Logical>, time: InputTime) {
        self.smithay_let_go(pos, time);
        let Some(loc) = self.space.element_location(&window) else { return };
        let initial = Rectangle::new(loc, window.geometry().size);
        if window.toplevel().is_some() {
            self.input.anchored = Some((window.clone(), edges, initial, false));
        }
        self.input.running = Some(Running::Resize { window, edges, from: pos, initial, size: initial.size });
    }

    fn grab_motion(&mut self, pos: Point<f64, Logical>) {
        match self.input.running.take() {
            Some(Running::Move { window, mut from, mut loc, mut unmaximize_on_drag }) => {
                let delta = pos - from;
                if unmaximize_on_drag {
                    if delta.x.hypot(delta.y) >= DRAG {
                        unmaximize_on_drag = false;
                        let old = match self.restore.get(&window).copied() {
                            Some(old) => {
                                self.toggle_maximize(&window);
                                Some(old)
                            }
                            // Out of its part of the screen (snap.rs), as out of maximized.
                            None => self.unsnap(&window).inspect(|old| self.set_window_rect(&window, Rectangle::new(old.loc, old.size), false)),
                        };
                        if let Some(old) = old {
                            // Out of maximized, its old size, under the pointer by its title bar.
                            let x = (pos.x - old.size.w as f64 / 2.0).round() as i32;
                            let y = pos.y.round() as i32 + crate::decor::TITLE_H / 2;
                            self.space.map_element(window.clone(), (x, y), true);
                            if let Some(xs) = window.x11_surface() {
                                let _ = xs.configure(Rectangle::new((x, y).into(), old.size));
                            }
                            loc = (x, y).into();
                            from = pos;
                        }
                    }
                } else {
                    self.snap_track(pos);
                    let to = (loc.to_f64() + delta).to_i32_round();
                    self.space.map_element(window.clone(), to, true);
                    if let Some(x) = window.x11_surface() {
                        let _ = x.configure(Rectangle::new(to, x.geometry().size));
                    }
                }
                self.input.running = Some(Running::Move { window, from, loc, unmaximize_on_drag });
                if self.input.tab_move.is_some() {
                    self.redraw_all();
                }
            }
            Some(Running::Resize { window, edges, from, initial, .. }) => {
                let size = self.within_limits(&window, resized(edges, initial, pos - from));
                let mut loc = initial.loc;
                if edges.intersects(ResizeEdge::LEFT) {
                    loc.x += initial.size.w - size.w;
                }
                if edges.intersects(ResizeEdge::TOP) {
                    loc.y += initial.size.h - size.h;
                }
                if let Some(x) = window.x11_surface() {
                    let _ = x.configure_with_sync(Rectangle::new(loc, size), None);
                } else if let Win::Native(n) = &window {
                    let n = n.clone();
                    self.native_configure(&n, size, false);
                    self.space.map_element(window.clone(), loc, true);
                } else if let Some(t) = window.toplevel() {
                    // Its location follows as the new size arrives (resize_commit).
                    t.with_pending_state(|s| {
                        s.states.set(xdg_toplevel::State::Resizing);
                        s.size = Some(size);
                    });
                    t.send_pending_configure();
                }
                self.input.running = Some(Running::Resize { window, edges, from, initial, size });
            }
            None => {}
        }
    }

    fn grab_end(&mut self, time: InputTime) {
        let tab_move = self.input.tab_move.take();
        match self.input.running.take() {
            Some(Running::Move { window, loc, .. }) => {
                self.remember(&window);
                // Snapped, it goes back later to where this drag took it from.
                self.snap_release(&window, loc);
                // Moved by its only tab and let go over another window's bar: it joins.
                if let Some((w, index)) = tab_move.filter(|(w, _)| *w == window)
                    && let Win::Native(n) = &w
                    && let Some(tab) = self.tab_id(&w, index)
                {
                    let pos = self.pointer_location();
                    match self.join_target(&w, pos, true) {
                        Ok((_, into, to)) => {
                            let event = WinEvent::TabJoin { window: n.id, tab, into, to };
                            tracing::info!("tab: {event:?}");
                            self.bus_send(n.client, vergence_protocol::bus::ServerMsg::Window(event));
                        }
                        Err(why) => tracing::info!("tab: window {} moved by its only tab, let go at {:.0},{:.0} {why}: stays a window", n.id, pos.x, pos.y),
                    }
                    self.redraw_all();
                }
            }
            Some(Running::Resize { window, size, .. }) => {
                self.remember(&window);
                if let Some(t) = window.toplevel() {
                    t.with_pending_state(|s| {
                        s.states.unset(xdg_toplevel::State::Resizing);
                        s.size = Some(size);
                    });
                    t.send_pending_configure();
                    if let Some(a) = self.input.anchored.as_mut() {
                        a.3 = true;
                    }
                }
            }
            None => {}
        }
        // The press that began it, if Smithay's pointer saw it, comes up there
        // too (no surface hears it: Smithay's focus is none now).
        if let Some(pointer) = self.seat.get_pointer() {
            for button in std::mem::take(&mut self.input.smithay_held) {
                pointer.button(self, &ButtonEvent { button, state: ButtonState::Released, serial: SERIAL_COUNTER.next_serial(), time });
            }
            pointer.frame(self);
        }
    }

    /// A Wayland surface committed: a window resized from its top or left
    /// edge is put so that its opposite edge stays where it was.
    pub fn resize_commit(&mut self, surface: &WlSurface) {
        let Some((window, edges, initial, ended)) = self.input.anchored.clone() else { return };
        if !window.toplevel().is_some_and(|t| t.wl_surface() == surface) {
            return;
        }
        if ended {
            self.input.anchored = None;
        }
        if !edges.intersects(ResizeEdge::TOP_LEFT) {
            return;
        }
        let Some(mut loc) = self.space.element_location(&window) else { return };
        let size = window.geometry().size;
        if edges.intersects(ResizeEdge::LEFT) {
            loc.x = initial.loc.x + (initial.size.w - size.w);
        }
        if edges.intersects(ResizeEdge::TOP) {
            loc.y = initial.loc.y + (initial.size.h - size.h);
        }
        self.space.map_element(window, loc, false);
    }

    /// The edges to resize from (Alt+right drag): the window's nearest third in each direction.
    fn nearest_edges(&self, window: &Win, at: Point<f64, Logical>) -> ResizeEdge {
        let Some(g) = self.space.element_geometry(window) else { return ResizeEdge::BOTTOM_RIGHT };
        let (fx, fy) = ((at.x - g.loc.x as f64) / g.size.w.max(1) as f64, (at.y - g.loc.y as f64) / g.size.h.max(1) as f64);
        let mut e = ResizeEdge::empty();
        if fx < 1.0 / 3.0 {
            e |= ResizeEdge::LEFT;
        } else if fx > 2.0 / 3.0 {
            e |= ResizeEdge::RIGHT;
        }
        if fy < 1.0 / 3.0 {
            e |= ResizeEdge::TOP;
        } else if fy > 2.0 / 3.0 {
            e |= ResizeEdge::BOTTOM;
        }
        if e.is_empty() { ResizeEdge::BOTTOM_RIGHT } else { e }
    }

    /// The screens went off (energy.rs): no grab, menu or held button outlives them.
    pub fn input_screens_off(&mut self) {
        let out = self.input.seat.screens_off();
        let pos = self.pointer_location();
        self.apply(out, pos, InputTime::now(), &Came::Nothing);
    }
}

/// A window's new size for a resize by `delta` from its edges, within its own limits.
/// The size a resize from `initial` by `delta` asks for (style.rs keeps it
/// within the window's limits, whatever road the window came by).
fn resized(edges: ResizeEdge, initial: Rectangle<i32, Logical>, delta: Point<f64, Logical>) -> Size<i32, Logical> {
    let (mut w, mut h) = (initial.size.w, initial.size.h);
    if edges.intersects(ResizeEdge::LEFT | ResizeEdge::RIGHT) {
        let dx = if edges.intersects(ResizeEdge::LEFT) { -delta.x } else { delta.x };
        w = (initial.size.w as f64 + dx) as i32;
    }
    if edges.intersects(ResizeEdge::TOP | ResizeEdge::BOTTOM) {
        let dy = if edges.intersects(ResizeEdge::TOP) { -delta.y } else { delta.y };
        h = (initial.size.h as f64 + dy) as i32;
    }
    Size::from((w.max(1), h.max(1)))
}
