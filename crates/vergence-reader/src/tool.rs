//! Reader's tools, Text and Hand: the one chosen last remembered for the
//! next window and start. The window itself is in main.rs.

use std::path::PathBuf;

/// The tool chosen last, kept for the next window and the next start
/// (see docs/reader-milestone-1.md):
/// ~/.local/state/vergence/reader-tool, "hand" or "text".
fn tool_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("vergence/reader-tool"))
}

pub(crate) fn hand_remembered() -> bool {
    tool_file().and_then(|f| std::fs::read_to_string(f).ok()).is_some_and(|t| t.trim() == "hand")
}

pub(crate) fn remember_tool(hand: bool) {
    if let Some(f) = tool_file() {
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(f, if hand { "hand\n" } else { "text\n" });
    }
}
