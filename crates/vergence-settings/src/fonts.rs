//! The Theme and fonts page (Page::Theme): themes, fonts and the terminal
//! colors; theme.rs reads and writes the settings file behind it.
//! The app's skeleton (the Program impl, the page list and routing, shared
//! state and helpers) is in main.rs.

use super::*;

// ---- Terminal: desktop.toml [appearance] terminal-colors, the default each
// terminal's own settings start from (vergence-term reads it) ----
const TERM_SCHEMES: [&str; 2] = ["Catppuccin", "Tango"];

fn term_scheme(d: &toml::Table) -> &'static str {
    match d
        .get("appearance")
        .and_then(|a| a.get("terminal-colors"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("tango") => "Tango",
        _ => "Catppuccin",
    }
}

impl App {
    pub(crate) fn set_x(&mut self, key: &str, value: &str) {
        match theme::set(key, value) {
            Ok(()) => {
                self.xsettings = theme::read();
                self.status.clear();
            }
            Err(e) => self.status = format!("Could not save: {e}"),
        }
    }

    pub(crate) fn theme_page(&self) -> Element<Message> {
        let with = |mut list: Vec<String>, cur: &str| {
            if !cur.is_empty() && !list.iter().any(|x| x == cur) {
                list.insert(0, cur.to_string());
            }
            list
        };
        let pick = |key: &'static str, list: &[String], name: &str| -> Element<Message> {
            let cur = theme::get(&self.xsettings, key);
            let list = with(list.to_vec(), &cur);
            let at = list.iter().position(|x| *x == cur);
            let options = list.clone();
            choice(options, at, move |i| Message::Pick(key, list[i].clone())).width(Length::Fixed(220.0)).name(name)
        };
        let num = |s: f32| if s.fract() == 0.0 { format!("{}", s as i32) } else { format!("{s}") };
        let font_row = |mono: bool| -> Element<Message> {
            let key = if mono { theme::MONO } else { theme::FONT };
            let (family, size) = theme::split_font(&theme::get(&self.xsettings, key));
            let families = with(if mono { self.monos.clone() } else { self.fonts.clone() }, &family);
            let at = families.iter().position(|f| *f == family);
            let sizes: Vec<String> = theme::sizes(size).into_iter().map(num).collect();
            let size_at = sizes.iter().position(|s| *s == num(size));
            let (fam_list, size_list) = (families.clone(), sizes.clone());
            row(vec![
                choice(families, at, move |i| Message::Pick(key, theme::join_font(&fam_list[i], size)))
                    .width(Length::Fixed(220.0))
                    .name(if mono { "Monospace font" } else { "Interface font" }),
                choice(sizes, size_at, move |i| Message::FontSize(mono, size_list[i].clone()))
                    .width(Length::Fixed(76.0))
                    .name("Size"),
            ])
            .spacing(6.0)
        };
        let schemes: Vec<String> = TERM_SCHEMES.iter().map(|s| s.to_string()).collect();
        let term_at = TERM_SCHEMES.iter().position(|s| *s == term_scheme(&self.desktop));
        self.page_layout(
            "Theme and fonts",
            "How apps look: their theme, icons, pointer and fonts. GTK and X11 apps follow at once; the newest GNOME apps keep their own look.",
            vec![
                self.group(Some("Theme"), vec![
                    setting("Applications", "Buttons, menus and windows in GTK apps.", pick(theme::THEME, &self.gtk_themes, "Applications theme")),
                    setting("Icons", "", pick(theme::ICONS, &self.icon_themes, "Icons")),
                    setting("Pointer", "The same size on every screen.", pick(theme::CURSOR, &self.cursor_themes, "Pointer")),
                ]),
                self.group(Some("Fonts"), vec![
                    setting("Interface", "Menus, buttons, labels, and the title bars.", font_row(false)),
                    setting("Monospace", "Terminals and code.", font_row(true)),
                ]),
                self.group(Some("Terminal"), vec![setting(
                    "Colors",
                    "Terminals' 16 colors, unless a terminal's own settings choose (Ctrl+, in it). Tango is the classic set that mc, htop and vim were tuned for; Catppuccin is softer.",
                    choice(schemes.clone(), term_at, move |i| Message::TermScheme(schemes[i].clone())).width(Length::Fixed(160.0)).name("Terminal colors"),
                )]),
                self.status_line(),
            ],
        )
    }

    /// This page's messages, as main.rs's handle routes them here.
    pub(crate) fn fonts_message(&mut self, m: Message) {
        match m {
            Message::Pick(key, value) => self.set_x(key, &value),
            Message::FontSize(mono, size) => {
                let key = if mono { theme::MONO } else { theme::FONT };
                let (family, _) = theme::split_font(&theme::get(&self.xsettings, key));
                if let Ok(sz) = size.parse::<f32>() {
                    self.set_x(key, &theme::join_font(&family, sz));
                }
            }
            // The desktop's terminal colors (app settings milestone 1): the default
            // a terminal's own settings start from (its [terminal] scheme overrides).
            Message::TermScheme(name) => {
                let v = name.to_ascii_lowercase();
                self.save_appearance(|t| {
                    t.insert("terminal-colors".into(), v.into());
                })
            }
            _ => {}
        }
    }
}
