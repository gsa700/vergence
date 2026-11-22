//! The sessions the login screen offers (Dave, 2026-11-16: "5 b", Vergence
//! and the installed Wayland sessions): the system's own wayland-sessions
//! entries, never a user's (threat model T12). The login screen and vdm read
//! the same list in the same order, so the greeter sends only an index and
//! vdm starts what its own reading says (review M2).

/// One session: its name and its entry's file (vdm reads its Exec line after
/// it has become the user, step 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub name: String,
    pub file: std::path::PathBuf,
}

/// Where the system keeps them.
pub const DIRS: [&str; 2] = ["/usr/share/wayland-sessions", "/usr/local/share/wayland-sessions"];

/// Every session, Vergence first, then by name; an entry that is hidden or
/// has no Name or Exec is left out; one file name in both directories once.
pub fn list() -> Vec<Session> {
    let mut out: Vec<Session> = Vec::new();
    for dir in DIRS {
        let Ok(read) = std::fs::read_dir(dir) else { continue };
        let mut files: Vec<std::path::PathBuf> =
            read.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "desktop")).collect();
        files.sort();
        for f in files {
            if out.iter().any(|s| s.file.file_name() == f.file_name()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            if let Some(name) = entry_name(&text) {
                out.push(Session { name, file: f });
            }
        }
    }
    out.sort_by_key(|s| (s.name != "Vergence", s.name.to_lowercase()));
    out
}

/// A session entry's Name, if it is one that can be started.
pub fn entry_name(text: &str) -> Option<String> {
    let (mut name, mut exec, mut hidden, mut group) = (None, false, false, false);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            group = line == "[Desktop Entry]";
            continue;
        }
        if !group {
            continue;
        }
        match line.split_once('=') {
            Some(("Name", v)) if name.is_none() => name = Some(v.trim().to_string()),
            Some(("Exec", v)) => exec = !v.trim().is_empty(),
            Some(("Hidden" | "NoDisplay", "true")) => hidden = true,
            _ => {}
        }
    }
    name.filter(|n| !n.is_empty() && exec && !hidden)
}

/// A session entry's Exec line, its field codes (%f, %U ...) left out.
pub fn entry_exec(text: &str) -> Option<String> {
    let mut group = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            group = line == "[Desktop Entry]";
            continue;
        }
        if group && let Some(v) = line.strip_prefix("Exec=") {
            let words: Vec<&str> = v.split_whitespace().filter(|w| !(w.len() == 2 && w.starts_with('%'))).collect();
            return (!words.is_empty()).then(|| words.join(" "));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_entry() {
        let e = "[Desktop Entry]\nName=Vergence\nExec=/usr/local/bin/vergence-session\nType=Application\n";
        assert_eq!(entry_name(e).as_deref(), Some("Vergence"));
        assert_eq!(entry_name("[Desktop Entry]\nName=X\n"), None, "nothing to start");
        assert_eq!(entry_name("[Desktop Entry]\nName=X\nExec=x\nHidden=true\n"), None);
        assert_eq!(entry_name("[Other]\nName=X\nExec=x\n"), None);
        assert_eq!(entry_exec(e).as_deref(), Some("/usr/local/bin/vergence-session"));
        assert_eq!(
            entry_exec("[Desktop Entry]\nExec=sway --unsupported-gpu %U\n").as_deref(),
            Some("sway --unsupported-gpu")
        );
    }
}
