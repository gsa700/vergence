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

/// Why the Style and Title bar rows are locked: the desktop's Style forces
/// every app (App default lets each app choose; Dark or Light is for all;
/// see docs/app-settings-milestone-1.md).
pub fn forced_by_desktop(text: &str) -> Option<String> {
    vergence_ui::desktop_pick(text)
        .map(|light| format!("{} for every app: Settings > Appearance > Style", if light { "Light" } else { "Dark" }))
}

pub const FORGET: u32 = 1;

impl Rows for TermRows {
    fn parts(&self, text: &str) -> Vec<Part> {
        let settings = crate::Settings::from_text(&crate::config("settings.toml"));
        let (family, size) = (settings.mono_family.clone(), settings.mono_size);
        let sizes: Vec<Choice> = [8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24]
            .iter()
            .map(|s| Choice::number(&s.to_string(), *s))
            .collect();
        let fonts: Vec<Choice> = self.monos.iter().map(|f| Choice::text(f, f)).collect();
        let desktop_colors = match vergence_ui::table_value(text, "[appearance]", "terminal-colors").as_deref() {
            Some(c) if c.eq_ignore_ascii_case("tango") => "Tango",
            _ => "Catppuccin",
        };
        let opacity = vergence_ui::opacity_setting(text, "[appearance]");
        let styles = vec![Choice::text("Dark", "dark"), Choice::text("Light", "light")];
        vec![
            Part {
                title: "Terminal".into(),
                rows: vec![
                    Row { key: "font".into(), name: "Font".into(), help: String::new(), control: Control::Choose { choices: fonts, desktop: Some(format!("Desktop default ({family})")), app_default: 0 } , locked: None },
                    Row { key: "font-size".into(), name: "Text size".into(), help: "The text size menu's − and + change it for a while.".into(), control: Control::Choose { choices: sizes, desktop: Some(format!("Desktop default ({})", (size * 10.0).round() / 10.0)), app_default: 0 } , locked: None },
                    Row {
                        key: "scheme".into(),
                        name: "Colors".into(),
                        help: "Tango is the classic set mc, htop and vim were tuned for; Catppuccin is softer.".into(),
                        control: Control::Choose { choices: vec![Choice::text("Catppuccin", "catppuccin"), Choice::text("Tango", "tango")], desktop: Some(format!("Desktop default ({desktop_colors})")), app_default: 0 },
                        locked: None,
                    },
                    Row {
                        key: "scrollback".into(),
                        name: "Lines kept".into(),
                        help: "How far back each tab can scroll.".into(),
                        control: Control::Choose { choices: vec![Choice::number("1,000", 1000), Choice::number("5,000", 5000), Choice::number("10,000", 10000), Choice::number("50,000", 50000), Choice::number("100,000", 100000)], desktop: None, app_default: 2 },
                        locked: None,
                    },
                    Row { key: "scroll-on-output".into(), name: "Scroll on output".into(), help: "While scrolled back, new output brings the view down to it. Off: the view stays on what you are reading.".into(), control: Control::Switch { default: false } , locked: None },
                    Row { key: "cursor".into(), name: "Cursor".into(), help: String::new(), control: Control::Choose { choices: vec![Choice::text("Block", "block"), Choice::text("Bar", "bar"), Choice::text("Underline", "underline")], desktop: None, app_default: 0 } , locked: None },
                    Row { key: "blink".into(), name: "Cursor blinks".into(), help: String::new(), control: Control::Switch { default: true } , locked: None },
                    Row { key: "ask-close".into(), name: "Ask before closing".into(), help: "When a window has several tabs, or a program running in one.".into(), control: Control::Switch { default: true } , locked: None },
                ],
            },
            Part {
                title: "On this desktop".into(),
                rows: vec![
                    Row::new("style", "Style", "While the desktop's Style is App default.", Control::Choose { choices: styles.clone(), desktop: Some("Built-in (Dark)".into()), app_default: 0 }).locked(forced_by_desktop(text)),
                    Row::new("bar-style", "Title bar", "", Control::Choose { choices: styles, desktop: Some("Built-in (Dark)".into()), app_default: 0 }).locked(forced_by_desktop(text)),
                    Row::new("opacity", "Opacity", "30 to 100: how much of the window's ground shows; the text stays as it is.", Control::Percent { min: 30, desktop: format!("Desktop default ({:.0})", opacity * 100.0), now: (opacity * 100.0).round() as u32 }),
                    Row::new("frosted", "Frosted glass", "Below 100% opacity, what is behind the window is blurred, so its text is not read through yours; a little clearer while the terminal is in front.", Control::Switch { default: true }),
                    Row { key: String::new(), name: "Window places".into(), help: "Where the desktop puts the terminal's windows, for each set of screens.".into(), control: Control::Button { label: "Forget…".into(), id: FORGET } , locked: None },
                ],
            },
        ]
    }

    fn button(&self, id: u32) {
        if id == FORGET
            && let Err(e) = std::process::Command::new("vergence-settings")
                .args(["--page", "windows", "--forget", "vergence-term"])
                .spawn()
        {
            eprintln!("vergence-term: vergence-settings: {e}");
        }
    }
}
