// The terminal's settings window (app settings milestone 1): the toolkit's
// (vergence-widgets prefs), with the terminal's rows. Its own part: text
// size and font (the desktop's Monospace by default), colors (the desktop's
// terminal colors by default), lines kept, the cursor, the question before
// closing. On this desktop: its content's and its bar's style, opacity, and
// forgetting its window places. Everything is the [terminal] table of
// desktop.toml; the terminal re-reads it within a second.

use vergence_widgets::prefs::{Choice, Control, Part, Row, Rows};

pub struct TermRows {
    /// The monospace families installed, found once (fc-list).
    monos: Vec<String>,
}

impl TermRows {
    pub fn new() -> TermRows {
        let out = std::process::Command::new("fc-list").args([":spacing=mono", "family"]).output();
        let mut monos: Vec<String> = out
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split(',').next().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect();
        monos.sort_by_key(|n| n.to_lowercase());
        monos.dedup();
        TermRows { monos }
    }
}

/// What the desktop's Style makes light or dark now, as a row names it.
fn desktop_light_word(text: &str, builtin_light: bool) -> &'static str {
    if vergence_ui::desktop_pick(text).unwrap_or(builtin_light) { "Light" } else { "Dark" }
}

pub const FORGET: u32 = 1;

impl Rows for TermRows {
    fn parts(&self, text: &str) -> Vec<Part> {
        let settings = crate::Settings::from_text(&crate::config("settings.toml"));
        let (family, size) = (settings.mono_family.clone(), settings.mono_size);
        let sizes: Vec<Choice> = [8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24].iter().map(|s| Choice::number(&s.to_string(), *s)).collect();
        let fonts: Vec<Choice> = self.monos.iter().map(|f| Choice::text(f, f)).collect();
        let desktop_colors = match vergence_ui::table_value(text, "[appearance]", "terminal-colors").as_deref() {
            Some(c) if c.eq_ignore_ascii_case("tango") => "Tango",
            _ => "Catppuccin",
        };
        let opacity = vergence_ui::opacity_setting(text, "[appearance]");
        let opacities: Vec<Choice> = [1.0, 0.95, 0.9, 0.85, 0.8, 0.75, 0.7].iter().map(|o: &f32| Choice::decimal(&format!("{:.0}%", o * 100.0), *o)).collect();
        let styles = vec![Choice::text("Dark", "dark"), Choice::text("Light", "light")];
        vec![
            Part {
                title: "Terminal".into(),
                rows: vec![
                    Row { key: "font".into(), name: "Font".into(), help: String::new(), control: Control::Choose { choices: fonts, desktop: Some(family), app_default: 0 } },
                    Row { key: "font-size".into(), name: "Text size".into(), help: "The text size menu's − and + change it for a while.".into(), control: Control::Choose { choices: sizes, desktop: Some(format!("{}", (size * 10.0).round() / 10.0)), app_default: 0 } },
                    Row {
                        key: "scheme".into(),
                        name: "Colors".into(),
                        help: "Tango is the classic set mc, htop and vim were tuned for; Catppuccin is softer.".into(),
                        control: Control::Choose { choices: vec![Choice::text("Catppuccin", "catppuccin"), Choice::text("Tango", "tango")], desktop: Some(desktop_colors.into()), app_default: 0 },
                    },
                    Row {
                        key: "scrollback".into(),
                        name: "Lines kept".into(),
                        help: "How far back each tab can scroll.".into(),
                        control: Control::Choose { choices: vec![Choice::number("1,000", 1000), Choice::number("5,000", 5000), Choice::number("10,000", 10000), Choice::number("50,000", 50000), Choice::number("100,000", 100000)], desktop: None, app_default: 2 },
                    },
                    Row { key: "cursor".into(), name: "Cursor".into(), help: String::new(), control: Control::Choose { choices: vec![Choice::text("Block", "block"), Choice::text("Bar", "bar"), Choice::text("Underline", "underline")], desktop: None, app_default: 0 } },
                    Row { key: "blink".into(), name: "Cursor blinks".into(), help: String::new(), control: Control::Switch { default: true } },
                    Row { key: "ask-close".into(), name: "Ask before closing".into(), help: "When a window has several tabs, or a program running in one.".into(), control: Control::Switch { default: true } },
                ],
            },
            Part {
                title: "On this desktop".into(),
                rows: vec![
                    Row { key: "style".into(), name: "Style".into(), help: "The terminal's own light or dark wins over the desktop's.".into(), control: Control::Choose { choices: styles.clone(), desktop: Some(desktop_light_word(text, false).into()), app_default: 0 } },
                    Row { key: "bar-style".into(), name: "Title bar".into(), help: String::new(), control: Control::Choose { choices: styles, desktop: Some(desktop_light_word(text, false).into()), app_default: 0 } },
                    Row { key: "opacity".into(), name: "Opacity".into(), help: String::new(), control: Control::Choose { choices: opacities, desktop: Some(format!("{:.0}%", opacity * 100.0)), app_default: 0 } },
                    Row { key: String::new(), name: "Window places".into(), help: "Where the desktop puts the terminal's windows, for each set of screens.".into(), control: Control::Button { label: "Forget…".into(), id: FORGET } },
                ],
            },
        ]
    }

    fn button(&self, id: u32) {
        if id == FORGET
            && let Err(e) = std::process::Command::new("vergence-settings").args(["--page", "windows", "--forget", "vergence-term"]).spawn()
        {
            eprintln!("vergence-term: vergence-settings: {e}");
        }
    }
}
