// Where the settings app was (session milestone 1; Dave, 2026-11-21:
// "Settings is one where I would go the other way: during a session
// remember everything, so if I am trying to get a setting just right but
// have to close settings to do so, it should re-open on the same page and
// location as I closed it. Maybe have it reset if the computer restarts or
// a period of time has gone by?").
//
// ~/.local/state/vergence/settings-page (owner only): the page (its
// sidebar name) or the app whose page is open, how far it is scrolled,
// and, when the user closed the window, when and in which session:
//
//     page Mouse            (or: app org.mozilla.firefox)
//     scroll 412.5
//     closed 1763750000 3f9c0a1b2d4e5f60
//
// No "closed" line: the app was ended from outside (a logout, a restart, a
// power cut), and it opens there again at the next start if the session
// restores (`[session] restore`). With one: it opens there again only in
// the same session and within `[session] settings-keep` minutes.

use std::path::PathBuf;
use vergence_widgets::session;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Kept {
    /// The page's sidebar name ("Mouse", "Theme and fonts").
    pub page: Option<String>,
    /// The app whose own page was open (Settings > Apps).
    pub app: Option<String>,
    pub scroll: f32,
    /// When the user closed the window, and the session it was in.
    pub closed: Option<(u64, String)>,
}

fn path() -> Option<PathBuf> {
    Some(session::state_dir()?.join("settings-page"))
}

fn one_line(s: &str) -> bool {
    !s.is_empty() && s.len() < 256 && !s.contains(['\n', '\r'])
}

pub fn text(k: &Kept) -> String {
    let mut s = String::new();
    if let Some(a) = k.app.as_deref().filter(|a| one_line(a)) {
        s += &format!("app {a}\n");
    } else if let Some(p) = k.page.as_deref().filter(|p| one_line(p)) {
        s += &format!("page {p}\n");
    }
    if k.scroll.is_finite() && k.scroll > 0.0 {
        s += &format!("scroll {:.1}\n", k.scroll);
    }
    if let Some((at, id)) = &k.closed
        && !id.contains(char::is_whitespace)
    {
        s += &format!("closed {at} {id}\n");
    }
    s
}

pub fn parse(text: &str) -> Kept {
    let mut k = Kept::default();
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("page ").filter(|p| one_line(p)) {
            k.page = Some(p.to_string());
        } else if let Some(a) = line.strip_prefix("app ").filter(|a| one_line(a)) {
            k.app = Some(a.to_string());
        } else if let Some(s) = line.strip_prefix("scroll ") {
            k.scroll = s.trim().parse::<f32>().ok().filter(|s| s.is_finite() && *s >= 0.0).unwrap_or(0.0);
        } else if let Some(c) = line.strip_prefix("closed ") {
            let mut f = c.split_whitespace();
            if let Some(at) = f.next().and_then(|a| a.parse().ok()) {
                k.closed = Some((at, f.next().unwrap_or("").to_string()));
            }
        }
    }
    k
}

pub fn save(k: &Kept) {
    if let Some(p) = path() {
        let _ = session::write_private(&p, &text(k));
    }
}

/// What to open at, if anything: kept from a session that ended (when
/// sessions restore), or from a close by the user in this session within
/// the time set.
pub fn wanted(k: Kept, now: u64, session_id: &str, restore: bool, keep_minutes: i64) -> Option<Kept> {
    if k.page.is_none() && k.app.is_none() {
        return None;
    }
    let good = match &k.closed {
        None => restore,
        Some((at, id)) => session::still_kept(*at, id, now, session_id, keep_minutes),
    };
    good.then_some(k)
}

/// What the last run kept, if it is to be used now.
pub fn load() -> Option<Kept> {
    let text = std::fs::read_to_string(path()?).ok()?;
    wanted(parse(&text), session::now(), &session::id(), session::restore(), session::settings_keep())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(closed: Option<(u64, &str)>) -> Kept {
        Kept { page: Some("Theme and fonts".into()), app: None, scroll: 412.5, closed: closed.map(|(a, s)| (a, s.to_string())) }
    }

    #[test]
    fn written_and_read_back() {
        for k in [kept(None), kept(Some((1_763_750_000, "3f9c0a1b2d4e5f60"))), Kept { app: Some("org.mozilla.firefox".into()), ..Kept::default() }] {
            assert_eq!(parse(&text(&k)), k);
        }
        assert_eq!(parse("page \nscroll -4\nscroll NaN\nclosed x y\n"), Kept::default());
    }

    #[test]
    fn ended_from_outside_comes_back_when_the_session_restores() {
        assert!(wanted(kept(None), 5000, "new", true, 30).is_some());
        assert!(wanted(kept(None), 5000, "new", false, 30).is_none(), "start with no apps open");
    }

    #[test]
    fn closed_by_the_user_comes_back_for_a_while_in_the_same_session() {
        let k = kept(Some((5000, "s1")));
        assert!(wanted(k.clone(), 5000 + 600, "s1", true, 30).is_some());
        assert!(wanted(k.clone(), 5000 + 600, "s1", false, 30).is_some(), "the close's own time, whatever restore says");
        assert!(wanted(k.clone(), 5000 + 3600, "s1", true, 30).is_none(), "too long ago");
        assert!(wanted(k.clone(), 5000 + 600, "s2", true, 30).is_none(), "a new session: the front page");
        assert!(wanted(k, 5000 + 600, "s1", true, 0).is_none(), "keeping off");
        assert!(wanted(Kept::default(), 0, "", true, 30).is_none(), "nothing kept");
    }
}
