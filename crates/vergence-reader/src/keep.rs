//! Reader's windows kept across restarts (Dave, 2026-11-21: after a
//! restart "Reader re-opened but had neither of the two pdf's i had
//! open"), as Edit keeps its own (vergence-edit keep.rs): each window's
//! token (window memory puts it back where it was), its documents in tab
//! order and which was in front. Each document's page and view are kept
//! apart already (pdf::places). In ~/.local/state/vergence/reader-session
//! (owner only), written when a window's documents change, because a
//! logout or a restart ends the program from outside; a window the user
//! closes leaves it.

use std::path::{Path, PathBuf};

/// The most windows and documents read back.
const MAX_WINDOWS: usize = 32;
const MAX_TABS: usize = 64;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snap {
    pub token: String,
    pub tabs: Vec<PathBuf>,
    /// The document in front (an index into `tabs`).
    pub front: usize,
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("vergence/reader-session"))
}

/// "window <token>", "front <k>", then a line a document: "tab <path>".
pub fn text(windows: &[Snap]) -> String {
    let mut s = String::new();
    for w in windows.iter().filter(|w| !w.tabs.is_empty()) {
        s.push_str(&format!("window {}\nfront {}\n", w.token.replace(['\n', ' '], ""), w.front));
        for t in &w.tabs {
            let p = t.to_string_lossy();
            if !p.contains('\n') {
                s.push_str(&format!("tab {p}\n"));
            }
        }
    }
    s
}

pub fn parse(text: &str) -> Vec<Snap> {
    let mut out: Vec<Snap> = Vec::new();
    for line in text.lines() {
        if let Some(t) = line.strip_prefix("window ") {
            if out.len() >= MAX_WINDOWS {
                break;
            }
            out.push(Snap { token: t.trim().to_string(), ..Snap::default() });
        } else if let (Some(f), Some(w)) = (line.strip_prefix("front "), out.last_mut()) {
            w.front = f.trim().parse().unwrap_or(0);
        } else if let (Some(p), Some(w)) = (line.strip_prefix("tab "), out.last_mut()) {
            let p = Path::new(p);
            if p.is_absolute() && w.tabs.len() < MAX_TABS {
                w.tabs.push(p.to_path_buf());
            }
        }
    }
    out.retain(|w| !w.tabs.is_empty());
    for w in out.iter_mut() {
        w.front = w.front.min(w.tabs.len() - 1);
    }
    out
}

/// Writes what is kept, replacing what was (beside it, then moved).
pub fn save(windows: &[Snap]) {
    use std::os::unix::fs::OpenOptionsExt;
    let Some(p) = path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension("part");
    let ok = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).and_then(|mut f| {
        use std::io::Write;
        f.write_all(text(windows).as_bytes())
    });
    if ok.is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// What was kept, read back; documents no longer there left out.
pub fn load() -> Vec<Snap> {
    let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else { return Vec::new() };
    let mut all = parse(&text);
    for w in all.iter_mut() {
        let front = w.tabs.get(w.front).cloned();
        w.tabs.retain(|t| t.exists());
        w.front = front.and_then(|f| w.tabs.iter().position(|t| *t == f)).unwrap_or(0);
    }
    all.retain(|w| !w.tabs.is_empty());
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_come_back_as_kept() {
        let all = vec![
            Snap { token: "abc".into(), tabs: vec!["/home/d/a.pdf".into(), "/home/d/My Manual.pdf".into()], front: 1 },
            Snap { token: "def".into(), tabs: vec!["/tmp/b.pdf".into()], front: 0 },
            Snap { token: "empty".into(), tabs: Vec::new(), front: 0 },
        ];
        assert_eq!(parse(&text(&all)), all[..2].to_vec());
    }

    #[test]
    fn nonsense_is_left_out() {
        let got = parse("tab /before/any/window.pdf\nwindow t\nfront 9\ntab relative.pdf\ntab /x.pdf\n");
        assert_eq!(got, vec![Snap { token: "t".into(), tabs: vec!["/x.pdf".into()], front: 0 }]);
    }
}
