//! A sound file in the preview (preview milestone 1, step 6): Hymie's helper
//! (vergence-hymie, confined) parses and decodes it; this worker asks it for
//! the file's facts, tags and cover, then the waveform a piece at a time, and
//! plays it to the default output through vergence-sound when asked, the
//! samples converted to the output's rate and channels. The window reads the
//! shared [`State`] and adds [`Ask`]s; the worker wakes it as things come.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use vergence_hymie::Peak;
use vergence_hymie::helper::{Info, Sound};
use vergence_scene::Image;
use vergence_widgets::Waker;

/// What the window asks of the worker.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ask {
    Play,
    Pause,
    /// Play from this frame (the waveform clicked).
    Seek(u64),
    Stop,
}

#[derive(Default)]
pub struct State {
    pub info: Option<Info>,
    pub cover: Option<Arc<Image>>,
    /// The waveform so far, of `slices` slices.
    pub peaks: Vec<Peak>,
    pub error: Option<String>,
    /// The frame playing (the cursor), and whether it plays.
    pub at: u64,
    pub playing: bool,
    asks: Vec<Ask>,
}

/// The worker for one file: dropped, it stops (and the helper with it).
pub struct Listen {
    pub state: Arc<Mutex<State>>,
    nudge: Arc<Condvar>,
}

impl Drop for Listen {
    fn drop(&mut self) {
        self.ask(Ask::Stop);
    }
}

impl Listen {
    pub fn start(path: PathBuf, slices: u32, waker: Option<Waker>) -> Listen {
        let state: Arc<Mutex<State>> = Arc::default();
        let nudge = Arc::new(Condvar::new());
        let (s, n) = (state.clone(), nudge.clone());
        std::thread::spawn(move || work(path, slices, s, n, waker));
        Listen { state, nudge }
    }

    pub fn ask(&self, a: Ask) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).asks.push(a);
        self.nudge.notify_all();
    }
}

fn work(path: PathBuf, slices: u32, state: Arc<Mutex<State>>, nudge: Arc<Condvar>, waker: Option<Waker>) {
    let wake = || {
        if let Some(w) = &waker {
            w.wake();
        }
    };
    let mut sound = match Sound::open(&path) {
        Ok(s) => s,
        Err(e) => {
            state.lock().unwrap_or_else(|e| e.into_inner()).error = Some(e.to_string());
            return wake();
        }
    };
    {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.cover = sound.info.cover.take().map(|(w, h, rgba)| Arc::new(Image { width: w, height: h, rgba }));
        s.info = Some(sound.info.clone());
    }
    wake();
    // No picture in the file: the album's beside it (Deadwax never embeds;
    // AlbumWall looks the same way), decoded by the picture helper.
    if state.lock().unwrap_or_else(|e| e.into_inner()).cover.is_none()
        && let Some(side) = path.parent().and_then(sidecar)
        && let Ok(p) = vergence_decode::decode_file(
            &side,
            Some((vergence_hymie::helper::COVER_PX, vergence_hymie::helper::COVER_PX)),
        )
        && let Some(f) = p.frames.into_iter().next()
    {
        state.lock().unwrap_or_else(|e| e.into_inner()).cover =
            Some(Arc::new(Image { width: p.width, height: p.height, rgba: f.rgba }));
        wake();
    }
    let info = sound.info.clone();
    let mut out: Option<(Box<dyn vergence_sound::Stream>, Resample)> = None;
    let mut whole = false;
    let mut since_wake = 0u64;
    loop {
        let (asks, playing) = {
            let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
            (std::mem::take(&mut s.asks), s.playing)
        };
        let mut playing = playing;
        for a in asks {
            match a {
                Ask::Stop => return,
                Ask::Pause => {
                    playing = false;
                    out = None;
                }
                Ask::Play => {
                    // At the end, Play starts again from the top.
                    let at = state.lock().unwrap_or_else(|e| e.into_inner()).at;
                    if at >= info.frames {
                        let _ = sound.seek(0);
                        state.lock().unwrap_or_else(|e| e.into_inner()).at = 0;
                    }
                    playing = true;
                }
                Ask::Seek(f) => {
                    let f = f.min(info.frames);
                    if sound.seek(f).is_ok() {
                        state.lock().unwrap_or_else(|e| e.into_inner()).at = f;
                    }
                }
            }
        }
        if playing && out.is_none() {
            match vergence_sound::stream(info.rate, info.channels as u32) {
                Ok(st) => {
                    let r = Resample::new(info.rate, info.channels as usize, st.rate(), st.channels() as usize);
                    out = Some((st, r));
                }
                Err(e) => {
                    playing = false;
                    state.lock().unwrap_or_else(|e| e.into_inner()).error = Some(format!("No sound output: {e}"));
                    wake();
                }
            }
        }
        state.lock().unwrap_or_else(|e| e.into_inner()).playing = playing;
        if playing && let Some((st, r)) = out.as_mut() {
            // A tenth of a second at a time: the write waits while the output is full.
            let samples = match sound.read((info.rate / 10).max(256)) {
                Ok(s) => s,
                Err(e) => {
                    state.lock().unwrap_or_else(|e| e.into_inner()).error = Some(e.to_string());
                    out = None;
                    wake();
                    continue;
                }
            };
            let frames = (samples.len() / info.channels.max(1) as usize) as u64;
            if frames == 0 {
                // The end: stopped there, Play starts again.
                out = None;
                let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                s.playing = false;
                s.at = info.frames;
                drop(s);
                wake();
                continue;
            }
            if st.write(&r.convert(&samples)).is_err() {
                out = None;
            }
            state.lock().unwrap_or_else(|e| e.into_inner()).at += frames;
            // The waveform goes on filling in while it plays, a small piece
            // between pieces of sound (otherwise it stays invisible until
            // Pause): about three seconds' worth, decoded well inside
            // the output's buffer.
            if !whole {
                match sound.peaks_step(slices, 131_072) {
                    Ok((_, p)) if p.is_empty() => whole = true,
                    Ok((_, p)) => state.lock().unwrap_or_else(|e| e.into_inner()).peaks.extend(p),
                    Err(_) => whole = true,
                }
            }
            since_wake += frames;
            if since_wake >= info.rate as u64 / 10 {
                since_wake = 0;
                wake();
            }
            continue;
        }
        if !whole {
            match sound.peaks(slices) {
                Ok((_, p)) if p.is_empty() => whole = true,
                Ok((_, p)) => {
                    state.lock().unwrap_or_else(|e| e.into_inner()).peaks.extend(p);
                    wake();
                }
                Err(e) => {
                    whole = true;
                    state.lock().unwrap_or_else(|e| e.into_inner()).error = Some(e.to_string());
                    wake();
                }
            }
            continue;
        }
        // Nothing to do: wait to be asked.
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        if s.asks.is_empty() {
            let _ = nudge.wait_timeout(s, Duration::from_millis(500));
        }
    }
}

/// Samples converted to the output: its channels (the first ones kept, a
/// mono source given to all, stereo mixed down for a mono output) and its
/// rate (linear between neighbors, the phase carried from one piece to the next).
struct Resample {
    from_rate: u32,
    from_ch: usize,
    to_rate: u32,
    to_ch: usize,
    /// Where the next output frame falls in the input, from the last frame of the piece before.
    phase: f64,
    last: Vec<f32>,
}

impl Resample {
    fn new(from_rate: u32, from_ch: usize, to_rate: u32, to_ch: usize) -> Resample {
        Resample {
            from_rate,
            from_ch: from_ch.max(1),
            to_rate,
            to_ch: to_ch.max(1),
            phase: 0.0,
            last: vec![0.0; from_ch.max(1)],
        }
    }

    fn channel(&self, frame: &[f32], c: usize) -> f32 {
        match (self.from_ch, self.to_ch) {
            (1, _) => frame[0],
            (_, 1) => frame.iter().take(2).sum::<f32>() / frame.len().min(2) as f32,
            _ if c < self.from_ch => frame[c],
            _ => 0.0,
        }
    }

    fn convert(&mut self, input: &[f32]) -> Vec<i16> {
        let n = input.len() / self.from_ch;
        let to_i16 = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        let mut out = Vec::new();
        if self.from_rate == self.to_rate {
            for f in 0..n {
                let frame = &input[f * self.from_ch..(f + 1) * self.from_ch];
                for c in 0..self.to_ch {
                    out.push(to_i16(self.channel(frame, c)));
                }
            }
            return out;
        }
        let step = self.from_rate as f64 / self.to_rate as f64;
        // Input frame -1 is the last of the piece before.
        let frame_at = |i: isize, last: &[f32]| -> Vec<f32> {
            if i < 0 {
                last.to_vec()
            } else {
                input[i as usize * self.from_ch..(i as usize + 1) * self.from_ch].to_vec()
            }
        };
        let mut p = self.phase;
        while p < n as f64 - 1.0 {
            let i = p.floor() as isize;
            let t = (p - i as f64) as f32;
            let (a, b) = (frame_at(i, &self.last), frame_at(i + 1, &self.last));
            for c in 0..self.to_ch {
                out.push(to_i16(self.channel(&a, c) * (1.0 - t) + self.channel(&b, c) * t));
            }
            p += step;
        }
        self.phase = p - n as f64;
        if n > 0 {
            self.last = input[(n - 1) * self.from_ch..n * self.from_ch].to_vec();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Resample;

    #[test]
    fn samples_meet_the_output_rate_and_channels() {
        // The same rate: only the channels change, mono given to both.
        let mut same = Resample::new(48_000, 1, 48_000, 2);
        assert_eq!(same.convert(&[0.5, -0.5]), vec![16384, 16384, -16384, -16384]);
        // 44.1 to 48 kHz: about 48/44.1 as many frames, across pieces.
        let mut up = Resample::new(44_100, 2, 48_000, 2);
        let piece: Vec<f32> = (0..4410 * 2).map(|i| ((i / 2) as f32 / 4410.0) * 0.5).collect();
        let total: usize = (0..10).map(|_| up.convert(&piece).len() / 2).sum();
        assert!((47_950..=48_050).contains(&total), "{total}");
        // Stereo to a mono output: the two mixed.
        let mut down = Resample::new(8000, 2, 8000, 1);
        assert_eq!(down.convert(&[1.0, 0.0]), vec![16384]);
    }
}

/// An album's cover beside its files, as AlbumWall finds it (LibraryScanner's
/// ArtNames): the first of these names present, whatever its case.
fn sidecar(dir: &std::path::Path) -> Option<PathBuf> {
    const NAMES: [&str; 6] = ["cover.jpg", "cover.jpeg", "cover.png", "folder.jpg", "folder.jpeg", "front.jpg"];
    let present: Vec<PathBuf> =
        std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    NAMES.iter().find_map(|n| {
        present.iter().find(|p| p.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(n))).cloned()
    })
}
