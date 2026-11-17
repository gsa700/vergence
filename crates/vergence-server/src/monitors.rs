// One monitor, one identity, whichever input it is plugged into (2026-11-17).
//
// A monitor gives each of its inputs its own EDID: Techbench's LG Ultra HD
// on HDMI and on DisplayPort hashed to two ids, so it was two monitors, each
// with its own scale, place and window memory. Maker, product code and serial
// number are the same on every input (vergence_profile::model_key), so the
// first EDID seen for a monitor stands for it from then on: its id, and its
// part of the setup key, are worked out from that EDID on every input.
//
// ~/.local/state/vergence/monitors.toml (owner only):
//
//     [[monitor]]
//     key = "1e6d-5b09-52074-"   # model_key
//     edid = "00ffffff..."       # the first EDID seen, in hex
//
// Not used when two connected monitors share a key (identical twins whose
// serials are the same), and never for monitors without a serial
// (model_key is None): those stay apart by their EDIDs as before.

use std::path::PathBuf;

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Record {
    #[serde(default)]
    monitor: Vec<Entry>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct Entry {
    key: String,
    edid: String,
}

/// The most monitors remembered (the oldest go first).
const MAX: usize = 64;

fn path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("vergence/monitors.toml"))
}

fn read() -> Record {
    path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

/// Written beside and moved into place, readable by its owner only.
fn write(record: &Record) -> std::io::Result<()> {
    use std::{io::Write, os::unix::fs::{DirBuilderExt, OpenOptionsExt}};
    let Some(path) = path() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let text = toml::to_string(record).map_err(std::io::Error::other)?;
    let beside = path.with_extension("toml.new");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&beside)?;
    file.write_all(text.as_bytes())?;
    std::fs::rename(beside, path)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    (s.len() % 2 == 0 && s.len() <= 2 * 4096).then_some(())?;
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// The identity to use for a monitor whose EDID is `edid`: the EDID first
/// seen for the same monitor if there was one, else `edid` itself (which is
/// remembered). `others` are the EDIDs of the other connected monitors, as
/// read from them.
pub fn identity(edid: &[u8], others: &[Vec<u8>]) -> Vec<u8> {
    let Some(key) = vergence_profile::model_key(edid) else { return edid.to_vec() };
    if others.iter().any(|o| vergence_profile::model_key(o).as_deref() == Some(key.as_str())) {
        return edid.to_vec();
    }
    let mut record = read();
    if let Some(first) = record.monitor.iter().find(|e| e.key == key).and_then(|e| unhex(&e.edid)) {
        if vergence_profile::canonical(&first) != vergence_profile::canonical(edid) {
            tracing::info!("monitor {key}: on another input; known as {}", vergence_profile::monitor_id(&first));
        }
        return first;
    }
    record.monitor.push(Entry { key, edid: hex(vergence_profile::canonical(edid)) });
    let excess = record.monitor.len().saturating_sub(MAX);
    record.monitor.drain(..excess);
    if let Err(e) = write(&record) {
        tracing::warn!("monitors.toml: {e}");
    }
    edid.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let b = vec![0u8, 0xff, 0x1e, 0x6d];
        assert_eq!(unhex(&hex(&b)), Some(b));
        assert_eq!(unhex("0g"), None);
        assert_eq!(unhex("abc"), None);
    }
}
