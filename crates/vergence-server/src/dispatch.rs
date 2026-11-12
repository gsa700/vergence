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
    wayland::{compositor, shell::xdg::SurfaceCachedState},
};
use vergence_seat::{Edges, Out, Part, Target, WinId};

use crate::{decor::Hit, focus::KeyboardFocus, state::Server, win::Win};

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
    wins: HashMap<WinId, Win>,
    ids: HashMap<Win, WinId>,
    /// Surfaces by number: the surface and where its origin was when last hit.
    surfaces: HashMap<u64, (WlSurface, Point<f64, Logical>)>,
    surface_ids: HashMap<ObjectId, u64>,
    running: Option<Running>,
    /// Buttons Smithay's pointer has seen go down and not up.
    smithay_held: Vec<u32>,
    /// A Wayland window resized from its top or left: kept anchored as its
    /// commits arrive, until the one after the resize ended.
    anchored: Option<(Win, ResizeEdge, Rectangle<i32, Logical>, bool)>,
}

impl Server {
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
    fn dispatch_sync(&mut self) {
        let menu = if self.launcher_is_open() {
            Some(vergence_seat::Menu::Popup)
        } else if self.menu_is_open() {
            Some(vergence_seat::Menu::Menu)
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
        if self.menu_contains(pos) {
            return Some(Target::Server(Part::Menu));
        }
        if self.launcher_contains(pos) {
            return Some(Target::Server(Part::Popup));
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
            Hit::Shelf => Target::Server(Part::Dock),
            Hit::Nothing => Target::Server(Part::Desktop),
        })
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
        // Hover: each of the server's parts looks for itself.
        self.menu_hover(pos);
        self.notify_hover(pos);
        self.shelf_hover(pos);
        self.launcher_hover(pos);
        let under = self.target_at(pos);
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
        let out = self.input.seat.motion(under);
        self.apply(out, pos, time, &Came::Motion);
    }

    /// A button pressed or released. The mouse, a pointing finger and `vctl
    /// press`/`release` come here.
    pub fn handle_button(&mut self, button: u32, state: ButtonState, time: InputTime) {
        // A button with Super held is not a tap of Super (the key filter is input.rs's until step 3).
        self.launcher.super_alone = false;
        self.dispatch_sync();
        let pos = self.pointer_location();
        let under = self.target_at(pos);
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
    fn apply(&mut self, out: Vec<Out>, pos: Point<f64, Logical>, time: InputTime, came: &Came) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        for o in out {
            match o {
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
                Out::Unfocus => {
                    self.space.elements().for_each(|w| {
                        w.set_activated(false);
                        if let Some(t) = w.toplevel() {
                            t.send_pending_configure();
                        }
                    });
                    if let Some(k) = self.seat.get_keyboard() {
                        k.set_focus(self, Option::<KeyboardFocus>::None, SERIAL_COUNTER.next_serial());
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
                Out::CloseMenu => {
                    self.close_menu();
                    self.launcher_close();
                }
                Out::ToEdge => self.to_edge(pos, time, came),
                // Touch is touch.rs's until step 4.
                _ => {}
            }
        }
    }

    /// A press or release, to whatever the seat said.
    fn deliver_button(&mut self, t: Target, button: u32, pressed: bool, pos: Point<f64, Logical>, time: InputTime) {
        let serial = SERIAL_COUNTER.next_serial();
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
            Target::Server(Part::Popup) if pressed => self.launcher_press(pos),
            Target::Server(Part::Note) if pressed => self.notify_press(pos),
            // The dock: a left press is a click or the start of a drag, let go where
            // it ends; a right press is an icon's menu.
            Target::Server(Part::Dock) if pressed && button == vergence_seat::BTN_LEFT => self.shelf_press(pos, button),
            Target::Server(Part::Dock) if pressed && button == vergence_seat::BTN_RIGHT => self.shelf_menu(pos),
            Target::Server(Part::Dock) if !pressed && self.shelf.held.is_some_and(|(b, _)| b == button) => self.shelf_release(pos),
            Target::Server(Part::Desktop) if pressed && button == vergence_seat::BTN_RIGHT => self.open_menu(pos),
            Target::Server(_) => {}
        }
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
        let unmaximize_on_drag = self.restore.contains_key(&window);
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
                        if let Some(old) = self.restore.get(&window).copied() {
                            // Out of maximized, its old size, under the pointer by its title bar.
                            self.toggle_maximize(&window);
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
                    let to = (loc.to_f64() + delta).to_i32_round();
                    self.space.map_element(window.clone(), to, true);
                    if let Some(x) = window.x11_surface() {
                        let _ = x.configure(Rectangle::new(to, x.geometry().size));
                    }
                }
                self.input.running = Some(Running::Move { window, from, loc, unmaximize_on_drag });
            }
            Some(Running::Resize { window, edges, from, initial, .. }) => {
                let size = resized(&window, edges, initial, pos - from);
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
        match self.input.running.take() {
            Some(Running::Move { window, .. }) => self.remember(&window),
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
fn resized(window: &Win, edges: ResizeEdge, initial: Rectangle<i32, Logical>, delta: Point<f64, Logical>) -> Size<i32, Logical> {
    let (mut w, mut h) = (initial.size.w, initial.size.h);
    if edges.intersects(ResizeEdge::LEFT | ResizeEdge::RIGHT) {
        let dx = if edges.intersects(ResizeEdge::LEFT) { -delta.x } else { delta.x };
        w = (initial.size.w as f64 + dx) as i32;
    }
    if edges.intersects(ResizeEdge::TOP | ResizeEdge::BOTTOM) {
        let dy = if edges.intersects(ResizeEdge::TOP) { -delta.y } else { delta.y };
        h = (initial.size.h as f64 + dy) as i32;
    }
    let (min, max) = match window.toplevel() {
        Some(t) => compositor::with_states(t.wl_surface(), |states| {
            let mut guard = states.cached_state.get::<SurfaceCachedState>();
            let d = guard.current();
            (d.min_size, d.max_size)
        }),
        None => window.x11_surface().map(|x| (x.min_size().unwrap_or_default(), x.max_size().unwrap_or_default())).unwrap_or_default(),
    };
    let max_w = if max.w == 0 { i32::MAX } else { max.w };
    let max_h = if max.h == 0 { i32::MAX } else { max.h };
    Size::from((w.max(min.w.max(1)).min(max_w), h.max(min.h.max(1)).min(max_h)))
}
