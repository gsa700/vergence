//! Unsaved work kept across restarts (editor milestone 1, step 8), as Mac
//! and GNOME editors do: each window's token (window memory puts it back
//! where it was) and tabs: a file's path, an untitled document's number,
//! the caret, and the text of any document with changes not saved. Kept in
//! ~/.local/state/vergence/edit/ (owner only), written a second after
//! changes, because a logout ends the program from outside. A window the
//! user closes (after the save question) leaves it. Saved files are never
//! touched: only the copies of unsaved text are here.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// The most windows and tabs read back (a broken file opens no more).
const MAX_WINDOWS: usize = 32;
const MAX_TABS: usize = 128;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabSnap {
    pub path: Option<PathBuf>,
    pub untitled: u32,
    /// The text, if it has changes not saved.
    pub text: Option<String>,
    pub caret: (usize, usize),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowSnap {
    pub token: String,
    pub front: usize,
    pub tabs: Vec<TabSnap>,
}

pub fn dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("vergence/edit"))
}

/// The session file's text: "window <token>", "front <k>", then a line a tab:
/// untitled number, caret line, caret column, the text's file ("-": none),
/// the path (empty: none), tab-separated.
fn render(windows: &[WindowSnap], text_files: &[Vec<Option<String>>]) -> String {
    let mut out = String::new();
    for (w, files) in windows.iter().zip(text_files) {
        out += &format!("window {}\nfront {}\n", w.token.replace(['\n', '\t'], ""), w.front);
        for (t, f) in w.tabs.iter().zip(files) {
            let path = t.path.as_ref().map(|p| p.to_string_lossy().replace(['\n', '\t'], " ")).unwrap_or_default();
            out += &format!("tab {}\t{}\t{}\t{}\t{}\n", t.untitled, t.caret.0, t.caret.1, f.as_deref().unwrap_or("-"), path);
        }
    }
    out
}

/// Writes what is kept, replacing what was.
pub fn save(windows: &[WindowSnap]) {
    let Some(d) = dir() else { return };
    if fs::DirBuilder::new().recursive(true).mode(0o700).create(&d).is_err() {
        return;
    }
    let write = |name: &str, text: &str| -> bool {
        let tmp = d.join(format!("{name}.new"));
        let ok = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).and_then(|mut f| f.write_all(text.as_bytes())).is_ok();
        ok && fs::rename(&tmp, d.join(name)).is_ok()
    };
    // The unsaved texts first, then the list that names them.
    let mut n = 0;
    let mut names: Vec<Vec<Option<String>>> = Vec::new();
    let mut keep: Vec<String> = Vec::new();
    for w in windows {
        let mut ws = Vec::new();
        for t in &w.tabs {
            ws.push(t.text.as_ref().and_then(|text| {
                n += 1;
                let name = format!("text-{n}");
                write(&name, text).then(|| {
                    keep.push(name.clone());
                    name
                })
            }));
        }
        names.push(ws);
    }
    write("session", &render(windows, &names));
    // Texts no longer named go.
    if let Ok(rd) = fs::read_dir(&d) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("text-") && !keep.contains(&name) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

/// What was kept, read back.
pub fn load() -> Vec<WindowSnap> {
    let Some(d) = dir() else { return Vec::new() };
    let Ok(text) = fs::read_to_string(d.join("session")) else { return Vec::new() };
    parse(&text, &d)
}

pub fn parse(text: &str, d: &Path) -> Vec<WindowSnap> {
    let mut out: Vec<WindowSnap> = Vec::new();
    for line in text.lines() {
        if let Some(token) = line.strip_prefix("window ") {
            if out.len() >= MAX_WINDOWS {
                break;
            }
            out.push(WindowSnap { token: token.trim().chars().take(64).collect(), ..WindowSnap::default() });
        } else if let Some(k) = line.strip_prefix("front ") {
            if let Some(w) = out.last_mut() {
                w.front = k.trim().parse().unwrap_or(0);
            }
        } else if let Some(rest) = line.strip_prefix("tab ") {
            let Some(w) = out.last_mut() else { continue };
            if w.tabs.len() >= MAX_TABS {
                continue;
            }
            let f: Vec<&str> = rest.splitn(5, '\t').collect();
            if f.len() < 5 {
                continue;
            }
            // Only a text file of our own naming, in our own folder.
            let text = (f[3] != "-" && f[3].starts_with("text-") && !f[3].contains('/')).then(|| fs::read_to_string(d.join(f[3])).ok()).flatten();
            let path = (!f[4].is_empty() && f[4].starts_with('/')).then(|| PathBuf::from(f[4]));
            w.tabs.push(TabSnap { untitled: f[0].parse().unwrap_or(1), caret: (f[1].parse().unwrap_or(0), f[2].parse().unwrap_or(0)), text, path });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_kept_reads_back_and_a_bad_file_opens_little() {
        let d = std::env::temp_dir().join(format!("vergence-keep-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("text-1"), "unsaved\nwork").unwrap();
        let windows = vec![WindowSnap {
            token: "abc".into(),
            front: 1,
            tabs: vec![
                TabSnap { path: Some("/home/x/notes.txt".into()), untitled: 0, text: None, caret: (3, 4) },
                TabSnap { path: None, untitled: 2, text: Some("unsaved\nwork".into()), caret: (1, 2) },
            ],
        }];
        let text = render(&windows, &[vec![None, Some("text-1".into())]]);
        assert_eq!(parse(&text, &d), windows);
        // Junk, and a text file outside the folder, are passed over.
        let bad = "tab 1\t0\t0\t-\t/x\nwindow t\ntab 1\t0\t0\t../../etc/passwd\t\ntab short\nwindow u\nfront 9\n";
        let w = parse(bad, &d);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].tabs.len(), 1);
        assert_eq!(w[0].tabs[0].text, None);
        fs::remove_dir_all(&d).unwrap();
    }
}
