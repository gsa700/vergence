// Touch, the plain version: basic touch beside a keyboard and, later, a
// spoken interface; an on-screen keyboard later if ever (see
// docs/shelf-milestone-1.md).
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
// Which finger goes where is the seat's (crates/vergence-seat, touch.rs there;
// seat milestone 1, step 4): this file says which screen a panel is on, where
// a finger meant, when half a second has passed, and carries out what the
// seat decided for the fingers (Wayland touch through Smithay, a native
// window's over the bus, a window going with its finger).
//
// The settings app's Touchscreen page makes the tie from a list of screens;
// not yet by touching the screen itself.

use std::collections::{HashMap, HashSet};

use smithay::{
    backend::input::{InputTime, TouchSlot},
    input::touch::{DownEvent, MotionEvent, UpEvent},
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        wayland_server::{Resource, protocol::wl_surface::WlSurface},
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Serial},
};
use vergence_seat::{Out, Slot, Target, WinId};

use crate::{Server, decor::Hit, dispatch::Came};

/// How far off its mark a finger may come down and still have meant it, in points.
const REACH: f64 = 10.0;
/// How long a finger rests for a long press (how far it may wander is the seat's).
const LONG: std::time::Duration = std::time::Duration::from_millis(500);

#[derive(Default)]
pub struct Touch {
    /// Panels tied to screens: the panel's name, the screen's id.
    ties: Vec<(String, String)>,
    /// Each finger down: its slot as the panel named it, the serial of its
    /// down on a Wayland surface (a window's move request names it), and
    /// where it was last.
    fingers: HashMap<Slot, (TouchSlot, Option<(Serial, WlSurface)>, Point<f64, Logical>)>,
    /// Where the finger acting as the pointer came down (a long press's menu opens there).
    pointing_at: Option<Point<f64, Logical>>,
    /// A window going with a finger: where the window was.
    moving_from: Option<(WinId, Point<i32, Logical>)>,
    /// A finger was the last thing to point: the pointer's picture is not
    /// drawn until a mouse moves again (an arrow, or a window edge's shape,
    /// left where a finger lifted, points at nothing).
    pub fingers_last: bool,
    /// Panels already named in the log.
    said: HashSet<String>,
    /// Smithay's touch was told something since the last frame (a frame with
    /// nothing in it is a client bug in Smithay's eyes).
    unframed: bool,
}

/// The ties desktop.toml records.
pub fn load() -> Vec<(String, String)> {
    let table = crate::wallpaper::desktop_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| t.parse::<toml::Table>().ok());
    let panels = table.as_ref().and_then(|t| t.get("touch")?.get("panel")?.as_array());
    panels
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p.get("name")?.as_str()?.to_string(), p.get("screen")?.as_str()?.to_string())))
        .take(16)
        .collect()
}

fn slot_number(slot: TouchSlot) -> Slot {
    i32::from(slot) as Slot
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

    /// The id of the screen the panel named `panel` is on: its tie in
    /// desktop.toml, else the primary screen (as `touch_screen`, without the log).
    pub fn touch_screen_id(&self, panel: &str) -> Option<String> {
        let ids: Vec<String> = self.bus_outputs().iter().map(|o| o.id.clone()).collect();
        let tied = self
            .touch
            .ties
            .iter()
            .find(|(name, _)| name == panel)
            .map(|(_, screen)| screen.clone())
            .filter(|id| ids.contains(id));
        tied.or_else(|| vergence_profile::primary(self.shelf.primary.as_deref(), &ids).map(str::to_string))
    }

    /// The screen the panel named `panel` is on.
    pub fn touch_screen(&mut self, panel: &str) -> Option<Rectangle<i32, Logical>> {
        let outputs = self.bus_outputs();
        let ids: Vec<String> = outputs.iter().map(|o| o.id.clone()).collect();
        let tied = self
            .touch
            .ties
            .iter()
            .find(|(name, _)| name == panel)
            .map(|(_, screen)| screen.as_str())
            .filter(|id| ids.iter().any(|i| i == id));
        let id = tied.or_else(|| vergence_profile::primary(self.shelf.primary.as_deref(), &ids))?.to_string();
        if self.touch.said.insert(panel.to_string()) {
            tracing::info!(
                "touch: panel {panel:?} on screen {id}{}",
                if tied.is_some() { "" } else { " (the primary one: no tie in desktop.toml)" }
            );
        }
        let o = outputs.iter().find(|o| o.id == id)?;
        Some(Rectangle::new((o.x, o.y).into(), (o.width, o.height).into()))
    }

    /// A finger came down at `at`. The touch panel, and the control socket's
    /// `touch`, both come here.
    pub fn touch_down(&mut self, slot: TouchSlot, at: Point<f64, Logical>, time: InputTime) {
        if !self.touch.fingers_last {
            self.touch.fingers_last = true;
            self.redraw_all();
        }
        self.dispatch_sync();
        // Over something of the server's own that is open, a finger is the
        // pointer where it is; elsewhere, where it meant.
        let ours = self.menu_is_open() || self.launcher_is_open() || self.notify_at(at);
        let at = if ours { at } else { self.touch_meant(at) };
        let n = slot_number(slot);
        self.touch.fingers.insert(n, (slot, None, at));
        self.pointer_hover(at);
        let under = self.target_at(at);
        let now = self.start_time.elapsed().as_millis() as u64;
        let out = self.input.seat.touch_down(n, at.x, at.y, under, now);
        if out.iter().any(|o| matches!(o, Out::PointerTo { .. })) {
            self.touch.pointing_at = Some(at);
        }
        self.touch_apply(out, at, time, under);
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

    /// A window asks to be moved, giving the serial of the press that began
    /// it: if that press was a finger's on that window, the window goes
    /// with the finger from now on (the seat decides). A maximized window stays.
    pub fn touch_move_request(&mut self, surface: &WlSurface, serial: Serial) {
        // The press must be this program's own (§7.3: no moving by another's serial).
        let Some((n, down)) = self
            .touch
            .fingers
            .iter()
            .find(|(_, (_, s, _))| {
                s.as_ref().is_some_and(|(sr, on)| *sr == serial && on.id().same_client_as(&surface.id()))
            })
            .map(|(n, (_, _, at))| (*n, *at))
        else {
            return;
        };
        let Some(window) =
            self.space.elements().find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface)).cloned()
        else {
            return;
        };
        if self.restore.contains_key(&window) {
            return tracing::info!("touch: a maximized window is not moved by a finger");
        }
        let Some(from) = self.space.element_location(&window) else { return };
        let id = self.win_id(&window);
        if self.input.seat.touch_move_request(n, id, down.x, down.y) {
            tracing::info!("touch: a finger moves {:?}", self.window_title(&window));
            self.touch.moving_from = Some((id, from));
        }
    }

    pub fn touch_motion(&mut self, slot: TouchSlot, at: Point<f64, Logical>, time: InputTime) {
        let n = slot_number(slot);
        if let Some(f) = self.touch.fingers.get_mut(&n) {
            f.2 = at;
        }
        self.dispatch_sync();
        self.pointer_hover(at);
        let under = self.target_at(at);
        let out = self.input.seat.touch_motion(n, at.x, at.y, under);
        self.touch_apply(out, at, time, under);
    }

    pub fn touch_up(&mut self, slot: TouchSlot, time: InputTime) {
        let n = slot_number(slot);
        let at = self.touch.fingers.get(&n).map(|f| f.2).unwrap_or_else(|| self.pointer_location());
        self.dispatch_sync();
        let under = self.target_at(at);
        let now = self.start_time.elapsed().as_millis() as u64;
        let out = self.input.seat.touch_up(n, under, now);
        self.touch_apply(out, at, time, under);
        self.touch.fingers.remove(&n);
    }

    /// The fingers of one moment have all been told.
    pub fn touch_frame(&mut self) {
        if !std::mem::take(&mut self.touch.unframed) {
            return;
        }
        if let Some(touch) = self.seat.get_touch() {
            touch.frame(self);
        }
    }

    /// The panel took its touches back (a palm, a gesture of the system's).
    pub fn touch_cancel(&mut self, time: InputTime) {
        let at = self.pointer_location();
        let under = self.target_at(at);
        let now = self.start_time.elapsed().as_millis() as u64;
        let out = self.input.seat.touch_cancel(under, now);
        self.touch_apply(out, at, time, under);
        if let Some(touch) = self.seat.get_touch() {
            touch.cancel(self);
        }
        self.touch.fingers.clear();
        self.touch.moving_from = None;
    }

    /// Carry out what the seat decided for a finger at `at`.
    fn touch_apply(&mut self, out: Vec<Out>, at: Point<f64, Logical>, time: InputTime, under: Option<Target>) {
        for o in out {
            match o {
                Out::PointerTo { .. } => self.pointer_follow(at, time, under),
                Out::TouchDown(t, n) | Out::TouchMotion(t, n) | Out::TouchUp(t, n) | Out::TouchCancel(t, n) => {
                    self.touch_deliver(o, t, n, at, time)
                }
                Out::LongPressTimer(press) => {
                    let _ = self.loop_handle.insert_source(Timer::from_duration(LONG), move |_, _, st| {
                        st.touch_long_press(press);
                        TimeoutAction::Drop
                    });
                }
                Out::DockMenu => {
                    // The press the dock was holding is not a click any more.
                    self.shelf.held = None;
                    self.shelf_menu(at);
                    self.redraw_all();
                }
                Out::DesktopMenu => {
                    self.open_menu(at);
                    self.redraw_all();
                }
                Out::FingerMove { window, dx, dy } => {
                    if let (Some((id, from)), Some(w)) = (self.touch.moving_from, self.input.window(window))
                        && id == window
                    {
                        let to = (from.to_f64() + Point::from((dx, dy))).to_i32_round();
                        self.put(w, to, true);
                    }
                }
                Out::FingerMoveEnd(window) => {
                    self.touch.moving_from = None;
                    if let Some(w) = self.input.window(window) {
                        self.remember(&w);
                    }
                }
                // No finger rests where it tapped: no name appears beside a dock icon.
                Out::Unhover => self.shelf_unhover(),
                other => self.apply(vec![other], at, time, &Came::Nothing),
            }
        }
    }

    /// A finger on a window's content: Wayland touch through Smithay, or a
    /// native window's over the bus (phase 0 down, 1 moved, 2 up, 3 canceled).
    fn touch_deliver(&mut self, o: Out, t: Target, n: Slot, at: Point<f64, Logical>, time: InputTime) {
        let Some(&(slot, _, _)) = self.touch.fingers.get(&n) else { return };
        match t {
            Target::Surface(_, sid) => {
                let Some(touch) = self.seat.get_touch() else { return };
                self.touch.unframed = true;
                match o {
                    Out::TouchDown(..) => {
                        let serial = SERIAL_COUNTER.next_serial();
                        let focus = self.surface_of(sid);
                        if let (Some(f), Some((on, _))) = (self.touch.fingers.get_mut(&n), focus.as_ref()) {
                            f.1 = Some((serial, on.clone()));
                        }
                        touch.down(self, focus, &DownEvent { slot, location: at, serial, time });
                    }
                    Out::TouchMotion(..) => {
                        let focus = self.surface_of(sid);
                        touch.motion(self, focus, &MotionEvent { slot, location: at, time });
                    }
                    Out::TouchUp(..) => touch.up(self, &UpEvent { slot, serial: SERIAL_COUNTER.next_serial(), time }),
                    _ => {}
                }
            }
            Target::Native(id) => {
                let Some(w) = self.input.window(id) else { return };
                let phase = match o {
                    Out::TouchDown(..) => 0,
                    Out::TouchMotion(..) => 1,
                    Out::TouchUp(..) => 2,
                    _ => 3,
                };
                self.native_touch(&w, slot.into(), phase, at, time.millis());
            }
            Target::Server(_) => {}
        }
    }

    /// Half a second after press number `press` (the seat knows whether that
    /// finger is still resting on a dock icon or the bare desktop).
    fn touch_long_press(&mut self, press: u64) {
        let at = self.touch.pointing_at.unwrap_or_else(|| self.pointer_location());
        let out = self.input.seat.touch_long_press(press);
        self.touch_apply(out, at, InputTime::now(), None);
    }

    /// For `vctl touch`.
    pub fn touch_report(&self) -> String {
        let ties: Vec<String> = self.touch.ties.iter().map(|(name, screen)| format!("{name:?} -> {screen}")).collect();
        let (pointing, on_windows) = self.input.seat.touch_counts();
        format!(
            "ties: {}\npanels seen: {}\nfingers: {} as the pointer, {} on windows",
            if ties.is_empty() { "none".to_string() } else { ties.join("; ") },
            if self.touch.said.is_empty() {
                "none".to_string()
            } else {
                self.touch.said.iter().cloned().collect::<Vec<_>>().join("; ")
            },
            pointing,
            on_windows
        )
    }
}
