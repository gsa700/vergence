// Touch, the plain version (Dave, 2026-11-10: basic touch beside a keyboard
// and, later, a spoken interface; an on-screen keyboard later if ever).
//
// A finger on a window is the window's: it is brought forward and given the
// keyboard, and the touch goes to it as Wayland touch (wl_touch), every
// finger of it, for the application to make of what it will (Firefox and
// GTK scroll, tap and pinch by themselves). A first finger anywhere else,
// on the dock, a menu, the applications popup, a notification, a title bar
// the server draws, or the bare desktop, acts as the pointer with its left
// button held: what a click or a drag does there, a tap or a drag does.
//
// Which screen a panel is on: a touch panel says where on itself it was
// touched, and nothing of which monitor it is glued to. desktop.toml may
// say:
//
//     [[touch.panel]]
//     name = "Silicon Works Multi-touch SW4101C"   # as the system names it
//     screen = "<screen id>"
//
// A panel with no such tie, or tied to a screen that is not connected, is
// taken to be on the primary screen. Never across all of them.
//
// A finger is not a pointer's tip. One that comes down a little off a
// title bar's button, a window's edge or the dock is taken to have meant
// it ([`REACH`]). One held still half a second on a dock icon or the bare
// desktop is the right button there: the icon's menu, the desktop's. And a
// window that asks to be moved from a finger's press (our own toolkit's
// bar does) goes with that finger.
//
// The pointer's picture goes away at a finger's touch and comes back when
// a mouse moves.
//
// Not yet: a way to make the tie by touching the screen (settings app).

use crate::win::Win;
use std::collections::{HashMap, HashSet};

use smithay::{
    backend::input::{ButtonState, InputTime, TouchSlot},
    reexports::{calloop::timer::{TimeoutAction, Timer}, wayland_server::{Resource, protocol::wl_surface::WlSurface}},
    utils::Serial,
    input::touch::{DownEvent, MotionEvent, UpEvent},
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
};

use crate::{Server, decor::Hit};

const BTN_LEFT: u32 = 0x110;
/// How far off its mark a finger may come down and still have meant it, in points.
const REACH: f64 = 10.0;
/// How long a finger rests for a long press, and how far it may wander meanwhile.
const LONG: std::time::Duration = std::time::Duration::from_millis(500);
const STILL: f64 = 10.0;

/// A window going with the finger that asked to move it.
struct Moving {
    slot: TouchSlot,
    window: Win,
    /// Where the window was and where the finger came down.
    from: Point<i32, Logical>,
    down: Point<f64, Logical>,
}

#[derive(Default)]
pub struct Touch {
    /// Panels tied to screens: the panel's name, the screen's id.
    ties: Vec<(String, String)>,
    /// The finger that is acting as the pointer.
    pointing: Option<TouchSlot>,
    /// Where the pointing finger came down, while it has not moved away:
    /// a long press, if it stays. And which press this is, for its timer.
    resting: Option<Point<f64, Logical>>,
    presses: u64,
    /// The fingers that are on windows: each one's press (its serial, the
    /// surface, where).
    on_windows: HashMap<TouchSlot, (Serial, WlSurface, Point<f64, Logical>)>,
    moving: Option<Moving>,
    /// A finger was the last thing to point: the pointer's picture is not
    /// drawn until a mouse moves again (an arrow, or a window edge's shape,
    /// left where a finger lifted, points at nothing).
    pub fingers_last: bool,
    /// Panels already named in the log.
    said: HashSet<String>,
}

/// The ties desktop.toml records.
pub fn load() -> Vec<(String, String)> {
    let table = crate::wallpaper::desktop_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| t.parse::<toml::Table>().ok());
    let panels = table.as_ref().and_then(|t| t.get("touch")?.get("panel")?.as_array());
    panels
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p.get("name")?.as_str()?.to_string(), p.get("screen")?.as_str()?.to_string())))
        .take(16)
        .collect()
}

impl Server {
    /// Read the panels' ties again (at start, and when desktop.toml changes).
    pub fn touch_reload(&mut self) {
        self.touch.ties = load();
    }

    /// A mouse moved or pressed: the pointer is drawn again.
    pub fn touch_mouse_back(&mut self) {
        if self.touch.fingers_last {
            self.touch.fingers_last = false;
            self.redraw_all();
        }
    }

    /// The screen the panel named `panel` is on.
    pub fn touch_screen(&mut self, panel: &str) -> Option<Rectangle<i32, Logical>> {
        let outputs = self.bus_outputs();
        let ids: Vec<String> = outputs.iter().map(|o| o.id.clone()).collect();
        let tied = self.touch.ties.iter().find(|(name, _)| name == panel).map(|(_, screen)| screen.as_str()).filter(|id| ids.iter().any(|i| i == id));
        let id = tied.or_else(|| vergence_profile::primary(self.shelf.primary.as_deref(), &ids))?.to_string();
        if self.touch.said.insert(panel.to_string()) {
            tracing::info!("touch: panel {panel:?} on screen {id}{}", if tied.is_some() { "" } else { " (the primary one: no tie in desktop.toml)" });
        }
        let o = outputs.iter().find(|o| o.id == id)?;
        Some(Rectangle::new((o.x, o.y).into(), (o.width, o.height).into()))
    }

    /// A finger came down at `at`. The touch panel, and the control socket's
    /// `touch`, both come here.
    pub fn touch_down(&mut self, slot: TouchSlot, at: Point<f64, Logical>, time: InputTime) {
        self.launcher.super_alone = false;
        if !self.touch.fingers_last {
            self.touch.fingers_last = true;
            self.redraw_all();
        }
        // On a window, unless something of the server's own is open over it.
        let ours = self.menu_is_open() || self.launcher_is_open() || self.notify_at(at);
        let hit = if ours { Hit::Nothing } else { self.hit(at) };
        if let Hit::Surface(window, surface, origin) = hit {
            let serial = SERIAL_COUNTER.next_serial();
            self.focus_window(&window, serial);
            self.touch.on_windows.insert(slot, (serial, surface.clone(), at));
            if let Some(touch) = self.seat.get_touch() {
                touch.down(self, Some((surface, origin)), &DownEvent { slot, location: at, serial, time });
            }
            return;
        }
        // Elsewhere: the first finger is the pointer; others there do nothing.
        if self.touch.pointing.is_none() && self.touch.on_windows.is_empty() {
            let at = if ours { at } else { self.touch_meant(at) };
            self.touch.pointing = Some(slot);
            self.pointer_to(at, time);
            self.handle_button(BTN_LEFT, ButtonState::Pressed, time);
            // If it stays where it is, it is a long press.
            self.touch.presses += 1;
            self.touch.resting = (!ours).then_some(at);
            let press = self.touch.presses;
            let _ = self.loop_handle.insert_source(Timer::from_duration(LONG), move |_, _, state| {
                state.touch_long_press(press);
                TimeoutAction::Drop
            });
        }
    }

    /// Where a finger that came down at `at` meant: there, if something is
    /// there; else on what is within reach, a title bar's button before the
    /// bar, the dock, a window's edge or its title bar before the desktop.
    fn touch_meant(&self, at: Point<f64, Logical>) -> Point<f64, Logical> {
        let button = |h: &Hit| matches!(h, Hit::Close(_) | Hit::Maximize(_) | Hit::Minimize(_));
        let wanted: &dyn Fn(&Hit) -> bool = match self.hit(at) {
            Hit::Title(_) => &button,
            Hit::Nothing => &|h: &Hit| !matches!(h, Hit::Nothing | Hit::Surface(..) | Hit::Native(_)),
            _ => return at,
        };
        let d = REACH * 0.7;
        let around = [(-REACH, 0.0), (REACH, 0.0), (0.0, -REACH), (0.0, REACH), (-d, -d), (d, -d), (-d, d), (d, d)];
        around.iter().map(|(dx, dy)| Point::from((at.x + dx, at.y + dy))).find(|p| wanted(&self.hit(*p))).unwrap_or(at)
    }

    /// Half a second after press number `press`: if that finger is still
    /// down where it came down, on a dock icon or the bare desktop, it was
    /// the right button there.
    fn touch_long_press(&mut self, press: u64) {
        if press != self.touch.presses || self.touch.pointing.is_none() {
            return;
        }
        let Some(at) = self.touch.resting.take() else { return };
        let time = InputTime::now();
        match self.hit(at) {
            Hit::Shelf => {
                // The press the dock was holding is not a click any more.
                self.shelf.held = None;
                self.shelf_menu(at);
            }
            Hit::Nothing => {
                self.handle_button(BTN_LEFT, ButtonState::Released, time);
                self.open_menu(at);
            }
            _ => return,
        }
        // The finger has said what it had to: its lifting does nothing more.
        self.touch.pointing = None;
        self.redraw_all();
    }

    /// A window asks to be moved, giving the serial of the press that began
    /// it: if that press was a finger's on that window, the window goes
    /// with the finger from now on. (A pointer's press is the move grab's.)
    pub fn touch_move_request(&mut self, surface: &WlSurface, serial: Serial) {
        let Some((slot, down)) = self.touch.on_windows.iter().find(|(_, (s, on, _))| *s == serial && on.id().same_client_as(&surface.id())).map(|(slot, (_, _, at))| (*slot, *at)) else { return };
        let Some(window) = self.space.elements().find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface)).cloned() else { return };
        // A maximized window stays where it is.
        if self.restore.contains_key(&window) {
            tracing::info!("touch: a maximized window is not moved by a finger");
            return;
        }
        let Some(from) = self.space.element_location(&window) else { return };
        tracing::info!("touch: a finger moves {:?}", self.window_title(&window));
        self.touch.moving = Some(Moving { slot, window, from, down });
    }

    pub fn touch_motion(&mut self, slot: TouchSlot, at: Point<f64, Logical>, time: InputTime) {
        if self.touch.pointing == Some(slot) {
            if self.touch.resting.is_some_and(|r| (at.x - r.x).abs() > STILL || (at.y - r.y).abs() > STILL) {
                self.touch.resting = None;
            }
            self.pointer_to(at, time);
        } else if let Some(m) = self.touch.moving.as_ref().filter(|m| m.slot == slot) {
            // The window goes with the finger; its application hears no more of it.
            let to = (m.from.to_f64() + (at - m.down)).to_i32_round();
            let window = m.window.clone();
            self.space.map_element(window, to, true);
        } else if self.touch.on_windows.contains_key(&slot) {
            let focus = match self.hit(at) {
                Hit::Surface(_, surface, origin) => Some((surface, origin)),
                _ => None,
            };
            if let Some(touch) = self.seat.get_touch() {
                touch.motion(self, focus, &MotionEvent { slot, location: at, time });
            }
        }
    }

    pub fn touch_up(&mut self, slot: TouchSlot, time: InputTime) {
        if let Some(m) = self.touch.moving.take_if(|m| m.slot == slot) {
            self.remember(&m.window);
        }
        if self.touch.pointing == Some(slot) {
            self.touch.pointing = None;
            self.touch.resting = None;
            self.handle_button(BTN_LEFT, ButtonState::Released, time);
            // No finger rests where it tapped: no name appears beside a dock icon.
            self.shelf_unhover();
        } else if self.touch.on_windows.remove(&slot).is_some()
            && let Some(touch) = self.seat.get_touch()
        {
            touch.up(self, &UpEvent { slot, serial: SERIAL_COUNTER.next_serial(), time });
        }
    }

    /// The fingers of one moment have all been told.
    pub fn touch_frame(&mut self) {
        if let Some(touch) = self.seat.get_touch() {
            touch.frame(self);
        }
    }

    /// The panel took its touches back (a palm, a gesture of the system's).
    pub fn touch_cancel(&mut self, time: InputTime) {
        if self.touch.pointing.take().is_some() {
            self.handle_button(BTN_LEFT, ButtonState::Released, time);
            self.shelf_unhover();
        }
        self.touch.resting = None;
        self.touch.moving = None;
        self.touch.on_windows.clear();
        if let Some(touch) = self.seat.get_touch() {
            touch.cancel(self);
        }
    }

    /// For `vctl touch`.
    pub fn touch_report(&self) -> String {
        let ties: Vec<String> = self.touch.ties.iter().map(|(name, screen)| format!("{name:?} -> {screen}")).collect();
        format!(
            "ties: {}\npanels seen: {}\nfingers: {} as the pointer, {} on windows",
            if ties.is_empty() { "none".to_string() } else { ties.join("; ") },
            if self.touch.said.is_empty() { "none".to_string() } else { self.touch.said.iter().cloned().collect::<Vec<_>>().join("; ") },
            self.touch.pointing.iter().count(),
            self.touch.on_windows.len()
        )
    }
}
