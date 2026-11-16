//! What starts with the session (dock milestone 1, step 7): the user's
//! own autostart entries, freedesktop's Desktop Application Autostart:
//! $XDG_CONFIG_HOME/autostart/*.desktop (~/.config/autostart). The system's
//! directories (/etc/xdg/autostart) are not read: what they hold was put
//! there for other desktops, and Vergence starts only what its user chose.
//! An entry that is hidden, for other desktops only, or switched off the
//! GNOME way (X-GNOME-Autostart-enabled=false) is left alone.

use std::path::{Path, PathBuf};

use crate::desktop::{Entry, parse};

/// The user's autostart directory.
pub fn dir() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("autostart"))
}

/// The most entries started, so a directory full of junk cannot flood the session.
const MOST: usize = 64;

/// The entries in `dir` to start, by name.
pub fn entries(dir: &Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<PathBuf> = read.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "desktop") && p.is_file()).collect();
    names.sort();
    names
        .into_iter()
        .take(MOST)
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            let id = p.file_stem()?.to_string_lossy().into_owned();
            wanted(&id, &text)
        })
        .collect()
}

/// One entry's text: the entry, if it is to start.
pub fn wanted(id: &str, text: &str) -> Option<Entry> {
    let e = parse(id, text)?;
    let off = text.lines().any(|l| l.trim().replace(' ', "") == "X-GNOME-Autostart-enabled=false");
    (!e.hidden && !e.elsewhere && !off && !e.exec.is_empty()).then_some(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "[Desktop Entry]\nType=Application\nName=UPS tile\nExec=/usr/local/libexec/vergence/ups-tile ups@host\nNoDisplay=true\n";

    #[test]
    fn what_starts_and_what_does_not() {
        assert!(wanted("ups", APP).is_some(), "NoDisplay does not stop it");
        assert!(wanted("ups", &format!("{APP}OnlyShowIn=Vergence;\n")).is_some());
        assert!(wanted("g", &format!("{APP}OnlyShowIn=GNOME;\n")).is_none(), "for another desktop");
        assert!(wanted("n", &format!("{APP}NotShowIn=Vergence;\n")).is_none());
        assert!(wanted("h", &format!("{APP}Hidden=true\n")).is_none());
        assert!(wanted("o", &format!("{APP}X-GNOME-Autostart-enabled=false\n")).is_none());
        assert!(wanted("x", "[Desktop Entry]\nType=Application\nName=X\n").is_none(), "nothing to run");
    }

    #[test]
    fn a_directory_in_order_of_names() {
        let d = std::env::temp_dir().join(format!("vergence-autostart-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("b.desktop"), APP).unwrap();
        std::fs::write(d.join("a.desktop"), APP).unwrap();
        std::fs::write(d.join("notes.txt"), "not an entry").unwrap();
        let ids: Vec<String> = entries(&d).into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["a", "b"]);
        std::fs::remove_dir_all(&d).unwrap();
        assert!(entries(&d).is_empty(), "no directory, nothing");
    }
}
