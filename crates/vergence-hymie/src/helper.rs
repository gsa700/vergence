//! The sound helper (libexec/vergence/vergence-hymie) and the means to talk
//! to it: a file's bytes are parsed only there, confined as the picture
//! helper is (vergence-decode's `confine`: Capsicum on FreeBSD, seccomp on
//! Linux, every other descriptor closed, its limits lowered), never in the
//! program that shows the sound (whitepaper §6.10).
//!
//! On the pipes, as vergence-pdf's helper: a kind byte, a big-endian u32
//! length, the body. Requests:
//! - `D` the file's bytes: the answer is [`Info`] (its facts, tags, cover).
//! - `P` slices (u32), and the most frames to decode this time (u32,
//!   optional; about a million when left out: no request runs long; a
//!   player asks for less between pieces of sound): the first slice done
//!   (u32), how many (u32), then each slice's four floats (min and max of
//!   two channels); none when the waveform is whole.
//! - `S` a frame (u64): the playing cursor put there.
//! - `R` frames (u32): the next samples from the cursor, interleaved floats.
//!
//! An answer of kind 0 is the answer; kind 1 a refusal, its body why; kind 2
//! (to `D` only) "AAC: ask the AAC helper", which [`Sound`] answers by
//! starting libexec/vergence/vergence-hymie-aac and sending it the bytes
//! instead. That helper is the same program over a GPL decoder (FAAD2's, in
//! crates/vergence-hymie-aac), kept out of this dual-licensed crate: it runs
//! [`serve_with`] with an open that plays AAC. The waveform and the playing
//! have a track each, so drawing one never moves the other.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{Error, ReplayGain, Track};

/// How long the helper has for one request.
pub const TIMEOUT: Duration = Duration::from_secs(10);
/// Its processor time in all: a long track's waveform and its playing.
const CPU_SECONDS: u64 = 600;
/// Frames decoded for the waveform per request.
const PEAK_STEP: u64 = 1 << 20;
/// The most frames read at once.
const READ_MAX: u32 = 1 << 16;

/// The size the cover is shrunk to (never enlarged), each side at least this.
pub const COVER_PX: u32 = 600;

/// What the helper says of a file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Info {
    pub rate: u32,
    pub channels: u16,
    pub bits: u16,
    pub frames: u64,
    pub gain: ReplayGain,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// The cover, decoded here in the helper (vergence-decode's decoders, so
    /// the app never parses its bytes) and shrunk to about [`COVER_PX`]:
    /// width, height, straight RGBA.
    pub cover: Option<(u32, u32, Vec<u8>)>,
}

impl Info {
    pub fn seconds(&self) -> f64 {
        if self.rate == 0 { 0.0 } else { self.frames as f64 / self.rate as f64 }
    }

    fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&self.rate.to_le_bytes());
        b.extend_from_slice(&self.channels.to_le_bytes());
        b.extend_from_slice(&self.bits.to_le_bytes());
        b.extend_from_slice(&self.frames.to_le_bytes());
        b.push(self.gain.present as u8);
        for v in [self.gain.track_gain, self.gain.track_peak, self.gain.album_gain, self.gain.album_peak] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let text = |b: &mut Vec<u8>, s: &[u8]| {
            b.extend_from_slice(&(s.len() as u32).to_le_bytes());
            b.extend_from_slice(s);
        };
        for s in [&self.title, &self.artist, &self.album] {
            text(&mut b, s.as_deref().unwrap_or("").as_bytes());
        }
        let (w, h, rgba) = self.cover.as_ref().map(|(w, h, d)| (*w, *h, d.as_slice())).unwrap_or((0, 0, b""));
        b.extend_from_slice(&w.to_le_bytes());
        b.extend_from_slice(&h.to_le_bytes());
        text(&mut b, rgba);
        b
    }

    fn decode(b: &[u8]) -> Option<Info> {
        let mut at = 0usize;
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = b.get(at..at + n)?;
            at += n;
            Some(s)
        };
        let rate = u32::from_le_bytes(take(4)?.try_into().ok()?);
        let channels = u16::from_le_bytes(take(2)?.try_into().ok()?);
        let bits = u16::from_le_bytes(take(2)?.try_into().ok()?);
        let frames = u64::from_le_bytes(take(8)?.try_into().ok()?);
        let present = take(1)?[0] != 0;
        let mut f = [0f32; 4];
        for v in &mut f {
            *v = f32::from_le_bytes(take(4)?.try_into().ok()?);
        }
        let mut text = || -> Option<Vec<u8>> {
            let n = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
            Some(take(n)?.to_vec())
        };
        let word = |v: Vec<u8>| Some(String::from_utf8_lossy(&v).into_owned()).filter(|s| !s.is_empty());
        let (title, artist, album) = (word(text()?), word(text()?), word(text()?));
        drop(text);
        let w = u32::from_le_bytes(take(4)?.try_into().ok()?);
        let h = u32::from_le_bytes(take(4)?.try_into().ok()?);
        let n = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
        let data = take(n)?.to_vec();
        if !data.is_empty() && data.len() != w as usize * h as usize * 4 {
            return None;
        }
        Some(Info {
            rate,
            channels,
            bits,
            frames,
            gain: ReplayGain { present, track_gain: f[0], track_peak: f[1], album_gain: f[2], album_peak: f[3] },
            title,
            artist,
            album,
            cover: (!data.is_empty()).then_some((w, h, data)),
        })
    }
}

fn read_frame(r: &mut impl Read, limit: usize) -> std::io::Result<Option<(u8, Vec<u8>)>> {
    let mut head = [0u8; 5];
    match r.read_exact(&mut head) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > limit {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "a frame too large"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(Some((head[0], body)))
}

fn write_frame(w: &mut impl Write, kind: u8, body: &[u8]) -> std::io::Result<()> {
    w.write_all(&[kind])?;
    w.write_all(&(body.len() as u32).to_be_bytes())?;
    w.write_all(body)?;
    w.flush()
}

/// The answer kind that says "AAC: ask the AAC helper".
const NEEDS_AAC: u8 = 2;

/// The helper's work: requests from `input`, answers to `output`, until the pipe closes.
pub fn serve(input: &mut impl Read, output: &mut impl Write) -> std::io::Result<()> {
    serve_with(input, output, crate::open)
}

/// As [`serve`], with `open` making each track from a file's bytes (the AAC
/// helper's plays AAC too).
pub fn serve_with(input: &mut impl Read, output: &mut impl Write, open_track: impl Fn(Vec<u8>) -> Result<Box<dyn Track>, Error>) -> std::io::Result<()> {
    struct Open {
        playing: Box<dyn Track>,
        drawing: Box<dyn Track>,
        peaks: Vec<crate::Peak>,
        drawn: u64,
        told: usize,
    }
    let mut open: Option<Open> = None;
    while let Some((kind, body)) = read_frame(input, crate::MAX_FILE as usize + 16)? {
        let mut refusal = 1u8;
        let answer: Result<Vec<u8>, String> = match kind {
            b'D' => (|| -> Result<Vec<u8>, Error> {
                let tags = crate::tags(&body);
                let drawing = open_track(body.clone())?;
                let playing = open_track(body)?;
                let info = Info {
                    rate: playing.rate(),
                    channels: playing.channels(),
                    bits: playing.bits(),
                    frames: playing.frames(),
                    gain: playing.gain(),
                    title: tags.title,
                    artist: tags.artist,
                    album: tags.album,
                    cover: tags.picture.and_then(|p| vergence_decode::decode_bytes(&p.data, Some((COVER_PX, COVER_PX))).ok()).and_then(|pic| Some((pic.width, pic.height, pic.frames.into_iter().next()?.rgba))),
                };
                open = Some(Open { playing, drawing, peaks: Vec::new(), drawn: 0, told: 0 });
                Ok(info.encode())
            })()
            .map_err(|e| {
                // Told apart from a refusal by its own kind, so the client
                // need not read the words.
                if e == Error::NeedsAac {
                    refusal = NEEDS_AAC;
                }
                e.to_string()
            }),
            b'P' => (|| {
                let o = open.as_mut().ok_or("no file")?;
                let slices = u32::from_le_bytes(body.get(..4).ok_or("short")?.try_into().map_err(|_| "short")?).clamp(1, 1 << 16) as usize;
                let step = body.get(4..8).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64).map_or(PEAK_STEP, |s| s.clamp(4096, PEAK_STEP));
                let frames = o.drawing.frames().max(1);
                if o.peaks.len() != slices {
                    o.peaks = vec![[(0.0, 0.0); 2]; slices];
                    o.drawn = 0;
                    o.told = 0;
                    o.drawing.seek(0).map_err(|e| e.to_string())?;
                }
                let ch = o.drawing.channels() as usize;
                let mut buf = vec![0f32; 4096 * ch];
                let until = (o.drawn + step).min(frames);
                while o.drawn < until {
                    let n = o.drawing.read(&mut buf).map_err(|e| e.to_string())?;
                    if n == 0 {
                        o.drawn = frames;
                        break;
                    }
                    for f in 0..n {
                        let s = ((((o.drawn + f as u64) as u128) * slices as u128 / frames as u128) as usize).min(slices - 1);
                        for c in 0..ch.min(2) {
                            let v = buf[f * ch + c];
                            let p = &mut o.peaks[s][c];
                            p.0 = p.0.min(v);
                            p.1 = p.1.max(v);
                        }
                        if ch == 1 {
                            o.peaks[s][1] = o.peaks[s][0];
                        }
                    }
                    o.drawn += n as u64;
                }
                // The slices now whole (the last only at the end).
                let done = if o.drawn >= frames { slices } else { ((o.drawn as u128 * slices as u128 / frames as u128) as usize).min(slices) };
                let mut b = Vec::new();
                b.extend_from_slice(&(o.told as u32).to_le_bytes());
                b.extend_from_slice(&((done - o.told) as u32).to_le_bytes());
                for p in &o.peaks[o.told..done] {
                    for (lo, hi) in p {
                        b.extend_from_slice(&lo.to_le_bytes());
                        b.extend_from_slice(&hi.to_le_bytes());
                    }
                }
                o.told = done;
                Ok(b)
            })(),
            b'S' => (|| {
                let o = open.as_mut().ok_or("no file")?;
                let to = u64::from_le_bytes(body.get(..8).ok_or("short")?.try_into().map_err(|_| "short")?);
                o.playing.seek(to).map_err(|e| e.to_string())?;
                Ok(Vec::new())
            })(),
            b'R' => (|| {
                let o = open.as_mut().ok_or("no file")?;
                let n = u32::from_le_bytes(body.get(..4).ok_or("short")?.try_into().map_err(|_| "short")?).min(READ_MAX) as usize;
                let mut buf = vec![0f32; n * o.playing.channels() as usize];
                let got = o.playing.read(&mut buf).map_err(|e| e.to_string())?;
                Ok(buf[..got * o.playing.channels() as usize].iter().flat_map(|v| v.to_le_bytes()).collect())
            })(),
            _ => Err("an unknown request".into()),
        };
        match answer {
            Ok(b) => write_frame(output, 0, &b)?,
            Err(why) => write_frame(output, refusal, why.as_bytes())?,
        }
    }
    Ok(())
}

/// The helper's whole job: confined, then serving.
pub fn helper_main() -> std::process::ExitCode {
    helper_main_with(crate::open)
}

/// As [`helper_main`], with `open` making each track (the AAC helper's).
pub fn helper_main_with(open_track: impl Fn(Vec<u8>) -> Result<Box<dyn Track>, Error>) -> std::process::ExitCode {
    vergence_decode::confine(CPU_SECONDS);
    let (mut i, mut o) = (std::io::stdin().lock(), std::io::stdout().lock());
    match serve_with(&mut i, &mut o, open_track) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => std::process::ExitCode::from(1),
    }
}

/// The helper: beside this program (a checkout's target/), else
/// PREFIX/libexec/vergence, else /usr/local's.
fn helper_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("vergence-hymie"), dir.parent()?.join("libexec/vergence/vergence-hymie"), PathBuf::from("/usr/local/libexec/vergence/vergence-hymie")].into_iter().find(|p| p.is_file())
}

/// The AAC helper: beside the main helper that sent the file on (a
/// checkout's target/, or the same libexec), else where the main one is
/// looked for.
fn aac_helper_path(main: &Path) -> Option<PathBuf> {
    let beside = main.with_file_name("vergence-hymie-aac");
    if beside.is_file() {
        return Some(beside);
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("vergence-hymie-aac"), dir.parent()?.join("libexec/vergence/vergence-hymie-aac"), PathBuf::from("/usr/local/libexec/vergence/vergence-hymie-aac")].into_iter().find(|p| p.is_file())
}

/// A sound file open in its helper.
pub struct Sound {
    child: std::process::Child,
    to: std::process::ChildStdin,
    from: std::process::ChildStdout,
    pub info: Info,
}

impl Drop for Sound {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Sound {
    /// A sound file opened in a new helper.
    pub fn open(path: &Path) -> Result<Sound, Error> {
        let program = helper_path().ok_or_else(|| Error::Io("the sound helper (libexec/vergence/vergence-hymie) is not installed".into()))?;
        Sound::with_bytes(&program, crate::read_file(path)?)
    }

    /// A file's bytes in the helper at `program` (tests use the one cargo
    /// built). An AAC file that helper sends on goes to the AAC helper,
    /// beside it or installed; the app never parses the file itself.
    pub fn with_bytes(program: &Path, data: Vec<u8>) -> Result<Sound, Error> {
        match Sound::start(program, &data) {
            Err(Error::NeedsAac) => {
                let aac = aac_helper_path(program).ok_or_else(|| Error::Io("AAC is played by its own helper (libexec/vergence/vergence-hymie-aac), which is not installed".into()))?;
                Sound::start(&aac, &data)
            }
            other => other,
        }
    }

    fn start(program: &Path, data: &[u8]) -> Result<Sound, Error> {
        let mut child = std::process::Command::new(program).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().map_err(|e| Error::Io(e.to_string()))?;
        let (Some(to), Some(from)) = (child.stdin.take(), child.stdout.take()) else { return Err(Error::Io("no pipe".into())) };
        let mut s = Sound { child, to, from, info: Info::default() };
        s.info = Info::decode(&s.ask(b'D', data)?).ok_or_else(|| crate::damaged("a garbled answer"))?;
        Ok(s)
    }

    /// One request and its answer, timed: past the time the helper is stopped.
    fn ask(&mut self, op: u8, body: &[u8]) -> Result<Vec<u8>, Error> {
        let pid = self.child.id();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let watchman = std::thread::spawn(move || {
            if done_rx.recv_timeout(TIMEOUT) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                unsafe extern "C" {
                    fn kill(pid: i32, sig: i32) -> i32;
                }
                // SAFETY: kill(2) on our own child; no memory passed.
                unsafe { kill(pid as i32, 9) };
            }
        });
        let result = write_frame(&mut self.to, op, body).and_then(|_| read_frame(&mut self.from, 64 << 20));
        let _ = done_tx.send(());
        let _ = watchman.join();
        match result {
            Ok(Some((0, b))) => Ok(b),
            Ok(Some((NEEDS_AAC, _))) if op == b'D' => Err(Error::NeedsAac),
            Ok(Some((_, why))) => Err(crate::damaged(String::from_utf8_lossy(&why).into_owned())),
            _ => Err(Error::Io("the sound helper stopped (a limit, or a fault in the file)".into())),
        }
    }

    /// The waveform's next piece for `slices` slices: (the first slice, the
    /// slices); an empty piece when the waveform is whole.
    pub fn peaks(&mut self, slices: u32) -> Result<(usize, Vec<crate::Peak>), Error> {
        self.peaks_step(slices, PEAK_STEP as u32)
    }

    /// As [`Sound::peaks`], decoding at most `frames` frames this time (a
    /// player's small steps between pieces of sound).
    pub fn peaks_step(&mut self, slices: u32, frames: u32) -> Result<(usize, Vec<crate::Peak>), Error> {
        let mut body = slices.to_le_bytes().to_vec();
        body.extend_from_slice(&frames.to_le_bytes());
        let b = self.ask(b'P', &body)?;
        let first = u32::from_le_bytes(b.get(..4).ok_or_else(|| crate::damaged("short"))?.try_into().unwrap_or([0; 4])) as usize;
        let v: Vec<f32> = b.get(8..).unwrap_or(&[]).as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
        Ok((first, v.as_chunks::<4>().0.iter().map(|c| [(c[0], c[1]), (c[2], c[3])]).collect()))
    }

    pub fn seek(&mut self, frame: u64) -> Result<(), Error> {
        self.ask(b'S', &frame.to_le_bytes()).map(|_| ())
    }

    /// Up to `frames` frames from the playing cursor, interleaved.
    pub fn read(&mut self, frames: u32) -> Result<Vec<f32>, Error> {
        let b = self.ask(b'R', &frames.to_le_bytes())?;
        Ok(b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_helper_serves_info_peaks_and_samples() {
        let mut req = Vec::new();
        super::write_frame(&mut req, b'D', include_bytes!("../fixtures/tone16.flac")).unwrap();
        super::write_frame(&mut req, b'P', &100u32.to_le_bytes()).unwrap();
        super::write_frame(&mut req, b'S', &1000u64.to_le_bytes()).unwrap();
        super::write_frame(&mut req, b'R', &10u32.to_le_bytes()).unwrap();
        let mut out = Vec::new();
        super::serve(&mut req.as_slice(), &mut out).unwrap();
        let mut r = out.as_slice();
        let (k, info) = super::read_frame(&mut r, 1 << 26).unwrap().unwrap();
        assert_eq!(k, 0);
        let info = super::Info::decode(&info).unwrap();
        assert_eq!((info.rate, info.channels, info.frames), (44_100, 2, 66_150));
        assert_eq!(info.title.as_deref(), Some("Test Tone"));
        assert!(info.cover.is_some_and(|(w, h, d)| (w, h) == (64, 64) && d.len() == 64 * 64 * 4 && d[2] > d[0] + 100 && d[3] == 255), "the blue cover, decoded");
        let (_, peaks) = super::read_frame(&mut r, 1 << 26).unwrap().unwrap();
        assert_eq!(u32::from_le_bytes(peaks[4..8].try_into().unwrap()), 100, "a short track's waveform in one piece");
        let (k, _) = super::read_frame(&mut r, 1 << 26).unwrap().unwrap();
        assert_eq!(k, 0);
        let (_, samples) = super::read_frame(&mut r, 1 << 26).unwrap().unwrap();
        assert_eq!(samples.len(), 10 * 2 * 4);
    }

    #[test]
    fn an_aac_file_is_sent_on_to_the_aac_helper() {
        // Answered with its own kind, which the client takes for "start the
        // AAC helper", and nothing is left open to play.
        let mut req = Vec::new();
        super::write_frame(&mut req, b'D', include_bytes!("../fixtures/aac.m4a")).unwrap();
        super::write_frame(&mut req, b'R', &10u32.to_le_bytes()).unwrap();
        let mut out = Vec::new();
        super::serve(&mut req.as_slice(), &mut out).unwrap();
        let mut r = out.as_slice();
        let (k, why) = super::read_frame(&mut r, 1 << 26).unwrap().unwrap();
        assert_eq!(k, super::NEEDS_AAC, "{}", String::from_utf8_lossy(&why));
        let (k, _) = super::read_frame(&mut r, 1 << 26).unwrap().unwrap();
        assert_eq!(k, 1, "no file open");
    }
}
