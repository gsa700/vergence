//! Reader's settings: its rows in the settings window (ReaderRows) and the
//! window opened. The window itself is in main.rs.

use vergence_widgets::Window;
use vergence_widgets::prefs::{Choice, Control, Part, Prefs, Row, Rows};

use crate::{APP_ID, SECTION, Viewer};

struct ReaderRows;

impl Rows for ReaderRows {
    fn parts(&self, text: &str) -> Vec<Part> {
        let locked = vergence_ui::desktop_pick(text).map(|light| {
            format!("{} for every app: Settings > Appearance > Style", if light { "Light" } else { "Dark" })
        });
        let styles = vec![Choice::text("Dark", "dark"), Choice::text("Light", "light")];
        vec![
            Part {
                title: "Reader".into(),
                rows: vec![Row::new(
                    "ground",
                    "Around the pages",
                    "What shows between and beside the pages.",
                    Control::Choose {
                        choices: vec![
                            Choice::text("Dark", "dark"),
                            Choice::text("Light", "light"),
                            Choice::text("The window's color", "window"),
                        ],
                        desktop: None,
                        app_default: 0,
                    },
                )],
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
                            app_default: 1,
                        },
                    )
                    .locked(locked.clone()),
                    Row::new(
                        "bar-style",
                        "Title bar",
                        "",
                        Control::Choose { choices: styles, desktop: Some("Built-in (Dark)".into()), app_default: 0 },
                    )
                    .locked(locked),
                    Row::new(
                        "",
                        "Window places",
                        "Where the desktop puts Reader's windows, for each set of screens.",
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

impl Viewer {
    pub(crate) fn open_settings(&mut self, window: &mut Window) {
        if self.prefs_open.get() {
            return;
        }
        let waker = self.waker.clone();
        let prefs = Prefs::new("Reader", APP_ID, SECTION, true, Box::new(ReaderRows), self.prefs_open.clone())
            .on_write(move || {
                if let Some(w) = &waker {
                    w.wake();
                }
            });
        window.open_window(vergence_widgets::window(prefs));
    }
}
