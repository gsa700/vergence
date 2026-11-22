// The volume keys (sound milestone 1, step 4, docs/sound-milestone-1.md).
//
// Volume up and down change the default output's volume by 5% a press (Dave,
// 2026-11-11), to the nearest multiple of 5; up also unmutes. Mute turns it
// off or on. The new level shows for a moment as a notification of the
// server's own, which each press replaces. Through vergence-sound, as the
// settings app's Sound page does; where PipeWire has the sound the keys do
// nothing yet (decided the same day).

use crate::{
    Server,
    notify::{Origin, Request},
};

/// How long the level stays on screen.
const SHOWN_MS: i32 = 1500;

/// A volume a step from `now`: to the nearest multiple of 5, within 0 to 100.
pub fn stepped(now: u8, step: i8) -> u8 {
    let n = now as i32;
    let next = if step > 0 {
        (n / 5 + 1) * 5
    } else if n % 5 == 0 {
        n - 5
    } else {
        n / 5 * 5
    };
    next.clamp(0, 100) as u8
}

impl Server {
    /// A volume key: `step` +5, -5, or 0 for mute.
    pub fn volume_key(&mut self, step: i8) {
        let Some(mut sound) = vergence_sound::open() else {
            return tracing::debug!("volume key: no sound system of ours here");
        };
        let Some(out) = sound.outputs().ok().and_then(|o| o.into_iter().find(|o| o.default)) else {
            return tracing::debug!("volume key: no default output");
        };
        let result = match step {
            0 => sound.set_muted(&out.id, !out.muted),
            _ if !out.adjustable => Ok(()),
            s => sound
                .set_volume(&out.id, stepped(out.volume, s))
                .and_then(|_| if s > 0 && out.muted { sound.set_muted(&out.id, false) } else { Ok(()) }),
        };
        if let Err(e) = result {
            return tracing::warn!("volume key on {}: {e}", out.id);
        }
        let now = sound.outputs().ok().and_then(|o| o.into_iter().find(|o| o.id == out.id)).unwrap_or(out);
        let summary = if now.muted {
            "Muted".to_string()
        } else if now.adjustable {
            format!("Volume {}%", now.volume)
        } else {
            "Volume is set on the device".to_string()
        };
        let req = Request {
            app: "Sound".into(),
            replaces: self.volume_note,
            summary,
            body: now.name.clone(),
            urgency: 0,
            timeout_ms: SHOWN_MS,
            actions: Vec::new(),
        };
        self.volume_note = self.notify(Origin::Server, req);
        // The Sound tile says so at once (applets.rs).
        self.sound_tile_now();
    }
}

#[cfg(test)]
mod tests {
    use super::stepped;

    #[test]
    fn a_press_moves_five_to_the_nearest_five() {
        assert_eq!((stepped(50, 5), stepped(50, -5)), (55, 45));
        assert_eq!((stepped(69, 5), stepped(69, -5)), (70, 65), "off the grid: to the next five either way");
        assert_eq!((stepped(100, 5), stepped(0, -5), stepped(3, -5)), (100, 0, 0));
        assert_eq!(stepped(97, 5), 100);
    }
}
