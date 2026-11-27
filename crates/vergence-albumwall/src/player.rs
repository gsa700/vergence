//! Playing (docs/albumwall-milestone-1.md, step 6; the Avalonia app's
//! HymieEngine): one album's tracks, gaplessly, at each file's own rate.
//!
//! - **One stream, tracks fed back to back.** The decoders already trim each
//!   file's padding (Hymie: an MP3's LAME tag, an MP4's edit list), so one
//!   track's last sample and the next one's first meet in the same write.
//!   The next track is opened ahead (its own helper). The output is opened
//!   again only when the rate or the channels change.
//! - **ReplayGain** by album, by track, or off (clipping prevented by the
//!   peak, as mpv does), and the app's own volume, then dithered to 16 bits.
//! - **Where it is** is what has been heard: what was written less what the
//!   output still holds. A track change is heard that much later too, so
//!   the track shown changes when its first sample comes out.
//! - The player runs on a thread of its own; the window sends it commands
//!   and reads its state, and is woken when something changes.
//!
//! Decoders and the output are behind two small traits, so the engine is
//! tested with made-up tracks and a stream that records what it is given.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A decoded track the player reads from.
pub trait Source: Send {
    fn rate(&self) -> u32;
    fn channels(&self) -> u16;
    /// Its length in frames.
    fn frames(&self) -> u64;
    fn gain(&self) -> vergence_hymie::ReplayGain;
    /// Up to `frames` frames from where it is, interleaved; fewer only at the end.
    fn read(&mut self, frames: u32) -> Result<Vec<f32>, String>;
    fn seek(&mut self, frame: u64) -> Result<(), String>;
}

/// A file's track, in Hymie's confined helper.
impl Source for vergence_hymie::helper::Sound {
    fn rate(&self) -> u32 {
        self.info.rate
    }
    fn channels(&self) -> u16 {
        self.info.channels
    }
    fn frames(&self) -> u64 {
        self.info.frames
    }
    fn gain(&self) -> vergence_hymie::ReplayGain {
        self.info.gain
    }
    fn read(&mut self, frames: u32) -> Result<Vec<f32>, String> {
        vergence_hymie::helper::Sound::read(self, frames).map_err(|e| e.to_string())
    }
    fn seek(&mut self, frame: u64) -> Result<(), String> {
        vergence_hymie::helper::Sound::seek(self, frame).map_err(|e| e.to_string())
    }
}

/// Opens a track; opens the output for a rate and channels.
pub type OpenTrack = dyn Fn(&std::path::Path) -> Result<Box<dyn Source>, String> + Send + Sync;
pub type OpenOutput = dyn Fn(u32, u32) -> std::io::Result<Box<dyn vergence_sound::Stream>> + Send + Sync;

/// The installed decoders and output.
pub fn system() -> (Arc<OpenTrack>, Arc<OpenOutput>) {
    system_with(Arc::new(|_| None))
}

/// The sound bytes of a track that is not a file (a music server's song),
/// or None for a file.
pub type Remote = dyn Fn(&std::path::Path) -> Option<Result<Arc<Vec<u8>>, String>> + Send + Sync;

/// As [`system`], a track `remote` knows played from its bytes (a server's
/// song, fetched whole: milestone 2, step 3), any other from its file.
pub fn system_with(remote: Arc<Remote>) -> (Arc<OpenTrack>, Arc<OpenOutput>) {
    let track: Arc<OpenTrack> = Arc::new(move |p: &std::path::Path| match remote(p) {
        Some(Ok(bytes)) => vergence_hymie::helper::Sound::open_bytes(&bytes)
            .map(|s| Box::new(s) as Box<dyn Source>)
            .map_err(|e| e.to_string()),
        Some(Err(why)) => Err(why),
        None => {
            vergence_hymie::helper::Sound::open(p).map(|s| Box::new(s) as Box<dyn Source>).map_err(|e| e.to_string())
        }
    });
    let output: Arc<OpenOutput> = Arc::new(|rate, ch| vergence_sound::named_stream(rate, ch, "AlbumWall"));
    (track, output)
}

/// How loud: ReplayGain by album, by track, or not at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loudness {
    Album,
    Track,
    Off,
}

/// What the window asks of the player.
pub enum Command {
    /// Play `tracks` (an album's, in order) from `from`; `tag` says which
    /// album they are (the window's), handed back in the state.
    Play {
        tracks: Vec<PathBuf>,
        from: usize,
        tag: usize,
        loudness: Loudness,
    },
    /// As Play, loaded paused at `seconds` into its track (picking up where
    /// it was left): nothing is heard until Pause(false).
    Load {
        tracks: Vec<PathBuf>,
        from: usize,
        tag: usize,
        loudness: Loudness,
        seconds: f64,
    },
    Pause(bool),
    Next,
    /// Back a track, or to the start of this one if more than 3 s in.
    Previous,
    Seek(f64),
    Stop,
    /// The app's own volume, 0 to 1.
    Volume(f32),
    /// Every song given, in this order (a shuffle of the wall's songs):
    /// each its path, its album (the window's) and its number in that
    /// album, which the state reports as `Now`'s tag and track. When the
    /// last one ends, `ended` is [`SHUFFLE`].
    Shuffle {
        tracks: Vec<(PathBuf, usize, usize)>,
        loudness: Loudness,
    },
    /// The meters read after ReplayGain (true) or as mastered.
    MetersAfterGain(bool),
}

/// The tag a shuffle of songs plays under, and `ended` when it is over.
pub const SHUFFLE: usize = usize::MAX;

/// What is playing, as heard.
#[derive(Debug, Clone, PartialEq)]
pub struct Now {
    pub tag: usize,
    pub track: usize,
    pub seconds: f64,
    pub length: f64,
    pub paused: bool,
    pub rate: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    pub now: Option<Now>,
    /// The last album played to its end (its tag): what to do next is the window's.
    pub ended: Option<usize>,
    /// Why nothing plays ("No sound output: …"), until something plays.
    pub problem: Option<String>,
    /// The meters' levels, by where they are in the output (Spindle's level blocks).
    pub meter: Meter,
}

/// Frames in a level block (Spindle's LevelShift 10), and blocks kept.
const BLOCK_SHIFT: u32 = 10;
const BLOCKS: usize = 512;

/// The RMS of each 1024-frame block written, as the record was mastered
/// (before ReplayGain and volume), keyed by its place in the output's
/// timeline; read for the block being heard now, so the meters move with
/// what comes out of the speakers, not with what was sent 200 ms before.
#[derive(Debug, Clone, PartialEq)]
pub struct Meter {
    blocks: Vec<(i64, f32, f32)>,
    written: u64,
    latency: u64,
    rate: u32,
    at: Option<std::time::Instant>,
    paused: bool,
}

impl Default for Meter {
    fn default() -> Meter {
        Meter { blocks: vec![(-1, 0.0, 0.0); BLOCKS], written: 0, latency: 0, rate: 1, at: None, paused: false }
    }
}

impl Meter {
    /// The RMS of each side being heard at `now`, in dBFS (negative
    /// infinity: silence, paused, nothing playing).
    pub fn dbfs(&self, now: std::time::Instant) -> [f64; 2] {
        let silent = [f64::NEG_INFINITY; 2];
        let Some(at) = self.at.filter(|_| !self.paused) else { return silent };
        // The output has gone on playing since the last write.
        let since = (now.saturating_duration_since(at).as_secs_f64() * self.rate as f64) as u64;
        let heard = (self.written.saturating_sub(self.latency) + since).min(self.written);
        // The block being heard, or the last one finished when that one is
        // still being summed (an output that buffers nothing).
        let block = (heard >> BLOCK_SHIFT) as i64;
        let found = [block, block - 1]
            .into_iter()
            .filter(|b| *b >= 0)
            .map(|b| self.blocks[b as usize % BLOCKS])
            .find(|(b, _, _)| *b == block || *b == block - 1);
        let Some((_, l, r)) = found else { return silent };
        let db = |ms: f32| if ms > 0.0 { 10.0 * (ms as f64).log10() } else { f64::NEG_INFINITY };
        [db(l), db(r)]
    }
}

/// The player: its thread, the commands it takes, its state.
pub struct Player {
    tx: Sender<Command>,
    pub state: Arc<Mutex<State>>,
}

/// How much is decoded and written at once.
const PIECE: Duration = Duration::from_millis(50);

impl Player {
    pub fn new(track: Arc<OpenTrack>, output: Arc<OpenOutput>, wake: impl Fn() + Send + 'static) -> Player {
        let (tx, rx) = channel();
        let state: Arc<Mutex<State>> = Arc::default();
        let s = state.clone();
        let _ = std::thread::Builder::new()
            .name("player".into())
            .spawn(move || Engine::new(track, output, s, Box::new(wake)).run(rx));
        Player { tx, state }
    }

    pub fn send(&self, c: Command) {
        let _ = self.tx.send(c);
    }

    pub fn state(&self) -> State {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// One track open: its number in the queue and its source.
struct Open {
    index: usize,
    source: Box<dyn Source>,
}

/// A track being opened on a thread of its own: its place in the queue,
/// what the thread hands back, and whether it already failed (then not
/// tried again until the track itself starts, which says why).
/// Where an opening thread leaves the track, or why it could not.
type Slot = Arc<Mutex<Option<Result<Box<dyn Source>, String>>>>;

struct Ahead {
    index: usize,
    slot: Slot,
    failed: bool,
}

/// A point in the output's timeline where a track starts: frames written
/// before it, the track, and the frame of the track it starts at (a seek).
#[derive(Clone, Copy)]
struct Mark {
    at: u64,
    track: usize,
    from: u64,
}

struct Engine {
    open_track: Arc<OpenTrack>,
    open_output: Arc<OpenOutput>,
    state: Arc<Mutex<State>>,
    wake: Box<dyn Fn() + Send>,
    tracks: Vec<PathBuf>,
    tag: usize,
    /// A shuffle's album and track for each place in `tracks` (empty: one album's).
    map: Vec<(usize, usize)>,
    loudness: Loudness,
    /// The meters summed after the gain rather than before.
    after_gain: bool,
    volume: f32,
    current: Option<Open>,
    next: Option<Open>,
    /// The next track being opened on a thread of its own (a server's song
    /// is fetched first; a sleeping drive takes its time): taken at the seam.
    ahead: Option<Ahead>,
    /// A track being opened to play (a server's song is fetched first): its
    /// index and the second to start at. The album has not ended; commands
    /// are still taken while it waits.
    waiting: Option<(usize, f64)>,
    out: Option<Box<dyn vergence_sound::Stream>>,
    /// Frames written to the output since it opened, and where tracks start in that.
    written: u64,
    marks: VecDeque<Mark>,
    paused: bool,
    dither: u32,
    /// The level block being summed: its number, the sums of squares, frames.
    level: (i64, f32, f32, u32),
    /// When the window was last woken: at most 4 times a second unless
    /// something changed (the battery: the window keeps its own time).
    woke: Option<std::time::Instant>,
}

impl Engine {
    fn new(
        open_track: Arc<OpenTrack>,
        open_output: Arc<OpenOutput>,
        state: Arc<Mutex<State>>,
        wake: Box<dyn Fn() + Send>,
    ) -> Engine {
        Engine {
            open_track,
            open_output,
            state,
            wake,
            tracks: Vec::new(),
            tag: 0,
            map: Vec::new(),
            loudness: Loudness::Album,
            after_gain: false,
            volume: 1.0,
            current: None,
            next: None,
            ahead: None,
            waiting: None,
            out: None,
            written: 0,
            marks: VecDeque::new(),
            paused: false,
            dither: 0x1234_5678,
            level: (0, 0.0, 0.0, 0),
            woke: None,
        }
    }

    fn run(mut self, rx: Receiver<Command>) {
        loop {
            // Idle or paused: wait for a command. Playing: take what came, then play a piece.
            let idle = self.waiting.is_none() && (self.current.is_none() || self.paused);
            let c = if idle {
                match rx.recv() {
                    Ok(c) => Some(c),
                    Err(_) => return,
                }
            } else {
                match rx.try_recv() {
                    Ok(c) => Some(c),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => return,
                }
            };
            if let Some(c) = c {
                self.command(c);
                continue;
            }
            self.piece();
        }
    }

    fn command(&mut self, c: Command) {
        match c {
            Command::Play { tracks, from, tag, loudness } => {
                self.tracks = tracks;
                self.tag = tag;
                self.map.clear();
                self.loudness = loudness;
                self.paused = false;
                self.next = None;
                self.ahead = None;
                self.waiting = None;
                self.flush();
                self.start(from, 0.0);
            }
            Command::Load { tracks, from, tag, loudness, seconds } => {
                self.tracks = tracks;
                self.tag = tag;
                self.map.clear();
                self.loudness = loudness;
                self.next = None;
                self.ahead = None;
                self.waiting = None;
                self.flush();
                // Paused before it opens, so it is never heard until Pause(false).
                self.paused = true;
                self.start(from, seconds.max(0.0));
                if self.current.is_none() && self.waiting.is_none() {
                    self.paused = false;
                }
            }
            Command::Pause(on) => {
                if (self.current.is_some() || self.waiting.is_some()) && on != self.paused {
                    self.paused = on;
                    if let Some(o) = self.out.as_mut() {
                        let _ = o.pause(on);
                    }
                }
            }
            Command::Next => {
                if let Some(i) = self.place().map(|i| i + 1) {
                    self.flush();
                    if i < self.tracks.len() { self.start(i, 0.0) } else { self.finish() }
                }
            }
            Command::Previous => {
                if let Some(i) = self.place() {
                    let back = if self.heard().map_or(0.0, |n| n.seconds) > 3.0 { i } else { i.saturating_sub(1) };
                    self.flush();
                    self.start(back, 0.0);
                }
            }
            Command::Seek(seconds) => {
                if let Some((_, at)) = self.waiting.as_mut() {
                    // Not open yet: it starts there when it is.
                    *at = seconds.max(0.0);
                } else if let Some(i) = self.place() {
                    self.flush();
                    self.start(i, seconds.max(0.0));
                }
            }
            Command::Stop => {
                self.flush();
                self.current = None;
                self.next = None;
                self.ahead = None;
                self.waiting = None;
                self.out = None;
                self.publish(None);
            }
            Command::Volume(v) => self.volume = v.clamp(0.0, 1.0),
            Command::Shuffle { tracks, loudness } => {
                (self.tracks, self.map) = tracks.into_iter().map(|(p, a, t)| (p, (a, t))).unzip();
                self.tag = SHUFFLE;
                self.loudness = loudness;
                self.paused = false;
                self.next = None;
                self.ahead = None;
                self.waiting = None;
                self.flush();
                self.start(0, 0.0);
            }
            Command::MetersAfterGain(on) => self.after_gain = on,
        }
        self.publish(None);
    }

    /// What is buffered thrown away, so what comes next is heard at once.
    fn flush(&mut self) {
        if let Some(o) = self.out.as_mut() {
            let _ = o.flush();
            if self.paused {
                let _ = o.pause(false);
            }
        }
        self.paused = false;
        self.marks.clear();
        self.written = 0;
        self.level = (0, 0.0, 0.0, 0);
        self.state.lock().unwrap_or_else(|e| e.into_inner()).meter = Meter::default();
        // A stream that cannot flush still holds what it has; counted afresh from here.
    }

    /// Track `i` opened ahead: started on a thread if it is not under way,
    /// taken into `next` once it is ready.
    fn open_ahead(&mut self, i: usize) {
        match &mut self.ahead {
            Some(a) if a.index == i => {
                if a.failed {
                    return;
                }
                let ready = a.slot.lock().unwrap_or_else(|e| e.into_inner()).take();
                match ready {
                    Some(Ok(src)) => {
                        self.ahead = None;
                        self.waiting = None;
                        self.next = Some(Open { index: i, source: src });
                    }
                    Some(Err(_)) => a.failed = true,
                    None => {}
                }
            }
            _ => {
                let slot: Slot = Arc::default();
                let (open, path, out) = (self.open_track.clone(), self.tracks[i].clone(), slot.clone());
                let spawned = std::thread::Builder::new().name("open-ahead".into()).spawn(move || {
                    let r = open(&path);
                    *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
                });
                self.ahead = Some(Ahead { index: i, slot, failed: spawned.is_err() });
            }
        }
    }

    /// The track playing, or being opened to play.
    fn place(&self) -> Option<usize> {
        self.current.as_ref().map(|c| c.index).or(self.waiting.map(|(i, _)| i))
    }

    /// Track `i` started at its second `at`: opened on a thread of its own
    /// (or taken from its opening ahead), waited for briefly here (a file is
    /// open by then, as before), else on later pieces while commands are
    /// still taken (a server's song being fetched).
    fn start(&mut self, i: usize, at: f64) {
        if let Some(n) = self.next.take()
            && n.index == i
        {
            self.install(i, at, Ok(n.source));
            return;
        }
        if self.ahead.as_ref().is_none_or(|a| a.index != i || a.failed) {
            self.ahead = None;
            self.open_ahead(i);
        }
        self.current = None;
        self.waiting = Some((i, at));
        let until = std::time::Instant::now() + Duration::from_millis(150);
        while !self.poll_waiting(i, at) && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// The track being waited for taken if its opening is done: true when
    /// the wait is over (played, or said why not).
    fn poll_waiting(&mut self, i: usize, at: f64) -> bool {
        let ready = match &mut self.ahead {
            Some(a) if a.index == i => {
                let r = a.slot.lock().unwrap_or_else(|e| e.into_inner()).take();
                if r.is_none() && a.failed { Some(Err("it could not be opened".to_string())) } else { r }
            }
            // Nothing opening it (a thread could not be started): opened here.
            _ => Some(match self.tracks.get(i) {
                Some(p) => (self.open_track)(p),
                None => Err("no such track".into()),
            }),
        };
        let Some(r) = ready else { return false };
        self.install(i, at, r);
        true
    }

    /// Track `i`, opened, played from its second `at`: after what is
    /// already written when the output suits it (a seam stays gapless), else
    /// in a new output.
    fn install(&mut self, i: usize, at: f64, r: Result<Box<dyn Source>, String>) {
        self.ahead = None;
        self.waiting = None;
        match r {
            Ok(mut src) => {
                let from = (at.max(0.0) * src.rate() as f64) as u64;
                if from > 0 {
                    let _ = src.seek(from);
                }
                let suits =
                    self.out.as_ref().is_none_or(|o| o.rate() == src.rate() && o.channels() == src.channels() as u32);
                if !suits {
                    self.out = None;
                    self.written = 0;
                    self.marks.clear();
                }
                self.marks.push_back(Mark { at: self.written, track: i, from });
                self.current = Some(Open { index: i, source: src });
                if self.paused
                    && let Some(o) = self.out.as_mut()
                {
                    let _ = o.pause(true);
                }
                self.publish(None);
            }
            Err(e) => {
                self.current = None;
                self.publish(Some(format!("This track could not be played: {e}")));
            }
        }
    }

    /// The album played to its end.
    fn finish(&mut self) {
        self.current = None;
        self.next = None;
        self.ahead = None;
        self.waiting = None;
        let tag = self.tag;
        self.state.lock().unwrap_or_else(|e| e.into_inner()).ended = Some(tag);
        self.publish(None);
    }

    /// One piece decoded, made louder or softer, and written.
    fn piece(&mut self) {
        if let Some((i, at)) = self.waiting {
            if !self.poll_waiting(i, at) {
                std::thread::sleep(Duration::from_millis(10));
            }
            return;
        }
        let Some(cur) = self.current.as_mut() else { return };
        let (rate, ch) = (cur.source.rate(), cur.source.channels().max(1) as u32);
        let want = ((rate as f64 * PIECE.as_secs_f64()) as u32).max(1);
        let read = cur.source.read(want);
        let index = cur.index;
        let gain = match self.loudness {
            Loudness::Album => cur.source.gain().multiplier(true),
            Loudness::Track => cur.source.gain().multiplier(false),
            Loudness::Off => 1.0,
        } * self.volume;
        let mut samples = match read {
            Ok(s) => s,
            Err(e) => {
                self.publish(Some(format!("This track stopped: {e}")));
                Vec::new()
            }
        };
        let mut gain = gain;
        let mut short = samples.len() < (want * ch) as usize;
        // The next track opened ahead, on a thread of its own, ready for the seam.
        if self.next.is_none() && index + 1 < self.tracks.len() {
            self.open_ahead(index + 1);
        }
        // At the end: the next track's first frames go in the same write
        // when it is the same rate and channels (gapless), else after a new output.
        if short {
            match self.next.take() {
                Some(n) if n.source.rate() == rate && n.source.channels() as u32 == ch => {
                    let at = self.written + (samples.len() as u64 / ch as u64);
                    self.marks.push_back(Mark { at, track: n.index, from: 0 });
                    let rest = want - (samples.len() as u32 / ch);
                    let mut s = n.source;
                    let more = s.read(rest).unwrap_or_default();
                    // The rest of the piece at the new track's gain.
                    let g2 = match self.loudness {
                        Loudness::Album => s.gain().multiplier(true),
                        Loudness::Track => s.gain().multiplier(false),
                        Loudness::Off => 1.0,
                    } * self.volume;
                    self.write(&samples, gain, ch, rate);
                    samples = more;
                    gain = g2;
                    short = samples.len() < (rest * ch) as usize;
                    self.current = Some(Open { index: n.index, source: s });
                }
                Some(n) => {
                    self.write(&samples, gain, ch, rate);
                    // Another rate: the old output played out, a new one for it.
                    self.out = None;
                    self.written = 0;
                    self.marks.clear();
                    self.marks.push_back(Mark { at: 0, track: n.index, from: 0 });
                    self.current = Some(n);
                    self.publish(None);
                    return;
                }
                // The next track not open yet (still being fetched): waited
                // for, then on as at any seam; the album has not ended.
                None if index + 1 < self.tracks.len() => {
                    self.write(&samples, gain, ch, rate);
                    self.current = None;
                    self.start(index + 1, 0.0);
                    self.publish(None);
                    return;
                }
                None => {
                    self.write(&samples, gain, ch, rate);
                    self.finish();
                    return;
                }
            }
        }
        self.write(&samples, gain, ch, rate);
        if short {
            // The new track was shorter than the rest of the piece: let the next piece find its end.
        }
        self.publish(None);
    }

    /// Samples written to the output (opened for their rate and channels),
    /// at `gain`, dithered to 16 bits; the meters' levels kept.
    fn write(&mut self, samples: &[f32], gain: f32, ch: u32, rate: u32) {
        if samples.is_empty() {
            return;
        }
        if self.out.as_ref().is_none_or(|o| o.rate() != rate || o.channels() != ch) && self.out.is_none() {
            match (self.open_output)(rate, ch) {
                Ok(o) => self.out = Some(o),
                Err(e) => {
                    self.current = None;
                    self.publish(Some(format!("No sound output: {e}")));
                    return;
                }
            }
        }
        let Some(out) = self.out.as_mut() else { return };
        let (orate, och) = (out.rate(), out.channels().max(1));
        let frames = samples.len() / ch as usize;
        let mut pcm: Vec<i16> = Vec::with_capacity(frames * och as usize);
        let mut done_blocks: Vec<(i64, f32, f32)> = Vec::new();
        // The output's rate if it would not take the file's (an ALSA device
        // with a rate of its own): linear between frames.
        let step = rate as f64 / orate as f64;
        let n_out = if orate == rate { frames } else { ((frames as f64) / step).floor() as usize };
        for f in 0..n_out {
            let pos = f as f64 * step;
            let (i, t) = (pos.floor() as usize, (pos - pos.floor()) as f32);
            for c in 0..och as usize {
                let src = |frame: usize| -> f32 {
                    let frame = frame.min(frames - 1);
                    let base = frame * ch as usize;
                    match (ch, och) {
                        (1, _) => samples[base],
                        (_, 1) => (samples[base] + samples[base + 1.min(ch as usize - 1)]) / 2.0,
                        _ if c < ch as usize => samples[base + c],
                        _ => 0.0,
                    }
                };
                let raw = src(i) * (1.0 - t) + src(i + 1) * t;
                let v = raw * gain;
                // The level block, summed as mastered (before the gain),
                // or as ReplayGain sends it on, if chosen.
                let m = if self.after_gain { v } else { raw };
                if c == 0 {
                    self.level.1 += m * m;
                    if och == 1 {
                        self.level.2 += m * m;
                    }
                } else if c == 1 {
                    self.level.2 += m * m;
                }
                // TPDF dither: two uniform randoms of a half step each.
                let mut r = || {
                    self.dither ^= self.dither << 13;
                    self.dither ^= self.dither >> 17;
                    self.dither ^= self.dither << 5;
                    self.dither as f32 / u32::MAX as f32 - 0.5
                };
                let d = r() + r();
                pcm.push((v * 32767.0 + d).round().clamp(-32768.0, 32767.0) as i16);
            }
            self.level.3 += 1;
            let block = ((self.written + f as u64 + 1) >> BLOCK_SHIFT) as i64;
            if block != self.level.0 {
                let n = self.level.3.max(1) as f32;
                done_blocks.push((self.level.0, self.level.1 / n, self.level.2 / n));
                self.level = (block, 0.0, 0.0, 0);
            }
        }
        if let Err(e) = out.write(&pcm) {
            self.out = None;
            self.current = None;
            self.publish(Some(format!("Sound output stopped: {e}")));
            return;
        }
        self.written += n_out as u64;
        let latency = (out.latency().as_secs_f64() * orate as f64) as u64;
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let m = &mut st.meter;
        for (b, l, r) in done_blocks {
            if b >= 0 {
                m.blocks[b as usize % BLOCKS] = (b, l, r);
            }
        }
        m.written = self.written;
        m.latency = latency;
        m.rate = orate;
        m.at = Some(std::time::Instant::now());
        m.paused = self.paused;
    }

    /// What is being heard: the track whose start the output has reached, and how far in.
    fn heard(&self) -> Option<Now> {
        if let Some((i, at)) = self.waiting {
            // The track being fetched: shown at its start, its length not known yet.
            let rate = self.out.as_ref().map_or(44100, |o| o.rate());
            return Some(Now { tag: self.tag, track: i, seconds: at, length: 0.0, paused: self.paused, rate });
        }
        let cur = self.current.as_ref()?;
        let rate = self.out.as_ref().map_or(cur.source.rate(), |o| o.rate()).max(1);
        let latency = self.out.as_ref().map_or(0, |o| (o.latency().as_secs_f64() * rate as f64) as u64);
        let heard = self.written.saturating_sub(latency);
        let mark = self.marks.iter().rev().find(|m| m.at <= heard).or(self.marks.front()).copied()?;
        let seconds = (mark.from + heard.saturating_sub(mark.at)) as f64 / rate as f64;
        let length =
            if mark.track == cur.index { cur.source.frames() as f64 / cur.source.rate().max(1) as f64 } else { 0.0 };
        Some(Now { tag: self.tag, track: mark.track, seconds, length, paused: self.paused, rate: cur.source.rate() })
    }

    fn publish(&mut self, problem: Option<String>) {
        // Marks the output has passed are forgotten (all but the one being heard).
        let now = self.heard();
        if let Some(n) = &now {
            while self.marks.len() > 1 && self.marks.get(1).is_some_and(|m| m.track <= n.track && m.at <= self.written)
            {
                if self.marks[1].track == n.track {
                    self.marks.pop_front();
                } else {
                    break;
                }
            }
        }
        // A shuffle's place told as its album and track.
        let now = now.map(|n| match self.map.get(n.track) {
            Some(&(tag, track)) => Now { tag, track, ..n },
            None => n,
        });
        let important;
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let key = |n: &Now| (n.tag, n.track, n.paused);
            let changed = s.now.as_ref().map(key) != now.as_ref().map(key);
            important = changed || problem.is_some() || s.ended.is_some() || now.is_none();
            s.meter.paused = self.paused;
            s.now = now;
            if problem.is_some() {
                s.problem = problem;
            } else if s.now.is_some() {
                s.problem = None;
            }
            if changed && s.now.is_some() {
                s.ended = None;
            }
        }
        // The window keeps its own time between: woken for a change at once,
        // else at most 4 times a second (its clock and seek bar).
        let now = std::time::Instant::now();
        if important || self.woke.is_none_or(|t| now.saturating_duration_since(t) >= Duration::from_millis(250)) {
            self.woke = Some(now);
            (self.wake)();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up track: `frames` frames of a ramp, `ch` channels.
    struct Ramp {
        rate: u32,
        ch: u16,
        frames: u64,
        at: u64,
        level: f32,
    }

    impl Source for Ramp {
        fn rate(&self) -> u32 {
            self.rate
        }
        fn channels(&self) -> u16 {
            self.ch
        }
        fn frames(&self) -> u64 {
            self.frames
        }
        fn gain(&self) -> vergence_hymie::ReplayGain {
            vergence_hymie::ReplayGain::default()
        }
        fn read(&mut self, frames: u32) -> Result<Vec<f32>, String> {
            let n = (frames as u64).min(self.frames - self.at);
            self.at += n;
            Ok(vec![self.level; (n * self.ch as u64) as usize])
        }
        fn seek(&mut self, frame: u64) -> Result<(), String> {
            self.at = frame.min(self.frames);
            Ok(())
        }
    }

    /// An output that keeps what it is given (and takes no time).
    struct Tape {
        rate: u32,
        ch: u32,
        kept: Arc<Mutex<Vec<i16>>>,
    }

    impl vergence_sound::Stream for Tape {
        fn rate(&self) -> u32 {
            self.rate
        }
        fn channels(&self) -> u32 {
            self.ch
        }
        fn write(&mut self, samples: &[i16]) -> std::io::Result<()> {
            self.kept.lock().unwrap().extend_from_slice(samples);
            Ok(())
        }
    }

    fn engine(tracks: Vec<(u32, u64, f32)>, kept: Arc<Mutex<Vec<i16>>>) -> Engine {
        slow_engine(tracks, kept, Duration::ZERO)
    }

    /// As [`engine`], every track but the first taking `slow` to open (a
    /// server's song being fetched).
    fn slow_engine(tracks: Vec<(u32, u64, f32)>, kept: Arc<Mutex<Vec<i16>>>, slow: Duration) -> Engine {
        let tracks = Arc::new(tracks);
        let t2 = tracks.clone();
        let open: Arc<OpenTrack> = Arc::new(move |p: &std::path::Path| {
            let i: usize = p.to_string_lossy().parse().map_err(|_| "?".to_string())?;
            if i > 0 {
                std::thread::sleep(slow);
            }
            let (rate, frames, level) = t2[i];
            Ok(Box::new(Ramp { rate, ch: 2, frames, at: 0, level }) as Box<dyn Source>)
        });
        let out: Arc<OpenOutput> = Arc::new(move |rate, ch| {
            Ok(Box::new(Tape { rate, ch, kept: kept.clone() }) as Box<dyn vergence_sound::Stream>)
        });
        let mut e = Engine::new(open, out, Arc::default(), Box::new(|| {}));
        e.tracks = (0..tracks.len()).map(|i| PathBuf::from(i.to_string())).collect();
        e
    }

    fn play_out(e: &mut Engine) {
        for _ in 0..10_000 {
            if e.current.is_none() && e.waiting.is_none() {
                return;
            }
            e.piece();
        }
        panic!("never ended");
    }

    #[test]
    fn a_next_track_slow_to_open_is_waited_for_and_commands_still_taken() {
        let kept: Arc<Mutex<Vec<i16>>> = Arc::default();
        let tracks = vec![(44100, 3001, 0.25), (44100, 2999, 0.5), (44100, 1000, 0.75)];
        let mut e = slow_engine(tracks.clone(), kept.clone(), Duration::from_millis(400));
        e.command(Command::Play { tracks: e.tracks.clone(), from: 0, tag: 3, loudness: Loudness::Off });
        play_out(&mut e);
        // Every frame of all three, in order: the album did not end at the first wait.
        assert_eq!(kept.lock().unwrap().len(), (3001 + 2999 + 1000) * 2);
        assert_eq!(e.state.lock().unwrap().ended, Some(3));
        // While the next one is being fetched the window is told its place,
        // and Stop is taken at once rather than after the fetch.
        let kept: Arc<Mutex<Vec<i16>>> = Arc::default();
        let mut e = slow_engine(tracks, kept, Duration::from_millis(800));
        e.command(Command::Play { tracks: e.tracks.clone(), from: 1, tag: 4, loudness: Loudness::Off });
        assert_eq!(e.waiting.map(|(i, _)| i), Some(1));
        assert_eq!(e.state.lock().unwrap().now.as_ref().map(|n| n.track), Some(1));
        let t = std::time::Instant::now();
        e.command(Command::Stop);
        assert!(t.elapsed() < Duration::from_millis(100));
        assert!(e.waiting.is_none() && e.current.is_none());
    }

    #[test]
    fn tracks_are_played_back_to_back_with_nothing_between() {
        let kept: Arc<Mutex<Vec<i16>>> = Arc::default();
        let mut e = engine(vec![(44100, 3001, 0.25), (44100, 2999, 0.5)], kept.clone());
        e.command(Command::Play { tracks: e.tracks.clone(), from: 0, tag: 7, loudness: Loudness::Off });
        play_out(&mut e);
        let k = kept.lock().unwrap();
        assert_eq!(k.len(), (3001 + 2999) * 2, "every frame, none added");
        // The seam: the first track's last frame, then the second's first.
        let q = |v: f32| (v * 32767.0).round() as i32;
        let near = |s: i16, v: f32| (s as i32 - q(v)).abs() <= 1;
        assert!(near(k[3000 * 2], 0.25) && near(k[3001 * 2], 0.5));
        assert_eq!(e.state.lock().unwrap().ended, Some(7));
    }

    #[test]
    fn another_rate_opens_the_output_again() {
        let kept: Arc<Mutex<Vec<i16>>> = Arc::default();
        let mut e = engine(vec![(44100, 2000, 0.25), (96000, 2000, 0.5)], kept.clone());
        e.command(Command::Play { tracks: e.tracks.clone(), from: 0, tag: 1, loudness: Loudness::Off });
        play_out(&mut e);
        assert_eq!(kept.lock().unwrap().len(), 4000 * 2);
    }

    #[test]
    fn volume_and_position_and_next() {
        let kept: Arc<Mutex<Vec<i16>>> = Arc::default();
        let mut e = engine(vec![(48000, 48000 * 10, 1.0), (48000, 48000, 1.0)], kept.clone());
        e.command(Command::Volume(0.5));
        e.command(Command::Play { tracks: e.tracks.clone(), from: 0, tag: 3, loudness: Loudness::Off });
        for _ in 0..20 {
            e.piece();
        }
        let n = e.heard().unwrap();
        assert_eq!((n.tag, n.track), (3, 0));
        assert!((n.seconds - 1.0).abs() < 0.01, "{}", n.seconds);
        assert!((n.length - 10.0).abs() < 0.001);
        assert!((kept.lock().unwrap()[100] as f32 - 16383.5).abs() <= 1.5, "half volume");
        e.command(Command::Seek(5.0));
        e.piece();
        assert!((e.heard().unwrap().seconds - 5.05).abs() < 0.01);
        e.command(Command::Next);
        assert_eq!(e.heard().unwrap().track, 1);
        // Previous within 3 s of a track's start goes back a track.
        e.command(Command::Previous);
        assert_eq!(e.heard().unwrap().track, 0);
    }

    #[test]
    fn a_track_loaded_paused_is_where_it_was_and_silent() {
        let kept: Arc<Mutex<Vec<i16>>> = Arc::default();
        let mut e = engine(vec![(44100, 44100 * 60, 0.5)], kept.clone());
        e.command(Command::Load { tracks: e.tracks.clone(), from: 0, tag: 2, loudness: Loudness::Off, seconds: 30.0 });
        let n = e.heard().unwrap();
        assert!(n.paused && (n.seconds - 30.0).abs() < 0.01);
        assert!(kept.lock().unwrap().is_empty(), "nothing heard while paused");
        e.command(Command::Pause(false));
        e.piece();
        assert!(!kept.lock().unwrap().is_empty());
    }

    #[test]
    fn the_meter_reads_the_block_being_heard() {
        let kept: Arc<Mutex<Vec<i16>>> = Arc::default();
        // A level of 0.5 for 2 s: an RMS of -6 dBFS.
        let mut e = engine(vec![(48000, 96000, 0.5)], kept.clone());
        e.command(Command::Play { tracks: e.tracks.clone(), from: 0, tag: 0, loudness: Loudness::Off });
        for _ in 0..10 {
            e.piece();
        }
        let m = e.state.lock().unwrap().meter.clone();
        let db = m.dbfs(std::time::Instant::now());
        assert!((db[0] - -6.02).abs() < 0.1 && (db[1] - -6.02).abs() < 0.1, "{db:?}");
        e.command(Command::Pause(true));
        let m = e.state.lock().unwrap().meter.clone();
        assert_eq!(m.dbfs(std::time::Instant::now())[0], f64::NEG_INFINITY, "paused: the needles fall");
    }

    #[test]
    fn a_shuffle_tells_each_songs_album_and_number() {
        let kept: Arc<Mutex<Vec<i16>>> = Arc::default();
        let mut e = engine(vec![(44100, 4410, 0.5), (44100, 4410, 0.5)], kept.clone());
        let t = e.tracks.clone();
        e.command(Command::Shuffle {
            tracks: vec![(t[1].clone(), 9, 4), (t[0].clone(), 2, 0)],
            loudness: Loudness::Off,
        });
        let n = e.state.lock().unwrap().now.clone().unwrap();
        assert_eq!((n.tag, n.track), (9, 4));
        e.command(Command::Next);
        let n = e.state.lock().unwrap().now.clone().unwrap();
        assert_eq!((n.tag, n.track), (2, 0));
        play_out(&mut e);
        assert_eq!(e.state.lock().unwrap().ended, Some(SHUFFLE));
        // An album after it: its own numbers again.
        e.command(Command::Play { tracks: t, from: 1, tag: 5, loudness: Loudness::Off });
        let n = e.state.lock().unwrap().now.clone().unwrap();
        assert_eq!((n.tag, n.track), (5, 1));
    }

    #[test]
    fn the_meters_can_read_after_the_gain() {
        let kept: Arc<Mutex<Vec<i16>>> = Arc::default();
        let mut e = engine(vec![(48000, 96000, 0.5)], kept.clone());
        e.command(Command::Volume(0.5));
        e.command(Command::MetersAfterGain(true));
        e.command(Command::Play { tracks: e.tracks.clone(), from: 0, tag: 0, loudness: Loudness::Off });
        for _ in 0..10 {
            e.piece();
        }
        let db = e.state.lock().unwrap().meter.dbfs(std::time::Instant::now());
        assert!((db[0] - -12.04).abs() < 0.1, "{db:?}");
    }

    #[test]
    fn no_output_is_said_so() {
        let open: Arc<OpenTrack> = Arc::new(|_: &std::path::Path| {
            Ok(Box::new(Ramp { rate: 44100, ch: 2, frames: 9000, at: 0, level: 0.1 }) as Box<dyn Source>)
        });
        let out: Arc<OpenOutput> = Arc::new(|_, _| Err(std::io::Error::other("nothing here")));
        let mut e = Engine::new(open, out, Arc::default(), Box::new(|| {}));
        e.command(Command::Play { tracks: vec!["a".into()], from: 0, tag: 0, loudness: Loudness::Off });
        e.piece();
        let s = e.state.lock().unwrap().clone();
        assert_eq!(s.problem.as_deref(), Some("No sound output: nothing here"));
        assert!(s.now.is_none());
    }
}
