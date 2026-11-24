//! The terminal's tests (main.rs's `mod tests`).

use super::*;
use crate::grid::{Color as CellColor, Grid};
use crate::palette::{
    CATPPUCCIN, LATTE, LIGHT_FLOOR, TANGO, TANGO_LIGHT, palette_from, readable, readable_cached, rgb,
};
use crate::session::Screen;
use crate::settings::font_name;
use vergence_ui::bar::WinButton;
use vergence_ui::{Modifiers, sym};

const NONE: Modifiers =
    Modifiers { shift: false, ctrl: false, alt: false, logo: false, caps_lock: false, num_lock: false };

#[test]
fn colors_follow_the_desktop() {
    let same = |a: &Palette, b: &Palette| (a.bg, a.fg, a.accent, a.ansi) == (b.bg, b.fg, b.accent, b.ansi);
    assert!(same(&palette_from(""), &CATPPUCCIN));
    assert!(same(&palette_from("[terminal]\nscheme = \"Tango\"\n"), &TANGO));
    assert!(same(&palette_from("[appearance]\ncolor-scheme = \"light\"\n"), &LATTE));
    assert!(same(&palette_from("[appearance]\ncolor-scheme = \"default\"\n"), &CATPPUCCIN));
    assert!(same(
        &palette_from("[appearance]\ncolor-scheme = \"light\"\n[terminal]\nscheme = \"tango\"\n"),
        &TANGO_LIGHT
    ));
    // A key of the same name in another section is not ours.
    assert!(same(&palette_from("[wallpaper]\nscheme = \"tango\"\ncontrast = \"high\"\n"), &CATPPUCCIN));
    // High contrast: the text on black or white; the 16 colors stay.
    let p = palette_from("[appearance]\ncontrast = \"high\"\naccent = \"#ff8800\"\n");
    assert_eq!((p.bg, p.fg, p.accent), (Color::rgb(0, 0, 0), Color::rgb(255, 255, 255), Color::rgb(0xff, 0x88, 0)));
    assert_eq!(p.ansi, CATPPUCCIN.ansi);
    let p = palette_from("[appearance]\ncolor-scheme = \"light\"\ncontrast = \"high\"\n");
    assert_eq!((p.bg, p.fg), (Color::rgb(255, 255, 255), Color::rgb(0, 0, 0)));
    // A broken accent is no accent.
    for bad in ["blue", "#12345", "#+12345", "#gggggg", ""] {
        assert_eq!(palette_from(&format!("[appearance]\naccent = \"{bad}\"\n")).accent, CATPPUCCIN.accent, "{bad}");
    }
}

#[test]
fn the_scrollback_keys_reach_its_ends() {
    let mut s = Screen {
        output: 0,
        grid: Grid::new(10, 4),
        parser: vt::Parser::new(),
        scroll: 0,
        exited: false,
        copied: None,
        selection: None,
    };
    for i in 0..10 {
        let line = format!("line {i}\r\n");
        s.parser.advance(&mut s.grid, line.as_bytes());
    }
    assert!(s.grid.scrollback.len() > 3);
    s.scroll_by(2);
    s.scroll_by(isize::MAX);
    assert_eq!(s.scroll, s.grid.scrollback.len());
    s.scroll_by(isize::MIN);
    assert_eq!(s.scroll, 0);
}

#[test]
fn keys_send_what_xterm_sends() {
    let k = |s: u32, t: Option<&str>, m: Modifiers| Terminal::key_bytes(s, t, m, false);
    let ctrl = Modifiers { ctrl: true, ..NONE };
    let alt = Modifiers { alt: true, ..NONE };
    assert_eq!(k('a' as u32, Some("a"), NONE), Some(b"a".to_vec()));
    assert_eq!(k(0xe9, Some("é"), NONE), Some("é".as_bytes().to_vec()));
    assert_eq!(k(sym::RETURN, None, NONE), Some(b"\r".to_vec()));
    assert_eq!(k(sym::BACKSPACE, None, NONE), Some(vec![0x7f]));
    assert_eq!(k(sym::BACKSPACE, None, ctrl), Some(vec![0x08]));
    assert_eq!(k(sym::TAB, None, Modifiers { shift: true, ..NONE }), Some(b"\x1b[Z".to_vec()));
    assert_eq!(k(sym::UP, None, NONE), Some(b"\x1b[A".to_vec()));
    assert_eq!(Terminal::key_bytes(sym::UP, None, NONE, true), Some(b"\x1bOA".to_vec()), "application cursor keys");
    assert_eq!(k(sym::PAGE_DOWN, None, NONE), Some(b"\x1b[6~".to_vec()));
    assert_eq!(k(sym::DELETE, None, NONE), Some(b"\x1b[3~".to_vec()));
    assert_eq!(k(0xffbe, None, NONE), Some(b"\x1bOP".to_vec()), "F1");
    assert_eq!(k(0xffc1, None, NONE), Some(b"\x1bOS".to_vec()), "F4");
    assert_eq!(k(0xffc2, None, NONE), Some(b"\x1b[15~".to_vec()), "F5");
    assert_eq!(k(0xffc9, None, NONE), Some(b"\x1b[24~".to_vec()), "F12");
    assert_eq!(k('c' as u32, None, ctrl), Some(vec![3]));
    assert_eq!(k('C' as u32, None, Modifiers { shift: true, ..ctrl }), Some(vec![3]));
    assert_eq!(k('[' as u32, None, ctrl), Some(vec![0x1b]));
    assert_eq!(k(0x20, Some(" "), ctrl), Some(vec![0]));
    assert_eq!(k('x' as u32, Some("x"), alt), Some(b"\x1bx".to_vec()));
    assert_eq!(k('d' as u32, None, Modifiers { ctrl: true, alt: true, ..NONE }), Some(vec![0x1b, 4]));
    // A modifier by itself, or a key that types nothing, sends nothing.
    assert_eq!(k(0xffe1, None, NONE), None);
    assert_eq!(k(0xffeb, None, NONE), None);
}

#[test]
fn colors_come_from_the_palette_or_the_cube() {
    let p = TANGO;
    assert_eq!(p.resolve(CellColor::Default, p.fg, false), p.fg);
    assert_eq!(p.resolve(CellColor::Indexed(1), p.fg, false), rgb(0xcc0000));
    assert_eq!(p.resolve(CellColor::Indexed(1), p.fg, true), rgb(0xef2929), "bold is the bright one");
    assert_eq!(p.resolve(CellColor::Indexed(9), p.fg, true), rgb(0xef2929));
    assert_eq!(p.indexed(16), Color::rgb(0, 0, 0));
    assert_eq!(p.indexed(231), Color::rgb(255, 255, 255));
    assert_eq!(p.indexed(196), Color::rgb(255, 0, 0));
    assert_eq!(p.indexed(232), Color::rgb(8, 8, 8));
    assert_eq!(p.indexed(255), Color::rgb(238, 238, 238));
    assert_eq!(p.resolve(CellColor::Rgb(1, 2, 3), p.fg, false), Color::rgb(1, 2, 3));
    // The dark palettes keep their colors as they are.
    assert_eq!(TANGO.floor, 0.0);
    assert_eq!(readable(rgb(0x3465a4), TANGO.bg, TANGO.floor), rgb(0x3465a4));
}

#[test]
fn the_settings_file_is_followed() {
    let s = Settings::from_text(
        "[settings]\n\"Gtk/DecorationLayout\" = \"close,minimize:\"\n\"Gtk/FontName\" = \"Adwaita Sans 10\"\n\"Gtk/MonospaceFontName\" = \"Adwaita Mono 11\"\n",
    );
    assert_eq!((s.left.as_slice(), s.right.len()), (&[WinButton::Close, WinButton::Minimize][..], 0));
    assert_eq!((s.ui_family.as_str(), s.mono_family.as_str()), ("Adwaita Sans", "Adwaita Mono"));
    assert!((s.ui_size - 13.333).abs() < 0.01 && (s.mono_size - 14.667).abs() < 0.01);
    // Nothing set: buttons on the right, the machine's own fonts at 10 points.
    let s = Settings::from_text("");
    // No layout chosen: vergence-bar's default, on the left.
    assert_eq!(
        (s.left.as_slice(), s.right.len()),
        (&[WinButton::Close, WinButton::Minimize, WinButton::Maximize][..], 0)
    );
    assert_eq!((s.ui_family.as_str(), s.mono_family.as_str()), ("sans-serif", "monospace"));
    assert_eq!(font_name("Cantarell", "sans-serif", 10.0).0, "Cantarell");
    assert_eq!(
        font_name("Noto Sans 400", "sans-serif", 10.0),
        ("Noto Sans 400".to_string(), 10.0 * 96.0 / 72.0),
        "a size that is not one is part of the name"
    );
}

#[test]
fn the_view_scrolls_within_the_scrollback() {
    let mut s = Screen {
        output: 0,
        grid: Grid::new(20, 4),
        parser: vt::Parser::new(),
        scroll: 0,
        exited: false,
        copied: None,
        selection: None,
    };
    for i in 0..10 {
        let line = format!("line {i}\r\n");
        s.parser.advance(&mut s.grid, line.as_bytes());
    }
    let text = |s: &Screen, r: usize| s.shown(r).iter().map(|c| c.c).collect::<String>().trim_end().to_string();
    assert_eq!(text(&s, 0), "line 7");
    s.scroll_by(3);
    assert_eq!((s.scroll, text(&s, 0)), (3, "line 4".to_string()));
    s.scroll_by(100);
    assert_eq!(text(&s, 0), "line 0", "no further than the first line kept");
    s.scroll_by(-1000);
    assert_eq!((s.scroll, text(&s, 0)), (0, "line 7".to_string()));
}

/// On the light palettes every one of the 16 colors, plain or bold, reads
/// at the floor on the page (Tango's whites and brights vanish on white);
/// a color that already reads is left alone; the hue
/// stays (red stays reddest).
#[test]
fn the_remembered_answer_is_the_worked_out_one() {
    for (ink, ground) in
        [(rgb(0xef2929), rgb(0xffffff)), (rgb(0x2e3436), rgb(0xffffff)), (rgb(0xeeeeec), rgb(0x000000))]
    {
        let worked = readable(ink, ground, LIGHT_FLOOR);
        assert_eq!(readable_cached(ink, ground, LIGHT_FLOOR, LIGHT_FLOOR.to_bits()), worked);
        assert_eq!(readable_cached(ink, ground, LIGHT_FLOOR, LIGHT_FLOOR.to_bits()), worked, "and again from memory");
    }
}

#[test]
fn light_palettes_keep_text_readable() {
    let rgb3 = |c: Color| vergence_look::Rgb(c.r, c.g, c.b);
    for p in [&TANGO_LIGHT, &LATTE] {
        for i in 0..16u8 {
            for bold in [false, true] {
                let ink = readable(p.resolve(CellColor::Indexed(i), p.fg, bold), p.bg, p.floor);
                let c = vergence_look::contrast(rgb3(ink), rgb3(p.bg));
                assert!(c >= LIGHT_FLOOR - 0.01, "color {i} bold {bold}: {c:.2}");
            }
        }
    }
    assert_eq!(readable(rgb(0x2e3436), rgb(0xffffff), LIGHT_FLOOR), rgb(0x2e3436), "the text color already reads");
    let red = readable(rgb(0xef2929), rgb(0xffffff), LIGHT_FLOOR);
    assert!(red.r > red.g && red.r > red.b, "still red: {red:?}");
    // A cell with a ground of its own: kept against that ground, not the page.
    let on_black = readable(rgb(0xeeeeec), rgb(0x000000), LIGHT_FLOOR);
    assert_eq!(on_black, rgb(0xeeeeec));
}

#[test]
fn for_agent_is_one_argument_and_a_name() {
    let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(crate::for_agent_name(&a(&["vergence-term", "--for-agent=Claude"])), Some("Claude".into()));
    // The old two-argument form is not read: a name is never the next option.
    assert_eq!(crate::for_agent_name(&a(&["vergence-term", "--for-agent", "Claude"])), None);
    assert_eq!(crate::for_agent_name(&a(&["vergence-term", "--for-agent="])), None, "an empty name");
    assert_eq!(crate::for_agent_name(&a(&["vergence-term", "--for-agent=  \u{7}"])), None, "nothing but controls");
    assert_eq!(crate::for_agent_name(&a(&["vergence-term", "--for-agent=-e"])), Some("-e".into()), "just a name");
}
