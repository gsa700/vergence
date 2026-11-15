//! The look of the desktop's own drawing, in one place (Dave, 2026-11-10:
//! "I want consistency throughout").
//!
//! The server draws title bars and borders; the toolkit draws window bars,
//! tabs, buttons, menus and scrollbars. They are to look like one thing, so
//! the numbers both draw with live here, and neither keeps a copy: a change
//! here changes both. Lengths are in reference units (points at 96 to the
//! inch, whitepaper Contract 1); colors are sRGB.
//!
//! Bars and borders come in a dark and a light style ([`BarLook`]); the
//! rest are the defaults of a dark desktop so far. What the user's settings
//! choose (the accent, light or dark, contrast) is applied by whoever
//! draws; this crate knows nothing of settings.

/// A color, sRGB, opaque.
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

/// Everything a window's bar and border are drawn with, in one style: its
/// ground, what is written and drawn on it, the tab in front when the
/// program gives no page color of its own, the border line, and the shapes
/// on it (a tab behind, a button, +), each for the window that has the
/// keyboard and for the others. Look milestone 1, step 2 (2026-11-13): the
/// server's bars follow Settings > Appearance > Style; until then they were
/// dark only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarLook {
    pub ground: Rgb,
    pub ground_off: Rgb,
    pub ink: Rgb,
    pub ink_off: Rgb,
    pub front: Rgb,
    pub front_off: Rgb,
    pub edge: Rgb,
    pub edge_off: Rgb,
    /// The color shapes are laid in, and how much of it (of 255): a
    /// window button at rest (only while its window has the keyboard: the
    /// buttons of the others are their marks alone), a tab behind or +, and
    /// anything under the pointer.
    pub shape: Rgb,
    pub shape_rest: u8,
    pub tab_rest: u8,
    pub shape_hover: u8,
    /// A tab whose program runs as the superuser (its title white on it),
    /// and the bar's ground while such a tab is in front, focused or not.
    pub alert: Rgb,
    pub alert_ground: Rgb,
    pub alert_ground_off: Rgb,
    /// A tab with edits not saved: a stripe along its top, and a tint on
    /// it when it is not the one in front (editor milestone 1).
    pub edited: Rgb,
}

/// The dark style (the desktop's first look), and the light one. Their
/// contrast is a test (below): titles 4.5:1 on everything they stand on
/// while the window has the keyboard, 3:1 when it has not (WCAG excuses
/// inactive controls; these stay readable).
pub const BAR_DARK: BarLook = BarLook {
    ground: Rgb(51, 61, 82),
    ground_off: Rgb(56, 56, 56),
    ink: Rgb(235, 235, 235),
    ink_off: Rgb(172, 172, 172),
    front: Rgb(76, 90, 120),
    front_off: Rgb(82, 82, 82),
    edge: Rgb(107, 122, 158),
    edge_off: Rgb(87, 87, 87),
    shape: Rgb(255, 255, 255),
    shape_rest: 20,
    tab_rest: 26,
    shape_hover: 41,
    alert: Rgb(128, 26, 26),
    alert_ground: Rgb(56, 12, 14),
    alert_ground_off: Rgb(60, 32, 34),
    edited: Rgb(232, 168, 56),
};

pub const BAR_LIGHT: BarLook = BarLook {
    ground: Rgb(228, 230, 235),
    ground_off: Rgb(240, 240, 241),
    ink: Rgb(29, 29, 31),
    ink_off: Rgb(112, 112, 116),
    front: Rgb(250, 250, 251),
    front_off: Rgb(232, 232, 233),
    edge: Rgb(170, 174, 184),
    edge_off: Rgb(205, 205, 208),
    shape: Rgb(0, 0, 0),
    shape_rest: 16,
    tab_rest: 22,
    shape_hover: 30,
    alert: Rgb(148, 30, 28),
    alert_ground: Rgb(244, 212, 208),
    alert_ground_off: Rgb(246, 234, 232),
    edited: Rgb(214, 140, 20),
};

/// The bar look for light or dark.
pub const fn bar_look(light: bool) -> &'static BarLook {
    if light { &BAR_LIGHT } else { &BAR_DARK }
}

/// `top` laid over `base` at `alpha` of 255, as a renderer blends it.
pub const fn over(base: Rgb, top: Rgb, alpha: u8) -> Rgb {
    const fn mix(b: u8, t: u8, a: u8) -> u8 {
        ((b as u32 * (255 - a as u32) + t as u32 * a as u32 + 127) / 255) as u8
    }
    Rgb(mix(base.0, top.0, alpha), mix(base.1, top.1, alpha), mix(base.2, top.2, alpha))
}

/// A color's relative luminance (WCAG 2), 0 for black to 1 for white.
pub fn luminance(c: Rgb) -> f32 {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c.0) + 0.7152 * lin(c.1) + 0.0722 * lin(c.2)
}

/// The contrast ratio of two colors (WCAG 2), from 1 to 21.
pub fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The ink for text on a ground the program chose (a terminal's page color
/// on the tab in front): whichever of the two styles' inks reads better.
pub fn ink_on(ground: Rgb) -> Rgb {
    if contrast(BAR_DARK.ink, ground) >= contrast(BAR_LIGHT.ink, ground) { BAR_DARK.ink } else { BAR_LIGHT.ink }
}

/// The dark style's colors under their first names (what still asks for
/// them by name draws in the dark style).
pub const BAR_FOCUSED: Rgb = BAR_DARK.ground;
pub const BAR_UNFOCUSED: Rgb = BAR_DARK.ground_off;
pub const INK_FOCUSED: Rgb = BAR_DARK.ink;
pub const INK_UNFOCUSED: Rgb = BAR_DARK.ink_off;
pub const TAB_FRONT_FOCUSED: Rgb = BAR_DARK.front;
pub const TAB_FRONT_UNFOCUSED: Rgb = BAR_DARK.front_off;
pub const EDGE_FOCUSED: Rgb = BAR_DARK.edge;
pub const EDGE_UNFOCUSED: Rgb = BAR_DARK.edge_off;
/// The close button under the pointer (its mark white on it), and the
/// server's own close button.
pub const CLOSE: Rgb = Rgb(199, 77, 71);
/// The server's other title-bar buttons.
pub const BUTTON: Rgb = Rgb(184, 184, 184);
pub const SHAPE_REST: u8 = BAR_DARK.shape_rest;
pub const SHAPE_HOVER: u8 = BAR_DARK.shape_hover;
pub const SHELF: Rgb = SHELF_DARK.dock;
pub const SHELF_EDGE: Rgb = SHELF_DARK.dock_edge;
pub const SHELF_TILE: Rgb = SHELF_DARK.tile;
pub const MENU: Rgb = SHELF_DARK.menu;
pub const MENU_INK: Rgb = SHELF_DARK.menu_ink;
pub const MENU_INK_OFF: Rgb = SHELF_DARK.menu_ink_off;

/// The frost under a bar (look milestone 1, step 3): its own strength (the
/// Frost setting's Standard, of 1).
pub const FROST_STRENGTH: f32 = 0.5;
/// The Frost setting's steps (Clear, Standard, Heavy): the frost's own
/// strength; the contrast floor raises it wherever reading needs more.
pub const fn frost_strength(step: u32) -> f32 {
    match step {
        0 => 0.3,
        2 => 0.75,
        _ => FROST_STRENGTH,
    }
}
/// The tab in front when the Tab in front setting is Frosted: a veil of
/// white laid on the bar (of 255; in dark kept to what leaves every title at
/// 4.5:1 over any frost the floor allows, checked over a grid of colors).
pub const fn front_veil(light: bool) -> u8 {
    if light { 200 } else { 40 }
}
/// The blur's reach in points at Standard. The blur's weights fall off as
/// exp(-2r²) over the reach, so its spread (a CSS blur's px) is half of it:
/// 12 is the mock-up's 6 px that Dave chose (2026-11-13: "better, I see it
/// now"); 14 was there before the steps took the blur too.
pub const FROST_RADIUS: f32 = 12.0;
/// The Frost setting's steps (Clear, Standard, Heavy): the blur's reach in
/// points (spread 4, 6 and 10 px), stepped with the strength.
pub const fn frost_radius(step: u32) -> f32 {
    match step {
        0 => 8.0,
        2 => 20.0,
        _ => FROST_RADIUS,
    }
}

/// The contrast the bar's ink must keep over the bare frost so that it keeps
/// the bar's rule (4.5:1 focused, 3:1 not) on the shapes laid on the frost
/// too (a tab behind, a button, anything under the pointer). Worked out over
/// a grid of colors; the test below checks it. (A light bar over a dark
/// window frosts what is under it inverted, so its ground is never the dark
/// that made the focused bar look dimmer than the unfocused one: Dave,
/// 2026-11-14; 14:1 for the focused bar fixed that but washed out
/// everything under the bar.)
pub const fn frost_need(light: bool, focused: bool) -> f32 {
    match (light, focused) {
        (false, true) => 7.0,
        (false, false) => 5.1,
        (true, true) => 5.85,
        (true, false) => 4.05,
    }
}

/// How far a window button's drawn shape stands in from its press area, at
/// the sides and at the top and bottom (look milestone 1: the buttons float
/// on the tabs' line, the tabs stay joined to the window below; the press
/// area stays the whole box).
pub const BUTTON_INSET_X: f32 = 2.0;
pub const BUTTON_INSET_Y: f32 = 3.0;

/// The shelf (whitepaper §6.9): the strip's width and its icons' size, its
/// ground, and the line along the side that faces the windows.
pub const SHELF_W: f32 = 48.0;
/// The room between the shelf and the screen's edge, and between it and
/// the windows' work area, unless the settings say another: the shelf
/// floats, like a dock, a hair off its edge (Dave, 2026-11-10: 1 point).
pub const SHELF_GAP: f32 = 1.0;
/// How far the apps popup and an icon's menu stand off the dock.
pub const SHELF_POPUP_GAP: f32 = 6.0;
pub const SHELF_ICON: f32 = 32.0;
/// The accent, where the user's settings name none: the cursor's blue.
pub const ACCENT: Rgb = Rgb(114, 159, 207);

/// What the dock, the apps popup, the name beside a dock icon and the
/// server's menus are drawn with, in one style (look milestone 1, step 2,
/// 2026-11-13; dark only until then): the dock's ground and its line, the
/// tile behind the letter of an application with no icon and that letter,
/// the marks (the apps button's dots, a running application's mark), a
/// menu's ground, text, text for what cannot be chosen (and hints), the line
/// around a menu or popup, the row under the pointer, a menu's separators,
/// and the popup's search field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShelfLook {
    pub dock: Rgb,
    pub dock_edge: Rgb,
    pub tile: Rgb,
    pub tile_ink: Rgb,
    pub mark: Rgb,
    pub menu: Rgb,
    pub menu_ink: Rgb,
    pub menu_ink_off: Rgb,
    pub edge: Rgb,
    pub hover: Rgb,
    pub separator: Rgb,
    pub field: Rgb,
}

pub const SHELF_DARK: ShelfLook = ShelfLook {
    dock: Rgb(32, 35, 44),
    dock_edge: Rgb(64, 68, 80),
    tile: Rgb(70, 78, 98),
    tile_ink: Rgb(235, 235, 235),
    mark: BAR_DARK.ink_off,
    menu: Rgb(43, 46, 59),
    menu_ink: Rgb(235, 235, 235),
    menu_ink_off: Rgb(128, 128, 134),
    edge: Rgb(64, 68, 80),
    hover: Rgb(51, 92, 158),
    separator: Rgb(77, 79, 87),
    field: Rgb(32, 35, 44),
};

pub const SHELF_LIGHT: ShelfLook = ShelfLook {
    dock: Rgb(238, 239, 243),
    dock_edge: Rgb(196, 199, 208),
    tile: Rgb(208, 214, 228),
    tile_ink: Rgb(29, 29, 31),
    // The light style's ink, not its mid grey: a mid grey keeps 3:1 over
    // neither a dark nor a bright backdrop well, so the frost's floor kept a
    // light dock over a red and black wallpaper about 80% tinted whatever its
    // opacity (Hubble, 2026-11-13); this ink needs 30-45% there.
    mark: BAR_LIGHT.ink,
    menu: Rgb(250, 250, 251),
    menu_ink: Rgb(29, 29, 31),
    menu_ink_off: Rgb(128, 128, 134),
    edge: Rgb(200, 202, 210),
    hover: Rgb(207, 222, 247),
    separator: Rgb(222, 223, 228),
    field: Rgb(238, 239, 243),
};

/// The dock and menu look for light or dark.
pub const fn shelf_look(light: bool) -> &'static ShelfLook {
    if light { &SHELF_LIGHT } else { &SHELF_DARK }
}

/// The grounds and inks of a window built of widgets (the settings app's,
/// first; moved here 2026-11-10 so every tool of ours shares them). `hover`
/// is laid over what the pointer is on, at `hover_alpha` (0 to 255).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// The side list of pages.
    pub sidebar: Rgb,
    /// A page's ground.
    pub page: Rgb,
    /// A group of settings on a page.
    pub card: Rgb,
    /// Edges and dividing lines.
    pub line: Rgb,
    /// Text that explains.
    pub dim: Rgb,
    pub text: Rgb,
    pub hover: Rgb,
    pub hover_alpha: u8,
}

pub const PALETTE_DARK: Palette = Palette {
    sidebar: Rgb(0x26, 0x26, 0x28),
    page: Rgb(0x1c, 0x1c, 0x1e),
    card: Rgb(0x2c, 0x2c, 0x2e),
    line: Rgb(0x3a, 0x3a, 0x3c),
    dim: Rgb(0xa1, 0xa1, 0xa6),
    text: Rgb(0xf2, 0xf2, 0xf2),
    hover: Rgb(255, 255, 255),
    hover_alpha: 20,
};

pub const PALETTE_LIGHT: Palette = Palette {
    sidebar: Rgb(0xe7, 0xe5, 0xe2),
    page: Rgb(0xf7, 0xf7, 0xf6),
    card: Rgb(0xec, 0xeb, 0xea),
    line: Rgb(0xd9, 0xd8, 0xd6),
    dim: Rgb(0x6e, 0x6e, 0x73),
    text: Rgb(0x1d, 0x1d, 0x1f),
    hover: Rgb(0, 0, 0),
    hover_alpha: 15,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The Frost steps go from clear to heavy in the blur and the strength
    /// both; Standard is the mock-up's 6 px blur that Dave chose.
    #[test]
    fn frost_steps_blur_and_tint_together() {
        assert!(frost_radius(0) < frost_radius(1) && frost_radius(1) < frost_radius(2));
        assert!(frost_strength(0) < frost_strength(1) && frost_strength(1) < frost_strength(2));
        assert_eq!(frost_radius(1) / 2.0, 6.0, "Standard spreads 6 px, as the mock-up's CSS blur(6px)");
        assert_eq!(frost_radius(9), FROST_RADIUS);
    }

    /// Look milestone 1's contrast rule for the bars, both styles: titles
    /// 4.5:1 on all they stand on while the window has the keyboard, 3:1
    /// when it has not; the buttons' marks 3:1 on their shapes; the close
    /// button's white mark on its red.
    #[test]
    fn bar_text_is_readable_in_both_styles() {
        for (name, l) in [("dark", &BAR_DARK), ("light", &BAR_LIGHT)] {
            for focused in [true, false] {
                let (ground, ink, front) = if focused { (l.ground, l.ink, l.front) } else { (l.ground_off, l.ink_off, l.front_off) };
                let need = if focused { 4.5 } else { 3.0 };
                let grounds = [("ground", ground), ("tab behind", over(ground, l.shape, l.tab_rest)), ("under the pointer", over(ground, l.shape, l.shape_hover)), ("tab in front", front)];
                for (what, g) in grounds {
                    let c = contrast(ink, g);
                    assert!(c >= need, "{name}, focused {focused}: title on {what} {c:.2} < {need}");
                }
                let button = over(ground, l.shape, l.shape_rest);
                assert!(contrast(ink, button) >= 3.0, "{name}: button mark {:.2}", contrast(ink, button));
                // While a superuser's tab is in front: the same on the alarm ground.
                let alarm = if focused { l.alert_ground } else { l.alert_ground_off };
                for (what, g) in [("alarm ground", alarm), ("tab behind on it", over(alarm, l.shape, l.tab_rest)), ("under the pointer on it", over(alarm, l.shape, l.shape_hover))] {
                    let c = contrast(ink, g);
                    assert!(c >= need, "{name}, focused {focused}: title on {what} {c:.2} < {need}");
                }
            }
            // The superuser's tab: its white title on its red, alone and under the pointer.
            for g in [l.alert, over(l.alert, l.shape, l.shape_hover)] {
                let c = contrast(Rgb(255, 255, 255), g);
                assert!(c >= 4.5, "{name}: white on the alarm tab {c:.2}");
            }
        }
        assert!(contrast(Rgb(255, 255, 255), CLOSE) >= 4.5);
    }

    /// The dock's and menus' text and marks, both styles: text 4.5:1 on a
    /// menu, the row under the pointer, the search field and a letter tile;
    /// hints and what cannot be chosen 3:1; the dock's marks 3:1.
    #[test]
    fn dock_and_menu_text_is_readable_in_both_styles() {
        for (name, l) in [("dark", &SHELF_DARK), ("light", &SHELF_LIGHT)] {
            for (what, ink, ground, need) in [
                ("menu", l.menu_ink, l.menu, 4.5),
                ("row under the pointer", l.menu_ink, l.hover, 4.5),
                ("search field", l.menu_ink, l.field, 4.5),
                ("letter tile", l.tile_ink, l.tile, 4.5),
                ("hint on the menu", l.menu_ink_off, l.menu, 3.0),
                ("hint in the field", l.menu_ink_off, l.field, 3.0),
                ("dock mark", l.mark, l.dock, 3.0),
            ] {
                let c = contrast(ink, ground);
                assert!(c >= need, "{name}: {what} {c:.2} < {need}");
            }
        }
    }

    /// A frost that keeps frost_need keeps the bar's rule with every shape
    /// laid on it, over a grid of colors.
    #[test]
    fn the_frost_floor_covers_the_shapes_on_it() {
        for light in [false, true] {
            let l = bar_look(light);
            for focused in [true, false] {
                let (ink, rule) = if focused { (l.ink, 4.5) } else { (l.ink_off, 3.0) };
                let need = frost_need(light, focused);
                let shapes = [l.shape_rest, l.tab_rest, l.shape_hover];
                for r in (0..=255).step_by(15) {
                    for g in (0..=255).step_by(15) {
                        for b in (0..=255).step_by(15) {
                            let ground = Rgb(r as u8, g as u8, b as u8);
                            if contrast(ink, ground) < need {
                                continue;
                            }
                            for a in shapes {
                                let c = contrast(ink, over(ground, l.shape, a));
                                assert!(c >= rule, "light {light} focused {focused}: {ground:?} with shape {a}: {c:.2}");
                            }
                        }
                    }
                }
            }
        }
    }

    /// The frosted tab in front keeps the bar's rule over any frost that
    /// keeps the floor (and over the solid grounds).
    #[test]
    fn the_frosted_front_tab_keeps_titles_readable() {
        for light in [false, true] {
            let l = bar_look(light);
            let veil = front_veil(light);
            let white = Rgb(255, 255, 255);
            for r in (0..=255).step_by(15) {
                for g in (0..=255).step_by(15) {
                    for b in (0..=255).step_by(15) {
                        let ground = Rgb(r as u8, g as u8, b as u8);
                        if contrast(l.ink, ground) >= frost_need(light, true) {
                            let c = contrast(l.ink, over(ground, white, veil));
                            assert!(c >= 4.5, "light {light}: {ground:?} {c:.2}");
                        }
                    }
                }
            }
            for ground in [l.ground, l.alert_ground] {
                assert!(contrast(l.ink, over(ground, white, veil)) >= 4.5, "light {light}: solid {ground:?}");
            }
        }
    }

    #[test]
    fn ink_on_a_page_color_reads() {
        for page in [Rgb(0x1e, 0x1e, 0x2e), Rgb(0xff, 0xff, 0xff), Rgb(0xef, 0xf1, 0xf5), Rgb(0x30, 0x0a, 0x24), Rgb(0x28, 0x2a, 0x36)] {
            assert!(contrast(ink_on(page), page) >= 4.5, "{page:?}");
        }
        assert_eq!(over(Rgb(0, 0, 0), Rgb(255, 255, 255), 255), Rgb(255, 255, 255));
        assert_eq!(over(Rgb(10, 20, 30), Rgb(255, 255, 255), 0), Rgb(10, 20, 30));
    }

    #[test]
    fn colors_as_fractions() {
        assert_eq!(Rgb(255, 0, 51).floats(), [1.0, 0.0, 0.2, 1.0]);
        assert_eq!(Rgb(0, 255, 0).floats3(), [0.0, 1.0, 0.0]);
    }

    #[test]
    fn the_bar_holds_its_items() {
        // Tabs and buttons stand on the bar's bottom edge with a margin above them.
        assert!(BAR_ITEM_H < BAR_H && RADIUS * 2.0 < BAR_ITEM_H && TITLE_H < BAR_H);
    }
}
