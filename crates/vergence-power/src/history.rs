//! The battery's history (Dave, 2026-11-14: "chart and track the battery
//! longer than two minutes ... battery users will want it"): the server
//! writes one line a minute while there is a battery, and keeps a week;
//! the Battery page charts the charge over a day or a week and sums up the
//! time on battery and the power drawn.
//!
//! ~/.local/state/vergence/battery.csv, owner only:
//!
//! ```text
//! 1763151600,87,9.4,D
//! ```
//!
//! seconds since 1970, percent, watts (0 when not known), and the state:
//! C charging, D discharging (on battery), F full, N plugged in not
//! charging, U unknown. A gap of more than a few minutes between lines is
//! the machine asleep or off.

use std::path::{Path, PathBuf};

use crate::State;

/// How long the history is kept.
pub const KEEP: u64 = 7 * 24 * 3600;
/// How often the server writes a line.
pub const EVERY: u64 = 60;
/// Lines further apart than this have the machine asleep or off between them.
pub const GAP: u64 = 3 * EVERY;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Record {
    /// Seconds since 1970.
    pub t: u64,
    pub percent: u8,
    pub watts: f32,
    pub state: State,
}

fn state_char(s: State) -> char {
    match s {
        State::Charging => 'C',
        State::Discharging => 'D',
        State::Full => 'F',
        State::NotCharging => 'N',
        State::Unknown => 'U',
    }
}

fn char_state(c: &str) -> State {
    match c {
        "C" => State::Charging,
        "D" => State::Discharging,
        "F" => State::Full,
        "N" => State::NotCharging,
        _ => State::Unknown,
    }
}

impl Record {
    pub fn line(&self) -> String {
        format!("{},{},{:.1},{}\n", self.t, self.percent.min(100), self.watts.max(0.0), state_char(self.state))
    }

    /// A line read back; anything malformed is skipped.
    pub fn parse(line: &str) -> Option<Record> {
        let mut f = line.trim().split(',');
        let t = f.next()?.parse().ok()?;
        let percent: u8 = f.next()?.parse().ok()?;
        let watts: f32 = f.next()?.parse().ok()?;
        let state = char_state(f.next()?);
        (percent <= 100 && watts.is_finite() && watts >= 0.0).then_some(Record { t, percent, watts, state })
    }
}

/// Where the history lives.
pub fn path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("vergence/battery.csv"))
}

/// Seconds since 1970, now.
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The records from `since` on, oldest first.
pub fn read(path: &Path, since: u64) -> Vec<Record> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let mut out: Vec<Record> = text.lines().filter_map(Record::parse).filter(|r| r.t >= since).collect();
    out.sort_by_key(|r| r.t);
    out
}

fn open_private(path: &Path, append: bool) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let mut o = std::fs::OpenOptions::new();
    o.create(true).mode(0o600);
    if append {
        o.append(true)
    } else {
        o.write(true).truncate(true)
    };
    o.open(path)
}

/// Add a record.
pub fn append(path: &Path, r: &Record) -> std::io::Result<()> {
    use std::io::Write;
    open_private(path, true)?.write_all(r.line().as_bytes())
}

/// Drop what is older than [`KEEP`] (written beside, moved into place).
pub fn prune(path: &Path, now: u64) -> std::io::Result<()> {
    use std::io::Write;
    let keep = read(path, now.saturating_sub(KEEP));
    let beside = path.with_extension("csv.new");
    let mut f = open_private(&beside, false)?;
    for r in &keep {
        f.write_all(r.line().as_bytes())?;
    }
    std::fs::rename(beside, path)
}

/// What a stretch of history comes to.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Summary {
    /// Seconds on battery (between lines no further apart than [`GAP`]).
    pub on_battery: u64,
    /// The average power drawn on battery, watts.
    pub avg_watts: Option<f32>,
    /// Percentage points used on battery (the drops added up).
    pub used: u32,
}

pub fn summary(records: &[Record]) -> Summary {
    let mut s = Summary::default();
    let (mut wsum, mut wn) = (0.0f64, 0u32);
    for pair in records.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a.state == State::Discharging && b.state == State::Discharging && b.t > a.t && b.t - a.t <= GAP {
            s.on_battery += b.t - a.t;
            s.used += a.percent.saturating_sub(b.percent) as u32;
        }
    }
    for r in records.iter().filter(|r| r.state == State::Discharging && r.watts > 0.0) {
        wsum += r.watts as f64;
        wn += 1;
    }
    s.avg_watts = (wn > 0).then(|| (wsum / wn as f64) as f32);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(t: u64, percent: u8, watts: f32, state: State) -> Record {
        Record { t, percent, watts, state }
    }

    #[test]
    fn lines_round_trip_and_junk_is_skipped() {
        let r = rec(1_763_151_600, 87, 9.4, State::Discharging);
        assert_eq!(r.line(), "1763151600,87,9.4,D\n");
        assert_eq!(Record::parse(&r.line()), Some(r));
        assert_eq!(Record::parse("garbage"), None);
        assert_eq!(Record::parse("1,101,1.0,D"), None, "no more than 100%");
        assert_eq!(Record::parse("1,50,NaN,D"), None);
        assert_eq!(Record::parse("1,50,-3,D"), None);
    }

    #[test]
    fn a_stretch_on_battery_sums_up() {
        let mut v = Vec::new();
        // An hour on battery from 100% to 80% at 10 W, a gap (asleep), then charging.
        for i in 0..=60u64 {
            v.push(rec(i * 60, 100 - (i / 3) as u8, 10.0, State::Discharging));
        }
        v.push(rec(60 * 60 + 3600, 80, 0.0, State::Discharging));
        v.push(rec(60 * 60 + 3660, 81, 20.0, State::Charging));
        let s = summary(&v);
        assert_eq!(s.on_battery, 3600, "the gap is not counted");
        assert_eq!(s.used, 20);
        assert_eq!(s.avg_watts, Some(10.0), "the zero-watt reading is left out");
    }

    #[test]
    fn written_kept_and_pruned() {
        let dir = std::env::temp_dir().join(format!("vergence-power-history-{}", std::process::id()));
        let path = dir.join("battery.csv");
        let now = 10 * 24 * 3600;
        append(&path, &rec(now - 9 * 24 * 3600, 50, 8.0, State::Discharging)).unwrap();
        append(&path, &rec(now - 60, 49, 8.0, State::Discharging)).unwrap();
        append(&path, &rec(now, 48, 8.0, State::Discharging)).unwrap();
        assert_eq!(read(&path, 0).len(), 3);
        prune(&path, now).unwrap();
        let kept = read(&path, 0);
        assert_eq!(kept.len(), 2, "older than a week is dropped");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(dir).ok();
    }
}
