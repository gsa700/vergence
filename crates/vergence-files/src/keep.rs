//! FileView's windows kept across restarts (files milestone 3, step 1; the
//! rules of session milestone 1): each window's token (window memory puts
//! it back where it was), its tabs in order (a folder each, or Recent) and
//! which was in front. Ended from outside (a logout, a restart, a power
//! cut), the windows come back as they were; a window the user closes
//! leaves it. In ~/.local/state/vergence/files-session (owner only),
//! written when a window's tabs change. The Trash window keeps nothing.

use std::path::{Path, PathBuf};

use vergence_widgets::session;

/// The most windows and tabs read back (a broken file opens no more).
const MAX_WINDOWS: usize = 32;
const MAX_TABS: usize = 64;

/// What a kept tab shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Kept {
    Folder(PathBuf),
    Recent,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snap {
    pub token: String,
    pub tabs: Vec<Kept>,
    /// The tab in front (an index into `tabs`).
    pub front: usize,
}

fn path() -> Option<PathBuf> {
    Some(session::state_dir()?.join("files-session"))
}

/// "window <token>", "front <k>", then a line a tab: "tab <folder>" or "recent".
pub fn text(windows: &[Snap]) -> String {
    let mut s = String::new();
    for w in windows.iter().filter(|w| !w.tabs.is_empty()) {
        s.push_str(&format!("window {}\nfront {}\n", w.token.replace(['\n', ' '], ""), w.front));
        for t in &w.tabs {
            match t {
                Kept::Recent => s.push_str("recent\n"),
                Kept::Folder(p) => {
                    let p = p.to_string_lossy();
                    if !p.contains('\n') {
                        s.push_str(&format!("tab {p}\n"));
                    }
                }
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
        } else if let Some(w) = out.last_mut().filter(|w| w.tabs.len() < MAX_TABS) {
            if line == "recent" {
                w.tabs.push(Kept::Recent);
            } else if let Some(p) = line.strip_prefix("tab ").map(Path::new).filter(|p| p.is_absolute()) {
                w.tabs.push(Kept::Folder(p.to_path_buf()));
            }
        }
    }
    out.retain(|w| !w.tabs.is_empty());
    for w in out.iter_mut() {
        w.front = w.front.min(w.tabs.len() - 1);
    }
    out
}

/// Writes what is kept, replacing what was.
pub fn save(windows: &[Snap]) {
    if let Some(p) = path() {
        let _ = session::write_private(&p, &text(windows));
    }
}

/// What was kept, read back; folders no longer there left out.
pub fn load() -> Vec<Snap> {
    let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else { return Vec::new() };
    let mut all = parse(&text);
    for w in all.iter_mut() {
        let front = w.tabs.get(w.front).cloned();
        w.tabs.retain(|t| match t {
            Kept::Folder(p) => p.is_dir(),
            Kept::Recent => true,
        });
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
            Snap { token: "abc".into(), tabs: vec![Kept::Folder("/home/d".into()), Kept::Recent, Kept::Folder("/home/d/My Pictures".into())], front: 2 },
            Snap { token: "def".into(), tabs: vec![Kept::Folder("/tmp".into())], front: 0 },
            Snap { token: "empty".into(), tabs: Vec::new(), front: 0 },
        ];
        assert_eq!(parse(&text(&all)), all[..2].to_vec());
    }

    #[test]
    fn nonsense_is_left_out() {
        let got = parse("tab /before/any/window\nwindow t\nfront 9\ntab relative\nnonsense\ntab /x\n");
        assert_eq!(got, vec![Snap { token: "t".into(), tabs: vec![Kept::Folder("/x".into())], front: 0 }]);
    }
}
