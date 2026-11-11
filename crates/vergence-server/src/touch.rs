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
// Not yet: a way to make the tie by touching the screen (settings app);
// hit areas forgiving of a finger; a long press as the right button; the
// pointer's arrow stays where the finger last was.

use std::collections::HashSet;

use smithay::{
    backend::input::{ButtonState, InputTime, TouchSlot},
    input::touch::{DownEvent, MotionEvent, UpEvent},
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
};

use crate::{Server, decor::Hit};

const BTN_LEFT: u32 = 0x110;

#[derive(Default)]
pub struct Touch {
    /// Panels tied to screens: the panel's name, the screen's id.
    ties: Vec<(String, String)>,
    /// The finger that is acting as the pointer.
    pointing: Option<TouchSlot>,
    /// The fingers that are on windows.
    on_windows: HashSet<TouchSlot>,
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
        // On a window, unless something of the server's own is open over it.
        let ours = self.menu_is_open() || self.launcher_is_open() || self.notify_at(at);
        let hit = if ours { Hit::Nothing } else { self.hit(at) };
        if let Hit::Surface(window, surface, origin) = hit {
            let serial = SERIAL_COUNTER.next_serial();
            self.focus_window(&window, serial);
            self.touch.on_windows.insert(slot);
            if let Some(touch) = self.seat.get_touch() {
                touch.down(self, Some((surface, origin)), &DownEvent { slot, location: at, serial, time });
            }
            return;
        }
        // Elsewhere: the first finger is the pointer; others there do nothing.
        if self.touch.pointing.is_none() && self.touch.on_windows.is_empty() {
            self.touch.pointing = Some(slot);
            self.pointer_to(at, time);
            self.handle_button(BTN_LEFT, ButtonState::Pressed, time);
        }
    }

    pub fn touch_motion(&mut self, slot: TouchSlot, at: Point<f64, Logical>, time: InputTime) {
        if self.touch.pointing == Some(slot) {
            self.pointer_to(at, time);
        } else if self.touch.on_windows.contains(&slot) {
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
        if self.touch.pointing == Some(slot) {
            self.touch.pointing = None;
            self.handle_button(BTN_LEFT, ButtonState::Released, time);
            // No finger rests where it tapped: no name appears beside a dock icon.
            self.shelf_unhover();
        } else if self.touch.on_windows.remove(&slot)
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
