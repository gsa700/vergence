// The input devices, for the settings app (settings milestone 1; see
// docs/settings-milestone-1.md): separate Keyboard, Mouse, Touchpad and
// Touchscreen pages, Keyboard and Mouse always, Touchpad and Touchscreen once
// the machine has had one. The devices present go to subscribers of the bus
// topic `inputs` (protocol v7) with every kind this machine has had, which
// is remembered in ~/.local/state/vergence/inputs.toml (owner only):
//
//     seen = ["touchpad", "touchscreen"]
//
// A touchscreen counts as seen when one is present. A touchpad only once it
// has scrolled with fingers: a Logitech receiver says it has a touchpad
// whether or not anything paired with it has one (pointer.rs), and a page
// for a touchpad that is not there would be a lie.
// The device names are the devices' own: untrusted text, shown as such.

use std::path::PathBuf;

use smithay::reexports::input::{Device, DeviceCapability};
use vergence_protocol::bus::{InputInfo, InputKind, ServerMsg, Topic};

use crate::state::Server;

/// The longest device name passed on.
const MAX_NAME: usize = 120;

#[derive(Debug, Default)]
pub struct Inputs {
    /// Present now: each device's kinds, by its system name ("event5").
    present: Vec<(String, InputInfo)>,
    /// Every kind the machine has had, in the order first seen.
    seen: Vec<InputKind>,
    read: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Record {
    #[serde(default)]
    seen: Vec<String>,
}

fn path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("vergence/inputs.toml"))
}

fn read() -> Vec<InputKind> {
    let record: Record =
        path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| toml::from_str(&t).ok()).unwrap_or_default();
    let mut seen = Vec::new();
    for k in record.seen.iter().filter_map(|n| InputKind::from_name(n)) {
        if !seen.contains(&k) {
            seen.push(k);
        }
    }
    seen
}

/// Written beside and moved into place, readable by its owner only.
fn write(seen: &[InputKind]) -> std::io::Result<()> {
    use std::{
        io::Write,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    };
    let Some(path) = path() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let record = Record { seen: seen.iter().map(|k| k.name().to_string()).collect() };
    let text = toml::to_string(&record).map_err(std::io::Error::other)?;
    let beside = path.with_extension("toml.new");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&beside)?;
    file.write_all(text.as_bytes())?;
    std::fs::rename(beside, path)
}

/// A device's name as passed on: control characters dropped, length limited.
fn clean(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect()
}

/// The kinds a device is: one device may be several (a receiver is a
/// keyboard and a mouse). A "keyboard" without letters (a power button, a
/// laptop's video bus, a lid) is none.
fn kinds(d: &Device) -> Vec<InputKind> {
    const KEY_A: u32 = 30;
    let mut out = Vec::new();
    if d.has_capability(DeviceCapability::Keyboard) && d.keyboard_has_key(KEY_A) == Ok(true) {
        out.push(InputKind::Keyboard);
    }
    if d.has_capability(DeviceCapability::Pointer) {
        out.push(if crate::pointer::is_touchpad(d) { InputKind::Touchpad } else { InputKind::Mouse });
    }
    if d.has_capability(DeviceCapability::Touch) {
        out.push(InputKind::Touchscreen);
    }
    out
}

/// Whether a kind counts as seen as soon as a device of it is present (a
/// touchpad waits for its fingers).
fn seen_when_present(k: InputKind) -> bool {
    k != InputKind::Touchpad
}

impl Server {
    fn inputs_read(&mut self) {
        if !self.inputs.read {
            self.inputs.read = true;
            self.inputs.seen = read();
        }
    }

    /// A device appeared (pointer.rs passes every one on).
    pub fn inputs_device_added(&mut self, d: &Device) {
        self.inputs_read();
        let sys = d.sysname().to_string();
        let name = clean(&d.name());
        let kinds = kinds(d);
        for k in &kinds {
            self.inputs.present.push((sys.clone(), InputInfo { name: name.clone(), kind: *k }));
        }
        let mut new = false;
        for k in kinds.into_iter().filter(|k| seen_when_present(*k)) {
            if !self.inputs.seen.contains(&k) {
                self.inputs.seen.push(k);
                new = true;
            }
        }
        if new {
            self.inputs_save();
        }
        self.bus_inputs_changed();
    }

    pub fn inputs_device_removed(&mut self, d: &Device) {
        let sys = d.sysname();
        let before = self.inputs.present.len();
        self.inputs.present.retain(|(s, _)| s != sys);
        if self.inputs.present.len() != before {
            self.bus_inputs_changed();
        }
    }

    /// A touchpad scrolled with fingers (input.rs): one is really there.
    pub fn inputs_touchpad_used(&mut self) {
        self.inputs_read();
        if !self.inputs.seen.contains(&InputKind::Touchpad) {
            tracing::info!("input: a touchpad scrolled; the settings app shows its page from now on");
            self.inputs.seen.push(InputKind::Touchpad);
            self.inputs_save();
            self.bus_inputs_changed();
        }
    }

    fn inputs_save(&self) {
        if let Err(e) = write(&self.inputs.seen) {
            tracing::warn!("input: can't save the kinds seen: {e}");
        }
    }

    pub fn bus_inputs(&mut self) -> ServerMsg {
        self.inputs_read();
        ServerMsg::Inputs {
            serial: 0,
            devices: self.inputs.present.iter().map(|(_, i)| i.clone()).collect(),
            seen: self.inputs.seen.clone(),
        }
    }

    fn bus_inputs_changed(&mut self) {
        let ids = self.bus_subscribers(Topic::Inputs);
        if ids.is_empty() {
            return;
        }
        let m = self.bus_inputs();
        for id in ids {
            self.bus_send(id, m.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean("Logitech\u{1b}[31m USB\nReceiver"), "Logitech[31m USBReceiver");
        assert_eq!(clean(&"x".repeat(500)).len(), MAX_NAME);
    }

    #[test]
    fn a_touchpad_waits_for_its_fingers() {
        assert!(!seen_when_present(InputKind::Touchpad));
        assert!(seen_when_present(InputKind::Touchscreen));
        assert!(seen_when_present(InputKind::Keyboard));
    }
}
