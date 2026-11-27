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

mod lyricsview;
mod panelview;
mod playbar;
mod prefswin;
mod reading;
mod serverwin;
mod vumeter;
mod wallview;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use prefswin::Ask;
use reading::{Outcome, Reader};
use vergence_albumwall::colors::{self, Ground};
use vergence_albumwall::covers::{self, Covers};
use vergence_albumwall::libraries::{self, Libraries};
use vergence_albumwall::library::Album;
use vergence_albumwall::player::{Command, SHUFFLE};
use vergence_albumwall::scan::Unreachable;
use vergence_albumwall::settings::{AfterAlbum, Settings};
use vergence_albumwall::stats::{self, Stats};
use vergence_albumwall::{panel, wall};
use vergence_ui::{AppCommand as C, BarStyle, Role};
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Program, Theme, Waker, Window, button, column, label, panel, row,
    space, stack,
};

const SECTION: &str = vergence_albumwall::settings::SECTION;

/// HTTPS for a music server: the system's OpenSSL in the confined helper,
/// if it is installed.
pub fn tls() -> Option<Arc<dyn vergence_http::Connector>> {
    vergence_tls::Tls::system().map(|t| Arc::new(t) as Arc<dyn vergence_http::Connector>)
}
const APP_ID: &str = "vergence-albumwall";
/// A library that cannot be reached is looked for again this often.
const RETRY: Duration = Duration::from_secs(30);

mod cmd {
    pub const ADD_FOLDER: u32 = 1;
    pub const ABOUT: u32 = 2;
    pub const RESCAN: u32 = 3;
    pub const SETTINGS: u32 = 4;
    pub const CLOSE: u32 = 5;
    pub const PASTE: u32 = 6;
    pub const FIND: u32 = 7;
    pub const FULL_SCREEN: u32 = 8;
    pub const METERS: u32 = 9;
    pub const GO_TO_PLAYING: u32 = 10;
    pub const KEYS: u32 = 11;
    pub const ADD_SERVER: u32 = 12;
    pub const PLAY_PAUSE: u32 = 20;
    pub const NEXT: u32 = 21;
    pub const PREVIOUS: u32 = 22;
    pub const FORWARD: u32 = 23;
    pub const BACK: u32 = 24;
    /// When an album ends: AFTER + its place in AfterAlbum::ALL.
    pub const AFTER: u32 = 30;
    /// ReplayGain by album, by track, off.
    pub const GAIN: u32 = 40;
    pub const STATISTICS: u32 = 50;
    /// The libraries: LIBRARY + its place in the list.
    pub const LIBRARY: u32 = 100;
}

mod timer {
    pub const RETRY: u64 = 1;
    /// A library no watcher covers (a share), looked at again.
    pub const POLL: u64 = 2;
    /// The status line's moment over.
    pub const SAID: u64 = 3;
    /// The meters' needles stepped (30 a second, only while they move).
    pub const METERS: u64 = 4;
    /// The menu's Paste, once the search box has the keyboard.
    pub const PASTE: u64 = 5;
}

#[derive(Debug, Clone)]
pub enum Msg {
    ChooseFolder,
    /// A music server: the sign-in window.
    AddServer,
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
    /// The search box's text.
    Search(String),
    /// Show Lyrics / Show Tracks in the playing album's panel.
    Lyrics,
    /// A timed line of the lyrics clicked: the song there (seconds).
    LyricLine(f64),
}

/// The words of the song playing and the next one, read on threads of their
/// own (the Avalonia app read the next song's ahead, as the next track is
/// opened ahead: a song change finds its words waiting).
#[derive(Default)]
struct Words {
    /// Read: a song's path and its words (None: it has none).
    read: std::collections::HashMap<PathBuf, Option<Arc<vergence_albumwall::lyrics::Lyrics>>>,
    /// Being read now.
    reading: std::collections::HashSet<PathBuf>,
    /// Handed back by the reading threads.
    came: Arc<Mutex<Vec<Read>>>,
}

/// A song's words as a reading thread hands them back.
type Read = (PathBuf, Option<vergence_albumwall::lyrics::Lyrics>);

/// The open album and its motion (the Avalonia app's timings).
struct Opened {
    index: usize,
    /// Its panel unfolding (330 ms), then its contents fading in (110 ms).
    unfold: vergence_widgets::motion::Tween,
    /// Its contents changing in place (another album in the same row).
    swap: Option<vergence_widgets::motion::Tween>,
    /// Its tile turning over to its back (400 ms).
    turn: vergence_widgets::motion::Tween,
    /// Its panel sits above its row, pointing down at it: it took over the
    /// panel of the album above it in place (WallRows.PointsDown).
    down: bool,
    /// When its tile's second half of the turn began (its back's picture
    /// arrived), or the turn back began (that picture cannot be decoded).
    back_at: std::cell::Cell<Option<std::time::Instant>>,
}

/// The album folding away: its index, its height when open, the fold (330 ms).
struct Folding {
    index: usize,
    height: f32,
    fold: vergence_widgets::motion::Tween,
    down: bool,
}

const UNFOLD_MS: u64 = 330;
/// What the play bar shows: the album, its track, seconds in, its length, paused.
type BarShown = (usize, usize, f64, f64, bool);
/// The play bar sliding in or out (the Avalonia bar's TransportSlide).
const BAR_SLIDE: Duration = Duration::from_millis(620);
const CONTENTS_MS: u64 = 110;
const TURN_MS: u64 = 400;
/// The wall's albums for a search, as indexes in wall order.
type Order = Rc<Vec<usize>>;
/// The last search: its query, the library it ran on, what it found.
type Searched = (String, *const Vec<Album>, Order);
/// Shift+Left and Right: seconds back and on (the Avalonia app's SeekStep).
const SEEK_STEP: f64 = 10.0;
/// Keys vergence_ui::sym does not name: Enter on the keypad, F11.
const KP_ENTER: u32 = 0xff8d;
const F11: u32 = 0xffc8;
const F1: u32 = 0xffbe;
/// The meters' tick: 30 a second (VuMeter.cs: half the frames, the same spring).
const METER_TICK: Duration = Duration::from_millis(33);

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
    /// The choices in desktop.toml's [albumwall] (step 8), and when that
    /// file was last read.
    settings: Settings,
    stamp: Option<std::time::SystemTime>,
    /// Shared with the settings windows.
    link: prefswin::Shared,
    /// The numbers of the library on the wall, worked out when it was read.
    stats: Option<(PathBuf, Stats)>,
    /// Every song on the wall is being played shuffled; the album each one
    /// comes from is `playing`'s while it plays.
    shuffling: bool,
    played: std::collections::HashSet<usize>,
    scrub: Option<f64>,
    /// Where it was left last time, to pick up from once the library is read.
    resume: Option<Session>,
    /// A word for the status line for a moment (the play mode just chosen).
    said: Option<(String, std::time::Instant)>,
    /// The meters' needles and when they last moved. They move while music
    /// plays, in front or not (Dave, 2026-11-26: "they should work all the
    /// time"), and rest, asking for nothing, when it stops or pauses.
    needles: [vumeter::Needle; 2],
    /// The playing song's words, the next one's read ahead.
    words: Words,
    /// Show Lyrics pressed and not Show Tracks since: kept as songs and
    /// albums change, until turned off or the app closes (as the Avalonia app).
    lyrics_on: bool,
    /// The lyrics view's motion between frames.
    lyrics_motion: Rc<lyricsview::Motion>,
    stepped: std::time::Instant,
    meters_wanted: bool,
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
    /// The search box's text, and the wall's albums for it (cached by the
    /// query and the library: their indexes in wall order).
    query: String,
    order: std::cell::RefCell<Option<Searched>>,
    /// Where the wall was when a search began (the open album, the album
    /// at the top of the view), to put it back when the search is cleared:
    /// "searching is a detour, not a destination".
    before_search: Option<(Option<usize>, Option<usize>)>,
    /// The keyboard's album, and its track when it is in the open album's
    /// tracks (MainWindow.Keyboard.cs: the cursor is an album, not a control).
    keyed: Option<usize>,
    track: Option<usize>,
    /// The element with the keyboard ("search": typing in the search box),
    /// and where to put it at the next build.
    focus: Option<String>,
    focus_wanted: Option<Option<String>>,
    /// Full screen (F11), and a place to jump the wall to (no glide).
    fullscreen: bool,
    jump: Option<f32>,
    /// The menu's Paste, waiting for the search box to have the keyboard.
    paste_wanted: bool,
    /// The play bar's slide: whether it is coming or going, and since when;
    /// and what it showed last (album, track, seconds, length, paused), for
    /// its way out.
    bar_motion: std::cell::Cell<Option<(bool, std::time::Instant)>>,
    bar_last: std::cell::Cell<Option<BarShown>>,
    /// The window can be seen (bus v42): while not, the music goes on and
    /// what happens is kept, but nothing is drawn and the meters' tick stops.
    seen: bool,
}

impl AlbumWall {
    fn new() -> AlbumWall {
        let list_path = libraries::config_dir().map(|d| d.join("libraries"));
        let libraries = list_path.as_deref().map(Libraries::load).unwrap_or_default();
        let path = vergence_widgets::prefs::desktop_path();
        let desktop = path.as_ref().map(|p| vergence_config::effective_text_with(Some(p))).unwrap_or_default();
        let stamp = path.as_ref().and_then(|p| vergence_config::modified_with(Some(p)));
        let settings = Settings::from_text(&desktop);
        AlbumWall {
            settings,
            stamp,
            link: prefswin::Shared::default(),
            stats: None,
            shuffling: false,
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
            played: std::collections::HashSet::new(),
            scrub: None,
            needles: [vumeter::Needle::default(); 2],
            words: Words::default(),
            lyrics_on: false,
            lyrics_motion: Rc::default(),
            stepped: std::time::Instant::now(),
            meters_wanted: false,
            said: None,
            resume: Session::load(),
            query: String::new(),
            order: std::cell::RefCell::new(None),
            before_search: None,
            keyed: None,
            track: None,
            focus: None,
            focus_wanted: None,
            fullscreen: false,
            jump: None,
            paste_wanted: false,
            bar_motion: std::cell::Cell::new(None),
            bar_last: std::cell::Cell::new(None),
            seen: true,
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
        // A music server: its login, and HTTPS through the helper.
        let server = self
            .libraries
            .current()
            .and_then(|l| l.server.clone())
            .and_then(|login| vergence_albumwall::subsonic::Server::new(login, tls()).ok());
        self.reader.start(root, rescan, self.waker.clone(), server);
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
                // Its numbers, for Settings › Library, kept for when it is not on the wall.
                let s = Stats::of(&albums);
                stats::save(&root, &s);
                self.stats = Some((root.clone(), s));
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
        // The other half (milestone 2): a music server.
        let server = panel(
            column(vec![
                label("On a music server").size(15.0).weight(600),
                label("Navidrome, or any server that speaks Subsonic.").size(13.0).dim().wrap(),
                button(label("Sign in to a server…")).look(ButtonLook::Normal).on_press(Msg::AddServer),
            ])
            .spacing(14.0),
        )
        .padding(Padding::hv(20.0, 22.0))
        .radius(10.0)
        .width(Length::Fixed(420.0));
        let card = row(vec![card, server]).spacing(18.0);
        let mut lines = vec![
            label("AlbumWall").size(40.0).weight(700).align(Align::Center).width(Length::FILL),
            label("A wall of every album you own. Where is your music?")
                .size(17.0)
                .align(Align::Center)
                .width(Length::FILL)
                .wrap(),
            card,
            label("You can add more, and switch between them, in Settings › Library. AlbumWall never fetches anything from the internet, and changes your files only if you tell it to.")
                .size(13.0)
                .dim()
                .align(Align::Center)
                .wrap()
                .max_width(620.0),
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
        // A music server, in its own words: its address, why, looked for again.
        if let Some(login) = &lib.server {
            let why = match why {
                Unreachable::Failed(w) => w.clone(),
                _ => "no answer".into(),
            };
            let text = format!(
                "The server at {} is not answering ({why}). Nothing about the library has been forgotten, and it is looked for again every 30 seconds.",
                login.address
            );
            return centered(
                column(vec![
                    label(format!("{} cannot be reached", lib.name))
                        .size(21.0)
                        .weight(600)
                        .align(Align::Center)
                        .width(Length::FILL),
                    label(text).size(13.0).dim().wrap().align(Align::Center).max_width(620.0),
                    row(vec![button(label("Try again")).look(ButtonLook::Normal).on_press(Msg::TryAgain)])
                        .spacing(10.0),
                ])
                .spacing(14.0)
                .align(Align::Center),
            );
        }
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
        let order = self.order_of(albums);
        let status = if vergence_albumwall::search::normalize(&self.query).is_empty() {
            status
        } else {
            format!("{} of {} albums", group(order.len()), group(albums.len()))
        };
        let state = self.player.as_ref().map(|p| p.state()).unwrap_or_default();
        let status = match (&state.problem, &self.said) {
            (_, Some((w, at))) if at.elapsed() < std::time::Duration::from_secs(2) => w.clone(),
            (Some(p), _) if status.is_empty() => p.clone(),
            _ => status,
        };
        let names: Vec<String> = self.libraries.list.iter().map(|l| l.name.clone()).collect();
        // The play bar while an album of this library is playing (or paused),
        // and as it slides away: what it showed last.
        let now_bar = match (&state.now, &self.playing) {
            (Some(n), Some((root, a))) if Some(root) == self.libraries.current().map(|l| &l.folder) && n.tag == *a => {
                Some((*a, n.track, n.seconds, n.length, n.paused))
            }
            _ => None,
        };
        if now_bar.is_some() {
            self.bar_last.set(now_bar);
        }
        let shown_bar = self.bar_slide(now_bar.is_some(), t.reduced_motion);
        let bar = (shown_bar > 0.0).then(|| now_bar.or(self.bar_last.get())).flatten().and_then(
            |(a, track, seconds, length, paused)| {
                albums.get(a).map(|album| {
                    let shown = playbar::Shown {
                        album,
                        track,
                        seconds,
                        length,
                        paused,
                        ending: self.settings.after,
                        needles: [self.needles[0].x as f32, self.needles[1].x as f32],
                        meters: self.settings.meters,
                        scrub: self.scrub,
                    };
                    playbar::bar(&shown, &self.palette_of(album), self.covers.as_ref(), self.scale)
                })
            },
        );
        let foot = playbar::HEIGHT * shown_bar;
        let page = column(vec![
            wallview::wall(
                albums.clone(),
                order,
                self.keyed.filter(|_| self.track.is_none()),
                self.covers.clone(),
                self.scale,
                t,
                g,
                self.open_view(albums, t),
                self.fades.clone(),
                foot,
            )
            .height(Length::FILL),
            wallview::bar(status, &self.query, counts, names, self.libraries.current, &g),
        ]);
        // The play bar floats over the wall's foot, just above the bottom bar.
        // The tree keeps one shape whether or not it is there: the toolkit
        // knows an element by its place, and the wall moved into or out of a
        // stack was a new scroll, rebuilt from somewhere else (Dave,
        // 2026-11-26: an album started, the wall went to the top; one ended,
        // it "scrolled itself to the F section").
        // It slides up from behind the bottom bar and back down (620 ms, as
        // the Avalonia bar's TransportSlide): a clipping panel as tall as
        // the part of it out so far, the bar's top in it.
        let bar = panel(bar.unwrap_or_else(space))
            .clip()
            .radius(0.0)
            .width(Length::FILL)
            .height(Length::Fixed((playbar::HEIGHT * shown_bar).round()));
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
    /// measured) at the Light, Tint and bars chosen in Settings › Appearance
    /// (Light 21, Tint 2 and ink until then); a slider held there shows here
    /// as it moves.
    fn ground(&self) -> Ground {
        let root = self.libraries.current().map(|l| l.folder.clone());
        let hue = self
            .hue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .filter(|(r, _)| Some(r) == root.as_ref())
            .map_or(colors::FALLBACK_HUE, |(_, h)| *h);
        let s = &self.settings;
        let (light, tint) = self.link.borrow().preview.unwrap_or((s.light, s.tint));
        Ground::new(hue, light, tint, &colors::CHROMES[s.bars.min(colors::CHROMES.len() - 1)])
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

/// A number below `n` (0 for 0), from the clock and a counter: enough
/// for choosing what plays next.
fn random(n: usize) -> usize {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    if n == 0 {
        return 0;
    }
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let mut x = STATE.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed) ^ t;
    // splitmix64
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x % n as u64) as usize
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
        // Settings › Playback › Pick up where I left off.
        if !self.settings.resume {
            self.resume = None;
            return;
        }
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
        self.send(Command::Load {
            tracks,
            from: s.track,
            tag: a,
            loudness: self.settings.loudness(false),
            seconds: s.seconds,
        });
        // …and start playing straight away, if chosen.
        if self.settings.auto_play {
            self.send(Command::Pause(false));
        }
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
        let loudness = self.settings.loudness(false);
        self.shuffling = false;
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
        // The albums the wall shows: with a search in the box, only those.
        let order = self.order_of(albums);
        let n = order.len();
        if n == 0 {
            return;
        }
        let next = match self.settings.after {
            AfterAlbum::Stop => None,
            AfterAlbum::ShuffleSongs => {
                self.shuffle();
                return;
            }
            // After a shuffle, the album of its last song, on round.
            AfterAlbum::NextAlbum => {
                let at = if tag == SHUFFLE { self.playing.as_ref().map(|p| p.1) } else { Some(tag) };
                let k = at.and_then(|a| order.iter().position(|i| *i == a)).map_or(0, |k| (k + 1) % n);
                Some(order[k])
            }
            AfterAlbum::RandomAlbum => {
                // No album again until every one on the wall has played.
                if order.iter().all(|i| self.played.contains(i)) {
                    self.played.clear();
                }
                let unplayed: Vec<usize> = order.iter().copied().filter(|i| !self.played.contains(i)).collect();
                unplayed.get(random(unplayed.len())).copied()
            }
        };
        if let Some(a) = next {
            self.play(a, 0);
            if self.opened.as_ref().is_none_or(|o| o.index != a) {
                self.press(a);
            }
        }
    }

    /// The sign-in window for a music server.
    fn add_server(&mut self, window: &mut Window) {
        self.tell_link();
        window.open_window(vergence_widgets::window(serverwin::ServerWindow::new(self.link.clone(), APP_ID)));
    }

    /// Forward or back in the track playing, kept within it.
    fn seek_by(&mut self, step: f64) {
        if let Some(n) = self.player.as_ref().and_then(|p| p.state().now) {
            self.send(Command::Seek((n.seconds + step).clamp(0.0, n.length)));
        }
    }

    /// Every song the wall shows (with a search in the box, only its
    /// albums'), in a random order, each by track unless a ReplayGain
    /// choice says otherwise (MainWindow.StartSongShuffle).
    fn shuffle(&mut self) {
        let Some((root, albums, _)) = &self.shown else { return };
        let order = self.order_of(albums);
        let mut songs: Vec<(PathBuf, usize, usize)> = order
            .iter()
            .flat_map(|&a| albums[a].tracks.iter().enumerate().map(move |(t, x)| (x.path.clone(), a, t)))
            .collect();
        if songs.is_empty() {
            return;
        }
        // Fisher-Yates.
        for i in (1..songs.len()).rev() {
            songs.swap(i, random(i + 1));
        }
        let first = songs[0].1;
        self.playing = Some((root.clone(), first));
        self.shuffling = true;
        self.send(Command::Shuffle { tracks: songs, loudness: self.settings.loudness(true) });
        if self.opened.as_ref().is_none_or(|o| o.index != first) {
            self.press(first);
        }
    }

    /// In a shuffle of songs, the album of each song as it starts: the
    /// play bar shows it and the wall opens it ("the album change or song
    /// change in shuffle should move the displayed song/album right away").
    fn follow_shuffle(&mut self) {
        if !self.shuffling {
            return;
        }
        let Some(now) = self.player.as_ref().and_then(|p| p.state().now) else { return };
        let current = self.libraries.current().map(|l| l.folder.clone());
        let Some((root, a)) = self.playing.as_mut() else { return };
        // Another library on the wall: the play bar is not shown, nothing opens.
        if Some(&*root) != current.as_ref() {
            return;
        }
        if now.tag != *a && now.tag != SHUFFLE {
            *a = now.tag;
            let tag = now.tag;
            if self.opened.as_ref().is_none_or(|o| o.index != tag) {
                self.press(tag);
            }
        }
    }

    /// desktop.toml again, if it changed: the choices applied at once.
    fn reread(&mut self) -> bool {
        let Some(path) = vergence_widgets::prefs::desktop_path() else { return false };
        let stamp = vergence_config::modified_with(Some(&path));
        if stamp == self.stamp {
            return false;
        }
        self.stamp = stamp;
        self.desktop = vergence_config::effective_text_with(Some(&path));
        let before = self.settings.clone();
        self.settings = Settings::from_text(&self.desktop);
        if before.meters_after_gain != self.settings.meters_after_gain {
            self.send(Command::MetersAfterGain(self.settings.meters_after_gain));
        }
        // A mode that stops the shuffle takes effect when this song ends.
        if before.after != self.settings.after {
            self.played.clear();
        }
        true
    }

    /// A choice made on the window itself (the play bar's play mode)
    /// written to desktop.toml as Settings would write it.
    fn write_settings(&mut self, s: Settings) {
        let Some(path) = vergence_widgets::prefs::desktop_path() else { return };
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let new = s.write_into(&text);
        if new != text {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let beside = path.with_extension("toml.new");
            if let Err(e) = std::fs::write(&beside, &new).and_then(|_| std::fs::rename(&beside, &path)) {
                eprintln!("albumwall: can't write {}: {e}", path.display());
            }
        }
        self.settings = s;
        self.reread();
        self.link.borrow().tell_windows();
    }

    fn open_settings(&mut self, page: Option<prefswin::Page>, window: &mut Window) {
        let open = {
            let mut l = self.link.borrow_mut();
            if let Some(p) = page {
                l.page = p;
            }
            l.open
        };
        self.tell_link();
        if open {
            self.link.borrow().tell_windows();
        } else {
            window.open_window(vergence_widgets::window(prefswin::Prefs::new(self.link.clone(), APP_ID)));
        }
    }

    /// What the settings windows show of the wall, brought up to date.
    fn tell_link(&self) {
        // The servers signed in to, for the player to find a song's server by its path.
        vergence_albumwall::subsonic::register(self.libraries.list.iter().filter_map(|l| l.server.clone()).collect());
        let hue = self.hue.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let root = self.libraries.current().map(|l| l.folder.clone());
        let mut l = self.link.borrow_mut();
        let lines: Vec<prefswin::Line> = self
            .libraries
            .list
            .iter()
            .map(|x| prefswin::Line {
                name: x.name.clone(),
                folder: x.folder.clone(),
                server: x.server.as_ref().map(|s| s.address.clone()),
            })
            .collect();
        let reading = self.reader.running();
        let live = self.stats.clone().filter(|(f, _)| Some(f) == root.as_ref());
        let (h, own) = match hue.filter(|(r, _)| Some(r) == root.as_ref()) {
            Some((_, h)) => (h, true),
            None => (colors::FALLBACK_HUE, false),
        };
        let changed = l.libraries != lines
            || l.current != self.libraries.current
            || l.reading != reading
            || l.live != live
            || l.hue != h
            || l.hue_is_library != own;
        (l.libraries, l.current, l.reading, l.live, l.hue, l.hue_is_library) =
            (lines, self.libraries.current, reading, live, h, own);
        if changed {
            l.tell_windows();
        }
    }

    /// What the settings windows asked of the wall.
    fn take_asks(&mut self, window: &mut Window) -> bool {
        let asks = std::mem::take(&mut self.link.borrow_mut().asks);
        let any = !asks.is_empty();
        for a in asks {
            match a {
                Ask::Reread => {
                    self.reread();
                }
                Ask::SwitchTo(i) => self.update(Msg::Library(i), window),
                Ask::AddFolder => self.choose_folder(window),
                Ask::AddServers(list) => {
                    for (name, login) in list {
                        self.libraries.add_server(&name, login);
                    }
                    self.keep_libraries();
                    self.read(false);
                }
                Ask::Rescan => {
                    let files = self.shown.as_ref().map(|s| s.2);
                    self.link.borrow_mut().rescanned = files.map(|n| format!("Reading {} files again…", group(n)));
                    self.read(true);
                }
                Ask::Rename(i, name) => {
                    if let Some(l) = self.libraries.list.get_mut(i) {
                        l.name = name;
                        self.keep_libraries();
                    }
                }
                Ask::Forget(i) => self.forget(i),
                Ask::Search(q) => {
                    self.search(q.clone());
                    self.focus_wanted = Some(Some("search".into()));
                }
            }
        }
        if any {
            self.tell_link();
        }
        any
    }

    fn keep_libraries(&mut self) {
        if let Some(p) = &self.list_path
            && let Err(e) = self.libraries.save(p)
        {
            self.problem = Some(format!("The library list could not be kept: {e}"));
        }
    }

    /// A library taken off the list (nothing on disk touched). The one on
    /// the wall forgotten: the next one goes on the wall, or the welcome.
    fn forget(&mut self, i: usize) {
        if i >= self.libraries.list.len() {
            return;
        }
        let was_current = i == self.libraries.current;
        let gone = self.libraries.list.remove(i);
        // A server's kept list and covers go with it (its sign-in went with the line).
        if gone.server.is_some()
            && let Some((list, covers)) = vergence_albumwall::subsonic::cache_paths(&gone.folder)
        {
            let _ = std::fs::remove_file(list);
            let _ = std::fs::remove_dir_all(covers);
        }
        if self.libraries.current > i || self.libraries.current >= self.libraries.list.len() {
            self.libraries.current = self.libraries.current.saturating_sub(1);
        }
        self.keep_libraries();
        if was_current {
            self.send(Command::Stop);
            (self.playing, self.shown, self.opened, self.unreachable, self.shuffling) = (None, None, None, None, false);
            self.read(false);
        }
    }

    /// The paths of the song playing and the one after it, on the wall shown.
    fn playing_paths(&self) -> (Option<PathBuf>, Option<PathBuf>) {
        let found = (|| {
            let (root, a) = self.playing.as_ref()?;
            let (shown_root, albums, _) = self.shown.as_ref()?;
            let album = albums.get(*a).filter(|_| shown_root == root)?;
            let now = self.player.as_ref()?.state().now.filter(|n| n.tag == *a)?;
            let path = |i: usize| album.tracks.get(i).map(|t| t.path.clone());
            Some((path(now.track), path(now.track + 1)))
        })();
        found.unwrap_or((None, None))
    }

    /// The words of the song playing and the next one: taken as they come,
    /// read if not yet, the rest forgotten.
    fn words_for_playing(&mut self) {
        let came: Vec<_> = std::mem::take(&mut *self.words.came.lock().unwrap_or_else(|e| e.into_inner()));
        for (path, words) in came {
            self.words.reading.remove(&path);
            self.words.read.insert(path, words.map(Arc::new));
        }
        let (now, next) = self.playing_paths();
        let keep: Vec<PathBuf> = [now, next].into_iter().flatten().collect();
        self.words.read.retain(|p, _| keep.contains(p));
        for path in keep {
            if self.words.read.contains_key(&path) || !self.words.reading.insert(path.clone()) {
                continue;
            }
            let (came, waker) = (self.words.came.clone(), self.waker.clone());
            let _ = std::thread::Builder::new().name("lyrics".into()).spawn(move || {
                let words = if vergence_albumwall::subsonic::song_id(&path).is_some() {
                    vergence_albumwall::subsonic::song_lyrics(&path, tls())
                } else {
                    vergence_albumwall::lyrics::Lyrics::for_file(&path)
                };
                came.lock().unwrap_or_else(|e| e.into_inner()).push((path, words));
                if let Some(w) = &waker {
                    w.wake();
                }
            });
        }
    }

    /// The panel's words for open album `a`: the song playing's, if `a` is
    /// the album playing and the song has any.
    fn words_of(&self, a: usize) -> Option<panelview::Words> {
        self.playing_track(a)?;
        let (path, _) = self.playing_paths();
        let lyrics = self.words.read.get(&path?)?.clone()?;
        let now = self.player.as_ref()?.state().now?;
        Some(panelview::Words {
            lyrics,
            on: self.lyrics_on,
            seconds: now.seconds,
            length: now.length,
            motion: self.lyrics_motion.clone(),
            reduced: self.reduced(),
        })
    }

    /// The track playing of album `a` on the wall, if it is the album playing.
    fn playing_track(&self, a: usize) -> Option<usize> {
        let (root, playing) = self.playing.as_ref()?;
        let now = self.player.as_ref()?.state().now?;
        (*playing == a && now.tag == a && Some(root) == self.libraries.current().map(|l| &l.folder))
            .then_some(now.track)
    }

    /// The meters stepped on to now, toward the levels being heard; true
    /// while a needle still moves or music plays.
    fn step_meters(&mut self) -> bool {
        let now = std::time::Instant::now();
        let dt = now.saturating_duration_since(self.stepped).as_secs_f64();
        self.stepped = now;
        let state = self.player.as_ref().map(|p| p.state()).unwrap_or_default();
        let playing = state.now.as_ref().is_some_and(|n| !n.paused);
        let db = state.meter.dbfs(now);
        for (n, d) in self.needles.iter_mut().zip(db) {
            n.level(d);
            n.step(dt);
        }
        playing || self.needles.iter().any(|n| !n.resting())
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
                turn: self.turn_of(o, album, now),
                playing: self.playing_track(o.index),
                cursor: self.track.filter(|_| self.keyed == Some(o.index)),
                down: o.down,
                after: self.after_place(o.index, o.down),
                words: self.words_of(o.index),
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
            cursor: None,
            down: f.down,
            after: self.after_place(f.index, f.down),
            words: None,
        })
    }

    /// How far out the play bar is, 0 to 1, as it slides toward `wanted`
    /// (eased out; at once with motion reduced).
    fn bar_slide(&self, wanted: bool, reduced: bool) -> f32 {
        let now = std::time::Instant::now();
        match self.bar_motion.get() {
            Some((w, _)) if w == wanted => {}
            None if !wanted => return 0.0,
            _ => self.bar_motion.set(Some((wanted, now))),
        }
        let Some((w, at)) = self.bar_motion.get() else { return 0.0 };
        let f = if reduced {
            1.0
        } else {
            (now.saturating_duration_since(at).as_secs_f32() / BAR_SLIDE.as_secs_f32()).min(1.0)
        };
        let e = vergence_widgets::motion::ease_out(f);
        if w { e } else { 1.0 - e }
    }

    /// How far album `o`'s tile has turned toward its back, 0 to 1: the
    /// first half to edge-on, held there until the back's picture has
    /// come (the Avalonia app: "edge-on until the bitmap lands"), then the
    /// second half; a back that cannot be decoded turns the front back.
    fn turn_of(&self, o: &Opened, album: &Album, now: std::time::Instant) -> f32 {
        let first = o.turn.in_out(now);
        if first < 1.0 {
            return 0.5 * first;
        }
        let Some(back) = &album.back else { return 0.0 };
        let (_, cover) = wall::columns(self.wall_width());
        let key = covers::Key::of(back, covers::bucket((cover * self.scale).ceil()));
        let half = Duration::from_millis(TURN_MS / 2);
        let ease = |since: std::time::Instant| {
            let f = if self.reduced() {
                1.0
            } else {
                now.saturating_duration_since(since).as_secs_f32() / half.as_secs_f32()
            };
            vergence_widgets::motion::ease_in_out(f.min(1.0))
        };
        let Some(c) = self.covers.as_ref() else { return 0.5 };
        if c.failed(&key) {
            let since = o.back_at.get().unwrap_or(now);
            o.back_at.set(Some(since));
            return 0.5 * (1.0 - ease(since));
        }
        if o.back_at.get().is_none() && c.get(&key).is_some() {
            o.back_at.set(Some(now));
        }
        o.back_at.get().map_or(0.5, |since| 0.5 + 0.5 * ease(since))
    }

    /// For a panel pointing down: a place in the row above album `i`'s, the
    /// row the panel goes under.
    fn after_place(&self, i: usize, down: bool) -> Option<usize> {
        if !down {
            return None;
        }
        let (_, order) = self.wall_albums()?;
        let at = order.iter().position(|&x| x == i)?;
        let (cols, _) = self.grid();
        at.checked_sub(cols.max(1))
    }

    /// Album `i` pressed: it unfolds under its row (the wall gliding to
    /// center it); pressed again, it folds; an album in the row just above
    /// the open panel or just under it takes the panel over where it stands,
    /// its contents changing, pointing up or down at it (Dave's idea,
    /// 2026-10-08, in the Avalonia app: no rolling up, scrolling and
    /// unfolding another); anywhere else, the ordinary fold and unfold.
    fn press(&mut self, i: usize) {
        let reduced = self.reduced();
        let Some(albums) = self.shown.as_ref().map(|s| s.1.clone()) else { return };
        let Some(album) = albums.get(i) else { return };
        // An album the search does not show (the next album playing, say):
        // the search gives way, and the wall is the whole library again.
        if !self.order_of(&albums).contains(&i) {
            self.query.clear();
            self.before_search = None;
        }
        let order = self.order_of(&albums);
        let place = |i: usize| order.iter().position(|&x| x == i);
        let at = place(i).unwrap_or(i);
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
                    down: o.down,
                });
                return;
            }
            // The row the panel sits under, and whether it can be seen (a
            // panel scrolled away is folded and another unfolded as usual).
            if let Some(p) = place(o.index) {
                let panel_row = (p / cols).saturating_sub(usize::from(o.down));
                let top = wall::PAD_TOP + (panel_row + 1) as f32 * (tile_h + wall::GAP);
                let old = albums.get(o.index).map_or(0.0, |a| panel::height(a, text));
                let view_h = (self.size.1 - wallview::BAR).max(1.0);
                let seen = top < self.wall_offset + view_h && top + old > self.wall_offset;
                let row = at / cols;
                if seen && (row == panel_row || row == panel_row + 1) {
                    let down = row == panel_row + 1;
                    self.opened = Some(Opened {
                        index: i,
                        unfold: o.unfold,
                        swap: Some(tween(CONTENTS_MS * 2)),
                        turn: tween(TURN_MS / 2),
                        down,
                        back_at: std::cell::Cell::new(None),
                    });
                    // Nothing above the panel moves; pointing down, the wall
                    // scrolls only if the album's row would end below the view.
                    let bottom = top + full + wall::GAP + tile_h;
                    if down && bottom > self.wall_offset + view_h {
                        let target = (bottom - view_h + wall::GAP).min(top).max(0.0);
                        self.glide = Some((self.wall_offset, target, tween(UNFOLD_MS)));
                    }
                    return;
                }
            }
        }
        self.folding = None;
        self.opened = Some(Opened {
            index: i,
            unfold: tween(UNFOLD_MS),
            swap: None,
            turn: tween(TURN_MS / 2),
            down: false,
            back_at: std::cell::Cell::new(None),
        });
        // The album and its panel centered in the wall, as far as they fit.
        let row_top = wall::PAD_TOP + (at / cols) as f32 * (tile_h + wall::GAP);
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

    /// The wall's albums for the search box's text, as indexes in wall order.
    fn order_of(&self, albums: &Rc<Vec<Album>>) -> Rc<Vec<usize>> {
        let query = vergence_albumwall::search::normalize(&self.query);
        let key = Rc::as_ptr(albums);
        if let Some((q, k, order)) = &*self.order.borrow()
            && *q == query
            && *k == key
        {
            return order.clone();
        }
        let order = Rc::new(vergence_albumwall::search::found(albums, &query));
        *self.order.borrow_mut() = Some((query, key, order.clone()));
        order
    }

    /// The library on the wall and its albums in search order.
    fn wall_albums(&self) -> Option<(Rc<Vec<Album>>, Order)> {
        let albums = self.shown.as_ref()?.1.clone();
        let order = self.order_of(&albums);
        Some((albums, order))
    }

    /// The wall's columns and a tile's height, as laid out now.
    fn grid(&self) -> (usize, f32) {
        let (cols, cover) = wall::columns(self.wall_width());
        (cols.max(1), wall::tile_height(cover, self.text.get()))
    }

    /// The top of the row holding place `at`, the open panel above it counted.
    fn row_top(&self, at: usize, order: &[usize], albums: &[Album]) -> f32 {
        let (cols, tile_h) = self.grid();
        let mut y = wall::PAD_TOP + (at / cols) as f32 * (tile_h + wall::GAP);
        if let Some(o) = &self.opened
            && let Some(p) = order.iter().position(|&i| i == o.index)
            && (p / cols).saturating_sub(usize::from(o.down)) < at / cols
        {
            y += albums.get(o.index).map_or(0.0, |a| panel::height(a, self.text.get())) + wall::GAP;
        }
        y
    }

    /// The album at the top of the view now (by rows, near enough).
    fn top_album(&self) -> Option<usize> {
        let (_, order) = self.wall_albums()?;
        let (cols, tile_h) = self.grid();
        let row = ((self.wall_offset - wall::PAD_TOP).max(0.0) / (tile_h + wall::GAP)).floor() as usize;
        order.get((row * cols).min(order.len().saturating_sub(1))).copied()
    }

    /// The search box's text changed: the wall shows what it finds (live;
    /// the wall builds only the rows in view, so no pause for the keys).
    fn search(&mut self, text: String) {
        let was = !vergence_albumwall::search::normalize(&self.query).is_empty();
        let now = !vergence_albumwall::search::normalize(&text).is_empty();
        if now && !was && self.before_search.is_none() {
            // On the way in only: every key runs this.
            self.before_search = Some((self.opened.as_ref().map(|o| o.index), self.top_album()));
        }
        self.query = text;
        let Some((_, order)) = self.wall_albums() else { return };
        // A search that hides the open album closes it: a panel pointing at
        // a cover no longer on the wall is worse than none.
        if self.opened.as_ref().is_some_and(|o| !order.contains(&o.index)) {
            self.opened = None;
            self.folding = None;
            self.track = None;
        }
        if self.keyed.is_some_and(|k| !order.contains(&k)) {
            (self.keyed, self.track) = (None, None);
        }
        self.jump = Some(0.0);
        if !now && was {
            self.restore_after_search();
        }
    }

    /// The wall put back where the search interrupted it: the album that
    /// was open opened again (it carries its place with it), else the album
    /// that was at the top of the view at the top again.
    fn restore_after_search(&mut self) {
        let Some((open, top)) = self.before_search.take() else { return };
        let Some((albums, order)) = self.wall_albums() else { return };
        self.jump = None;
        if let Some(i) = open.filter(|&i| i < albums.len()) {
            if self.opened.as_ref().is_some_and(|o| o.index == i) {
                let at = order.iter().position(|&x| x == i).unwrap_or(i);
                self.jump = Some((self.row_top(at, &order, &albums) - wall::GAP).max(0.0));
            } else {
                self.press(i);
            }
            return;
        }
        if let Some(i) = top {
            let at = order.iter().position(|&x| x == i).unwrap_or(i);
            self.jump = Some((self.row_top(at, &order, &albums) - wall::GAP).max(0.0));
        }
    }

    /// The keyboard on album `i`: its row brought wholly into view, the wall
    /// not moving at all if it already is (walking along a row).
    fn key_to(&mut self, i: usize) {
        let Some((albums, order)) = self.wall_albums() else { return };
        let Some(at) = order.iter().position(|&x| x == i) else { return };
        (self.keyed, self.track) = (Some(i), None);
        let (_, tile_h) = self.grid();
        let top = self.row_top(at, &order, &albums);
        let view = (self.size.1 - wallview::BAR - if self.player_showing() { playbar::HEIGHT } else { 0.0 }).max(1.0);
        let at_y = self.wall_offset;
        if top < at_y {
            self.jump = Some((top - wall::GAP).max(0.0));
        } else if top + tile_h > at_y + view {
            self.jump = Some(top + tile_h - view + wall::GAP);
        }
    }

    /// The keyboard on track `t` of the open album, its line brought into view.
    fn track_to(&mut self, t: usize) {
        let Some(o) = &self.opened else { return };
        let i = o.index;
        let Some((albums, order)) = self.wall_albums() else { return };
        let Some(at) = order.iter().position(|&x| x == i) else { return };
        let Some(album) = albums.get(i) else { return };
        (self.keyed, self.track) = (Some(i), Some(t));
        let (_, tile_h) = self.grid();
        let text = self.text.get();
        // The track's line: its column's lines above it, under the panel's header.
        let (left, right) = panel::columns(album);
        let line_of = |lines: &[panel::Line]| {
            lines.iter().position(|l| matches!(l, panel::Line::Track(k) if *k == t)).map(|n| {
                lines[..n].iter().map(|l| if matches!(l, panel::Line::Disc(_)) { 30.0 } else { 0.0 }).sum::<f32>()
                    + n as f32 * (panel::LINE * text).ceil()
            })
        };
        let within = line_of(&left).or_else(|| line_of(&right)).unwrap_or(0.0);
        let header = panel::MARGIN + panel::PAD + 60.0 * text + panel::BUTTON_GAP + panel::LIST_GAP;
        let y = self.row_top(at, &order, &albums) + tile_h + wall::GAP + header + within;
        let line_h = (panel::LINE * text).ceil();
        let view = (self.size.1 - wallview::BAR - if self.player_showing() { playbar::HEIGHT } else { 0.0 }).max(1.0);
        if y < self.wall_offset {
            self.jump = Some((y - wall::GAP).max(0.0));
        } else if y + line_h > self.wall_offset + view {
            self.jump = Some(y + line_h - view + wall::GAP);
        }
    }

    /// The play bar is over the wall's foot.
    fn player_showing(&self) -> bool {
        self.player.as_ref().is_some_and(|p| p.state().now.is_some())
    }

    /// Where the keyboard starts: the open album, else the first at the top
    /// of the view.
    fn key_start(&mut self) {
        let Some((_, order)) = self.wall_albums() else { return };
        let open = self.opened.as_ref().map(|o| o.index).filter(|i| order.contains(i));
        if let Some(i) = open.or_else(|| self.top_album()).or_else(|| order.first().copied()) {
            self.key_to(i);
        }
    }

    /// The wall's keys (MainWindow.Keyboard.cs): the arrows from cover to
    /// cover, Down from the open album into its tracks, Enter to open or
    /// play, Home and End, Page Up and Down. True if the key was used.
    fn wall_key(&mut self, key: u32) -> bool {
        use vergence_ui::sym;
        let Some((albums, order)) = self.wall_albums() else { return false };
        if order.is_empty() {
            return false;
        }
        let open = self.opened.as_ref().map(|o| o.index);
        // In the open album's tracks.
        if let (Some(t), Some(k)) = (self.track, self.keyed)
            && open == Some(k)
        {
            let count = albums.get(k).map_or(0, |a| a.tracks.len());
            match key {
                sym::UP if t > 0 => self.track_to(t - 1),
                sym::UP => self.key_to(k),
                sym::DOWN if t + 1 < count => self.track_to(t + 1),
                sym::DOWN => {
                    // Off the end: on to the row under the album.
                    let (cols, _) = self.grid();
                    let at = order.iter().position(|&x| x == k).unwrap_or(0);
                    self.key_to(order[wall::row_move(at, 1, cols, order.len())]);
                }
                sym::HOME => self.track_to(0),
                sym::END => self.track_to(count.saturating_sub(1)),
                sym::RETURN | KP_ENTER => self.play(k, t),
                _ => return false,
            }
            return true;
        }
        let Some(at) = self.keyed.and_then(|k| order.iter().position(|&x| x == k)) else {
            // The first key lands somewhere sensible before it moves anything.
            if key == sym::RETURN || key == KP_ENTER {
                return false;
            }
            self.key_start();
            return true;
        };
        let (cols, tile_h) = self.grid();
        let view = (self.size.1 - wallview::BAR).max(1.0);
        let per_screen = ((view / (tile_h + wall::GAP)) as usize).max(1);
        let last = order.len() - 1;
        let k = order[at];
        match key {
            sym::LEFT => self.key_to(order[at.saturating_sub(1)]),
            sym::RIGHT => self.key_to(order[(at + 1).min(last)]),
            sym::UP => self.key_to(order[wall::row_move(at, -1, cols, order.len())]),
            sym::DOWN if open == Some(k) && albums.get(k).is_some_and(|a| !a.tracks.is_empty()) => self.track_to(0),
            sym::DOWN => self.key_to(order[wall::row_move(at, 1, cols, order.len())]),
            sym::HOME => self.key_to(order[0]),
            sym::END => self.key_to(order[last]),
            sym::PAGE_UP => self.key_to(order[at.saturating_sub(cols * per_screen)]),
            sym::PAGE_DOWN => self.key_to(order[(at + cols * per_screen).min(last)]),
            sym::RETURN | KP_ENTER => {
                // Open it; Enter on the open one goes into its tracks.
                if open != Some(k) {
                    self.press(k);
                    self.keyed = Some(k);
                } else if albums.get(k).is_some_and(|a| !a.tracks.is_empty()) {
                    self.track_to(0);
                }
            }
            _ => return false,
        }
        true
    }

    /// Escape: out of whatever was last gone into: the tracks back to their
    /// cover first (the wall's own key), then full screen ("in full screen it
    /// should return to the normal window on first press", Dave
    /// 2026-10-14), the search, the box, the album.
    fn escape(&mut self, window: &mut Window) -> bool {
        let typing = self.focus.as_deref() == Some("search");
        if let (Some(_), Some(k), false) = (self.track, self.keyed, typing) {
            self.key_to(k);
        } else if self.fullscreen {
            self.fullscreen = false;
            window.set_fullscreen(false);
        } else if !self.query.is_empty() {
            self.search(String::new());
        } else if typing {
            self.focus_wanted = Some(None);
        } else if !self.fold() {
            return false;
        }
        true
    }

    /// Every key of the window's own, before the widgets: true (changed) or
    /// false when taken, None to leave it to them (typing in the search box).
    fn key(&mut self, input: &vergence_widgets::Input, window: &mut Window) -> Option<bool> {
        use vergence_ui::sym;
        let vergence_widgets::Input::Key { sym: key, pressed: true, repeat, composing: false, modifiers: m, .. } =
            *input
        else {
            return None;
        };
        let typing = self.focus.as_deref() == Some("search");
        let plain = !m.ctrl && !m.alt && !m.logo && !m.shift;
        let letter = |c: char| key == c as u32 || key == c.to_ascii_uppercase() as u32;
        let loaded = self.player.as_ref().and_then(|p| p.state().now);
        match key {
            sym::ESCAPE if !m.ctrl && !m.alt => return Some(self.escape(window)).filter(|&used| used),
            F11 if !m.ctrl => {
                self.fullscreen = !self.fullscreen;
                window.set_fullscreen(self.fullscreen);
                return Some(false);
            }
            _ if m.ctrl && letter('f') || plain && !typing && key == '/' as u32 => {
                self.focus_wanted = Some(Some("search".into()));
                return Some(true);
            }
            // Settings (Ctrl+,), as every Vergence app's.
            0x2c if m.ctrl && !m.alt => {
                self.open_settings(None, window);
                return Some(true);
            }
            _ if m.ctrl && letter('l') => {
                self.update(Msg::GoToPlaying, window);
                return Some(true);
            }
            _ if key == ' ' as u32 && plain && !typing => {
                // One toggle a press, not one a repeat.
                if !repeat && (loaded.is_some() || self.playing.is_some()) {
                    self.update(Msg::PlayPause, window);
                }
                return Some(true);
            }
            sym::RIGHT | sym::LEFT if m.ctrl && !m.shift && !m.alt => {
                loaded?;
                self.update(if key == sym::RIGHT { Msg::Next } else { Msg::Previous }, window);
                return Some(true);
            }
            sym::RIGHT | sym::LEFT if m.shift && !m.ctrl && !m.alt && !typing => {
                loaded?;
                self.seek_by(if key == sym::RIGHT { SEEK_STEP } else { -SEEK_STEP });
                return Some(true);
            }
            // The menu's keys: Add a folder…, Rescan, Keys and search.
            _ if m.ctrl && !m.alt && !m.shift && letter('o') => {
                self.choose_folder(window);
                return Some(true);
            }
            _ if m.ctrl && !m.alt && !m.shift && letter('r') => {
                self.read(true);
                return Some(true);
            }
            F1 if !m.ctrl && !m.alt => {
                self.open_settings(Some(prefswin::Page::Keys), window);
                return Some(true);
            }
            _ => {}
        }
        // From the search box only Down leaves it: onto the first album
        // found, or with nothing typed, where the keyboard would start.
        if typing {
            if key != sym::DOWN || !plain {
                return None;
            }
            self.focus_wanted = Some(None);
            let first = self.wall_albums().and_then(|(_, o)| o.first().copied());
            match first {
                Some(i) if !self.query.is_empty() => self.key_to(i),
                _ => self.key_start(),
            }
            return Some(true);
        }
        // The wall's keys only where no other control has the keyboard (the
        // seek bar keeps its own arrows).
        if !plain || self.focus.is_some() {
            return None;
        }
        self.wall_key(key).then_some(true)
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
        if std::mem::take(&mut self.meters_wanted) {
            self.stepped = std::time::Instant::now();
            window.wake_after(timer::METERS, METER_TICK);
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
            Msg::AddServer => self.add_server(window),
            Msg::TryAgain => self.read(false),
            // A click: the keyboard's ring goes (a click never shows it).
            Msg::Open(i) => {
                (self.keyed, self.track) = (None, None);
                self.press(i);
            }
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
            // Shuffle all songs chosen while an album plays: the next song
            // is the shuffle's, not the album's (MainWindow.PlayNext).
            Msg::Next
                if self.settings.after == AfterAlbum::ShuffleSongs && !self.shuffling && self.playing.is_some() =>
            {
                self.shuffle()
            }
            Msg::Next => self.send(Command::Next),
            Msg::Previous => self.send(Command::Previous),
            Msg::Ending => {
                let after = self.settings.after.next();
                self.write_settings(Settings { after, ..self.settings.clone() });
                self.said = Some((after.words().to_string(), std::time::Instant::now()));
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
            Msg::Search(text) => self.search(text),
            Msg::Lyrics => self.lyrics_on = !self.lyrics_on,
            Msg::LyricLine(seconds) => self.send(Command::Seek(seconds)),
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
        self.tell_link();
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
        // A server's song is fetched whole and played from its bytes.
        let remote: Arc<vergence_albumwall::player::Remote> =
            Arc::new(|p: &std::path::Path| vergence_albumwall::subsonic::song_bytes(p, tls()));
        let (track, output) = vergence_albumwall::player::system_with(remote);
        let w = waker.clone();
        self.player = Some(vergence_albumwall::player::Player::new(track, output, move || w.wake()));
        self.link.borrow_mut().wall = Some(waker.clone());
        self.waker = Some(waker);
        self.send(Command::MetersAfterGain(self.settings.meters_after_gain));
        // The library on the wall last time, read as the window opens.
        self.read(false);
    }

    fn woken(&mut self, window: &mut Window) -> bool {
        let asked = self.take_asks(window);
        let reread = self.reread();
        let chosen = self.take_chosen();
        let read = self.take_outcome();
        if read {
            self.resume();
            if let Some(r) = self.link.borrow_mut().rescanned.as_mut().filter(|r| r.ends_with('…')) {
                *r = "Read again.".into();
            }
        }
        self.album_ended();
        self.follow_shuffle();
        self.words_for_playing();
        self.tell_link();
        // Music playing: the needles move (the tick keeps itself going while
        // they do).
        let playing = self.player.as_ref().and_then(|p| p.state().now).is_some_and(|n| !n.paused);
        if playing && self.seen && self.needles.iter().all(|n| n.resting()) {
            self.meters_wanted = true;
        }
        if self.library_changed() {
            self.read(false);
        }
        self.ask_retry(window);
        // Progress while a read is under way; covers and the hue as they come.
        // Unseen: nothing drawn (it is drawn whole when seen again).
        self.seen && (asked || reread || chosen || read || self.reader.running() || self.shown.is_some())
    }

    fn timer(&mut self, id: u64, window: &mut Window) -> bool {
        if id == timer::PASTE {
            if std::mem::take(&mut self.paste_wanted) && self.focus.as_deref() == Some("search") {
                window.paste();
            }
            return true;
        }
        if id == timer::RETRY && self.unreachable.is_some() {
            self.read(false);
        }
        if id == timer::METERS {
            // Unseen: the tick stops (seen again starts it).
            if !self.seen {
                return false;
            }
            if self.step_meters() {
                window.wake_after(timer::METERS, METER_TICK);
            }
            return true;
        }
        if id == timer::POLL {
            self.read(false);
            self.poll_wanted = true;
        }
        self.ask_retry(window);
        true
    }

    /// The app menu, as every Vergence app's: File, Edit and View, then
    /// Playback and Library, the everyday few on its first panel. The choices
    /// here are Settings' own (the Avalonia app's gear menu held the same:
    /// the play mode, ReplayGain, rescan, full screen, Keys and search).
    fn commands(&self) -> Option<(bool, Vec<C>)> {
        let s = &self.settings;
        let wall = self.shown.is_some();
        let loaded = self.player.as_ref().and_then(|p| p.state().now).is_some();
        let mut list = vec![
            C::own(cmd::ADD_FOLDER, "File", "Add a folder…", "Ctrl+O").featured(),
            C::own(cmd::ADD_SERVER, "File", "Add a music server…", ""),
            C::own(cmd::RESCAN, "File", "Rescan the library", "Ctrl+R").enabled(self.libraries.current().is_some()),
            C::role(cmd::CLOSE, Role::CloseWindow),
            C::role(cmd::PASTE, Role::Paste).enabled(wall),
            C::role(cmd::FIND, Role::Find).enabled(wall).featured(),
            C::role(cmd::FULL_SCREEN, Role::FullScreen).checked(self.fullscreen).featured(),
            C::own(cmd::METERS, "View", "Meters in the play bar", "").checked(s.meters),
            C::own(cmd::GO_TO_PLAYING, "View", "Go to what's playing", "Ctrl+L").enabled(self.playing.is_some()),
            C::own(cmd::KEYS, "View", "Keys and search…", "F1"),
            C::own(cmd::PLAY_PAUSE, "Playback", "Play / Pause", "Space")
                .enabled(loaded || self.playing.is_some())
                .featured(),
            C::own(cmd::NEXT, "Playback", "Next track", "Ctrl+Right").enabled(loaded),
            C::own(cmd::PREVIOUS, "Playback", "Previous track", "Ctrl+Left").enabled(loaded),
            C::own(cmd::FORWARD, "Playback", "Forward ten seconds", "Shift+Right").enabled(loaded),
            C::own(cmd::BACK, "Playback", "Back ten seconds", "Shift+Left").enabled(loaded),
        ];
        for (i, a) in AfterAlbum::ALL.iter().enumerate() {
            list.push(C::own(cmd::AFTER + i as u32, "Playback", a.words(), "").checked(s.after == *a));
        }
        let gain = s.gain.unwrap_or(vergence_albumwall::player::Loudness::Album);
        for (i, (g, words)) in [
            (vergence_albumwall::player::Loudness::Album, "ReplayGain by album"),
            (vergence_albumwall::player::Loudness::Track, "ReplayGain by track"),
            (vergence_albumwall::player::Loudness::Off, "ReplayGain off"),
        ]
        .into_iter()
        .enumerate()
        {
            list.push(C::own(cmd::GAIN + i as u32, "Playback", words, "").checked(gain == g));
        }
        for (i, l) in self.libraries.list.iter().enumerate().take(50) {
            list.push(C::own(cmd::LIBRARY + i as u32, "Library", &l.name, "").checked(i == self.libraries.current));
        }
        list.push(
            C::own(cmd::STATISTICS, "Library", "Library statistics…", "").enabled(!self.libraries.list.is_empty()),
        );
        list.extend([C::role(cmd::SETTINGS, Role::Settings), C::role(cmd::ABOUT, Role::About)]);
        Some((false, list))
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        match id {
            cmd::ADD_FOLDER => self.choose_folder(window),
            cmd::RESCAN => self.read(true),
            cmd::SETTINGS => self.open_settings(None, window),
            cmd::CLOSE => window.close(),
            // Into the search box: the window's one field.
            cmd::PASTE => {
                if self.focus.as_deref() == Some("search") {
                    window.paste();
                } else {
                    self.focus_wanted = Some(Some("search".into()));
                    self.paste_wanted = true;
                    window.wake_after(timer::PASTE, Duration::from_millis(60));
                }
            }
            cmd::FIND => self.focus_wanted = Some(Some("search".into())),
            cmd::FULL_SCREEN => {
                self.fullscreen = !self.fullscreen;
                window.set_fullscreen(self.fullscreen);
            }
            cmd::METERS => self.write_settings(Settings { meters: !self.settings.meters, ..self.settings.clone() }),
            cmd::GO_TO_PLAYING => self.update(Msg::GoToPlaying, window),
            cmd::KEYS => self.open_settings(Some(prefswin::Page::Keys), window),
            cmd::ADD_SERVER => self.add_server(window),
            cmd::STATISTICS => self.open_settings(Some(prefswin::Page::Library), window),
            cmd::PLAY_PAUSE => self.update(Msg::PlayPause, window),
            cmd::NEXT => self.update(Msg::Next, window),
            cmd::PREVIOUS => self.update(Msg::Previous, window),
            cmd::FORWARD | cmd::BACK => self.seek_by(if id == cmd::FORWARD { SEEK_STEP } else { -SEEK_STEP }),
            id if (cmd::AFTER..cmd::AFTER + 4).contains(&id) => {
                let after = AfterAlbum::ALL[(id - cmd::AFTER) as usize];
                self.write_settings(Settings { after, ..self.settings.clone() });
            }
            id if (cmd::GAIN..cmd::GAIN + 3).contains(&id) => {
                use vergence_albumwall::player::Loudness;
                let gain = [Loudness::Album, Loudness::Track, Loudness::Off][(id - cmd::GAIN) as usize];
                self.write_settings(Settings { gain: Some(gain), ..self.settings.clone() });
            }
            id if id >= cmd::LIBRARY => self.update(Msg::Library((id - cmd::LIBRARY) as usize), window),
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

    /// Minimized or the session locked (bus v42, Mind the battery): the
    /// music plays on, nothing is drawn; seen again, the needles start from
    /// where the music is and the window is drawn whole.
    fn seen(&mut self, seen: bool, window: &mut Window) {
        self.seen = seen;
        if seen {
            self.stepped = std::time::Instant::now();
            let playing = self.player.as_ref().and_then(|p| p.state().now).is_some_and(|n| !n.paused);
            self.meters_wanted = playing;
            self.ask_retry(window);
            window.redraw();
        } else {
            window.cancel_timer(timer::METERS);
        }
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
        if let Some(y) = self.jump.take() {
            self.glide = None;
            return vec![("wall".into(), y)];
        }
        let now = std::time::Instant::now();
        let Some((from, to, t)) = &self.glide else { return Vec::new() };
        let at = vergence_widgets::motion::mix(*from, *to, t.out(now));
        if !t.running(now) {
            self.glide = None;
        }
        vec![("wall".into(), at)]
    }

    /// The window's keys (step 7): the search, the wall's arrows, play,
    /// seek, full screen and the Escape chain, before any widget sees them.
    fn key_first(&mut self, input: &vergence_widgets::Input, window: &mut Window) -> Option<bool> {
        self.key(input, window)
    }

    fn focus_to(&mut self) -> Option<Option<String>> {
        self.focus_wanted.take()
    }

    fn focus_moved(&mut self, name: Option<String>) {
        self.focus = name;
    }

    fn animating(&self) -> bool {
        let now = std::time::Instant::now();
        // A little past each motion's end, so the last frame drawn is the
        // finished one (not one from just before it: a folded tile kept its ring).
        let past = std::time::Duration::from_millis(60);
        let busy = |t: &vergence_widgets::motion::Tween| now.saturating_duration_since(t.start) < t.duration + past;
        let half = std::time::Duration::from_millis(TURN_MS / 2);
        let opening = self.opened.as_ref().is_some_and(|o| {
            busy(&o.turn)
                || o.back_at.get().is_some_and(|s| now.saturating_duration_since(s) < half + past)
                || o.swap.as_ref().is_some_and(busy)
                || now.saturating_duration_since(o.unfold.start)
                    < o.unfold.duration
                        + past
                        + std::time::Duration::from_millis(if self.reduced() { 0 } else { CONTENTS_MS })
        });
        opening
            || self.folding.as_ref().is_some_and(|f| busy(&f.fold))
            || self.glide.is_some()
            || (self.lyrics_on && self.lyrics_motion.moving.get())
            || self.fades.fading(now)
            || self.bar_motion.get().is_some_and(|(_, at)| now.saturating_duration_since(at) < BAR_SLIDE + past)
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
