//! The look of the desktop's own drawing, in one place (Dave, 2026-11-10:
//! "I want consistency throughout").
//!
//! The server draws title bars and borders; the toolkit draws window bars,
//! tabs, buttons, menus and scrollbars. They are to look like one thing, so
//! the numbers both draw with live here, and neither keeps a copy: a change
//! here changes both. Lengths are in reference units (points at 96 to the
//! inch, whitepaper Contract 1); colours are sRGB.
//!
//! These are the defaults of a dark desktop. What the user's settings
//! change (the accent, light or dark, contrast) is applied over them by
//! whoever draws; this crate knows nothing of settings.

/// A colour, sRGB, opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Red, green, blue and an opaque alpha, each from 0 to 1.
    pub const fn floats(self) -> [f32; 4] {
        [self.0 as f32 / 255.0, self.1 as f32 / 255.0, self.2 as f32 / 255.0, 1.0]
    }

    /// Red, green and blue, each from 0 to 1.
    pub const fn floats3(self) -> [f32; 3] {
        [self.0 as f32 / 255.0, self.1 as f32 / 255.0, self.2 as f32 / 255.0]
    }
}

/// **The one corner radius**: of a button's shape, a tab's top corners, a
/// menu and its lines, a label, a scrollbar's ends as far as its width
/// allows, and a window's own corners when the server rounds them.
pub const RADIUS: f32 = 5.0;

/// The height of the title bar the server draws over a window that has none.
pub const TITLE_H: f32 = 24.0;
/// The height of the bar a toolkit window draws for itself (room for tabs),
/// and of the tabs and buttons that stand on its bottom edge.
pub const BAR_H: f32 = 36.0;
pub const BAR_ITEM_H: f32 = 30.0;
/// The border line around a window, and the band around it that resizes.
pub const EDGE: f32 = 1.0;
pub const RESIZE_BAND: f32 = 6.0;

/// A bar's ground, and what is written and drawn on it, for the window
/// that has the keyboard and for the others.
pub const BAR_FOCUSED: Rgb = Rgb(51, 61, 82);
pub const BAR_UNFOCUSED: Rgb = Rgb(56, 56, 56);
pub const INK_FOCUSED: Rgb = Rgb(235, 235, 235);
pub const INK_UNFOCUSED: Rgb = Rgb(158, 158, 158);
/// The border line around the window that has the keyboard, and the others.
pub const EDGE_FOCUSED: Rgb = Rgb(107, 122, 158);
pub const EDGE_UNFOCUSED: Rgb = Rgb(87, 87, 87);
/// The close button under the pointer, and the server's own close button.
pub const CLOSE: Rgb = Rgb(199, 77, 71);
/// The server's other title-bar buttons.
pub const BUTTON: Rgb = Rgb(184, 184, 184);

/// A shape on a bar that is not the one in front (a tab behind, a button,
/// +): white at this much of 255, at rest and under the pointer.
pub const SHAPE_REST: u8 = 20;
pub const SHAPE_HOVER: u8 = 41;

/// A menu's ground, its text, and its text for what cannot be chosen.
pub const MENU: Rgb = Rgb(43, 46, 59);
pub const MENU_INK: Rgb = Rgb(235, 235, 235);
pub const MENU_INK_OFF: Rgb = Rgb(128, 128, 134);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_as_fractions() {
        assert_eq!(Rgb(255, 0, 51).floats(), [1.0, 0.0, 0.2, 1.0]);
        assert_eq!(Rgb(0, 255, 0).floats3(), [0.0, 1.0, 0.0]);
    }

    #[test]
    fn the_bar_holds_its_items() {
        // Tabs and buttons stand on the bar's bottom edge with a margin above them.
        assert!(BAR_ITEM_H < BAR_H && RADIUS * 2.0 < BAR_ITEM_H && TITLE_H < BAR_H);
    }
}
