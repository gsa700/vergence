//! Input: the keys (the terminal's shortcuts, the scrollback's, xterm's
//! sequences to the program), fingers (taps, drags, flicks, a long press),
//! the wheel, paste, and the timers they set. The window itself is in
//! main.rs.

use std::time::Instant;

use vergence_ui::bar::{Part, WinButton};
use vergence_ui::{App, Button, Input, Modifiers, Window, sym};

use crate::grid::MouseMode;
use crate::panes::Axis;
use crate::session::{Tab, lock};
use crate::{
    BLINK, DOUBLE_CLICK, FLICK_FRICTION, FLICK_MAX, FLICK_MIN, FLICK_STOP, FLICK_TICK, Held, ROOT_TICK, TIMER_BLINK,
    TIMER_FLICK, TIMER_LONG_PRESS, TIMER_ROOT, Terminal, WHEEL_LINES, ZOOM_STEP, mouse,
};

impl Terminal {
    /// A finger dragged `lines` lines down (up, if negative) on the pane at
    /// x, y: the text goes with it. In the scrollback that is the way back
    /// through it; a full-screen program, which has none, gets arrow keys, as
    /// it does from the wheel.
    fn finger_scroll(&mut self, x: f32, y: f32, lines: isize, window: &mut Window) {
        let Some((id, area)) = self.pane_at(x, y).filter(|_| y >= self.bar_h) else { return };
        // A program that asked for the mouse gets the wheel, a notch for every
        // WHEEL_LINES lines the finger has gone.
        let mouse = self.session_mut(id).is_some_and(|session| {
            let s = lock(&session.screen);
            s.grid.mouse != MouseMode::Off && s.scroll == 0
        });
        if mouse {
            self.dragged_lines += lines;
            while self.dragged_lines.abs() >= WHEEL_LINES {
                let up = self.dragged_lines > 0;
                self.dragged_lines -= if up { WHEEL_LINES } else { -WHEEL_LINES };
                self.report(
                    id,
                    area,
                    if up { mouse::Action::WheelUp } else { mouse::Action::WheelDown },
                    x,
                    y,
                    Modifiers::default(),
                );
            }
            return;
        }
        let Some(session) = self.session_mut(id) else { return };
        let mut s = lock(&session.screen);
        if s.grid.alt {
            let key = if s.grid.app_cursor { "\x1bO" } else { "\x1b[" }.to_string() + if lines > 0 { "A" } else { "B" };
            drop(s);
            session.pty.write(key.repeat(lines.unsigned_abs().min(60)).as_bytes());
        } else {
            s.scroll_by(lines);
            window.redraw();
        }
    }

    /// A finger dragged `dy` on the text it came down on at `from`: the
    /// text goes with it, a line for every line's height.
    fn finger_drag(&mut self, dy: f32, from: (f32, f32), window: &mut Window) {
        let line = self.cell().map_or(18.0, |(_, h)| h.max(1.0));
        self.dragged += dy;
        let lines = (self.dragged / line).trunc();
        self.dragged -= lines * line;
        if lines != 0.0 {
            self.finger_scroll(from.0, from.1, lines as isize, window);
        }
    }

    /// What a key sends to the program (xterm's sequences).
    pub(crate) fn key_bytes(sym_: u32, text: Option<&str>, m: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
        let csi = |s: &str| Some(format!("\x1b[{s}").into_bytes());
        let cursor = |c: char| Some(if app_cursor { format!("\x1bO{c}") } else { format!("\x1b[{c}") }.into_bytes());
        const KP_ENTER: u32 = 0xff8d;
        const BACK_TAB: u32 = 0xfe20;
        const INSERT: u32 = 0xff63;
        const F1: u32 = 0xffbe;
        match sym_ {
            sym::RETURN | KP_ENTER => return Some(b"\r".to_vec()),
            sym::BACKSPACE => return Some(if m.ctrl { vec![0x08] } else { vec![0x7f] }),
            sym::TAB => return Some(if m.shift { b"\x1b[Z".to_vec() } else { b"\t".to_vec() }),
            BACK_TAB => return Some(b"\x1b[Z".to_vec()),
            sym::ESCAPE => return Some(vec![0x1b]),
            sym::UP => return cursor('A'),
            sym::DOWN => return cursor('B'),
            sym::RIGHT => return cursor('C'),
            sym::LEFT => return cursor('D'),
            sym::HOME => return cursor('H'),
            sym::END => return cursor('F'),
            sym::PAGE_UP => return csi("5~"),
            sym::PAGE_DOWN => return csi("6~"),
            INSERT => return csi("2~"),
            sym::DELETE => return csi("3~"),
            s if (F1..F1 + 4).contains(&s) => return Some(vec![0x1b, b'O', b'P' + (s - F1) as u8]),
            s if (F1 + 4..F1 + 12).contains(&s) => {
                return csi(["15~", "17~", "18~", "19~", "20~", "21~", "23~", "24~"][(s - F1 - 4) as usize]);
            }
            0x20 if m.ctrl => return Some(vec![0]),
            _ => {}
        }
        // Ctrl+letter: the control character.
        if m.ctrl
            && let Some(ch) = char::from_u32(sym_).map(|c| c.to_ascii_lowercase())
        {
            let b = match ch {
                'a'..='z' => Some(ch as u8 - b'a' + 1),
                '[' => Some(0x1b),
                '\\' => Some(0x1c),
                ']' => Some(0x1d),
                '6' => Some(0x1e),
                '-' | '/' => Some(0x1f),
                _ => None,
            };
            if let Some(b) = b {
                return Some(if m.alt { vec![0x1b, b] } else { vec![b] });
            }
        }
        let t = text.filter(|t| !t.is_empty())?;
        let mut out = Vec::new();
        if m.alt {
            out.push(0x1b);
        }
        out.extend_from_slice(t.as_bytes());
        Some(out)
    }

    pub(crate) fn on_timer(&mut self, id: u64, window: &mut Window) {
        let now = Instant::now();
        match id {
            // The cursor blinks while the window has the keyboard.
            TIMER_BLINK => {
                if self.focused && self.blink {
                    self.blink_on = !self.blink_on;
                    window.redraw();
                    window.wake_after(TIMER_BLINK, BLINK);
                } else if !self.blink_on {
                    self.blink_on = true;
                    window.redraw();
                }
            }
            TIMER_ROOT => {
                let now_root: Vec<bool> = self.tabs.iter().map(Tab::as_root).collect();
                let told: Vec<bool> =
                    self.bar_told.as_ref().map(|(tabs, ..)| tabs.iter().map(|t| t.2).collect()).unwrap_or_default();
                if now_root != told {
                    window.redraw();
                }
                window.wake_after(TIMER_ROOT, ROOT_TICK);
            }
            // A finger held still on the text is the right button there: the menu.
            TIMER_LONG_PRESS => {
                if let Some(vergence_ui::touch::Gesture::LongPress { x, y }) = self.fingers.long_press(now)
                    && y >= self.bar_h
                {
                    let modifiers = Modifiers { shift: false, ctrl: false, alt: false, logo: false };
                    self.input(&Input::PointerMoved { x, y }, window);
                    self.input(&Input::Button { button: Button::Right, pressed: true, x, y, modifiers }, window);
                    self.input(&Input::Button { button: Button::Right, pressed: false, x, y, modifiers }, window);
                }
            }
            TIMER_FLICK => {
                let Some((v, from, last)) = self.flick else { return };
                let dt = now.saturating_duration_since(last).as_secs_f32().min(0.1);
                self.finger_drag(v * dt, from, window);
                let v = v * (-FLICK_FRICTION * dt).exp();
                if v.abs() < FLICK_STOP {
                    self.flick = None;
                    (self.dragged, self.dragged_lines) = (0.0, 0);
                } else {
                    self.flick = Some((v, from, now));
                    window.wake_after(TIMER_FLICK, FLICK_TICK);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn on_input(&mut self, input: &Input, window: &mut Window) {
        const SHIFT_L: u32 = 0xffe1;
        const SHIFT_R: u32 = 0xffe2;
        match input {
            // The key itself says Shift went down or up; a button says how things stand.
            Input::Key { sym: SHIFT_L | SHIFT_R, pressed, repeat: false, .. } => self.shift = *pressed,
            Input::Button { modifiers, .. } => self.shift = modifiers.shift,
            Input::Focus(false) => self.shift = false,
            _ => {}
        }
        // A finger (vergence_ui::touch): a tap is a click where it tapped; a
        // drag up or down scrolls what is under where the finger came down,
        // the text moving with the finger.
        if let Input::Touch { phase, .. } = input {
            use vergence_ui::touch::{Gesture, LONG_PRESS, Phase};
            match phase {
                // A finger coming down stops a flick, and may be a long press.
                Phase::Down => {
                    self.flick = None;
                    window.wake_after(TIMER_LONG_PRESS, LONG_PRESS);
                }
                Phase::Up | Phase::Cancelled => window.cancel_timer(TIMER_LONG_PRESS),
                Phase::Moved => {}
            }
            match self.fingers.read(input) {
                Some(Gesture::Tap { x, y }) => {
                    let modifiers = Modifiers { shift: false, ctrl: false, alt: false, logo: false };
                    self.input(&Input::PointerMoved { x, y }, window);
                    self.input(&Input::Button { button: Button::Left, pressed: true, x, y, modifiers }, window);
                    self.input(&Input::Button { button: Button::Left, pressed: false, x, y, modifiers }, window);
                    self.input(&Input::PointerLeft, window);
                }
                // A drag that began on the bar is the pointer's with its button
                // held: the window is moved by its bar, a tab along the row.
                Some(Gesture::Drag { x, y, from, .. }) if from.1 < self.bar_h => {
                    if self.finger_pointer.is_none() {
                        let modifiers = Modifiers { shift: false, ctrl: false, alt: false, logo: false };
                        self.input(&Input::PointerMoved { x: from.0, y: from.1 }, window);
                        self.input(
                            &Input::Button { button: Button::Left, pressed: true, x: from.0, y: from.1, modifiers },
                            window,
                        );
                    }
                    self.finger_pointer = Some((x, y));
                    self.input(&Input::PointerMoved { x, y }, window);
                }
                Some(Gesture::Drag { dy, from, .. }) => {
                    window.cancel_timer(TIMER_LONG_PRESS);
                    self.drag_from = Some(from);
                    self.finger_drag(dy, from, window);
                }
                Some(Gesture::DragEnd { vy, .. }) => {
                    // A flick of the text goes on, slowing, after the finger lifts.
                    if self.finger_pointer.is_none()
                        && let Some(from) = self.drag_from.take()
                        && vy.abs() > FLICK_MIN
                    {
                        self.flick = Some((vy.clamp(-FLICK_MAX, FLICK_MAX), from, Instant::now()));
                        window.wake_after(TIMER_FLICK, FLICK_TICK);
                    } else {
                        (self.dragged, self.dragged_lines) = (0.0, 0);
                    }
                    if let Some((x, y)) = self.finger_pointer.take() {
                        let modifiers = Modifiers { shift: false, ctrl: false, alt: false, logo: false };
                        self.input(&Input::Button { button: Button::Left, pressed: false, x, y, modifiers }, window);
                        self.input(&Input::PointerLeft, window);
                    }
                }
                Some(Gesture::LongPress { .. }) | None => {}
            }
            return;
        }
        if self.menu_input(input, window) {
            return;
        }
        match input {
            // With something held, the pointer's moves are that thing's.
            Input::PointerMoved { x, y } if self.held.is_some() => match &self.held {
                Some(Held::Text) => self.drag_text(*x, *y, window),
                Some(Held::Thumb(grab)) => self.drag_thumb(*y, *grab, window),
                Some(Held::Divider(path)) => {
                    let path = path.clone();
                    if let Some(d) = self.areas().1.into_iter().find(|d| d.path == path)
                        && let Some(tab) = self.tabs.get_mut(self.shown)
                    {
                        tab.tree.set_share(&path, d.share_at(*x, *y));
                        window.redraw();
                    }
                }
                Some(Held::Reported(button)) => {
                    // The program hears of the drag wherever the pointer goes.
                    let button = *button;
                    if let Some((id, area)) = self.active() {
                        self.report(id, area, mouse::Action::Motion(Some(button)), *x, *y, Modifiers::default());
                    }
                }
                None => {}
            },
            Input::PointerMoved { x, y } => self.hover(*x, *y, window),
            Input::PointerLeft => {
                self.over = None;
                if self.hover.take().is_some() {
                    window.redraw();
                }
            }
            Input::Button { button, pressed: false, x, y, modifiers } if self.held.is_some() => match &self.held {
                Some(Held::Reported(told)) if told == button => {
                    self.held = None;
                    if let Some((id, area)) = self.active() {
                        self.report(id, area, mouse::Action::Release(*button), *x, *y, *modifiers);
                    }
                }
                Some(Held::Reported(_)) => {}
                Some(Held::Text) if *button == Button::Left => {
                    self.held = None;
                    self.edge = None;
                    // A press that selected nothing leaves nothing selected.
                    if let Some(session) = self.front() {
                        let mut s = lock(&session.screen);
                        if s.selection.is_some_and(|sel| sel.range().is_none()) {
                            s.selection = None;
                        }
                    }
                    window.redraw();
                }
                Some(Held::Thumb(_) | Held::Divider(_)) if *button == Button::Left => {
                    self.held = None;
                    window.redraw();
                }
                _ => {}
            },
            // One thing is held at a time: other buttons wait for its release.
            Input::Button { pressed: true, .. } if self.held.is_some() => {}
            Input::Button { button, pressed: true, x, y, modifiers } => match (button, self.bar_at(*x, *y)) {
                (_, None) if *y >= self.bar_h => self.press(*button, *x, *y, *modifiers, window),
                (Button::Left, Some(Part::Button(WinButton::Close))) => window.close(),
                (Button::Left, Some(Part::Button(WinButton::Minimize))) => window.minimize(),
                (Button::Left, Some(Part::Button(WinButton::Maximize))) => {
                    self.maximized = !self.maximized;
                    window.set_maximized(self.maximized);
                }
                (Button::Left, Some(Part::Tab(i))) => self.show_tab(i, window),
                (Button::Left, Some(Part::CloseTab(i))) | (Button::Middle, Some(Part::Tab(i) | Part::CloseTab(i))) => {
                    self.close_tab(i, window)
                }
                (Button::Left, Some(Part::Plus)) => self.new_tab(window),
                (Button::Left, Some(Part::Bar)) => {
                    // Twice on the bare bar: larger or back. Once: the user moves the window.
                    if self.bar_press.take().is_some_and(|t| t.elapsed() < DOUBLE_CLICK) {
                        self.maximized = !self.maximized;
                        window.set_maximized(self.maximized);
                    } else {
                        self.bar_press = Some(Instant::now());
                        window.start_move();
                    }
                }
                _ => {}
            },
            Input::Key { sym: key, text, pressed: true, composing: false, modifiers, .. } => {
                let m = *modifiers;
                // Typing shows the cursor at once; it blinks again from there.
                if self.blink && !self.blink_on {
                    self.blink_on = true;
                    window.redraw();
                }
                let letter = |c: char| *key == c as u32 || *key == c.to_ascii_uppercase() as u32;
                // The terminal's own shortcuts: with Ctrl+Shift, so the shell keeps Ctrl+T and the rest.
                // Ctrl+, : the terminal's settings, as every app's.
                if m.ctrl && !m.alt && !m.shift && *key == 0x2c {
                    return self.open_settings(window);
                }
                // Text size for a while: Ctrl with = (or +), - and 0, as GNOME Terminal.
                // Ctrl+_ (Shift with -) stays the shell's undo.
                if m.ctrl && !m.alt {
                    match *key {
                        0x3d | 0x2b | 0xffab => return self.zoom_by(Some(ZOOM_STEP), window),
                        0x2d | 0xffad if !m.shift => return self.zoom_by(Some(-ZOOM_STEP), window),
                        0x30 | 0xffb0 if !m.shift => return self.zoom_by(None, window),
                        _ => {}
                    }
                }
                if m.ctrl && m.shift {
                    if letter('n') {
                        return self.new_window(window);
                    }
                    if letter('t') {
                        return self.new_tab(window);
                    }
                    if letter('w') {
                        return self.close_active(window);
                    }
                    if letter('r') {
                        return self.split(Axis::Side, window);
                    }
                    if letter('d') {
                        return self.split(Axis::Stack, window);
                    }
                    if letter('c') {
                        self.copy(window);
                        return;
                    }
                }
                // The next tab and the one before: Ctrl+PageDown and Ctrl+PageUp, or
                // Ctrl+Tab and, with Shift, back (which arrives as its own key).
                const BACK_TAB: u32 = 0xfe20;
                let step = match *key {
                    sym::PAGE_UP if m.ctrl && !m.shift => Some(false),
                    sym::PAGE_DOWN if m.ctrl && !m.shift => Some(true),
                    sym::TAB if m.ctrl && !m.alt => Some(!m.shift),
                    BACK_TAB if m.ctrl && !m.alt => Some(false),
                    _ => None,
                };
                if let Some(forward) = step {
                    let n = self.tabs.len().max(1);
                    return self
                        .show_tab(if forward { (self.shown + 1) % n } else { (self.shown + n - 1) % n }, window);
                }
                // Ctrl+V pastes, with Shift or without (Dave, 2026-11-09). The control
                // character it used to send (the shell's "take the next key as it is") is
                // given up for it.
                if m.ctrl && !m.alt && letter('v') {
                    return window.paste();
                }
                // Ctrl+C copies when there is something selected, and lets it go, so that
                // the next Ctrl+C is the interrupt key again; with nothing selected it
                // always is.
                if m.ctrl && !m.alt && !m.shift && letter('c') && self.copy(window) {
                    if let Some(session) = self.front() {
                        lock(&session.screen).selection = None;
                    }
                    return window.redraw();
                }
                let Some(session) = self.front_mut() else { return };
                let mut s = lock(&session.screen);
                // The scrollback: with Shift, PageUp and PageDown a page, Home and End
                // its two ends. On the alternate screen there is none, and the keys
                // are the program's.
                if m.shift
                    && !m.ctrl
                    && !m.alt
                    && !s.grid.alt
                    && matches!(*key, sym::PAGE_UP | sym::PAGE_DOWN | sym::HOME | sym::END)
                {
                    let page = s.grid.rows.saturating_sub(1).max(1) as isize;
                    s.scroll_by(match *key {
                        sym::PAGE_UP => page,
                        sym::PAGE_DOWN => -page,
                        sym::HOME => isize::MAX,
                        _ => isize::MIN,
                    });
                } else if let Some(bytes) = Terminal::key_bytes(*key, text.as_deref(), m, s.grid.app_cursor) {
                    // Typing goes to the live screen, and what was selected is let go.
                    s.scroll = 0;
                    s.selection = None;
                    drop(s);
                    session.pty.write(&bytes);
                }
                window.redraw();
            }
            Input::Paste(text) => {
                let Some(session) = self.front_mut() else { return };
                let bracketed = lock(&session.screen).grid.bracketed_paste;
                // A pasted line end is the Return key; and the paste cannot end its own brackets.
                let text = text.replace("\r\n", "\r").replace('\n', "\r").replace("\x1b[201~", "");
                let bytes = if bracketed { format!("\x1b[200~{text}\x1b[201~") } else { text };
                session.pty.write(bytes.as_bytes());
            }
            Input::Scroll { x, y, dy, .. } => {
                // The wheel is the pane's under the pointer, whichever has the keys.
                let Some((id, area)) = self.pane_at(*x, *y).filter(|_| *y >= self.bar_h) else { return };
                let notches = (dy / 15.0).round() as isize;
                // A program with the mouse gets the wheel, a report for each notch.
                if !self.shift && notches != 0 {
                    let action = if notches < 0 { mouse::Action::WheelUp } else { mouse::Action::WheelDown };
                    if self.report(id, area, action, *x, *y, Modifiers::default()) {
                        for _ in 1..notches.unsigned_abs().min(20) {
                            self.report(id, area, action, *x, *y, Modifiers::default());
                        }
                        return;
                    }
                }
                let Some(session) = self.session_mut(id) else { return };
                let mut s = lock(&session.screen);
                if s.grid.alt {
                    // A full-screen program has no scrollback: the wheel is its arrow keys.
                    let key = if s.grid.app_cursor { "\x1bO" } else { "\x1b[" }.to_string()
                        + if notches < 0 { "A" } else { "B" };
                    drop(s);
                    session.pty.write(key.repeat((notches.unsigned_abs() * WHEEL_LINES as usize).min(60)).as_bytes());
                } else {
                    s.scroll_by(-notches * WHEEL_LINES);
                    window.redraw();
                }
            }
            Input::Focus(on) => {
                self.focused = *on;
                self.blink_on = true;
                if *on && self.blink {
                    window.wake_after(TIMER_BLINK, BLINK);
                }
                window.redraw();
            }
            _ => {}
        }
    }
}
