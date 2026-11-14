//! The seat: who gets each pointer, keyboard and touch event, as plain
//! decisions (docs/seat-milestone-1.md, step 1; whitepaper §6.3).
//!
//! The server tells the seat what is under a point ([`Target`]) and what the
//! hand did; the seat answers with what is to happen ([`Out`]), and the
//! server applies it **after** the call, never inside one. Nothing here
//! looks at windows, draws, or knows Smithay: a window is a number the
//! server gave it, and so is a Wayland surface.
//!
//! The rules, each with its tests:
//!
//! - **Pointer focus and the implicit grab** ([`pointer`]): the target under
//!   the pointer has it, except while a button is held: then the target the
//!   first press went to keeps every motion and every release. Enter and
//!   leave come from one place.
//! - **Grabs as states** ([`Grab`]): moving or resizing a window, a menu or
//!   the apps popup open (a press outside closes it and goes no further),
//!   and the edge's (a Wayland drag and drop, Smithay's until it ends). One
//!   at a time.
//! - **The keyboard** ([`keyboard`]): the server's own keys are filtered
//!   first (Ctrl+Alt+Backspace, the consoles, the volume keys, Super tapped
//!   alone, Escape for a menu, everything while the popup is open, Alt+Tab
//!   and Alt+` switching windows, Super with an arrow for the window keys),
//!   then the focused window has them. The seat keeps the order in which
//!   windows last had the keyboard (windows milestone 1).
//! - **Touch** ([`touch`]): a finger on a window is the window's from its
//!   first contact to its last; the first finger anywhere else is the
//!   pointer with its left button held; a long, still press there is the
//!   right button.

pub mod keyboard;
pub mod pointer;
pub mod touch;

#[cfg(test)]
mod fuzz;

/// A window, by the number the server gave it.
pub type WinId = u64;
/// A Wayland or X11 surface, by the number the server gave it.
pub type SurfaceId = u64;
/// A finger, by the slot its panel gave it.
pub type Slot = u32;

/// The left, right and middle buttons (evdev's codes, as the server has them).
pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

/// Which edges of a window a resize moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Edges(pub u8);

impl Edges {
    pub const TOP: Edges = Edges(1);
    pub const BOTTOM: Edges = Edges(2);
    pub const LEFT: Edges = Edges(4);
    pub const RIGHT: Edges = Edges(8);
}

/// Something the server draws itself, under a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Part {
    /// A window's title bar.
    Title(WinId),
    /// A window's border or resize band.
    Edge(WinId, Edges),
    Close(WinId),
    Maximize(WinId),
    Minimize(WinId),
    /// A tab of a window's bar (by its place in the row), a tab's close
    /// button, and the + (bar milestone 1): the server draws them, the
    /// program hears what was done.
    Tab(WinId, u32),
    CloseTab(WinId, u32),
    Plus(WinId),
    /// The dock (the shelf).
    Dock,
    /// The applications popup.
    Popup,
    /// The desktop menu or an icon's menu.
    Menu,
    /// A notification card.
    Note,
    /// The bare desktop.
    Desktop,
}

/// What is under a point, as the server sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    /// A Wayland or X11 surface of a window: the edge (Smithay) delivers to it.
    Surface(WinId, SurfaceId),
    /// A native window (native.rs): the server delivers over the bus.
    Native(WinId),
    /// The server's own.
    Server(Part),
}

impl Target {
    /// The window this target belongs to, if any (its surface, its frame).
    pub fn window(&self) -> Option<WinId> {
        match *self {
            Target::Surface(w, _) | Target::Native(w) => Some(w),
            Target::Server(Part::Title(w) | Part::Edge(w, _) | Part::Close(w) | Part::Maximize(w) | Part::Minimize(w) | Part::Tab(w, _) | Part::CloseTab(w, _) | Part::Plus(w)) => Some(w),
            Target::Server(_) => None,
        }
    }

    /// The application's own content (not its frame, not the server's).
    pub fn is_content(&self) -> bool {
        matches!(self, Target::Surface(..) | Target::Native(_))
    }
}

/// Which of the server's menus is open over everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    /// The desktop menu or an icon's menu.
    Menu,
    /// The applications popup.
    Popup,
}

/// A grab: for as long as it lasts, the pointer is its.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grab {
    /// A window going with the pointer, until `button` is let go.
    Move { window: WinId, button: u32 },
    /// A window's edges going with the pointer. `edges` None: the server picks
    /// the nearest (Alt+right drag).
    Resize { window: WinId, edges: Option<Edges>, button: u32 },
    /// A Wayland drag and drop: Smithay's `DnDGrab` has the pointer until
    /// the server says it ended ([`Seat::edge_end`]).
    Edge,
}

/// What the server is to do, in order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Out {
    // ---- the pointer ----
    /// The pointer came onto a target.
    Enter(Target),
    /// The pointer left a target.
    Leave(Target),
    /// The pointer moved on the target that has it.
    Motion(Target),
    Press(Target, u32),
    Release(Target, u32),
    /// A wheel or a finger's scroll, for this target.
    Axis(Target),
    // ---- windows ----
    /// Raise the window and give it the keyboard.
    Focus(WinId),
    /// No window has the keyboard (a press on the bare desktop).
    Unfocus,
    Maximize(WinId),
    Close(WinId),
    Minimize(WinId),
    /// A grab began; motion now goes to it ([`Out::GrabMotion`]).
    MoveBegin(WinId),
    ResizeBegin(WinId, Option<Edges>),
    GrabMotion,
    /// The move or resize ended: remember where the window is.
    GrabEnd(WinId),
    /// Close the open menu or popup.
    CloseMenu,
    /// Pass the pointer event to Smithay's drag and drop as it came.
    ToEdge,
    // ---- touch ----
    /// Put the pointer where the pointing finger is (its picture stays hidden).
    PointerTo { x: f64, y: f64 },
    /// A finger came down on, moved on, or left a window's content.
    TouchDown(Target, Slot),
    TouchMotion(Target, Slot),
    TouchUp(Target, Slot),
    /// The panel took its touches back from this window.
    TouchCancel(Target, Slot),
    /// Start the long-press timer; when it fires, call [`Seat::touch_long_press`] with this number.
    LongPressTimer(u64),
    /// A long press on the dock: the icon's menu (the press it was holding is not a click any more).
    DockMenu,
    /// A long press on the bare desktop: the desktop menu.
    DesktopMenu,
    /// A window moved by the finger that asked (its toolkit's bar): put it where the finger says.
    FingerMove { window: WinId, dx: f64, dy: f64 },
    /// That finger lifted: remember where the window is.
    FingerMoveEnd(WinId),
    /// No finger rests on the dock any more (no name beside an icon).
    Unhover,
    // ---- switching ----
    /// Alt+Tab's switching ended without a choice (the screens went off).
    SwitchCancel,
}

/// The seat: one pointer, one keyboard, the touch panels.
#[derive(Debug, Default)]
pub struct Seat {
    pub(crate) pointer: pointer::Pointer,
    pub(crate) keyboard: keyboard::Keyboard,
    pub(crate) touch: touch::Touch,
    pub(crate) grab: Option<Grab>,
    pub(crate) menu: Option<Menu>,
}

impl Seat {
    pub fn new() -> Seat {
        Seat::default()
    }

    /// The grab in progress, if any.
    pub fn grab(&self) -> Option<Grab> {
        self.grab
    }

    /// The menu or popup open, if any.
    pub fn menu(&self) -> Option<Menu> {
        self.menu
    }

    /// The target the pointer is on (or held on), if any.
    pub fn pointer_focus(&self) -> Option<Target> {
        self.pointer.focus
    }

    /// Whether a button is held (the implicit grab).
    pub fn buttons_held(&self) -> bool {
        !self.pointer.held.is_empty()
    }

    /// The window that has the keyboard, if any.
    pub fn keyboard_focus(&self) -> Option<WinId> {
        self.keyboard.focus
    }

    /// Every window that has had the keyboard, the most recent first (Alt+Tab's
    /// order). A window stays in it while minimized and leaves when it goes.
    pub fn order_of_use(&self) -> &[WinId] {
        &self.keyboard.used
    }

    /// The server opened a menu or the popup (it takes the keyboard's keys
    /// it wants and closes at a press outside).
    pub fn menu_opened(&mut self, menu: Menu) {
        self.menu = Some(menu);
    }

    /// The server closed it (an entry was chosen, or it closed itself).
    pub fn menu_closed(&mut self) {
        self.menu = None;
    }

    /// A Wayland program began a drag and drop: Smithay's grab has the pointer.
    pub fn edge_begin(&mut self) -> Vec<Out> {
        let mut out = Vec::new();
        self.end_grab(&mut out);
        if let Some(t) = self.pointer.focus.take() {
            out.push(Out::Leave(t));
        }
        self.pointer.held.clear();
        self.grab = Some(Grab::Edge);
        out
    }

    /// The drag and drop ended: the pointer is ours again, on what is under it.
    pub fn edge_end(&mut self, under: Option<Target>) -> Vec<Out> {
        let mut out = Vec::new();
        if self.grab == Some(Grab::Edge) {
            self.grab = None;
            self.pointer.held.clear();
            self.pointer.refocus(under, &mut out);
        }
        out
    }

    /// A window went (closed, crashed, unmapped): nothing of the seat's
    /// stays on it. A grab of it ends without a [`Out::GrabEnd`] (there is
    /// nothing to remember).
    pub fn window_gone(&mut self, window: WinId) -> Vec<Out> {
        let mut out = Vec::new();
        if matches!(self.grab, Some(Grab::Move { window: w, .. } | Grab::Resize { window: w, .. }) if w == window) {
            self.grab = None;
            self.pointer.held.clear();
        }
        if self.pointer.focus.and_then(|t| t.window()) == Some(window) {
            // Its frame and content are gone: no leave to send to them.
            self.pointer.focus = None;
            self.pointer.held.clear();
        }
        if self.keyboard.focus == Some(window) {
            self.keyboard.focus = None;
        }
        self.keyboard.used.retain(|&w| w != window);
        self.touch.window_gone(window, &mut out);
        out
    }

    /// The screens went off (energy.rs): no grab or menu outlives them, no
    /// button stays held, every finger is let go.
    pub fn screens_off(&mut self) -> Vec<Out> {
        let mut out = Vec::new();
        self.end_grab(&mut out);
        if self.menu.take().is_some() {
            out.push(Out::CloseMenu);
        }
        if self.keyboard.switching.take().is_some() {
            out.push(Out::SwitchCancel);
        }
        if let Some(t) = self.pointer.focus.take() {
            if !self.pointer.held.is_empty() {
                for b in std::mem::take(&mut self.pointer.held) {
                    out.push(Out::Release(t, b));
                }
            }
            out.push(Out::Leave(t));
        }
        self.pointer.held.clear();
        self.pointer.swallow.clear();
        self.touch.cancel_into(&mut out);
        out
    }

    /// End a move or resize grab (it is remembered); an edge grab is Smithay's to end.
    fn end_grab(&mut self, out: &mut Vec<Out>) {
        match self.grab {
            Some(Grab::Move { window, .. } | Grab::Resize { window, .. }) => {
                self.grab = None;
                out.push(Out::GrabEnd(window));
            }
            Some(Grab::Edge) | None => {}
        }
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;
    pub const W1: WinId = 1;
    pub const W2: WinId = 2;
    pub fn s1() -> Target {
        Target::Surface(W1, 10)
    }
    pub fn s2() -> Target {
        Target::Surface(W2, 20)
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::*;
    use super::*;

    #[test]
    fn a_closed_window_takes_its_grab_and_focus_with_it() {
        let mut seat = Seat::new();
        seat.motion(Some(s1()));
        seat.button(BTN_LEFT, true, Some(s1()), keyboard::Mods { alt: true, ..Default::default() }, 0);
        assert!(matches!(seat.grab(), Some(Grab::Move { window: W1, .. })));
        let out = seat.window_gone(W1);
        assert!(!out.contains(&Out::GrabEnd(W1)));
        assert_eq!(seat.grab(), None);
        assert_eq!(seat.keyboard_focus(), None);
        // The release that follows goes nowhere.
        assert!(seat.button(BTN_LEFT, false, None, Default::default(), 10).is_empty());
    }

    #[test]
    fn screens_off_ends_everything() {
        let mut seat = Seat::new();
        seat.motion(Some(s1()));
        seat.button(BTN_LEFT, true, Some(s1()), Default::default(), 0);
        seat.menu_opened(Menu::Popup);
        let out = seat.screens_off();
        assert!(out.contains(&Out::CloseMenu));
        assert!(out.contains(&Out::Release(s1(), BTN_LEFT)));
        assert!(out.contains(&Out::Leave(s1())));
        assert_eq!(seat.menu(), None);
        assert_eq!(seat.pointer_focus(), None);
    }

    #[test]
    fn a_drag_and_drop_has_the_pointer_until_it_ends() {
        let mut seat = Seat::new();
        seat.motion(Some(s1()));
        seat.button(BTN_LEFT, true, Some(s1()), Default::default(), 0);
        let out = seat.edge_begin();
        assert_eq!(out, vec![Out::Leave(s1())]);
        assert_eq!(seat.motion(Some(s2())), vec![Out::ToEdge]);
        assert_eq!(seat.button(BTN_LEFT, false, Some(s2()), Default::default(), 5), vec![Out::ToEdge]);
        assert_eq!(seat.edge_end(Some(s2())), vec![Out::Enter(s2())]);
        assert_eq!(seat.motion(Some(s2())), vec![Out::Motion(s2())]);
    }
}
