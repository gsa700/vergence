// The Default apps page (files milestone 1, step 2): which application
// opens each everyday kind of file and link. A choice is written to
// Vergence's own list, ~/.config/vergence-mimeapps.list (vergence-types),
// read before the shared mimeapps.list in a Vergence session; the shared one
// is never written, so other desktops keep theirs (see docs/files-milestone-1.md). A kind
// covers every type of it (Pictures: PNG, JPEG, GIF, ...); its menu offers
// the apps that open its first type, and "Not chosen here" forgets
// Vergence's choice so the shared list's counts again.
//
// Text can also be opened by a terminal editor, for people who edit in nano
// and the like: the ones on the PATH are offered as "nano (in Terminal)" and
// so on. Choosing one writes a small desktop entry of our own, ~/.local/share/applications/
// vergence-term-<program>.desktop (Terminal=true, so opening runs it in
// our terminal; NoDisplay, so the launcher does not list it), and makes it
// the default like any other app (see docs/apps-roadmap.md).

use super::*;

/// One kind on the page: its name, a sentence, and the types it sets (the
/// first is the one its current app and its menu are read from).
pub struct Kind {
    pub name: &'static str,
    pub detail: &'static str,
    pub types: fn() -> Vec<&'static str>,
}

pub const KINDS: &[Kind] = &[
    Kind {
        name: "Web browser",
        detail: "Web links, and web pages saved as files.",
        types: || vec!["x-scheme-handler/https", "x-scheme-handler/http", "text/html", "application/xhtml+xml"],
    },
    Kind { name: "Mail", detail: "Mail links (mailto:).", types: || vec!["x-scheme-handler/mailto"] },
    Kind {
        name: "Text",
        detail: "Plain text, and every kind of it without an app of its own.",
        types: || vec!["text/plain"],
    },
    Kind { name: "Pictures", detail: "PNG, JPEG, GIF, WebP and the rest.", types: || vergence_types::of_kind("image") },
    Kind { name: "PDF", detail: "PDF documents.", types: || vec!["application/pdf"] },
    Kind { name: "Music", detail: "MP3, FLAC, Ogg, WAV and the rest.", types: || vergence_types::of_kind("audio") },
    Kind { name: "Video", detail: "MPEG-4, Matroska, WebM and the rest.", types: || vergence_types::of_kind("video") },
    Kind { name: "Folders", detail: "The app that opens a folder.", types: || vec![vergence_types::FOLDER] },
];

/// Terminal programs that edit text: (program, name shown, its command
/// line for files). Offered for Text when on the PATH.
pub const TERMINAL_EDITORS: &[(&str, &str, &str)] = &[
    ("nano", "nano", "nano %F"),
    ("micro", "micro", "micro %F"),
    ("vim", "Vim", "vim %F"),
    ("nvim", "Neovim", "nvim %F"),
    ("vi", "vi", "vi %F"),
    ("emacs", "Emacs", "emacs -nw %F"),
    ("mg", "mg", "mg %F"),
    ("ed", "ed", "ed %f"),
];

/// The desktop id of our entry for a terminal program.
fn terminal_entry_id(program: &str) -> String {
    format!("vergence-term-{program}.desktop")
}

/// The entry's text: run in our terminal, not listed, opens text.
fn terminal_entry_text(name: &str, exec: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={name} (in Terminal)\nComment=Made by Vergence's Settings > Default apps\nExec={exec}\nTerminal=true\nNoDisplay=true\nMimeType=text/plain;\n"
    )
}

/// Writes our entry for a terminal program (owner-writable, readable, as
/// desktop entries are), by a temporary file renamed over.
fn write_terminal_entry(program: &str) -> std::io::Result<()> {
    let Some(&(_, name, exec)) = TERMINAL_EDITORS.iter().find(|(p, _, _)| *p == program) else {
        return Err(std::io::Error::other("not a terminal editor this knows"));
    };
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
        .ok_or_else(|| std::io::Error::other("no home folder"))?;
    let dir = data.join("applications");
    std::fs::create_dir_all(&dir)?;
    let to = dir.join(terminal_entry_id(program));
    let tmp = to.with_extension("desktop.new");
    std::fs::write(&tmp, terminal_entry_text(name, exec))?;
    std::fs::rename(&tmp, &to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminal_editor_entry_runs_in_the_terminal_unlisted() {
        let t = terminal_entry_text("nano", "nano %F");
        let e = vergence_shell::desktop::parse("vergence-term-nano", &t).expect("parses");
        assert!(e.terminal && e.no_display);
        assert_eq!(e.name, "nano (in Terminal)");
        assert!(e.mime_types.iter().any(|m| m == "text/plain"));
        assert_eq!(terminal_entry_id("nano"), "vergence-term-nano.desktop");
    }
}

#[derive(Debug, Clone)]
pub enum DefaultsMsg {
    /// A kind (index in KINDS) and the app chosen (a desktop id), or None to forget the choice.
    Choose(usize, Option<String>),
    /// A kind and a terminal program to open it with (its entry made first).
    ChooseTerminal(usize, String),
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
            DefaultsMsg::ChooseTerminal(k, program) => {
                let Some(kind) = KINDS.get(k) else { return };
                if let Err(e) = write_terminal_entry(&program) {
                    return eprintln!("vergence-settings: the entry for {program}: {e}");
                }
                if let Err(e) = vergence_types::choose_defaults(&(kind.types)(), Some(&terminal_entry_id(&program))) {
                    eprintln!("vergence-settings: {}: {e}", vergence_types::own_list_path().display());
                }
                self.apps = vergence_shell::apps::installed();
                self.defaults_read();
            }
        }
    }

    /// What an app is called, by its desktop id.
    fn entry_name(&self, id: &str) -> String {
        self.apps
            .iter()
            .find(|e| vergence_shell::open::desktop_id(&e.id) == id)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| id.trim_end_matches(".desktop").to_string())
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
                    return setting(
                        kind.name,
                        kind.detail,
                        label("No app installed opens these").size(12.0).dim().width(Length::Fixed(230.0)),
                    );
                }
                // The menu: the apps that open it, best first (the first is today's
                // default), and last "Not chosen here" while Vergence's list chooses.
                let here = self.chosen_here(first);
                let mut names: Vec<String> = apps.iter().map(|a| self.entry_name(a)).collect();
                // Text: terminal editors on the PATH not yet made into entries.
                let terminals: Vec<String> = if first == "text/plain" {
                    TERMINAL_EDITORS
                        .iter()
                        .filter(|(p, _, _)| vergence_shell::apps::on_path(p) && !apps.contains(&terminal_entry_id(p)))
                        .map(|(p, _, _)| p.to_string())
                        .collect()
                } else {
                    Vec::new()
                };
                for p in &terminals {
                    let name = TERMINAL_EDITORS.iter().find(|(q, _, _)| q == p).map_or(p.as_str(), |(_, n, _)| n);
                    names.push(format!("{name} (in Terminal)"));
                }
                if here {
                    names.push("Not chosen here (the shared list's)".into());
                }
                let ids = apps.clone();
                setting(
                    kind.name,
                    kind.detail,
                    choice(names, Some(0), move |i| {
                        if i < ids.len() {
                            Message::Defaults(DefaultsMsg::Choose(k, Some(ids[i].clone())))
                        } else if let Some(p) = terminals.get(i - ids.len()) {
                            Message::Defaults(DefaultsMsg::ChooseTerminal(k, p.clone()))
                        } else {
                            Message::Defaults(DefaultsMsg::Choose(k, None))
                        }
                    })
                    .width(Length::Fixed(230.0))
                    .name(kind.name),
                )
            })
            .collect();
        self.page_layout(
            "Default apps",
            "Which app opens each kind of file and link, from FileView, links in apps, and vergence-open. Chosen here for Vergence only: other desktops keep their own.",
            vec![self.group(None, rows)],
        )
    }
}
