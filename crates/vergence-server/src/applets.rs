//! The desktop's own live tiles besides the clock (dock milestone 1, step
//! 4): Sound, Network and Power. What they show is read on a thread of its
//! own (sound systems, network counters and batteries are devices and
//! files, never read during a frame) and handed to the main loop, which
//! puts the tiles up. The network is read every second while its card is
//! open and every two seconds otherwise; sound and power every four.
//! A card's controls on the desktop's tiles are carried out here.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use smithay::reexports::calloop::channel::{self, Event};
use vergence_protocol::tile::Tile;
use vergence_shell::tiles;

use crate::{Server, tiles::Owner};

/// What the reading thread found.
pub enum Found {
    Set(Box<Tile>),
    Gone(&'static str),
}

/// How many seconds of what came in the network card's chart shows.
const HISTORY: usize = 60;

/// Whether the network card is open, so its figures come every second.
#[derive(Default, Clone)]
pub struct Live(Arc<AtomicBool>);

impl Live {
    pub fn set(&self, on: bool) {
        self.0.store(on, Ordering::Relaxed);
    }
}

/// The default output now, from vergence-sound.
fn sound_now() -> Option<tiles::SoundNow> {
    let mut s = vergence_sound::open()?;
    let o = s.outputs().ok()?.into_iter().find(|o| o.default)?;
    Some(tiles::SoundNow { name: o.name, volume: o.volume, muted: o.muted, adjustable: o.adjustable })
}

fn read(tx: channel::Sender<Found>, live: Live) {
    let mut last: Option<(Instant, String, u64, u64)> = None;
    let mut history: Vec<f32> = Vec::new();
    let mut tick = 0u32;
    let (mut sound_was, mut power_was) = (None::<Tile>, None::<Tile>);
    loop {
        // The network: the rate in and out of the main interface since the last look.
        let ifaces = vergence_net::ifaces::list().unwrap_or_default();
        let now = Instant::now();
        let (mut rate_in, mut rate_out) = (0.0, 0.0);
        if let Some(f) = tiles::main_iface(&ifaces) {
            let (rx, txb) = (f.rx_bytes.unwrap_or(0), f.tx_bytes.unwrap_or(0));
            if let Some((then, name, rx0, tx0)) = &last
                && *name == f.name
            {
                let secs = now.duration_since(*then).as_secs_f64().max(0.001);
                rate_in = vergence_net::ifaces::delta(*rx0, rx) as f64 / secs;
                rate_out = vergence_net::ifaces::delta(*tx0, txb) as f64 / secs;
            }
            last = Some((now, f.name.clone(), rx, txb));
        } else {
            last = None;
        }
        history.push(rate_in as f32);
        if history.len() > HISTORY {
            history.remove(0);
        }
        if tx.send(Found::Set(Box::new(tiles::network(&ifaces, rate_in, rate_out, &history)))).is_err() {
            return;
        }
        // Sound and power less often, and only when they changed.
        if tick.is_multiple_of(4) {
            let sound = tiles::sound(sound_now().as_ref());
            if sound != sound_was {
                let msg = sound.clone().map_or(Found::Gone("sound"), |t| Found::Set(Box::new(t)));
                if tx.send(msg).is_err() {
                    return;
                }
                sound_was = sound;
            }
            let power = tiles::power(&vergence_power::power());
            if power != power_was {
                let msg = power.clone().map_or(Found::Gone("power"), |t| Found::Set(Box::new(t)));
                if tx.send(msg).is_err() {
                    return;
                }
                power_was = power;
            }
        }
        tick = tick.wrapping_add(1);
        std::thread::sleep(if live.0.load(Ordering::Relaxed) { Duration::from_secs(1) } else { Duration::from_secs(2) });
    }
}

impl Server {
    /// Start reading for the desktop's tiles.
    pub fn start_applets(&mut self) {
        let (tx, rx) = channel::channel::<Found>();
        let res = self.loop_handle.insert_source(rx, |event, _, state| {
            if let Event::Msg(found) = event {
                match found {
                    Found::Set(tile) => state.tile_set(Owner::Desktop, *tile),
                    Found::Gone(id) => {
                        state.tile_remove(Owner::Desktop, id);
                    }
                }
            }
        });
        if let Err(e) = res {
            return tracing::warn!("applets: {e}");
        }
        let live = self.tiles.live.clone();
        if let Err(e) = std::thread::Builder::new().name("applets".into()).spawn(move || read(tx, live)) {
            tracing::warn!("applets: can't start reading: {e}");
        }
    }

    /// A control on one of the desktop's own cards was used.
    pub fn desktop_tile_action(&mut self, tile: &str, control: &str, value: f64) {
        match (tile, control) {
            ("sound", "volume" | "mute") => {
                let Some(mut sound) = vergence_sound::open() else { return };
                let Some(out) = sound.outputs().ok().and_then(|o| o.into_iter().find(|o| o.default)) else { return };
                let result = if control == "volume" {
                    // Turning it up takes the mute off, as the volume keys do.
                    let v = value.round().clamp(0.0, 100.0) as u8;
                    sound.set_volume(&out.id, v).and_then(|_| if out.muted && v > out.volume { sound.set_muted(&out.id, false) } else { Ok(()) })
                } else {
                    sound.set_muted(&out.id, value >= 0.5)
                };
                if let Err(e) = result {
                    tracing::warn!("applets: sound on {}: {e}", out.id);
                }
                self.sound_tile_now();
            }
            // An agent's timed grant, one more span (agents.rs).
            ("agent", "extend") => self.agent_extend(),
            _ => tracing::debug!("applets: {tile} has no control {control}"),
        }
    }

    /// The Sound tile at once, after the volume changed here (a card's
    /// control, a volume key), not on the thread's next look.
    pub fn sound_tile_now(&mut self) {
        if let Some(t) = tiles::sound(sound_now().as_ref()) {
            self.tile_set(Owner::Desktop, t);
        }
    }
}
