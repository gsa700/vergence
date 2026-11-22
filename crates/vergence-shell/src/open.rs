//! Which application opens which files and links (files milestone 1,
//! step 2), shared by the server (the bus's `Open`, the portal) and the
//! `vergence-open` command: each item's type (vergence-types), the
//! application chosen for it, and the items grouped per application in the
//! order given, so one program gets all of its files at once.

use std::path::PathBuf;

use crate::desktop::Entry;

/// Something to open: what to pass the application, and its type.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub given: String,
    pub arg: String,
    pub mime: String,
}

/// An item as given: a link (by its scheme), a `file:` link or a path (by
/// the file's type). Relative paths are taken from the current directory.
pub fn item(given: &str) -> Item {
    if let Some(mime) = vergence_types::of_link(given) {
        return Item { given: given.into(), arg: given.into(), mime };
    }
    let path = vergence_types::path_of_link(given).unwrap_or_else(|| PathBuf::from(given));
    let path = std::path::absolute(&path).unwrap_or(path);
    let mime = vergence_types::of_file(&path).to_string();
    Item { given: given.into(), arg: path.to_string_lossy().into_owned(), mime }
}

/// A desktop file id ("firefox.desktop") for an entry id or an id given either way.
pub fn desktop_id(id: &str) -> String {
    if id.ends_with(".desktop") { id.to_string() } else { format!("{id}.desktop") }
}

/// The applications installed, as vergence-types sees them.
fn known<'a>(entries: &'a [Entry], ids: &'a [String]) -> Vec<vergence_types::App<'a>> {
    entries
        .iter()
        .zip(ids)
        .filter(|(e, _)| !e.hidden)
        .map(|(e, id)| vergence_types::App { id, types: &e.mime_types })
        .collect()
}

/// The applications that open an item's type, best first (desktop file ids).
pub fn apps_for(mime: &str, entries: &[Entry], lists: &[vergence_types::MimeApps]) -> Vec<String> {
    let ids: Vec<String> = entries.iter().map(|e| desktop_id(&e.id)).collect();
    vergence_types::apps_for(mime, &known(entries, &ids), lists)
}

/// What to start: each application with its items (arguments), and the
/// items nothing opens, with why. With `app` given (a desktop id), every
/// item goes to it.
pub fn plan(
    items: &[Item],
    app: Option<&str>,
    entries: &[Entry],
    lists: &[vergence_types::MimeApps],
) -> (Vec<(Entry, Vec<String>)>, Vec<String>) {
    let ids: Vec<String> = entries.iter().map(|e| desktop_id(&e.id)).collect();
    let apps = known(entries, &ids);
    let entry_of = |id: &str| entries.iter().find(|e| !e.hidden && desktop_id(&e.id) == desktop_id(id)).cloned();
    let mut groups: Vec<(Entry, Vec<String>)> = Vec::new();
    let mut failed = Vec::new();
    if let Some(app) = app {
        match entry_of(app) {
            Some(e) => groups.push((e, items.iter().map(|i| i.arg.clone()).collect())),
            None => failed.push(format!("no application {app}")),
        }
        return (groups, failed);
    }
    for i in items {
        match vergence_types::default_for(&i.mime, &apps, lists).and_then(|id| entry_of(&id)) {
            Some(e) => match groups.iter_mut().find(|(g, _)| g.id == e.id) {
                Some((_, args)) => args.push(i.arg.clone()),
                None => groups.push((e, vec![i.arg.clone()])),
            },
            None => failed.push(format!(
                "{}: no application opens {} ({})",
                i.given,
                vergence_types::describe(&i.mime),
                i.mime
            )),
        }
    }
    (groups, failed)
}

/// The commands that start an entry with files: its own (`command_for`),
/// in our terminal (`vergence-term -e`) if it wants one.
pub fn commands(entry: &Entry, files: &[String]) -> Vec<Vec<String>> {
    crate::desktop::command_for(entry, files)
        .into_iter()
        .map(|argv| {
            if entry.terminal {
                let mut t = vec!["vergence-term".to_string(), "-e".to_string()];
                t.extend(argv);
                t
            } else {
                argv
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, exec: &str, types: &str, terminal: bool) -> Entry {
        let text = format!(
            "[Desktop Entry]\nType=Application\nName={id}\nExec={exec}\nMimeType={types}\nTerminal={terminal}\n"
        );
        crate::desktop::parse(id, &text).unwrap()
    }

    #[test]
    fn items_go_to_their_apps_grouped_in_order() {
        let entries = vec![
            entry("edit", "edit %F", "text/plain;", false),
            entry("view", "view %f", "image/png;", false),
            entry("vi", "vi %F", "text/plain;", true),
        ];
        let items = vec![
            Item { given: "a.txt".into(), arg: "/a.txt".into(), mime: "text/plain".into() },
            Item { given: "b.png".into(), arg: "/b.png".into(), mime: "image/png".into() },
            Item { given: "c.md".into(), arg: "/c.md".into(), mime: "text/markdown".into() },
            Item { given: "d.zip".into(), arg: "/d.zip".into(), mime: "application/zip".into() },
        ];
        let (groups, failed) = plan(&items, None, &entries, &[]);
        let got: Vec<(&str, Vec<String>)> = groups.iter().map(|(e, a)| (e.id.as_str(), a.clone())).collect();
        assert_eq!(
            got,
            vec![("edit", vec!["/a.txt".to_string(), "/c.md".to_string()]), ("view", vec!["/b.png".to_string()])]
        );
        assert_eq!(failed.len(), 1);
        assert!(failed[0].contains("Zip archive"));
        let lists = [vergence_types::MimeApps::parse("[Default Applications]\ntext/plain=vi.desktop\n")];
        let (groups, _) = plan(&items[..1], None, &entries, &lists);
        assert_eq!(groups[0].0.id, "vi");
        assert_eq!(commands(&groups[0].0, &groups[0].1), vec![vec!["vergence-term", "-e", "vi", "/a.txt"]]);
        let (groups, failed) = plan(&items, Some("view"), &entries, &[]);
        assert_eq!((groups.len(), groups[0].1.len(), failed.len()), (1, 4, 0));
        let (groups, failed) = plan(&items, Some("nothing.desktop"), &entries, &[]);
        assert!(groups.is_empty() && failed.len() == 1);
    }

    #[test]
    fn links_and_paths_are_items() {
        assert_eq!(item("https://example.org").mime, "x-scheme-handler/https");
        let i = item("file:///etc/hosts");
        assert_eq!(i.arg, "/etc/hosts");
        assert_eq!(desktop_id("firefox"), "firefox.desktop");
        assert_eq!(desktop_id("firefox.desktop"), "firefox.desktop");
    }
}
