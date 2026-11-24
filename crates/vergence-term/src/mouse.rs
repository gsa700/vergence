//! The mouse as the program in the terminal sees it (xterm's mouse
//! reporting): what a press, a release, a move or the wheel sends, for the
//! mode the program asked for.

use crate::grid::MouseMode;
use vergence_ui::{Button, Modifiers};

/// A mouse event for the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Press(Button),
    Release(Button),
    /// The pointer moved to another cell, with a button held or not.
    Motion(Option<Button>),
    WheelUp,
    WheelDown,
}

/// The three buttons programs know, as xterm numbers them.
fn number(button: Button) -> Option<u32> {
    match button {
        Button::Left => Some(0),
        Button::Middle => Some(1),
        Button::Right => Some(2),
        Button::Other(_) => None,
    }
}

/// What a mouse event sends to the program, or None if its mode does not
/// ask for the event. `cell` is (column, row) of the screen, from 0.
/// SGR (mode 1006): ESC [ < code ; x ; y M (press, motion) or m (release).
/// Otherwise the old encoding, ESC [ M and three bytes of 32 + value, which
/// cannot say a column or row past 223 (then nothing is sent).
pub fn bytes(mode: MouseMode, sgr: bool, action: Action, cell: (usize, usize), mods: Modifiers) -> Option<Vec<u8>> {
    let (mut code, release) = match (mode, action) {
        (MouseMode::Off, _) => return None,
        (MouseMode::Press, Action::Release(_) | Action::Motion(_)) => return None,
        (MouseMode::Click, Action::Motion(_)) | (MouseMode::Drag, Action::Motion(None)) => return None,
        (_, Action::Press(b)) => (number(b)?, false),
        // The old encoding cannot say which button was released: 3.
        (_, Action::Release(b)) => (if sgr { number(b)? } else { 3 }, true),
        (_, Action::Motion(None)) => (32 + 3, false),
        (_, Action::Motion(Some(b))) => (32 + number(b)?, false),
        (_, Action::WheelUp) => (64, false),
        (_, Action::WheelDown) => (65, false),
    };
    // X10 mode reports no modifiers.
    if mode != MouseMode::Press {
        code += if mods.shift { 4 } else { 0 } + if mods.alt { 8 } else { 0 } + if mods.ctrl { 16 } else { 0 };
    }
    let (x, y) = (cell.0 + 1, cell.1 + 1);
    if sgr {
        return Some(format!("\x1b[<{code};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes());
    }
    if 32 + x > 255 || 32 + y > 255 {
        return None;
    }
    Some(vec![0x1b, b'[', b'M', 32 + code as u8, (32 + x) as u8, (32 + y) as u8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use Action::*;
    use Button::*;

    const NONE: Modifiers = Modifiers { shift: false, ctrl: false, alt: false, logo: false, caps_lock: false, num_lock: false };

    fn sgr(mode: MouseMode, a: Action, cell: (usize, usize), m: Modifiers) -> Option<String> {
        bytes(mode, true, a, cell, m).map(|b| String::from_utf8(b).unwrap())
    }

    #[test]
    fn sgr_reports() {
        let ctrl = Modifiers { ctrl: true, ..NONE };
        let alt = Modifiers { alt: true, ..NONE };
        assert_eq!(sgr(MouseMode::Click, Press(Left), (69, 23), NONE).as_deref(), Some("\x1b[<0;70;24M"));
        assert_eq!(sgr(MouseMode::Click, Release(Left), (69, 23), NONE).as_deref(), Some("\x1b[<0;70;24m"));
        assert_eq!(sgr(MouseMode::Click, Press(Right), (0, 0), ctrl).as_deref(), Some("\x1b[<18;1;1M"));
        assert_eq!(sgr(MouseMode::Click, WheelUp, (4, 2), NONE).as_deref(), Some("\x1b[<64;5;3M"));
        assert_eq!(sgr(MouseMode::Click, WheelDown, (4, 2), NONE).as_deref(), Some("\x1b[<65;5;3M"));
        // Motion only in the modes that ask for it.
        assert_eq!(sgr(MouseMode::Click, Motion(Some(Left)), (1, 1), NONE), None);
        assert_eq!(sgr(MouseMode::Drag, Motion(Some(Left)), (1, 1), NONE).as_deref(), Some("\x1b[<32;2;2M"));
        assert_eq!(sgr(MouseMode::Drag, Motion(None), (1, 1), NONE), None);
        assert_eq!(sgr(MouseMode::Motion, Motion(None), (1, 1), NONE).as_deref(), Some("\x1b[<35;2;2M"));
        // Off: nothing. X10: presses only, no modifiers. Buttons past the third: nothing.
        assert_eq!(sgr(MouseMode::Off, Press(Left), (0, 0), NONE), None);
        assert_eq!(sgr(MouseMode::Press, Release(Left), (0, 0), NONE), None);
        assert_eq!(sgr(MouseMode::Press, Press(Middle), (0, 0), alt).as_deref(), Some("\x1b[<1;1;1M"));
        assert_eq!(sgr(MouseMode::Click, Press(Other(0x113)), (0, 0), NONE), None);
    }

    #[test]
    fn the_old_encoding() {
        let b = bytes(MouseMode::Click, false, Press(Left), (69, 23), NONE).unwrap();
        assert_eq!(b, [0x1b, b'[', b'M', 32, 32 + 70, 32 + 24]);
        // Releases are button 3; past column 223 nothing can be sent.
        let b = bytes(MouseMode::Click, false, Release(Right), (0, 0), NONE).unwrap();
        assert_eq!(b[3], 35);
        assert_eq!(bytes(MouseMode::Click, false, Press(Left), (230, 0), NONE), None);
    }
}
