//! FileView's settings: its rows in the settings window (FilesRows), the
//! window opened, and a key written to desktop.toml's `[files]`. The window
//! itself is in main.rs.

use vergence_widgets::Window;
use vergence_widgets::prefs::{Choice, Control, Part, Prefs, Row, Rows};

use crate::{APP_ID, SECTION, Viewer};

pub(crate) struct FilesRows;

impl Rows for FilesRows {
    fn parts(&self, text: &str) -> Vec<Part> {
        let opacity = vergence_ui::opacity_setting(text, "[appearance]");
        let locked = vergence_ui::desktop_pick(text).map(|light| {
            format!("{} for every app: Settings > Appearance > Style", if light { "Light" } else { "Dark" })
        });
        let styles = vec![Choice::text("Dark", "dark"), Choice::text("Light", "light")];
        vec![
            Part {
                title: "FileView".into(),
                rows: vec![
                    Row::new(
                        "view",
                        "New windows show",
                        "A folder as a list or as icons.",
                        Control::Choose {
                            choices: vec![Choice::text("A list", "list"), Choice::text("Icons", "icons")],
                            desktop: None,
                            app_default: 0,
                        },
                    ),
                    Row::new(
                        "preview-pane",
                        "Preview pane",
                        "The chosen file at the window's right.",
                        Control::Switch { default: false },
                    ),
                    Row::new(
                        "preview-autoplay",
                        "Play sounds in the preview",
                        "Space on a sound file plays it at once.",
                        Control::Switch { default: true },
                    ),
                    Row::new(
                        "hidden",
                        "Hidden files",
                        "Names that start with a dot.",
                        Control::Switch { default: false },
                    ),
                    Row::new(
                        "row-size",
                        "Row size",
                        "How tall the list's rows are.",
                        Control::Choose {
                            choices: vec![
                                Choice::text("Small", "small"),
                                Choice::text("Medium", "medium"),
                                Choice::text("Large", "large"),
                            ],
                            desktop: None,
                            app_default: 1,
                        },
                    ),
                    Row::new(
                        "group-by",
                        "Group by",
                        "The list under headings: by kind (Folders, Documents, Pictures…), by date (Today, Yesterday…), by size, or none.",
                        Control::Choose {
                            choices: vec![
                                Choice::text("Kind", "kind"),
                                Choice::text("Date", "date"),
                                Choice::text("Size", "size"),
                                Choice::text("None", "none"),
                            ],
                            desktop: None,
                            app_default: 0,
                        },
                    ),
                    Row::new(
                        "recent",
                        "Recent in the sidebar",
                        "The files opened most lately, under the sidebar's Locations.",
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
                        "30 to 100: how much of the window's ground shows.",
                        Control::Percent {
                            min: 30,
                            desktop: format!("Desktop default ({:.0})", opacity * 100.0),
                            now: (opacity * 100.0).round() as u32,
                        },
                    ),
                    Row::new(
                        "",
                        "Window places",
                        "Where the desktop puts FileView's windows, for each set of screens.",
                        Control::Button { label: "Forget…".into(), id: 1 },
                    ),
                ],
            },
        ]
    }

    fn button(&self, id: u32) {
        if id == 1 {
            let _ = crate::start(
                std::process::Command::new("vergence-settings").args(["--page", "windows", "--forget", APP_ID]),
            );
        }
    }
}

impl Viewer {
    pub(crate) fn write_key(&mut self, key: &str, value: Option<&str>) {
        let Some(path) = vergence_widgets::prefs::desktop_path() else { return };
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let new = vergence_widgets::prefs::set_key(&text, SECTION, key, value);
        if new != text {
            let beside = path.with_extension("toml.new");
            let _ = std::fs::write(&beside, &new).and_then(|_| std::fs::rename(&beside, &path));
        }
        // Seen at once, not at the next look at the file.
        self.desktop = new;
    }

    pub(crate) fn open_settings(&mut self, window: &mut Window) {
        if self.prefs_open.get() {
            return;
        }
        let waker = self.waker.clone();
        let prefs = Prefs::new("FileView", APP_ID, SECTION, false, Box::new(FilesRows), self.prefs_open.clone())
            .on_write(move || {
                if let Some(w) = &waker {
                    w.wake();
                }
            });
        window.open_window(vergence_widgets::window(prefs));
    }
}
