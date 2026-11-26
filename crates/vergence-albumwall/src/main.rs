//! AlbumWall for Vergence (docs/albumwall-milestone-1.md): a wall of every
//! album you own, rebuilt in Rust on the toolkit and Hymie, looking and
//! working as the Avalonia AlbumWall does.
//!
//! Step 1, the skeleton: the window, the first-run welcome (nothing is read
//! until a folder is chosen, as in the Avalonia app), the library list kept
//! in ~/.config/vergence/albumwall, the app menu (Add a folder…, About).
//! Reading the library is step 2.
//!
//! The window is AlbumWall's own dark one whatever the desktop's scheme
//! (question 3: the covers are the point); its bar is the server's, dark.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use vergence_albumwall::libraries::{self, Libraries};
use vergence_ui::{AppCommand as C, BarStyle, Role};
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Program, Theme, Waker, Window, button, column, label, panel, space,
};

const SECTION: &str = "[albumwall]";
const APP_ID: &str = "vergence-albumwall";

mod cmd {
    pub const ADD_FOLDER: u32 = 1;
    pub const ABOUT: u32 = 2;
}

#[derive(Debug, Clone)]
enum Msg {
    ChooseFolder,
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
}

impl AlbumWall {
    fn new() -> AlbumWall {
        let list_path = libraries::config_dir().map(|d| d.join("libraries"));
        let libraries = list_path.as_deref().map(Libraries::load).unwrap_or_default();
        let desktop = vergence_widgets::prefs::desktop_path()
            .map(|p| vergence_config::effective_text_with(Some(&p)))
            .unwrap_or_default();
        AlbumWall { libraries, list_path, desktop, chosen: Arc::default(), waker: None, problem: None }
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

    /// A folder the dialog chose: added, put on the wall, kept.
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

    /// A library chosen: reading it is step 2.
    fn library(&self, t: &Theme) -> Element<Msg> {
        let lib = self.libraries.current().expect("a library");
        let mut lines = vec![
            label("Reading your library").size(17.0).align(Align::Center).width(Length::FILL),
            label(lib.folder.to_string_lossy().into_owned()).size(13.0).dim().align(Align::Center).width(Length::FILL),
        ];
        if let Some(p) = &self.problem {
            lines.push(label(p.clone()).size(13.0).color(t.accent).wrap());
        }
        centered(column(lines).spacing(10.0))
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
        if self.libraries.current().is_some() { self.library(t) } else { self.welcome(t) }
    }

    fn update(&mut self, message: Msg, window: &mut Window) {
        match message {
            Msg::ChooseFolder => self.choose_folder(window),
        }
    }

    fn waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    fn woken(&mut self, _window: &mut Window) -> bool {
        self.take_chosen()
    }

    fn commands(&self) -> Option<(bool, Vec<C>)> {
        Some((
            false,
            vec![C::own(cmd::ADD_FOLDER, "Library", "Add a folder…", "").featured(), C::role(cmd::ABOUT, Role::About)],
        ))
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        match id {
            cmd::ADD_FOLDER => self.choose_folder(window),
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
