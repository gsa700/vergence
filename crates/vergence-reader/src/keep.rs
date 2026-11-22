//! Reader's windows kept across restarts (Dave, 2026-11-21: after a
//! restart "Reader re-opened but had neither of the two pdf's i had
//! open"), as Edit keeps its own (vergence-edit keep.rs): each window's
//! token (window memory puts it back where it was), its documents in tab
//! order and which was in front, and each document's place: the page, how
//! far down it, and the zoom (session milestone 1: after a restart a
//! document comes back scrolled to where it was; one the user opens again
//! starts at its first page). In ~/.local/state/vergence/reader-session
//! (owner only), written when a window's documents change, because a
//! logout or a restart ends the program from outside; a window the user
//! closes leaves it.

use std::path::{Path, PathBuf};

/// The most windows and documents read back.
const MAX_WINDOWS: usize = 32;
const MAX_TABS: usize = 64;

/// Where a document was: the first page of the row at the view's top, how
/// far down that row (points), and the zoom (pdf::places' words).
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub page: usize,
    pub within: f32,
    pub mode: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeptTab {
    pub path: PathBuf,
    pub at: Option<Place>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snap {
    pub token: String,
    pub tabs: Vec<KeptTab>,
    /// The document in front (an index into `tabs`).
    pub front: usize,
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("vergence/reader-session"))
}

/// "window <token>", "front <k>", then a line a document: "tab <path>",
/// each followed by its place if it has one: "at <page> <within> <zoom>".
pub fn text(windows: &[Snap]) -> String {
    let mut s = String::new();
    for w in windows.iter().filter(|w| !w.tabs.is_empty()) {
        s.push_str(&format!("window {}\nfront {}\n", w.token.replace(['\n', ' '], ""), w.front));
        for t in &w.tabs {
            let p = t.path.to_string_lossy();
            if !p.contains('\n') {
                s.push_str(&format!("tab {p}\n"));
                if let Some(a) = &t.at
                    && a.within.is_finite()
                    && !a.mode.contains(char::is_whitespace)
                {
                    s.push_str(&format!("at {} {} {}\n", a.page, a.within, a.mode));
                }
            }
        }
    }
    s
}

pub fn parse(text: &str) -> Vec<Snap> {
    let mut out: Vec<Snap> = Vec::new();
    // The last line was a document taken: an "at" line is its place.
    let mut took = false;
    for line in text.lines() {
        let after_tab = std::mem::take(&mut took);
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
                w.tabs.push(KeptTab { path: p.to_path_buf(), at: None });
                took = true;
            }
        } else if let (Some(a), true, Some(t)) =
            (line.strip_prefix("at "), after_tab, out.last_mut().and_then(|w| w.tabs.last_mut()))
        {
            let f: Vec<&str> = a.split_whitespace().collect();
            if let [page, within, mode] = f[..]
                && let (Ok(page), Ok(within)) = (page.parse::<usize>(), within.parse::<f32>())
                && within.is_finite()
                && within >= 0.0
            {
                t.at = Some(Place { page, within, mode: mode.to_string() });
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
    let ok =
        std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).and_then(|mut f| {
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
        let front = w.tabs.get(w.front).map(|t| t.path.clone());
        w.tabs.retain(|t| t.path.exists());
        w.front = front.and_then(|f| w.tabs.iter().position(|t| t.path == f)).unwrap_or(0);
    }
    all.retain(|w| !w.tabs.is_empty());
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(p: &str, at: Option<(usize, f32, &str)>) -> KeptTab {
        KeptTab { path: p.into(), at: at.map(|(page, within, mode)| Place { page, within, mode: mode.into() }) }
    }

    #[test]
    fn windows_come_back_as_kept() {
        let all = vec![
            Snap {
                token: "abc".into(),
                tabs: vec![
                    tab("/home/d/a.pdf", Some((39, 212.5, "width"))),
                    tab("/home/d/My Manual.pdf", Some((0, 0.0, "1.250"))),
                ],
                front: 1,
            },
            Snap { token: "def".into(), tabs: vec![tab("/tmp/b.pdf", None)], front: 0 },
            Snap { token: "empty".into(), tabs: Vec::new(), front: 0 },
        ];
        assert_eq!(parse(&text(&all)), all[..2].to_vec());
    }

    #[test]
    fn a_session_from_before_places_still_reads() {
        let got = parse("window t\nfront 0\ntab /a.pdf\ntab /b.pdf\n");
        assert_eq!(
            got,
            vec![Snap { token: "t".into(), tabs: vec![tab("/a.pdf", None), tab("/b.pdf", None)], front: 0 }]
        );
    }

    #[test]
    fn nonsense_is_left_out() {
        let got = parse(
            "tab /before/any/window.pdf\nat 3 0 width\nwindow t\nfront 9\ntab relative.pdf\nat 1 2 page\ntab /x.pdf\nat x 2 page\nat 4 -1 page\nat 4 NaN page\ntab /y.pdf\nat 2 5 page\ntab no/z.pdf\nat 7 0 page\n",
        );
        assert_eq!(
            got,
            vec![Snap {
                token: "t".into(),
                tabs: vec![tab("/x.pdf", None), tab("/y.pdf", Some((2, 5.0, "page")))],
                front: 1
            }],
            "a place goes only with the document just before it"
        );
    }
}
