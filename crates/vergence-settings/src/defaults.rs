// The Default apps page (files milestone 1, step 2): which application
// opens each everyday kind of file and link. A choice is written to
// Vergence's own list, ~/.config/vergence-mimeapps.list (vergence-types),
// read before the shared mimeapps.list in a Vergence session; the shared one
// is never written (Dave, 2026-11-15), so other desktops keep theirs. A kind
// covers every type of it (Pictures: PNG, JPEG, GIF, ...); its menu offers
// the apps that open its first type, and "Not chosen here" forgets
// Vergence's choice so the shared list's counts again.

use super::*;

/// One kind on the page: its name, a sentence, and the types it sets (the
/// first is the one its current app and its menu are read from).
pub struct Kind {
    pub name: &'static str,
    pub detail: &'static str,
    pub types: fn() -> Vec<&'static str>,
}

pub const KINDS: &[Kind] = &[
    Kind { name: "Web browser", detail: "Web links, and web pages saved as files.", types: || vec!["x-scheme-handler/https", "x-scheme-handler/http", "text/html", "application/xhtml+xml"] },
    Kind { name: "Mail", detail: "Mail links (mailto:).", types: || vec!["x-scheme-handler/mailto"] },
    Kind { name: "Text", detail: "Plain text, and every kind of it without an app of its own.", types: || vec!["text/plain"] },
    Kind { name: "Pictures", detail: "PNG, JPEG, GIF, WebP and the rest.", types: || vergence_types::of_kind("image") },
    Kind { name: "PDF", detail: "PDF documents.", types: || vec!["application/pdf"] },
    Kind { name: "Music", detail: "MP3, FLAC, Ogg, WAV and the rest.", types: || vergence_types::of_kind("audio") },
    Kind { name: "Video", detail: "MPEG-4, Matroska, WebM and the rest.", types: || vergence_types::of_kind("video") },
    Kind { name: "Folders", detail: "The app that opens a folder.", types: || vec![vergence_types::FOLDER] },
];

#[derive(Debug, Clone)]
pub enum DefaultsMsg {
    /// A kind (index in KINDS) and the app chosen (a desktop id), or None to forget the choice.
    Choose(usize, Option<String>),
}

impl App {
    /// The mimeapps.list files as they are now.
    pub fn defaults_read(&mut self) {
        self.default_lists = vergence_types::lists();
    }

    pub fn defaults_message(&mut self, m: DefaultsMsg) {
        match m {
            DefaultsMsg::Choose(k, app) => {
                let Some(kind) = KINDS.get(k) else { return };
                if let Err(e) = vergence_types::choose_defaults(&(kind.types)(), app.as_deref()) {
                    eprintln!("vergence-settings: {}: {e}", vergence_types::own_list_path().display());
                }
                self.defaults_read();
            }
        }
    }

    /// What an app is called, by its desktop id.
    fn entry_name(&self, id: &str) -> String {
        self.apps.iter().find(|e| vergence_shell::open::desktop_id(&e.id) == id).map(|e| e.name.clone()).unwrap_or_else(|| id.trim_end_matches(".desktop").to_string())
    }

    /// Whether Vergence's own list chooses for this type.
    fn chosen_here(&self, mime: &str) -> bool {
        let own = vergence_types::own_list_path();
        let list = std::fs::read_to_string(&own).map(|t| vergence_types::MimeApps::parse(&t)).unwrap_or_default();
        let m = vergence_types::canonical(mime);
        list.defaults.iter().any(|(d, _)| *d == m)
    }

    pub fn defaults_page(&self) -> Element<Message> {
        let rows: Vec<Element<Message>> = KINDS
            .iter()
            .enumerate()
            .map(|(k, kind)| {
                let first = (kind.types)()[0];
                let apps = vergence_shell::open::apps_for(first, &self.apps, &self.default_lists);
                if apps.is_empty() {
                    return setting(kind.name, kind.detail, label("No app installed opens these").size(12.0).dim().width(Length::Fixed(230.0)));
                }
                // The menu: the apps that open it, best first (the first is today's
                // default), and last "Not chosen here" while Vergence's list chooses.
                let here = self.chosen_here(first);
                let mut names: Vec<String> = apps.iter().map(|a| self.entry_name(a)).collect();
                if here {
                    names.push("Not chosen here (the shared list's)".into());
                }
                let ids = apps.clone();
                setting(
                    kind.name,
                    kind.detail,
                    choice(names, Some(0), move |i| Message::Defaults(DefaultsMsg::Choose(k, ids.get(i).cloned())))
                        .width(Length::Fixed(230.0))
                        .name(kind.name),
                )
            })
            .collect();
        self.page_layout(
            "Default apps",
            "Which app opens each kind of file and link, from File Viewer, links in apps, and vergence-open. Chosen here for Vergence only: other desktops keep their own.",
            vec![self.group(None, rows)],
        )
    }
}
