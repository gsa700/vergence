//! AlbumWall for Vergence (docs/albumwall-milestone-1.md): a wall of every
//! album you own, rebuilt in Rust on the toolkit and Hymie, looking and
//! working as the Avalonia AlbumWall does.
//!
//! So far (steps 1 and 2): the window, the first-run welcome (nothing is
//! read until a folder is chosen, as in the Avalonia app), the library list
//! kept in ~/.config/vergence/albumwall, the app menu (Add a folder…,
//! Rescan, About); the library read in the background (reading.rs), its
//! progress shown, "cannot be reached" said as the Avalonia app says it and
//! looked for again every 30 seconds. The wall is step 4.
//!
//! The window is AlbumWall's own dark one whatever the desktop's scheme
//! (question 3: the covers are the point); its bar is the server's, dark.

mod panelview;
mod playbar;
mod reading;
mod wallview;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reading::{Outcome, Reader};
use vergence_albumwall::colors::{self, Ground};
use vergence_albumwall::covers::{self, Covers};
use vergence_albumwall::libraries::{self, Libraries};
use vergence_albumwall::library::Album;
use vergence_albumwall::player::{Command, Loudness};
use vergence_albumwall::scan::Unreachable;
use vergence_albumwall::{panel, wall};
use vergence_ui::{AppCommand as C, BarStyle, Role};
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Program, Theme, Waker, Window, button, column, label, panel, row,
    space, stack,
};

const SECTION: &str = "[albumwall]";
const APP_ID: &str = "vergence-albumwall";
/// A library that cannot be reached is looked for again this often.
const RETRY: Duration = Duration::from_secs(30);

mod cmd {
    pub const ADD_FOLDER: u32 = 1;
    pub const ABOUT: u32 = 2;
    pub const RESCAN: u32 = 3;
}

mod timer {
    pub const RETRY: u64 = 1;
    /// A library no watcher covers (a share), looked at again.
    pub const POLL: u64 = 2;
    /// The status line's moment over.
    pub const SAID: u64 = 3;
}

#[derive(Debug, Clone)]
pub enum Msg {
    ChooseFolder,
    TryAgain,
    /// A tile pressed: its album unfolds (or folds, if it is the open one).
    Open(usize),
    /// A library chosen in the bottom bar's picker.
    Library(usize),
    /// Play album `.0` from its track `.1`.
    Play(usize, usize),
    PlayPause,
    Next,
    Previous,
    /// The play mode: what happens when the album ends (cycled).
    Ending,
    /// The seek bar dragged to (seconds), and let go.
    Scrub(f64),
    SeekDone,
    /// The play bar's cover pressed: the playing album opened on the wall.
    GoToPlaying,
}

/// The open album and its motion (the Avalonia app's timings).
struct Opened {
    index: usize,
    /// Its panel unfolding (330 ms), then its contents fading in (110 ms).
    unfold: vergence_widgets::motion::Tween,
    /// Its contents changing in place (another album in the same row).
    swap: Option<vergence_widgets::motion::Tween>,
    /// Its tile turning over to its back (400 ms).
    turn: vergence_widgets::motion::Tween,
}

/// The album folding away: its index, its height when open, the fold (330 ms).
struct Folding {
    index: usize,
    height: f32,
    fold: vergence_widgets::motion::Tween,
}

const UNFOLD_MS: u64 = 330;
const CONTENTS_MS: u64 = 110;
const TURN_MS: u64 = 400;

struct AlbumWall {
    libraries: Libraries,
    /// Where the library list is kept; None without a home folder (then
    /// nothing is kept between runs).
    list_path: Option<PathBuf>,
    desktop: String,
    /// The folder dialog's answer, handed over from its window.
    chosen: Arc<Mutex<Option<PathBuf>>>,
    waker: Option<Waker>,
    /// A problem to show (a folder that could not be kept).
    problem: Option<String>,
    reader: Reader,
    /// The folder the reader is reading (or last read).
    reading: Option<PathBuf>,
    /// The library on the wall: its folder, albums and files.
    shown: Option<(PathBuf, Rc<Vec<Album>>, usize)>,
    /// Decoded covers (started with the window's waker).
    covers: Option<Arc<Covers>>,
    /// Device pixels per point (covers are decoded for it).
    scale: f32,
    /// The library's hue (its folder and the hue), measured from its covers.
    hue: Arc<Mutex<Option<(PathBuf, f64)>>>,
    /// The open album, and one folding away.
    opened: Option<Opened>,
    folding: Option<Folding>,
    /// How far down the wall is scrolled (told by the toolkit), and a glide
    /// to center an album as it opens: from, to, its timing.
    wall_offset: f32,
    glide: Option<(f32, f32, vergence_widgets::motion::Tween)>,
    /// The window's size in points.
    size: (f32, f32),
    /// The covers' fade-ins.
    fades: Rc<wallview::Fades>,
    /// The screen's text factor, as last laid out.
    text: std::cell::Cell<f32>,
    /// The player (started with the window's waker), the album playing
    /// (its library and index on the wall), the play mode, the albums a
    /// random play has had, the seek bar while dragged.
    player: Option<vergence_albumwall::player::Player>,
    playing: Option<(PathBuf, usize)>,
    ending: playbar::Ending,
    played: std::collections::HashSet<usize>,
    scrub: Option<f64>,
    /// Where it was left last time, to pick up from once the library is read.
    resume: Option<Session>,
    /// A word for the status line for a moment (the play mode just chosen).
    said: Option<(String, std::time::Instant)>,
    /// The meters' needles and when they last moved (VU ballistics).
    needles: std::cell::Cell<([f32; 2], std::time::Instant)>,
    /// The library on the wall cannot be reached, and why (and how many
    /// tracks it held last time).
    unreachable: Option<(Unreachable, usize)>,
    /// Ask for the retry timer (set where no window is at hand).
    retry_wanted: bool,
    /// The library on the wall watched for changes (None: a share, looked
    /// at again every 15 minutes instead).
    watcher: Option<(PathBuf, Option<vergence_albumwall::watch::Watcher>)>,
    poll_wanted: bool,
    /// When an unwatched library was last looked at again on the window
    /// coming to the front (at most once a minute).
    looked: Option<std::time::Instant>,
}

impl AlbumWall {
    fn new() -> AlbumWall {
        let list_path = libraries::config_dir().map(|d| d.join("libraries"));
        let libraries = list_path.as_deref().map(Libraries::load).unwrap_or_default();
        let desktop = vergence_widgets::prefs::desktop_path()
            .map(|p| vergence_config::effective_text_with(Some(&p)))
            .unwrap_or_default();
        AlbumWall {
            libraries,
            list_path,
            desktop,
            chosen: Arc::default(),
            waker: None,
            problem: None,
            reader: Reader::default(),
            reading: None,
            shown: None,
            unreachable: None,
            retry_wanted: false,
            watcher: None,
            poll_wanted: false,
            looked: None,
            covers: None,
            scale: 1.0,
            hue: Arc::default(),
            opened: None,
            folding: None,
            wall_offset: 0.0,
            glide: None,
            size: (1444.0, 1080.0),
            fades: Rc::default(),
            text: std::cell::Cell::new(1.0),
            player: None,
            playing: None,
            ending: playbar::Ending::Stop,
            played: std::collections::HashSet::new(),
            scrub: None,
            needles: std::cell::Cell::new(([0.0; 2], std::time::Instant::now())),
            said: None,
            resume: Session::load(),
        }
    }

    fn choose_folder(&mut self, window: &mut Window) {
        let chosen = self.chosen.clone();
        let waker = self.waker.clone();
        let d = vergence_widgets::filedialog::FileDialog::choose_folder(
            APP_ID,
            "Choose your music folder",
            self.theme(),
            move |paths| {
                if let Some(p) = paths.into_iter().next() {
                    *chosen.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
                    if let Some(w) = &waker {
                        w.wake();
                    }
                }
            },
        );
        window.open_window(vergence_widgets::window(d));
    }

    /// A folder the dialog chose: added, put on the wall, kept, read.
    fn take_chosen(&mut self) -> bool {
        let Some(folder) = self.chosen.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            return false;
        };
        self.problem = match self.libraries.add_folder(&folder) {
            Ok(()) => match &self.list_path {
                Some(p) => self.libraries.save(p).err().map(|e| format!("The library list could not be kept: {e}")),
                None => None,
            },
            Err(why) => Some(why),
        };
        self.read(false);
        true
    }

    /// The library on the wall read (`rescan`: every file opened). A read of
    /// another library under way is stopped; this one starts when it ends.
    fn read(&mut self, rescan: bool) {
        let Some(root) = self.libraries.current().map(|l| l.folder.clone()) else { return };
        if self.reader.running() {
            if self.reading.as_ref() != Some(&root) {
                self.reader.stop();
            }
            return;
        }
        self.reading = Some(root.clone());
        self.reader.start(root, rescan, self.waker.clone());
    }

    /// A finished read taken in: the wall, or "cannot be reached" (a wall of
    /// this library already shown stays: its music is only out of reach).
    fn take_outcome(&mut self) -> bool {
        let Some(outcome) = self.reader.take() else { return false };
        let current = self.libraries.current().map(|l| l.folder.clone());
        let Some(root) = self.reading.clone() else { return true };
        if current.as_ref() != Some(&root) {
            // Another library was chosen meanwhile: read that one now.
            self.read(false);
            return true;
        }
        match outcome {
            Outcome::Albums { albums, files } => {
                self.watch(&root);
                self.measure_hue(&root, &albums);
                if self.shown.as_ref().is_none_or(|s| s.0 != root) {
                    self.opened = None;
                    self.folding = None;
                } else if let (Some(o), Some((_, before, _))) = (self.opened.as_mut(), self.shown.as_ref()) {
                    // The same library read again: the open album found by
                    // what it is (an album added before it moves it along).
                    let was = before.get(o.index).map(|a| (a.album_artist.clone(), a.title.clone()));
                    match was.and_then(|w| albums.iter().position(|a| (a.album_artist.clone(), a.title.clone()) == w)) {
                        Some(i) => o.index = i,
                        None => self.opened = None,
                    }
                }
                self.shown = Some((root, Rc::new(albums), files));
                self.unreachable = None;
            }
            Outcome::Unreachable { why, held } => {
                if self.shown.as_ref().is_some_and(|s| s.0 != root) {
                    self.shown = None;
                }
                self.unreachable = Some((why, held));
                self.retry_wanted = true;
            }
        }
        true
    }

    /// The first run: where is your music? (The Avalonia app's welcome; the
    /// music server's half comes with Navidrome in milestone 2.)
    fn welcome(&self, t: &Theme) -> Element<Msg> {
        let card = panel(
            column(vec![
                label("On this computer").size(15.0).weight(600),
                label("A folder, an external drive, or a share on your network.").size(13.0).dim().wrap(),
                button(label("Choose a folder…")).look(ButtonLook::Normal).on_press(Msg::ChooseFolder),
            ])
            .spacing(14.0),
        )
        .padding(Padding::hv(20.0, 22.0))
        .radius(10.0)
        .width(Length::Fixed(420.0));
        let mut lines = vec![
            label("AlbumWall").size(40.0).weight(700).align(Align::Center).width(Length::FILL),
            label("A wall of every album you own. Where is your music?")
                .size(17.0)
                .align(Align::Center)
                .width(Length::FILL)
                .wrap(),
            card,
        ];
        if let Some(p) = &self.problem {
            lines.push(label(p.clone()).size(13.0).color(t.accent).wrap());
        }
        centered(column(lines).spacing(22.0).align(Align::Center))
    }

    /// Reading, with no wall yet: "Reading your library" and a true fraction.
    fn reading_view(&self, t: &Theme) -> Element<Msg> {
        let lib = self.libraries.current().expect("a library");
        let p = self.reader.progress().unwrap_or_default();
        let (done, total) = (p.done.min(p.total), p.total);
        let words = if total == 0 { "Looking for music…".to_string() } else { format!("{done} of {total} files") };
        // The bar: the part done in the accent, the rest a faint track.
        let share = |n: usize| ((n * 1000) / total.max(1)).max(1) as u16;
        let bar = row(vec![
            panel(space()).height(Length::Fixed(4.0)).width(Length::Fill(share(done))).radius(2.0).color(t.accent),
            panel(space())
                .height(Length::Fixed(4.0))
                .width(Length::Fill(share(total.saturating_sub(done))))
                .radius(2.0)
                .color(t.line),
        ])
        .width(Length::Fixed(360.0));
        centered(
            column(vec![
                label("Reading your library").size(17.0).align(Align::Center).width(Length::FILL),
                label(lib.folder.to_string_lossy().into_owned())
                    .size(13.0)
                    .dim()
                    .align(Align::Center)
                    .width(Length::FILL),
                bar,
                label(words).size(13.0).dim().align(Align::Center).width(Length::FILL),
            ])
            .spacing(12.0)
            .align(Align::Center),
        )
    }

    /// The library cannot be reached, in the Avalonia app's words.
    fn unreachable_view(&self, why: &Unreachable, held: usize) -> Element<Msg> {
        let lib = self.libraries.current().expect("a library");
        let root = lib.folder.to_string_lossy().into_owned();
        let (title, text, choose) = if held > 0 {
            let after = " Nothing about the library has been forgotten, and it is looked for again every 30 seconds.";
            let text = match why {
                Unreachable::Empty => format!(
                    "{root} is there but empty, and last time it held {held} tracks. That usually means the drive or share behind it is not connected or not mounted.{after} If the music really has gone, Rescan, in the menu, will believe it."
                ),
                Unreachable::Failed(_) => format!("{root} stopped answering while it was being read.{after}"),
                Unreachable::Missing => format!(
                    "{root} is not there right now. If it lives on a NAS or a drive, that may be asleep, unplugged or not mounted yet.{after}"
                ),
            };
            (format!("{} cannot be reached", lib.name), text, "Choose another folder…")
        } else {
            (
                "No music here yet".to_string(),
                format!(
                    "{root} cannot be reached. If it is a drive or a share, check that it is connected; otherwise point the app at wherever your records live."
                ),
                "Choose your music folder…",
            )
        };
        centered(
            column(vec![
                label(title).size(21.0).weight(600).align(Align::Center).width(Length::FILL),
                label(text).size(13.0).dim().wrap().align(Align::Center).max_width(620.0),
                row(vec![
                    button(label("Try again")).look(ButtonLook::Normal).on_press(Msg::TryAgain),
                    button(label(choose)).look(ButtonLook::Normal).on_press(Msg::ChooseFolder),
                ])
                .spacing(10.0),
            ])
            .spacing(14.0)
            .align(Align::Center),
        )
    }

    /// The library read: its wall over its ground, and the bottom bar.
    fn library_view(&self, albums: &Rc<Vec<Album>>, files: usize, t: &Theme) -> Element<Msg> {
        let g = self.ground();
        self.fades.reduced.set(t.reduced_motion);
        let tracks: usize = albums.iter().map(|a| a.tracks.len()).sum();
        let mut counts = format!("{} albums · {} tracks", group(albums.len()), group(tracks));
        if files > tracks {
            counts.push_str(&format!(" · {} not played", group(files - tracks)));
        }
        let status = if self.unreachable.is_some() {
            let name = self.libraries.current().map(|l| l.name.clone()).unwrap_or_default();
            format!("{name} cannot be reached · showing it as it was")
        } else if self.reader.running() {
            match self.reader.progress() {
                Some(p) if p.opened > 0 => format!("rescanning · {}", group(p.opened)),
                _ => "rescanning".into(),
            }
        } else {
            String::new()
        };
        let state = self.player.as_ref().map(|p| p.state()).unwrap_or_default();
        let status = match (&state.problem, &self.said) {
            (_, Some((w, at))) if at.elapsed() < std::time::Duration::from_secs(2) => w.clone(),
            (Some(p), _) if status.is_empty() => p.clone(),
            _ => status,
        };
        let names: Vec<String> = self.libraries.list.iter().map(|l| l.name.clone()).collect();
        // The play bar while an album of this library is playing (or paused).
        let bar = match (&state.now, &self.playing) {
            (Some(n), Some((root, a))) if Some(root) == self.libraries.current().map(|l| &l.folder) && n.tag == *a => {
                albums.get(*a).map(|album| {
                    let shown = playbar::Shown {
                        album,
                        track: n.track,
                        seconds: n.seconds,
                        length: n.length,
                        paused: n.paused,
                        ending: self.ending,
                        levels: self.needles(if n.paused { [0.0; 2] } else { state.levels }),
                        scrub: self.scrub,
                    };
                    playbar::bar(&shown, &self.palette_of(album), self.covers.as_ref(), self.scale)
                })
            }
            _ => None,
        };
        let foot = if bar.is_some() { playbar::HEIGHT } else { 0.0 };
        let page = column(vec![
            wallview::wall(
                albums.clone(),
                self.covers.clone(),
                self.scale,
                t,
                g,
                self.open_view(albums, t),
                self.fades.clone(),
                foot,
            )
            .height(Length::FILL),
            wallview::bar(status, counts, names, self.libraries.current, &g),
        ]);
        let Some(bar) = bar else { return page.width(Length::FILL).height(Length::FILL) };
        // The play bar floats over the wall's foot, just above the bottom bar.
        stack(vec![
            page.width(Length::FILL).height(Length::FILL),
            column(vec![space().height(Length::FILL), bar, space().height(Length::Fixed(wallview::BAR))])
                .width(Length::FILL)
                .height(Length::FILL),
        ])
        .width(Length::FILL)
        .height(Length::FILL)
    }

    /// The wall's colors: the library's hue (the fallback until it is
    /// measured) at Light 21, Tint 2 and the ink bars (Preferences, step 8).
    fn ground(&self) -> Ground {
        let root = self.libraries.current().map(|l| l.folder.clone());
        let hue = self
            .hue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .filter(|(r, _)| Some(r) == root.as_ref())
            .map_or(colors::FALLBACK_HUE, |(_, h)| *h);
        Ground::new(hue, colors::DEFAULT_LIGHT, colors::DEFAULT_TINT, &colors::CHROMES[colors::DEFAULT_CHROME])
    }

    /// The library's hue from 200 of its covers, spread across it, measured
    /// on a thread of its own (Ground.HueOf); the window woken when known.
    fn measure_hue(&self, root: &std::path::Path, albums: &[Album]) {
        let arts: Vec<_> =
            colors::spread(albums.len(), 200).into_iter().filter_map(|i| albums[i].front.clone()).collect();
        let (hue, waker, root) = (self.hue.clone(), self.waker.clone(), root.to_path_buf());
        let _ = std::thread::Builder::new().name("hue".into()).spawn(move || {
            let decode = covers::helpers();
            let sums: Vec<_> = arts
                .iter()
                .filter_map(|art| {
                    let (w, h, rgba) = decode(&covers::Key::of(art, covers::BUCKETS[0]))?;
                    Some(covers::cover_of(w, h, rgba).hue)
                })
                .collect();
            let (h, _) = colors::library_hue(&sums);
            *hue.lock().unwrap_or_else(|e| e.into_inner()) = Some((root, h));
            if let Some(w) = &waker {
                w.wake();
            }
        });
    }
}

/// 4581 as "4,581".
fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Where AlbumWall was left: the library, the album (by what it is), its
/// track and how far in. `~/.config/vergence/albumwall/session`, a few
/// lines of our own.
#[derive(Debug, Clone, PartialEq)]
struct Session {
    folder: PathBuf,
    album_artist: String,
    album: String,
    track: usize,
    seconds: f64,
}

impl Session {
    fn path() -> Option<PathBuf> {
        libraries::config_dir().map(|d| d.join("session"))
    }

    fn load() -> Option<Session> {
        let text = std::fs::read_to_string(Session::path()?).ok()?;
        let mut l = text.lines().skip_while(|l| l.starts_with('#'));
        Some(Session {
            folder: PathBuf::from(l.next()?),
            album_artist: l.next()?.to_string(),
            album: l.next()?.to_string(),
            track: l.next()?.parse().ok()?,
            seconds: l.next()?.parse().ok()?,
        })
    }

    fn save(&self) {
        let Some(p) = Session::path() else { return };
        if [self.album_artist.as_str(), self.album.as_str()].iter().any(|s| s.contains('\n'))
            || self.folder.to_string_lossy().contains('\n')
        {
            return;
        }
        let text = format!(
            "# AlbumWall for Vergence: where it was left (library, album artist, album, track, seconds)\n{}\n{}\n{}\n{}\n{:.1}\n",
            self.folder.to_string_lossy(),
            self.album_artist,
            self.album,
            self.track,
            self.seconds
        );
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = p.with_extension("new");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

impl AlbumWall {
    /// Where it is now, to pick up from next time.
    fn session(&self) -> Option<Session> {
        let (root, a) = self.playing.as_ref()?;
        let (shown_root, albums, _) = self.shown.as_ref()?;
        let album = albums.get(*a).filter(|_| shown_root == root)?;
        let now = self.player.as_ref()?.state().now.filter(|n| n.tag == *a)?;
        Some(Session {
            folder: root.clone(),
            album_artist: album.album_artist.clone(),
            album: album.title.clone(),
            track: now.track,
            seconds: now.seconds,
        })
    }

    fn keep_session(&self) {
        if let Some(s) = self.session() {
            s.save();
        }
    }

    /// Picking up where it was left, once, when its library is first on
    /// the wall: the album open, paused at the track and second it was at.
    fn resume(&mut self) {
        let Some(s) = self.resume.take() else { return };
        let Some((root, albums, _)) = &self.shown else {
            self.resume = Some(s);
            return;
        };
        if *root != s.folder {
            return;
        }
        let Some(a) = albums.iter().position(|x| x.album_artist == s.album_artist && x.title == s.album) else {
            return;
        };
        let album = &albums[a];
        if s.track >= album.tracks.len() {
            return;
        }
        let tracks = album.tracks.iter().map(|t| t.path.clone()).collect();
        self.playing = Some((root.clone(), a));
        self.send(Command::Load { tracks, from: s.track, tag: a, loudness: Loudness::Album, seconds: s.seconds });
        self.press(a);
    }

    fn send(&self, c: Command) {
        if let Some(p) = &self.player {
            p.send(c);
        }
    }

    /// Album `a` played from its track `t`.
    fn play(&mut self, a: usize, t: usize) {
        let Some((root, albums, _)) = &self.shown else { return };
        let Some(album) = albums.get(a) else { return };
        let tracks = album.tracks.iter().map(|t| t.path.clone()).collect();
        let loudness = if self.ending == playbar::Ending::RandomAlbum { Loudness::Track } else { Loudness::Album };
        self.playing = Some((root.clone(), a));
        self.played.insert(a);
        self.send(Command::Play { tracks, from: t, tag: a, loudness });
    }

    /// An album played to its end: what the play mode says comes next (the
    /// wall opening it as it starts).
    fn album_ended(&mut self) {
        let Some(p) = &self.player else { return };
        let ended = {
            let mut s = p.state.lock().unwrap_or_else(|e| e.into_inner());
            s.ended.take()
        };
        let (Some(tag), Some((_, albums, _))) = (ended, &self.shown) else { return };
        let n = albums.len();
        if n == 0 {
            return;
        }
        let next = match self.ending {
            playbar::Ending::Stop => None,
            playbar::Ending::NextAlbum => Some((tag + 1) % n),
            playbar::Ending::RandomAlbum => {
                // No album again until every one has played.
                if self.played.len() >= n {
                    self.played.clear();
                }
                let unplayed: Vec<usize> = (0..n).filter(|i| !self.played.contains(i)).collect();
                let seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.subsec_nanos() as usize);
                unplayed.get(seed % unplayed.len().max(1)).copied()
            }
        };
        if let Some(a) = next {
            self.play(a, 0);
            if self.opened.as_ref().is_none_or(|o| o.index != a) {
                self.press(a);
            }
        }
    }

    /// The track playing of album `a` on the wall, if it is the album playing.
    fn playing_track(&self, a: usize) -> Option<usize> {
        let (root, playing) = self.playing.as_ref()?;
        let now = self.player.as_ref()?.state().now?;
        (*playing == a && now.tag == a && Some(root) == self.libraries.current().map(|l| &l.folder))
            .then_some(now.track)
    }

    /// The meters' needles: toward the levels heard, with a VU meter's
    /// 300 ms ballistics.
    fn needles(&self, target: [f32; 2]) -> [f32; 2] {
        let (mut v, then) = self.needles.get();
        let now = std::time::Instant::now();
        let k = (now.saturating_duration_since(then).as_secs_f32() / 0.3).min(1.0);
        for c in 0..2 {
            v[c] += (target[c] - v[c]) * k;
        }
        self.needles.set((v, now));
        v
    }

    fn reduced(&self) -> bool {
        self.theme().reduced_motion
    }

    /// An album's palette: from its cover's colors once decoded, else the neutral.
    fn palette_of(&self, album: &Album) -> colors::Palette {
        album
            .front
            .as_ref()
            .and_then(|art| {
                let px = (wall::columns(self.wall_width()).1 * self.scale).ceil();
                self.covers.as_ref()?.get(&covers::Key::of(art, covers::bucket(px)))
            })
            .map_or(colors::Palette::neutral(colors::DEFAULT_LIGHT), |c| {
                colors::Palette::of(&c.colors, colors::DEFAULT_LIGHT)
            })
    }

    /// The wall's width for its tiles, as the grid will have it (the
    /// window less the wall's margins and the scroll bar's gutter).
    fn wall_width(&self) -> f32 {
        (self.size.0 - 2.0 * wall::PAD_SIDE - 10.0).max(1.0)
    }

    /// The open (or folding) album as the wall shows it this frame.
    fn open_view(&self, albums: &[Album], t: &Theme) -> Option<wallview::OpenView> {
        let now = std::time::Instant::now();
        self.text.set(t.text_factor);
        if let Some(o) = &self.opened {
            let album = albums.get(o.index)?;
            let full = panel::height(album, t.text_factor);
            let unfold = o.unfold.out(now);
            let since = now.saturating_duration_since(o.unfold.start).as_millis() as u64;
            // The contents fade in once the panel is open (and again when they swap).
            let mut shown = if unfold < 1.0 {
                0.0
            } else if self.reduced() {
                1.0
            } else {
                ((since.saturating_sub(UNFOLD_MS)) as f32 / CONTENTS_MS as f32).clamp(0.0, 1.0)
            };
            if let Some(sw) = &o.swap {
                shown = shown.min(sw.out(now));
            }
            return Some(wallview::OpenView {
                index: o.index,
                height: full * unfold,
                shown,
                palette: self.palette_of(album),
                turn: o.turn.in_out(now),
                playing: self.playing_track(o.index),
            });
        }
        let f = self.folding.as_ref()?;
        let album = albums.get(f.index)?;
        let left = 1.0 - f.fold.out(now);
        (left > 0.0).then(|| wallview::OpenView {
            index: f.index,
            height: f.height * left,
            shown: 0.0,
            palette: self.palette_of(album),
            turn: left,
            playing: None,
        })
    }

    /// Album `i` pressed: it unfolds under its row (the wall gliding to
    /// center it); pressed again, it folds; another in the same row takes
    /// the panel's place where it stands, its contents changing.
    fn press(&mut self, i: usize) {
        let reduced = self.reduced();
        let Some((_, albums, _)) = &self.shown else { return };
        let Some(album) = albums.get(i) else { return };
        let text = self.text.get();
        let (cols, cover) = wall::columns(self.wall_width());
        let tile_h = wall::tile_height(cover, text);
        let full = panel::height(album, text);
        let tween = |ms| vergence_widgets::motion::Tween::new(ms, reduced);
        if let Some(o) = self.opened.take() {
            if o.index == i {
                self.folding = Some(Folding {
                    index: i,
                    height: albums.get(i).map_or(0.0, |a| panel::height(a, text)),
                    fold: tween(UNFOLD_MS),
                });
                return;
            }
            if o.index / cols == i / cols {
                self.opened = Some(Opened {
                    index: i,
                    unfold: o.unfold,
                    swap: Some(tween(CONTENTS_MS * 2)),
                    turn: tween(TURN_MS),
                });
                return;
            }
        }
        self.folding = None;
        self.opened = Some(Opened { index: i, unfold: tween(UNFOLD_MS), swap: None, turn: tween(TURN_MS) });
        // The album and its panel centered in the wall, as far as they fit.
        let row_top = wall::PAD_TOP + (i / cols) as f32 * (tile_h + wall::GAP);
        let view_h = (self.size.1 - wallview::BAR).max(1.0);
        let block = tile_h + wall::GAP + full;
        let target = (row_top - ((view_h - block) / 2.0).max(0.0)).max(0.0);
        self.glide = Some((self.wall_offset, target, tween(UNFOLD_MS)));
    }

    /// Escape: the open album folds.
    fn fold(&mut self) -> bool {
        let Some(o) = &self.opened else { return false };
        self.press(o.index);
        true
    }

    /// The retry timer asked for when a read found the library out of reach;
    /// the 15-minute look for a library no watcher covers.
    fn ask_retry(&mut self, window: &mut Window) {
        if std::mem::take(&mut self.retry_wanted) {
            window.wake_after(timer::RETRY, RETRY);
        }
        if std::mem::take(&mut self.poll_wanted) {
            window.wake_after(timer::POLL, vergence_albumwall::watch::POLL);
        }
    }

    /// `root` watched (once per library on the wall): changes read after
    /// they settle; a share, looked at again every 15 minutes.
    fn watch(&mut self, root: &std::path::Path) {
        if self.watcher.as_ref().is_some_and(|(r, _)| r == root) {
            return;
        }
        let waker = self.waker.clone();
        let w = vergence_albumwall::watch::Watcher::start(root, move || {
            if let Some(w) = &waker {
                w.wake();
            }
        });
        self.poll_wanted = w.is_none();
        self.watcher = Some((root.to_path_buf(), w));
    }

    /// Whether the watched library changed (taken once).
    fn library_changed(&self) -> bool {
        self.watcher.as_ref().and_then(|(_, w)| w.as_ref()).is_some_and(|w| w.take_changed())
    }
}

/// `child` in the middle of the window.
fn centered(child: Element<Msg>) -> Element<Msg> {
    panel(
        column(vec![space().height(Length::FILL), child, space().height(Length::FILL)])
            .align(Align::Center)
            .width(Length::FILL)
            .height(Length::FILL),
    )
    .padding(26.0)
    .radius(0.0)
    .width(Length::FILL)
    .height(Length::FILL)
}

impl Program for AlbumWall {
    type Message = Msg;

    fn title(&self) -> String {
        "AlbumWall".into()
    }

    fn app_id(&self) -> String {
        APP_ID.into()
    }

    /// AlbumWall's dark window whatever the desktop's scheme; the desktop's
    /// fonts, text size, accent, contrast and motion kept.
    fn theme(&self) -> Theme {
        let mut t = Theme::from_desktop(&self.desktop, true, SECTION);
        if !t.dark {
            let d = Theme::new(true, t.accent, t.high_contrast);
            t = Theme {
                dark: true,
                sidebar: d.sidebar,
                page: d.page,
                card: d.card,
                line: d.line,
                dim: d.dim,
                text: d.text,
                hover: d.hover,
                ..t
            };
        }
        // The library on the wall: its ground and text are the window's.
        if self.shown.is_some() {
            let g = self.ground();
            t.page = wallview::color(g.ground);
            t.text = wallview::color(g.text);
            t.dim = wallview::color(g.text_dim);
        }
        t
    }

    fn bar_style(&self) -> Option<BarStyle> {
        Some(BarStyle::Dark)
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        let Some(lib) = self.libraries.current() else { return self.welcome(t) };
        if let Some((root, albums, files)) = &self.shown
            && *root == lib.folder
        {
            return self.library_view(albums, *files, t);
        }
        match &self.unreachable {
            Some((why, held)) if !self.reader.running() => self.unreachable_view(why, *held),
            _ => self.reading_view(t),
        }
    }

    fn update(&mut self, message: Msg, window: &mut Window) {
        match message {
            Msg::ChooseFolder => self.choose_folder(window),
            Msg::TryAgain => self.read(false),
            Msg::Open(i) => self.press(i),
            Msg::Play(album, track) => self.play(album, track),
            Msg::PlayPause => {
                if let Some(p) = &self.player {
                    match p.state().now {
                        Some(n) => {
                            p.send(Command::Pause(!n.paused));
                            self.keep_session();
                        }
                        None => {
                            if let Some((_, a)) = self.playing {
                                self.play(a, 0);
                            }
                        }
                    }
                }
            }
            Msg::Next => self.send(Command::Next),
            Msg::Previous => self.send(Command::Previous),
            Msg::Ending => {
                self.ending = self.ending.next();
                self.said = Some((self.ending.words().to_string(), std::time::Instant::now()));
                window.wake_after(timer::SAID, std::time::Duration::from_secs(2));
            }
            Msg::Scrub(v) => self.scrub = Some(v),
            Msg::SeekDone => {
                if let Some(v) = self.scrub.take() {
                    self.send(Command::Seek(v));
                }
            }
            Msg::GoToPlaying => {
                if let Some((_, a)) = self.playing
                    && self.opened.as_ref().is_none_or(|o| o.index != a)
                {
                    self.press(a);
                }
            }
            Msg::Library(i) => {
                if i < self.libraries.list.len() && i != self.libraries.current {
                    self.libraries.current = i;
                    if let Some(p) = &self.list_path {
                        let _ = self.libraries.save(p);
                    }
                    self.read(false);
                }
            }
        }
        self.ask_retry(window);
    }

    /// The window coming to the front: a library no watcher covers (a
    /// share; any library on FreeBSD for now) is looked at again, at most
    /// once a minute (Dave, 2026-11-25: "the lean is fine").
    fn saw(&mut self, input: &vergence_widgets::Input) {
        let unwatched = self.watcher.as_ref().is_some_and(|(_, w)| w.is_none());
        if matches!(input, vergence_widgets::Input::Focus(true))
            && unwatched
            && self.looked.is_none_or(|t| t.elapsed() >= Duration::from_secs(60))
        {
            self.looked = Some(std::time::Instant::now());
            self.read(false);
        }
    }

    fn scale(&mut self, scale: f32) -> bool {
        let changed = (scale - self.scale).abs() > 0.001;
        self.scale = scale.max(0.25);
        changed
    }

    fn waker(&mut self, waker: Waker) {
        // Covers decoded on two workers; each one ready wakes the window.
        let w = waker.clone();
        self.covers = Some(Arc::new(Covers::new(2, covers::helpers(), move || w.wake())));
        // The player on a thread of its own; it wakes the window as it plays.
        let (track, output) = vergence_albumwall::player::system();
        let w = waker.clone();
        self.player = Some(vergence_albumwall::player::Player::new(track, output, move || w.wake()));
        self.waker = Some(waker);
        // The library on the wall last time, read as the window opens.
        self.read(false);
    }

    fn woken(&mut self, window: &mut Window) -> bool {
        let chosen = self.take_chosen();
        let read = self.take_outcome();
        if read {
            self.resume();
        }
        self.album_ended();
        if self.library_changed() {
            self.read(false);
        }
        self.ask_retry(window);
        // Progress while a read is under way; covers and the hue as they come.
        chosen || read || self.reader.running() || self.shown.is_some()
    }

    fn timer(&mut self, id: u64, window: &mut Window) -> bool {
        if id == timer::RETRY && self.unreachable.is_some() {
            self.read(false);
        }
        if id == timer::POLL {
            self.read(false);
            self.poll_wanted = true;
        }
        self.ask_retry(window);
        true
    }

    fn commands(&self) -> Option<(bool, Vec<C>)> {
        Some((
            false,
            vec![
                C::own(cmd::ADD_FOLDER, "Library", "Add a folder…", "").featured(),
                C::own(cmd::RESCAN, "Library", "Rescan the library", ""),
                C::role(cmd::ABOUT, Role::About),
            ],
        ))
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        match id {
            cmd::ADD_FOLDER => self.choose_folder(window),
            cmd::RESCAN => self.read(true),
            cmd::ABOUT => {
                let about = vergence_widgets::about::About::new(
                    APP_ID,
                    "AlbumWall for Vergence",
                    env!("CARGO_PKG_VERSION"),
                    "A wall of every album you own, played gaplessly at each file's own rate.",
                    self.theme(),
                );
                window.open_window(vergence_widgets::window(about));
            }
            _ => {}
        }
        window.redraw();
    }

    fn closed(&mut self) {
        self.keep_session();
    }

    fn session_ending(&mut self) {
        self.keep_session();
    }

    fn close_asked(&mut self, _window: &mut Window) -> bool {
        self.keep_session();
        true
    }

    fn resized(&mut self, width: f32, height: f32) {
        self.size = (width, height);
    }

    fn scrolled(&mut self, name: &str, offset: f32) {
        if name == "wall" {
            self.wall_offset = offset;
        }
    }

    /// The wall gliding to center an album that opened.
    fn scroll_to(&mut self) -> Vec<(String, f32)> {
        let now = std::time::Instant::now();
        let Some((from, to, t)) = &self.glide else { return Vec::new() };
        let at = vergence_widgets::motion::mix(*from, *to, t.out(now));
        if !t.running(now) {
            self.glide = None;
        }
        vec![("wall".into(), at)]
    }

    fn input(&mut self, input: &vergence_widgets::Input, _window: &mut Window) -> bool {
        match input {
            vergence_widgets::Input::Key { sym, pressed: true, .. } if *sym == vergence_ui::sym::ESCAPE => self.fold(),
            _ => false,
        }
    }

    fn animating(&self) -> bool {
        let now = std::time::Instant::now();
        // A little past each motion's end, so the last frame drawn is the
        // finished one (not one from just before it: a folded tile kept its ring).
        let past = std::time::Duration::from_millis(60);
        let busy = |t: &vergence_widgets::motion::Tween| now.saturating_duration_since(t.start) < t.duration + past;
        let opening = self.opened.as_ref().is_some_and(|o| {
            busy(&o.turn)
                || o.swap.as_ref().is_some_and(busy)
                || now.saturating_duration_since(o.unfold.start)
                    < o.unfold.duration
                        + past
                        + std::time::Duration::from_millis(if self.reduced() { 0 } else { CONTENTS_MS })
        });
        opening
            || self.folding.as_ref().is_some_and(|f| busy(&f.fold))
            || self.glide.is_some()
            || self.fades.fading(now)
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { min: (640.0, 420.0), ..Default::default() }
    }
}

fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("vergence-albumwall {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    // `--scan FOLDER [--rescan]`: the library read without the window, its
    // albums printed with the time it took (measuring a scan; checking one).
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `--facts FILE`: what Hymie's helper reads of one file, or why it refuses it.
    if let Some(i) = args.iter().position(|a| a == "--facts") {
        let Some(file) = args.get(i + 1) else {
            eprintln!("vergence-albumwall: --facts needs a file");
            std::process::exit(2);
        };
        let result = vergence_hymie::helper::Scanner::new().and_then(|mut s| s.facts(std::path::Path::new(file)));
        match result {
            Ok(f) => println!("{f:#?}"),
            Err(e) => println!("refused: {e}"),
        }
        return;
    }
    // `--colors FOLDER`: the library's covers decoded (a spread of 200) and
    // its ground's hue, and a few albums' palettes, with the time it took.
    if let Some(i) = args.iter().position(|a| a == "--colors") {
        let Some(folder) = args.get(i + 1) else {
            eprintln!("vergence-albumwall: --colors needs a folder");
            std::process::exit(2);
        };
        std::process::exit(colors_and_print(std::path::Path::new(folder)));
    }
    if let Some(i) = args.iter().position(|a| a == "--scan") {
        let Some(folder) = args.get(i + 1) else {
            eprintln!("vergence-albumwall: --scan needs a folder");
            std::process::exit(2);
        };
        std::process::exit(scan_and_print(std::path::Path::new(folder), args.iter().any(|a| a == "--rescan")));
    }
    // The Avalonia app's default size.
    if let Err(e) = vergence_widgets::run(AlbumWall::new(), (1444, 1080)) {
        eprintln!("vergence-albumwall: {e}");
        std::process::exit(1);
    }
}

/// `--scan`: the albums of `folder`, one a line, then the counts and the time.
fn scan_and_print(folder: &std::path::Path, rescan: bool) -> i32 {
    use vergence_albumwall::{index, scan};
    let index_path = index::path_for(folder);
    let mut idx = index_path.as_deref().map(index::Index::load).unwrap_or_default();
    let mut scanner = match vergence_hymie::helper::Scanner::new() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("vergence-albumwall: {e}");
            return 1;
        }
    };
    let started = std::time::Instant::now();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    match scan::scan(folder, &mut idx, !rescan, false, &mut scanner, &cancel, |_| {}) {
        Ok(found) => {
            for a in &found.albums {
                let art = match &a.front {
                    Some(f) if f.embedded => format!("embedded {}x{}", f.width, f.height),
                    Some(f) => {
                        format!("{} {}x{}", f.path.file_name().unwrap_or_default().to_string_lossy(), f.width, f.height)
                    }
                    None => "no art".into(),
                };
                println!("{} — {} ({}) · {} tracks · {art}", a.album_artist, a.title, a.year, a.tracks.len());
            }
            let mut unplayed: Vec<&std::path::PathBuf> =
                idx.tracks.iter().filter(|(p, k)| p.starts_with(folder) && !k.facts.readable).map(|(p, _)| p).collect();
            unplayed.sort();
            for p in unplayed {
                println!("not played: {}", p.display());
            }
            println!(
                "{} albums, {} files, {} opened, {:.2} s",
                found.albums.len(),
                found.files,
                found.opened,
                started.elapsed().as_secs_f64()
            );
            if let Some(p) = &index_path
                && let Err(e) = idx.save(p)
            {
                eprintln!("vergence-albumwall: the index could not be kept: {e}");
            }
            0
        }
        Err(why) => {
            eprintln!("vergence-albumwall: {folder:?} cannot be read: {why:?}");
            1
        }
    }
}

/// `--colors`: the library read (with its index), then 200 of its covers
/// decoded at 256 px through the helpers, the ground's hue from them.
fn colors_and_print(folder: &std::path::Path) -> i32 {
    use vergence_albumwall::{colors, covers, index, scan};
    let index_path = index::path_for(folder);
    let mut idx = index_path.as_deref().map(index::Index::load).unwrap_or_default();
    let Ok(mut scanner) = vergence_hymie::helper::Scanner::new() else {
        eprintln!("vergence-albumwall: no sound helper");
        return 1;
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let Ok(found) = scan::scan(folder, &mut idx, true, false, &mut scanner, &cancel, |_| {}) else {
        eprintln!("vergence-albumwall: {folder:?} cannot be read");
        return 1;
    };
    let decode = covers::helpers();
    let started = std::time::Instant::now();
    let (mut sums, mut failed, mut shown) = (Vec::new(), 0, 0);
    for i in colors::spread(found.albums.len(), 200) {
        let a = &found.albums[i];
        let Some(art) = &a.front else { continue };
        let key = covers::Key::of(art, 256);
        match decode(&key) {
            Some((w, h, rgba)) => {
                let c = covers::cover_of(w, h, rgba);
                sums.push(c.hue);
                if shown < 6 {
                    shown += 1;
                    let p = colors::Palette::of(&c.colors, colors::DEFAULT_LIGHT);
                    println!(
                        "{} — {}: {}x{} {}, hue {:.0} sat {:.2}, panel {:?} text {:?} light {:?}",
                        a.album_artist,
                        a.title,
                        w,
                        h,
                        if art.embedded { "embedded" } else { "sidecar" },
                        c.colors.hue,
                        c.colors.saturation,
                        p.panel,
                        p.text,
                        p.light
                    );
                }
            }
            None => {
                failed += 1;
                println!("not decoded: {}", art.path.display());
            }
        }
    }
    let n = sums.len() + failed;
    let elapsed = started.elapsed().as_secs_f64();
    let (hue, agree) = colors::library_hue(&sums);
    let g =
        colors::Ground::new(hue, colors::DEFAULT_LIGHT, colors::DEFAULT_TINT, &colors::CHROMES[colors::DEFAULT_CHROME]);
    println!(
        "{n} covers decoded ({failed} failed) in {elapsed:.2} s ({:.0} ms each); library hue {hue:.0} (agreement {agree:.2}): ground {:?} bar {:?}",
        elapsed * 1000.0 / n.max(1) as f64,
        g.ground,
        g.bar
    );
    0
}
