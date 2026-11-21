//! What an app brings back when it starts (session milestone 1; Dave,
//! 2026-11-21). Ended from outside (a logout, a restart, a power cut, a
//! crash), everything comes back as it was, places inside documents too.
//! Closed by the user, a document starts fresh. Settings keeps its page for
//! a while.
//!
//! desktop.toml `[session]`: `restore` (the apps reopened at the start;
//! false also means the apps forget the windows they kept), `places` (a
//! reopened document goes back to its place), `settings-keep` (minutes the
//! settings app keeps its page after a close; 0 never, -1 until the next
//! session). The server gives each session an id, `VERGENCE_SESSION`, so an
//! app can tell a new session from the same one.

use std::path::{Path, PathBuf};

/// Minutes the settings app keeps its page after a close, by default.
pub const SETTINGS_KEEP: i64 = 30;
/// `settings-keep` meaning "until the next session".
pub const UNTIL_NEXT_SESSION: i64 = -1;

/// This session's id (empty outside a Vergence session).
pub fn id() -> String {
    std::env::var("VERGENCE_SESSION").unwrap_or_default()
}

/// `[session] restore`: true unless it says false.
pub fn restore() -> bool {
    read_bool("restore").unwrap_or(true)
}

/// `[session] places`: true unless it says false.
pub fn places() -> bool {
    read_bool("places").unwrap_or(true)
}

/// `[session] settings-keep`, in minutes (0 never, -1 until the next session).
pub fn settings_keep() -> i64 {
    desktop_text().and_then(|t| value(&t, "session", "settings-keep")).and_then(|v| v.parse().ok()).filter(|m: &i64| *m >= UNTIL_NEXT_SESSION).unwrap_or(SETTINGS_KEEP)
}

/// Whether a window told the session is ending (Program::session_ending,
/// at `told`) is closing for the session's sake rather than the user's.
/// Held for a little longer than an exit waits (8 s): if the exit stopped
/// (a question about work not saved), a later close is the user's again.
pub fn ending(told: Option<std::time::Instant>) -> bool {
    told.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(15))
}

/// Seconds since 1970.
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Whether something kept when the user closed it (at `kept_at`, in session
/// `kept_in`) is still to be used now: in the same session, and within
/// `minutes` (0 never; -1 for the whole session).
pub fn still_kept(kept_at: u64, kept_in: &str, now: u64, session: &str, minutes: i64) -> bool {
    if minutes == 0 || kept_in != session {
        return false;
    }
    minutes < 0 || (now >= kept_at && now - kept_at <= minutes as u64 * 60)
}

/// ~/.local/state/vergence (XDG_STATE_HOME's vergence).
pub fn state_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("vergence"))
}

/// Writes `text` to `path`, readable by its owner only: beside it, then
/// moved into place, so a power cut leaves the old or the new.
pub fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let beside = path.with_extension("part");
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&beside)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&beside, path)
}

fn desktop_text() -> Option<String> {
    crate::prefs::desktop_path().and_then(|p| std::fs::read_to_string(p).ok())
}

fn read_bool(key: &str) -> Option<bool> {
    match value(&desktop_text()?, "session", key)?.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// The value of `key` in table `[section]` of a TOML text, as written
/// (quotes taken off a string); enough for the plain keys read here.
fn value(text: &str, section: &str, key: &str) -> Option<String> {
    let head = format!("[{section}]");
    let mut inside = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            inside = l == head;
            continue;
        }
        if !inside || l.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = l.split_once('=')
            && k.trim().trim_matches('"') == key
        {
            let v = v.split(" #").next().unwrap_or("").trim();
            return Some(v.trim_matches('"').to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_read_from_their_table() {
        let t = "[shelf]\nrestore = false\n\n[session]\n# restore = false\nrestore = true\nsettings-keep = 10 # minutes\nplaces = false\n";
        assert_eq!(value(t, "session", "restore").as_deref(), Some("true"));
        assert_eq!(value(t, "session", "settings-keep").as_deref(), Some("10"));
        assert_eq!(value(t, "session", "places").as_deref(), Some("false"));
        assert_eq!(value(t, "shelf", "places"), None);
    }

    #[test]
    fn kept_for_a_while_in_the_same_session() {
        let at = 1_000_000;
        assert!(still_kept(at, "s1", at + 29 * 60, "s1", 30));
        assert!(!still_kept(at, "s1", at + 31 * 60, "s1", 30), "too long ago");
        assert!(!still_kept(at, "s1", at + 60, "s2", 30), "a new session");
        assert!(!still_kept(at, "s1", at + 60, "s1", 0), "never kept");
        assert!(still_kept(at, "s1", at + 86_400, "s1", UNTIL_NEXT_SESSION));
        assert!(!still_kept(at, "s1", at - 60, "s1", 30), "the clock went back");
    }
}
