//! Edit's settings: desktop.toml's `[editor]` read and written, how a
//! document's area shows and edits (with completion's words from the
//! other documents), the text size for a while, the settings window and
//! its rows. The window itself is in main.rs.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::SystemTime;

use vergence_widgets::prefs::{Choice, Control, Part, Prefs, Row, Rows};
use vergence_widgets::{AreaOptions, Program, Window};

use crate::{APP_ID, DocTab, EditWindow, SECTION, ZOOM_MAX, ZOOM_MIN, cmd, is_prose};

/// ~/.config/vergence/desktop.toml and when it last changed.
pub(crate) fn desktop_text() -> (String, Option<SystemTime>) {
    match vergence_widgets::prefs::desktop_path() {
        Some(p) => {
            (std::fs::read_to_string(&p).unwrap_or_default(), std::fs::metadata(&p).and_then(|m| m.modified()).ok())
        }
        None => (String::new(), None),
    }
}

impl EditWindow {
    fn setting(&self, key: &str) -> Option<String> {
        vergence_ui::table_value(&self.desktop, SECTION, key)
    }

    pub(crate) fn flag(&self, key: &str, default: bool) -> bool {
        match self.setting(key).as_deref() {
            Some("true") => true,
            Some("false") => false,
            _ => default,
        }
    }

    /// How a document's area shows and edits, from the settings.
    fn options_for(&self, t: &DocTab) -> AreaOptions {
        let size = self.setting("text-size").and_then(|s| s.parse::<f32>().ok()).filter(|s| (6.0..=72.0).contains(s));
        // Its own size, else the desktop's monospace font's (Gtk/MonospaceFontName,
        // as Terminal; text milestone 1, step 5).
        let base = size.unwrap_or_else(|| self.theme().mono_size);
        AreaOptions {
            font: self.setting("font").filter(|f| !f.is_empty()).unwrap_or_else(|| "monospace".into()),
            size: Some((base * self.zoom).clamp(6.0, 96.0)),
            line_numbers: self.flag("line-numbers", true),
            wrap: self.flag("wrap", true),
            tab_width: self.setting("tab-width").and_then(|s| s.parse().ok()).unwrap_or(4),
            spaces: t.spaces.unwrap_or_else(|| self.flag("spaces", true)),
            current_line: self.flag("current-line", true),
            editable: t.read_only.is_none(),
            // The right-click menu as full as the terminal's (Dave's list):
            // the editor's own lines after the text's.
            commands: vec![
                ("New Tab".into(), cmd::NEW_TAB, "Ctrl+T"),
                ("New Window".into(), cmd::NEW_WINDOW, "Ctrl+N"),
                ("Find…".into(), cmd::FIND, "Ctrl+F"),
            ],
            complete: self.flag("complete", true),
            // Its own choice, else the desktop's ([text] spellcheck), on by default.
            spell: match self.setting("spellcheck").as_deref() {
                Some("true") => true,
                Some("false") => false,
                _ => vergence_ui::table_value(&self.desktop, "[text]", "spellcheck").as_deref() != Some("false"),
            },
            line_spacing: Some(self.spacing_for(t)),
        }
    }

    /// The document's line spacing (text milestone 1, step 6): the chosen
    /// one, else by the kind of file (Automatic): prose roomier, code closer.
    fn spacing_for(&self, t: &DocTab) -> f32 {
        match self.setting("line-spacing").as_deref() {
            Some("compact") => vergence_widgets::CODE_SPACING,
            Some("comfortable") => 1.45,
            Some("loose") => 1.6,
            _ if is_prose(t.path.as_deref()) => vergence_widgets::PROSE_SPACING,
            _ => vergence_widgets::CODE_SPACING,
        }
    }

    pub(crate) fn apply_options(&mut self) {
        for i in 0..self.tabs.len() {
            let o = self.options_for(&self.tabs[i]);
            self.tabs[i].doc.borrow_mut().set_options(o);
            // Completion also offers the words of the window's other documents.
            let others: Vec<vergence_widgets::WeakDoc> =
                self.tabs.iter().enumerate().filter(|(k, _)| *k != i).map(|(_, t)| t.doc.downgrade()).collect();
            self.tabs[i].doc.borrow_mut().set_words(Some(other_words(others)));
        }
    }

    pub(crate) fn open_settings(&mut self, window: &mut Window) {
        if self.prefs_open.get() {
            return;
        }
        let waker = self.waker.clone();
        let prefs = Prefs::new("Edit", APP_ID, SECTION, false, Box::new(EditRows::new()), self.prefs_open.clone())
            .on_write(move || {
                if let Some(w) = &waker {
                    w.wake();
                }
            });
        window.open_window(vergence_widgets::window(prefs));
    }

    /// desktop.toml again, if it changed.
    pub(crate) fn reread(&mut self) -> bool {
        let (text, stamp) = desktop_text();
        if stamp == self.stamp {
            return false;
        }
        (self.desktop, self.stamp) = (text, stamp);
        self.apply_options();
        true
    }

    fn write_key(&mut self, key: &str, value: Option<&str>) {
        let Some(path) = vergence_widgets::prefs::desktop_path() else { return };
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let new = vergence_widgets::prefs::set_key(&text, SECTION, key, value);
        if new != text {
            let beside = path.with_extension("toml.new");
            if std::fs::write(&beside, &new).and_then(|_| std::fs::rename(&beside, &path)).is_ok() {
                self.reread();
            }
        }
    }

    pub(crate) fn toggle_setting(&mut self, key: &str, default: bool) {
        let now = self.flag(key, default);
        // Back to the default is the key taken away.
        let value = if !now == default { None } else { Some(if now { "false" } else { "true" }) };
        self.write_key(key, value);
    }

    pub(crate) fn set_zoom(&mut self, z: f32) {
        self.zoom = z.clamp(ZOOM_MIN, ZOOM_MAX);
        self.apply_options();
    }
}

/// Words from other documents for completion: each document's words with
/// how often each comes, found again only when it changed.
fn other_words(docs: Vec<vergence_widgets::WeakDoc>) -> vergence_widgets::Words {
    type Counted = Vec<(String, u32)>;
    let cache: RefCell<Vec<(u64, Counted)>> = RefCell::new(vec![(u64::MAX, Vec::new()); docs.len()]);
    Rc::new(move |prefix: &str, limit: usize| {
        let want = prefix.to_lowercase();
        let mut cache = cache.borrow_mut();
        let mut found: Vec<(String, u32)> = Vec::new();
        for (k, weak) in docs.iter().enumerate() {
            let Some(doc) = weak.upgrade() else { continue };
            let d = doc.borrow();
            if cache[k].0 != d.revision() {
                let mut counts: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
                for line in d.buffer().lines() {
                    for w in line.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '\'')) {
                        let w = w.trim_matches('\'');
                        if w.chars().count() >= 3 {
                            *counts.entry(w).or_insert(0) += 1;
                        }
                    }
                }
                cache[k] = (d.revision(), counts.into_iter().map(|(w, n)| (w.to_string(), n)).collect());
            }
            for (w, n) in &cache[k].1 {
                if w.len() > prefix.len() && w.to_lowercase().starts_with(&want) {
                    match found.iter_mut().find(|(f, _)| f == w) {
                        Some(f) => f.1 += n,
                        None => found.push((w.clone(), *n)),
                    }
                }
            }
        }
        found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        found.into_iter().take(limit).map(|(w, _)| w).collect()
    })
}

/// Its settings window's rows.
struct EditRows {
    /// Monospaced families first, then the rest (fc-list, once).
    fonts: Vec<String>,
}

impl EditRows {
    fn new() -> EditRows {
        let list = |args: &[&str]| -> Vec<String> {
            let out = std::process::Command::new("fc-list").args(args).output();
            let mut v: Vec<String> = out
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default()
                .lines()
                .filter_map(|l| l.split(',').next().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect();
            v.sort_by_key(|n| n.to_lowercase());
            v.dedup();
            v
        };
        let mut fonts = list(&[":spacing=mono", "family"]);
        let rest: Vec<String> = list(&[":", "family"]).into_iter().filter(|f| !fonts.contains(f)).collect();
        fonts.extend(rest);
        EditRows { fonts }
    }
}

impl Rows for EditRows {
    fn parts(&self, text: &str) -> Vec<Part> {
        let fonts: Vec<Choice> = self.fonts.iter().map(|f| Choice::text(f, f)).collect();
        let sizes: Vec<Choice> = [8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 28]
            .iter()
            .map(|s| Choice::number(&s.to_string(), *s))
            .collect();
        let tabs: Vec<Choice> = [2, 3, 4, 8].iter().map(|s| Choice::number(&s.to_string(), *s)).collect();
        let opacity = vergence_ui::opacity_setting(text, "[appearance]");
        let locked = vergence_ui::desktop_pick(text).map(|light| {
            format!("{} for every app: Settings > Appearance > Style", if light { "Light" } else { "Dark" })
        });
        let styles = vec![Choice::text("Dark", "dark"), Choice::text("Light", "light")];
        vec![
            Part {
                title: "Text".into(),
                rows: vec![
                    Row::new(
                        "font",
                        "Font",
                        "",
                        Control::Choose { choices: fonts, desktop: Some("Monospace".into()), app_default: 0 },
                    ),
                    Row::new(
                        "text-size",
                        "Text size",
                        "Ctrl+= and Ctrl+− change it for a while.",
                        Control::Choose { choices: sizes, desktop: Some("Desktop default".into()), app_default: 0 },
                    ),
                    Row::new(
                        "line-spacing",
                        "Line spacing",
                        "Automatic: roomier for prose (.md, .txt, untitled), closer for code.",
                        Control::Choose {
                            choices: vec![
                                Choice::text("Automatic", "auto"),
                                Choice::text("Compact", "compact"),
                                Choice::text("Comfortable", "comfortable"),
                                Choice::text("Loose", "loose"),
                            ],
                            desktop: None,
                            app_default: 0,
                        },
                    ),
                    Row::new("line-numbers", "Line numbers", "", Control::Switch { default: true }),
                    Row::new(
                        "wrap",
                        "Wrap lines",
                        "Long lines go on below at the window's edge; off, the text scrolls sideways.",
                        Control::Switch { default: true },
                    ),
                    Row::new("current-line", "Shade the current line", "", Control::Switch { default: true }),
                    Row::new(
                        "spellcheck",
                        "Check spelling",
                        "Words spelled wrong are underlined in red; a right click on one gives suggestions.",
                        Control::Choose {
                            choices: vec![Choice::text("On", "true"), Choice::text("Off", "false")],
                            desktop: Some(format!(
                                "Desktop default ({})",
                                if vergence_ui::table_value(text, "[text]", "spellcheck").as_deref() == Some("false") {
                                    "Off"
                                } else {
                                    "On"
                                }
                            )),
                            app_default: 0,
                        },
                    ),
                    Row::new(
                        "complete",
                        "Suggest words as you type",
                        "The likely rest of a word shows in gray; Tab takes it. Alt+Escape, F5 or Ctrl+Space lists them all.",
                        Control::Switch { default: true },
                    ),
                ],
            },
            Part {
                title: "Indenting".into(),
                rows: vec![
                    Row::new(
                        "tab-width",
                        "Tab width",
                        "How many spaces wide a tab is.",
                        Control::Choose { choices: tabs, desktop: Some("4".into()), app_default: 0 },
                    ),
                    Row::new(
                        "spaces",
                        "Indent with spaces",
                        "Tab inserts spaces; a file indented with tabs keeps its tabs.",
                        Control::Switch { default: true },
                    ),
                ],
            },
            Part {
                title: "On this desktop".into(),
                rows: vec![
                    Row::new(
                        "style",
                        "Style",
                        "While the desktop's Style is App default.",
                        Control::Choose {
                            choices: styles.clone(),
                            desktop: Some("Built-in (Light)".into()),
                            app_default: 0,
                        },
                    )
                    .locked(locked.clone()),
                    Row::new(
                        "bar-style",
                        "Title bar",
                        "",
                        Control::Choose { choices: styles, desktop: Some("Built-in (Light)".into()), app_default: 0 },
                    )
                    .locked(locked),
                    Row::new(
                        "opacity",
                        "Opacity",
                        "30 to 100: how much of the window's ground shows; the text stays as it is.",
                        Control::Percent {
                            min: 30,
                            desktop: format!("Desktop default ({:.0})", opacity * 100.0),
                            now: (opacity * 100.0).round() as u32,
                        },
                    ),
                    Row::new(
                        "",
                        "Window places",
                        "Where the desktop puts the editor's windows, for each set of screens.",
                        Control::Button { label: "Forget…".into(), id: 1 },
                    ),
                ],
            },
        ]
    }

    fn button(&self, id: u32) {
        if id == 1 {
            let _ =
                std::process::Command::new("vergence-settings").args(["--page", "windows", "--forget", APP_ID]).spawn();
        }
    }
}
