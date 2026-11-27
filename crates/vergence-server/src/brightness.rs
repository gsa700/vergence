//! The screens' brightness (brightness milestone 1, docs/brightness-milestone-1.md):
//! the brightness keys, the Display tile with a slider per screen that has a
//! control, and the level shown as the volume keys show theirs.
//!
//! The levels are read and set on a thread of its own: vdm does the setting
//! (a request on its power socket, vergence_dm::brightness::ask), which must
//! not hold up a frame, and a slider dragged sends many values of which only
//! the latest matters, so the thread takes what has piled up and sets the last
//! for each screen. It reads every screen's level again after setting and
//! every few seconds otherwise (a laptop's firmware may change its panel by
//! itself), and hands the levels to the main loop, which rebuilds the tile
//! when they changed.
//!
//! Each screen's level is kept (question 5 a), by its monitor's identity (its
//! EDID's: it follows the screen to any port), in
//! ~/.local/state/vergence/brightness.toml, and set again the first time the
//! screen is seen in a session: a laptop's panel comes back at the
//! firmware's level after a restart. After that, every change is written
//! down, whoever made it (a key, a slider, the settings app, the laptop's own
//! keys), on the thread.
//!
//! `VERGENCE_BRIGHTNESS_ROOT=<dir>`: the controls are looked for under that
//! directory instead of `/` and set there directly, not through vdm (the
//! display suite's stand-in backlight; never a real one).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use smithay::reexports::calloop::channel::{self, Event};
use vergence_protocol::tile::{Card, Control, ControlKind, Tile};

use crate::Server;
use crate::notify::{Origin, Request};
use crate::tiles::Owner;

pub const TILE: &str = "display";
/// How long the level stays up after a key, as the volume's.
const SHOWN_MS: i32 = 1500;
/// How often the levels are read when nothing was set.
const LOOK_EVERY: Duration = Duration::from_secs(4);

/// The brightness, as the main loop knows it.
#[derive(Default)]
pub struct Brightness {
    /// Each screen with a control (its connector) and its level in percent,
    /// in the order found; a level set here counts at once, before the thread
    /// has read it back.
    pub levels: Vec<(String, u32)>,
    jobs: Option<mpsc::Sender<Job>>,
    /// The level shown after a key: replaced by the next.
    note: u32,
    /// The levels kept, by monitor identity, as last written.
    kept: HashMap<String, u32>,
    /// The screens (connectors) whose kept level was set again this session.
    restored: HashSet<String>,
}

enum Job {
    Set(String, u32),
    /// Write the kept levels down.
    Keep(HashMap<String, u32>),
}

fn kept_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("vergence/brightness.toml"))
}

/// The kept levels: `[levels]`, a monitor's identity = its percent.
fn read_kept() -> HashMap<String, u32> {
    let text = kept_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
    text.parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get("levels").and_then(|l| l.as_table()).cloned())
        .map(|l| {
            l.into_iter()
                .filter_map(|(k, v)| v.as_integer().filter(|v| (0..=100).contains(v)).map(|v| (k, v as u32)))
                .collect()
        })
        .unwrap_or_default()
}

fn write_kept(kept: &HashMap<String, u32>) {
    let Some(file) = kept_file() else { return };
    let mut levels = toml::Table::new();
    for (k, v) in kept {
        levels.insert(k.clone(), toml::Value::Integer(*v as i64));
    }
    let mut t = toml::Table::new();
    t.insert("levels".into(), toml::Value::Table(levels));
    let text = format!(
        "# Each screen's brightness, by its monitor's identity, set again at login (Vergence).\n{}",
        toml::to_string(&t).unwrap_or_default()
    );
    let tmp = file.with_extension("toml.new");
    let done = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| std::fs::write(&tmp, text))
        .and_then(|_| std::fs::rename(&tmp, &file));
    if let Err(e) = done {
        tracing::warn!("brightness: {}: {e}", file.display());
    }
}

fn root() -> (PathBuf, bool) {
    match std::env::var_os("VERGENCE_BRIGHTNESS_ROOT").map(PathBuf::from).filter(|p| p.is_absolute()) {
        Some(p) => (p, true),
        None => (PathBuf::from("/"), false),
    }
}

/// Every screen's level now.
fn read(root: &std::path::Path) -> Vec<(String, u32)> {
    vergence_dm::brightness::controls_in(root)
        .iter()
        .filter_map(|c| vergence_dm::brightness::level(c).map(|l| (c.connector.clone(), l)))
        .collect()
}

fn work(jobs: mpsc::Receiver<Job>, tx: channel::Sender<Vec<(String, u32)>>) {
    let (root, standin) = root();
    let mut wait = Duration::from_millis(300);
    loop {
        // The first look soon after starting: by then the screens are the
        // server's outputs, and a kept level comes back at once after login.
        let first = match jobs.recv_timeout(std::mem::replace(&mut wait, LOOK_EVERY)) {
            Ok(j) => Some(j),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        // What piled up: the last level asked for each screen, the last kept.
        let mut wanted: HashMap<String, u32> = HashMap::new();
        let mut keep = None;
        for job in first.into_iter().chain(std::iter::from_fn(|| jobs.try_recv().ok())) {
            match job {
                Job::Set(connector, level) => {
                    wanted.insert(connector, level);
                }
                Job::Keep(k) => keep = Some(k),
            }
        }
        if let Some(k) = keep {
            write_kept(&k);
        }
        for (connector, level) in wanted {
            let result = if standin {
                vergence_dm::brightness::controls_in(&root)
                    .iter()
                    .find(|c| c.connector == connector)
                    .ok_or_else(|| "no such screen".to_string())
                    .and_then(|c| vergence_dm::brightness::set(c, level))
            } else {
                vergence_dm::brightness::ask(&connector, level)
            };
            match result {
                Ok(()) => tracing::info!("brightness: {connector} {level}%"),
                Err(e) => tracing::warn!("brightness: {connector} {level}%: {e}"),
            }
        }
        // Sent each time, changed or not: a screen that became an output
        // since (its level waiting to be set again) is seen to.
        if tx.send(read(&root)).is_err() {
            return;
        }
    }
}

impl Server {
    /// Start the brightness thread (at startup).
    pub fn start_brightness(&mut self) {
        let (tx, rx) = channel::channel::<Vec<(String, u32)>>();
        let res = self.loop_handle.insert_source(rx, |event, _, state| {
            if let Event::Msg(levels) = event {
                let waiting = levels
                    .iter()
                    .any(|(c, _)| !state.brightness.restored.contains(c) && state.brightness_monitor(c).is_some());
                if levels != state.brightness.levels || waiting {
                    state.brightness.levels = levels;
                    state.brightness_tile();
                }
            }
        });
        if let Err(e) = res {
            return tracing::warn!("brightness: {e}");
        }
        let (jobs_tx, jobs_rx) = mpsc::channel();
        // Read once before the first frame's tile: the levels as they are.
        self.brightness.levels = read(&root().0);
        self.brightness.kept = read_kept();
        match std::thread::Builder::new().name("brightness".into()).spawn(move || work(jobs_rx, tx)) {
            Ok(_) => self.brightness.jobs = Some(jobs_tx),
            Err(e) => tracing::warn!("brightness: can't start its thread: {e}"),
        }
        self.brightness_tile();
    }

    /// A screen's name for people: a laptop's panel is the built-in display
    /// (it often has no make and model), another its model, else its connector.
    fn screen_label(&self, connector: &str) -> String {
        if vergence_dm::brightness::is_internal(connector) {
            return "Built-in display".into();
        }
        self.space
            .outputs()
            .find(|o| o.name() == connector)
            .map(|o| o.physical_properties().model)
            .filter(|m| !m.is_empty() && m.as_str() != connector.split('-').next().unwrap_or(""))
            .unwrap_or_else(|| connector.to_string())
    }

    /// Set a screen's level: counted at once here, carried out on the thread.
    pub fn brightness_set(&mut self, connector: &str, level: u32) {
        let level = level.min(100);
        let Some(entry) = self.brightness.levels.iter_mut().find(|(c, _)| c == connector) else { return };
        entry.1 = level;
        if let Some(jobs) = &self.brightness.jobs {
            let _ = jobs.send(Job::Set(connector.to_string(), level));
        }
        self.brightness_tile();
    }

    /// A screen's monitor identity, once the screen is one of the server's
    /// outputs (before that its level is neither set again nor kept).
    fn brightness_monitor(&self, connector: &str) -> Option<String> {
        self.space
            .outputs()
            .find(|o| o.name() == connector)
            .and_then(|o| o.user_data().get::<crate::memory::MonitorId>().map(|m| m.0.clone()))
    }

    /// The kept levels against the levels now: a screen seen for the first
    /// time this session gets its kept level back; after that, a level that
    /// changed is kept.
    fn brightness_keep(&mut self) {
        let mut restore = Vec::new();
        let mut changed = false;
        for (connector, level) in self.brightness.levels.clone() {
            let Some(id) = self.brightness_monitor(&connector) else { continue };
            if self.brightness.restored.insert(connector.clone()) {
                match self.brightness.kept.get(&id) {
                    Some(&want) if want != level => {
                        tracing::info!("brightness: {connector} back to {want}% (kept)");
                        restore.push((connector, want));
                        continue;
                    }
                    Some(_) => continue,
                    // Never seen: kept as it is.
                    None => {}
                }
            }
            if self.brightness.kept.get(&id) != Some(&level) {
                self.brightness.kept.insert(id, level);
                changed = true;
            }
        }
        if changed && let Some(jobs) = &self.brightness.jobs {
            let _ = jobs.send(Job::Keep(self.brightness.kept.clone()));
        }
        for (connector, level) in restore {
            self.brightness_set(&connector, level);
        }
    }

    /// A brightness key: `step` percent up or down, on the built-in panel if
    /// there is one, else the screen the pointer is on, else the first with
    /// a control; the level shown as the volume keys show theirs.
    pub fn brightness_key(&mut self, step: i8) {
        let at = self.output_at_pointer();
        let under = self
            .space
            .outputs()
            .find(|o| at.is_some_and(|a| self.space.output_geometry(o) == Some(a)))
            .map(|o| o.name());
        let levels = &self.brightness.levels;
        let Some((connector, level)) = levels
            .iter()
            .find(|(c, _)| vergence_dm::brightness::is_internal(c))
            .or_else(|| levels.iter().find(|(c, _)| Some(c) == under.as_ref()))
            .or_else(|| levels.first())
            .cloned()
        else {
            return tracing::debug!("brightness key: no screen with a control");
        };
        let next = (level as i32 + step as i32).clamp(0, 100) as u32;
        self.brightness_set(&connector, next);
        let req = Request {
            app: "Display".into(),
            replaces: self.brightness.note,
            summary: format!("Brightness {next}%"),
            body: self.screen_label(&connector),
            urgency: 0,
            timeout_ms: SHOWN_MS,
            actions: Vec::new(),
        };
        self.brightness.note = self.notify(Origin::Server, req);
    }

    /// The Display tile from the levels: a slider for each screen with a
    /// control, none at all on a machine without one.
    pub fn brightness_tile(&mut self) {
        self.brightness_keep();
        let levels = self.brightness.levels.clone();
        let Some((_, first)) = levels.first() else {
            self.tile_remove(Owner::Desktop, TILE);
            return;
        };
        let controls: Vec<Control> = levels
            .iter()
            .map(|(connector, level)| Control {
                id: connector.clone(),
                label: self.screen_label(connector),
                kind: ControlKind::Slider { value: *level as f64, min: 0.0, max: 100.0, step: 5.0 },
            })
            .collect();
        let tile = Tile {
            id: TILE.into(),
            value: format!("{first}%"),
            label: "Display".into(),
            group: "System".into(),
            card: Card { title: "Brightness".into(), controls, ..Card::default() },
            page: "screens".into(),
            ..Tile::default()
        };
        self.tile_set(Owner::Desktop, tile);
    }
}
