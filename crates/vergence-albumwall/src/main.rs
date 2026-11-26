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

mod reading;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reading::{Outcome, Reader};
use vergence_albumwall::libraries::{self, Libraries};
use vergence_albumwall::library::Album;
use vergence_albumwall::scan::Unreachable;
use vergence_ui::{AppCommand as C, BarStyle, Role};
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Program, Theme, Waker, Window, button, column, label, panel, row,
    space,
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
}

#[derive(Debug, Clone)]
enum Msg {
    ChooseFolder,
    TryAgain,
}

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
    shown: Option<(PathBuf, Vec<Album>, usize)>,
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
                self.shown = Some((root, albums, files));
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

    /// The library read: until the wall (step 4), what it holds.
    fn library_view(&self, albums: &[Album], files: usize) -> Element<Msg> {
        let tracks: usize = albums.iter().map(|a| a.tracks.len()).sum();
        let mut status = format!("{} albums · {} tracks", albums.len(), tracks);
        if files > tracks {
            status.push_str(&format!(" · {} files not played", files - tracks));
        }
        if self.reader.running() {
            status.push_str(" · rescanning");
        }
        if self.unreachable.is_some() {
            let name = self.libraries.current().map(|l| l.name.clone()).unwrap_or_default();
            status = format!("{name} cannot be reached · showing it as it was");
        }
        centered(label(status).size(15.0).align(Align::Center).width(Length::FILL))
    }
}

impl AlbumWall {
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
    column(vec![space().height(Length::FILL), child, space().height(Length::FILL)])
        .align(Align::Center)
        .padding(26.0)
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
        let t = Theme::from_desktop(&self.desktop, true, SECTION);
        if t.dark {
            return t;
        }
        let d = Theme::new(true, t.accent, t.high_contrast);
        Theme {
            dark: true,
            sidebar: d.sidebar,
            page: d.page,
            card: d.card,
            line: d.line,
            dim: d.dim,
            text: d.text,
            hover: d.hover,
            ..t
        }
    }

    fn bar_style(&self) -> Option<BarStyle> {
        Some(BarStyle::Dark)
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        let Some(lib) = self.libraries.current() else { return self.welcome(t) };
        if let Some((root, albums, files)) = &self.shown
            && *root == lib.folder
        {
            return self.library_view(albums, *files);
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

    fn waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
        // The library on the wall last time, read as the window opens.
        self.read(false);
    }

    fn woken(&mut self, window: &mut Window) -> bool {
        let chosen = self.take_chosen();
        let read = self.take_outcome();
        if self.library_changed() {
            self.read(false);
        }
        self.ask_retry(window);
        // Progress: drawn again while a read is under way.
        chosen || read || self.reader.running()
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
