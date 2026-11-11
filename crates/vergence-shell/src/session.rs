//! Session restore's rule: whether a session that is starting brings back
//! the applications the last one had running.
//!
//! The danger in restoring is a loop: an application that takes the session
//! down as it starts would take it down again at every start. So a session
//! that ends soon after a restore counts against restoring, and after two
//! in a row the next start is a clean one.

/// A session that ends within this many seconds of its restore ended "soon".
pub const SOON: u64 = 60;
/// This many sessions in a row ending soon after a restore, and the next
/// one starts clean.
pub const STRIKES: u32 = 2;

/// At a session's start: `pending` says the last session restored and had
/// not yet lasted [`SOON`] seconds when it ended; `strikes` is how many
/// before it did the same. Whether to restore now, and the strikes to
/// record.
pub fn at_start(pending: bool, strikes: u32) -> (bool, u32) {
    let strikes = if pending { strikes.saturating_add(1) } else { 0 };
    if strikes >= STRIKES { (false, 0) } else { (true, strikes) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_short_sessions_after_a_restore_and_the_next_starts_clean() {
        // The usual start: the last session lasted.
        assert_eq!(at_start(false, 0), (true, 0));
        // One that ended soon after its restore: restored once more, and counted.
        assert_eq!(at_start(true, 0), (true, 1));
        // A second in a row: a clean start, and the count begins again.
        assert_eq!(at_start(true, 1), (false, 0));
        // A session that lasted in between forgives the one before it.
        assert_eq!(at_start(false, 1), (true, 0));
        assert_eq!(at_start(true, u32::MAX), (false, 0));
    }
}
