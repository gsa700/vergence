//! The applications a system has: every desktop entry in the XDG data
//! directories, a user's entry hiding the system's of the same id.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use crate::desktop::{self, Entry, MAX_FILE};

/// How deep under `applications/` entries are looked for, and how many
/// files one directory may give: far more than any real system has.
const MAX_DEPTH: usize = 4;
const MAX_ENTRIES: usize = 4096;

/// The data directories, the user's first: `$XDG_DATA_HOME` (else
/// `~/.local/share`), then each of `$XDG_DATA_DIRS` (else
/// `/usr/local/share` and `/usr/share`, which are where FreeBSD's and
/// Linux's packages put things).
pub fn data_dirs() -> Vec<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    let mut dirs = Vec::new();
    match var("XDG_DATA_HOME") {
        Some(home) => dirs.push(PathBuf::from(home)),
        None => {
            if let Some(home) = var("HOME") {
                dirs.push(PathBuf::from(home).join(".local/share"));
            }
        }
    }
    match var("XDG_DATA_DIRS") {
        Some(list) => dirs.extend(std::env::split_paths(&list).filter(|p| p.is_absolute())),
        None => dirs.extend([PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")]),
    }
    let mut seen = HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

/// The entry files under one `applications` directory, with their ids
/// ("sub/dir/app.desktop" is "sub-dir-app").
fn files(root: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let mut pending = vec![(root.to_path_buf(), String::new(), 0)];
    while let Some((dir, prefix, depth)) = pending.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for item in read.flatten() {
            let (path, name) = (item.path(), item.file_name().to_string_lossy().into_owned());
            if path.is_dir() {
                if depth < MAX_DEPTH {
                    pending.push((path, format!("{prefix}{name}-"), depth + 1));
                }
            } else if let Some(stem) = name.strip_suffix(".desktop")
                && found.len() < MAX_ENTRIES
            {
                found.push((format!("{prefix}{stem}"), path));
            }
        }
    }
    found
}

/// True if a program named (or at the path) `program` can be run.
pub fn on_path(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let runnable = |p: &Path| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    if program.contains('/') {
        return runnable(Path::new(program));
    }
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| runnable(&dir.join(program))))
}

/// Every application entry found under `dirs` (each a data directory, the
/// most important first), by id: the first directory that has an id wins,
/// and an entry marked Hidden there removes the id altogether. Entries not
/// meant for a list (NoDisplay) are kept and marked: they can still own
/// windows. One whose TryExec program is missing is left out. Sorted by
/// name, without regard to case.
pub fn installed_in(dirs: &[PathBuf]) -> Vec<Entry> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut entries = Vec::new();
    for dir in dirs {
        let mut here = files(&dir.join("applications"));
        // Within one directory the order is the names', so the same files always give the same list.
        here.sort();
        for (id, path) in here {
            if seen.contains(&id) {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            if meta.len() as usize > MAX_FILE {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Some(entry) = desktop::parse(&id, &text) else { continue };
            seen.insert(id);
            if entry.hidden || (!entry.try_exec.is_empty() && !on_path(&entry.try_exec)) {
                continue;
            }
            entries.push(entry);
        }
    }
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    entries
}

/// The applications of this machine ([`installed_in`] the [`data_dirs`]).
pub fn installed() -> Vec<Entry> {
    installed_in(&data_dirs())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A directory of its own under the system's temporary one, removed when dropped.
    pub(crate) struct Scratch(pub PathBuf);

    impl Scratch {
        pub(crate) fn new(name: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!("vergence-shell-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        pub(crate) fn write(&self, path: &str, text: &str) {
            let at = self.0.join(path);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(at, text).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn entry(name: &str, more: &str) -> String {
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec={}\n{more}", name.to_lowercase())
    }

    #[test]
    fn a_users_entry_hides_the_systems() {
        let s = Scratch::new("apps");
        s.write("system/applications/firefox.desktop", &entry("Firefox", ""));
        s.write("system/applications/htop.desktop", &entry("Htop", "Terminal=true\n"));
        s.write("system/applications/gone.desktop", &entry("Gone", ""));
        s.write("system/applications/quiet.desktop", &entry("Quiet", "NoDisplay=true\n"));
        s.write("system/applications/kde/konsole.desktop", &entry("Konsole", ""));
        s.write("system/applications/absent.desktop", &entry("Absent", "TryExec=/no/such/program-anywhere\n"));
        s.write("system/applications/notes.txt", "not an entry");
        s.write("system/applications/broken.desktop", "[Desktop Entry]\nType=Application\nName=Broken\nExec=\"open\n");
        s.write("user/applications/firefox.desktop", &entry("My Firefox", ""));
        s.write("user/applications/gone.desktop", "[Desktop Entry]\nHidden=true\n");
        let found = installed_in(&[s.0.join("user"), s.0.join("system"), s.0.join("not-there")]);
        let names: Vec<(&str, &str)> = found.iter().map(|e| (e.id.as_str(), e.name.as_str())).collect();
        assert_eq!(names, [("htop", "Htop"), ("kde-konsole", "Konsole"), ("firefox", "My Firefox"), ("quiet", "Quiet")]);
        assert!(found.iter().find(|e| e.id == "htop").unwrap().terminal);
        assert!(found.iter().find(|e| e.id == "quiet").unwrap().no_display);
    }

    #[test]
    fn programs_are_found_on_the_path() {
        assert!(on_path("sh"), "every machine has sh on its PATH");
        assert!(on_path("/bin/sh"));
        assert!(!on_path("no-such-program-anywhere-at-all"));
        assert!(!on_path("/no/such/program"));
        assert!(!on_path("/"), "a directory is not a program");
    }
}
