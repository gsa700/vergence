//! AlbumWall's settings window (step 8), laid out as the Avalonia app's
//! PrefsWindow.axaml on the toolkit: a sidebar of pages, About set apart at
//! its foot, and the page on a light sheet beside it, each page scrolling on
//! its own. Dave, 2026-11-26: "There are too many settings for a single page
//! layout. we learned that building albumwall. so use our toolkit but mimic
//! the layout from avalonia". Every Vergence app's shared settings window
//! (vergence_widgets::prefs) is one page; this one is AlbumWall's own.
//!
//! Light, as the Avalonia app's ("the music dark, the paperwork light",
//! 2026-10-08), in its colors: the grey desk, the off-white sheet, near-black
//! ink, the library line picked a shade darker than the sheet. Choices are
//! written straight through to desktop.toml's `[albumwall]` (no OK button);
//! the wall follows at once. Light and Tint show on the wall as they move
//! and are written when let go.
//!
//! The window and the wall share a [`Link`]: the wall puts there what the
//! window shows of it (the libraries, the numbers, the hue), the window puts
//! there what it asks of the wall (switch to a library, add a folder, read
//! it again, search), and each wakes the other.
//!
//! Not ported here: Updates (Vergence's package updates AlbumWall), the
//! Navidrome button, the equalizer and the LED meters' peak hold (milestone
//! 2), the Start Menu and install sections (Windows).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use vergence_albumwall::colors;
use vergence_albumwall::player::Loudness;
use vergence_albumwall::settings::{AfterAlbum, Settings};
use vergence_albumwall::stats::{self, Stats};
use vergence_scene::Color;
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Program, Theme, Waker, Window, button, checkbox, choice, column,
    field, label, panel, radio, row, scroll, slider, space,
};

/// The pages, in the sidebar's order (the Page enum of PrefsWindow.axaml.cs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Library,
    Playback,
    Appearance,
    Keys,
    About,
}

impl Page {
    const ALL: [Page; 5] = [Page::Library, Page::Playback, Page::Appearance, Page::Keys, Page::About];

    fn name(self) -> &'static str {
        match self {
            Page::Library => "Library",
            Page::Playback => "Playback",
            Page::Appearance => "Appearance",
            Page::Keys => "Keys and search",
            Page::About => "About",
        }
    }
}

/// One library as the window shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub name: String,
    pub folder: PathBuf,
    /// A music server's address (its folder is then the server's key).
    pub server: Option<String>,
}

/// What the window asks of the wall.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    SwitchTo(usize),
    AddFolder,
    /// The library on the wall read again (every file opened).
    Rescan,
    Rename(usize, String),
    Forget(usize),
    /// The wall searched for this (the numbers' "show on the wall").
    Search(String),
    /// desktop.toml changed: read it again.
    Reread,
    /// Music servers signed in to: each a library, named.
    AddServers(Vec<(String, vergence_albumwall::subsonic::Login)>),
}

/// Shared between the wall and its settings windows (one thread: the
/// toolkit runs a program's windows together).
#[derive(Default)]
pub struct Link {
    // From the wall.
    pub libraries: Vec<Line>,
    pub current: usize,
    /// The library on the wall being read now.
    pub reading: bool,
    /// The numbers of the library on the wall, as it is now.
    pub live: Option<(PathBuf, Stats)>,
    /// The hue the wall is built from, and whether it is the library's own
    /// (false: the warm fallback, the covers leaning no one way yet).
    pub hue: f64,
    pub hue_is_library: bool,
    /// The last Rescan's word ("Read again: 4,581 tracks").
    pub rescanned: Option<String>,
    // From the windows.
    pub asks: Vec<Ask>,
    /// Light and Tint while a slider is held (shown on the wall, not yet written).
    pub preview: Option<(u32, u32)>,
    // The windows.
    pub open: bool,
    pub page: Page,
    /// The library whose numbers are shown (None: the one on the wall).
    pub selected: Option<PathBuf>,
    pub lib_open: Option<usize>,
    pub wall: Option<Waker>,
    pub prefs: Option<Waker>,
    pub lib: Option<Waker>,
}

pub type Shared = Rc<RefCell<Link>>;

impl Link {
    /// The wall told something changed.
    pub fn ask(shared: &Shared, a: Ask) {
        let w = {
            let mut l = shared.borrow_mut();
            l.asks.push(a);
            l.wall.clone()
        };
        if let Some(w) = w {
            w.wake();
        }
    }

    /// The settings windows told the wall changed.
    pub fn tell_windows(&self) {
        for w in [&self.prefs, &self.lib].into_iter().flatten() {
            w.wake();
        }
    }
}

// The Avalonia window's colors (PrefsWindow.axaml): the desk, the sheet,
// its edge, the picked line, the ink, the dim text, the sidebar's ink.
const DESK: Color = Color::rgb(0xE4, 0xE4, 0xE2);
const SHEET: Color = Color::rgb(0xF6, 0xF6, 0xF4);
const EDGE: Color = Color::rgb(0xC2, 0xC2, 0xC6);
const PICKED: Color = Color::rgb(0xCD, 0xCD, 0xD0);
const INK: Color = Color::rgb(0x1D, 0x1D, 0x20);
const DIM: Color = Color::rgb(0x62, 0x62, 0x6A);
const SIDE_INK: Color = Color::rgb(0x3C, 0x3C, 0x42);
/// "Button.panel": a faint dark ground and edge.
const PANEL_FILL: Color = Color { r: 0, g: 0, b: 0, a: 0x12 };
const PANEL_EDGE: Color = Color { r: 0, g: 0, b: 0, a: 0x38 };
/// The notes under a check box or radio button start under its words.
const UNDER: f32 = 24.0;

/// The sheet's theme: light, in the Avalonia window's colors, with the
/// desktop's accent, fonts, text size, contrast and motion; high contrast
/// keeps its own colors.
pub fn sheet_theme(desktop: &str) -> Theme {
    let mut t = Theme::from_desktop(desktop, false, "[settings window]");
    let d = Theme::new(false, t.accent, t.high_contrast);
    t = Theme {
        dark: false,
        sidebar: d.sidebar,
        page: d.page,
        card: d.card,
        line: d.line,
        dim: d.dim,
        text: d.text,
        hover: d.hover,
        ..t
    };
    if !t.high_contrast {
        (t.page, t.card, t.line, t.text, t.dim, t.sidebar) = (DESK, SHEET, EDGE, INK, DIM, DESK);
    }
    t
}

fn panel_button<M: Clone>(words: &str) -> Element<M> {
    button(label(words.to_string()).size(13.0))
        .look(ButtonLook::Custom { fill: Some(PANEL_FILL), edge: Some((PANEL_EDGE, 1.0)), radius: 4.0 })
        .padding(Padding::hv(5.0, 12.0))
        .name(words.to_string())
}

fn title<M>(words: &str) -> Element<M> {
    label(words.to_string()).size(18.0).weight(600)
}

fn heading<M>(words: &str) -> Element<M> {
    panel(label(words.to_string()).size(14.0).weight(600)).padding(Padding { top: 6.0, ..Padding::all(0.0) })
}

fn note<M>(words: impl Into<String>) -> Element<M> {
    label(words).size(13.0).dim().wrap().width(Length::FILL)
}

/// A note under a check box or radio button, in line with its words.
fn under<M>(words: impl Into<String>) -> Element<M> {
    panel(note(words)).padding(Padding { left: UNDER, ..Padding::all(0.0) }).width(Length::FILL)
}

/// A check box or radio button with its note under it.
fn with_note<M>(control: Element<M>, words: &str) -> Element<M> {
    column(vec![control, under(words.to_string())]).spacing(4.0).width(Length::FILL)
}

#[derive(Debug, Clone)]
pub enum Msg {
    Page(Page),
    Resume(bool),
    AutoPlay(bool),
    After(AfterAlbum),
    Gain(Loudness),
    MetersAfterGain(bool),
    Light(f32),
    Tint(f32),
    LetGo,
    Bars(usize),
    Meters(bool),
    Defaults,
    Select(PathBuf),
    SwitchTo(usize),
    LibrarySettings(usize),
    AddFolder,
    AddServer,
    Search(String),
}

pub struct Prefs {
    pub link: Shared,
    pub app_id: String,
    desktop: String,
    stamp: Option<std::time::SystemTime>,
    settings: Settings,
    /// The sliders while held.
    held: Option<(u32, u32)>,
    /// The element with the keyboard (a page's line: the arrows move the page).
    focus: Option<String>,
    focus_wanted: Option<String>,
    size: Option<(f32, f32)>,
}

impl Prefs {
    pub fn new(link: Shared, app_id: &str) -> Prefs {
        link.borrow_mut().open = true;
        let mut p = Prefs {
            link,
            app_id: app_id.into(),
            desktop: String::new(),
            stamp: None,
            settings: Settings::default(),
            held: None,
            focus: None,
            focus_wanted: None,
            size: None,
        };
        p.reread();
        p
    }

    fn reread(&mut self) -> bool {
        let Some(path) = vergence_widgets::prefs::desktop_path() else { return false };
        let stamp = vergence_config::modified_with(Some(&path));
        if stamp == self.stamp && self.stamp.is_some() {
            return false;
        }
        self.stamp = stamp;
        self.desktop = vergence_config::effective_text_with(Some(&path));
        self.settings = Settings::from_text(&self.desktop);
        true
    }

    /// The choices written to the person's own desktop.toml, and the wall told.
    fn write(&mut self, s: Settings) {
        let Some(path) = vergence_widgets::prefs::desktop_path() else { return };
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let new = s.write_into(&text);
        if new != text {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let beside = path.with_extension("toml.new");
            if let Err(e) = std::fs::write(&beside, &new).and_then(|_| std::fs::rename(&beside, &path)) {
                eprintln!("albumwall settings: can't write {}: {e}", path.display());
                return;
            }
        }
        self.settings = s;
        self.stamp = None;
        self.reread();
        Link::ask(&self.link, Ask::Reread);
    }

    fn sidebar(&self, page: Page) -> Element<Msg> {
        let mut items = Vec::new();
        for p in Page::ALL {
            let picked = p == page;
            let words = label(p.name())
                .size(13.0)
                .weight(if picked { 600 } else { 400 })
                .color(if picked || self.high_contrast() { INK } else { SIDE_INK });
            let item = button(row(vec![words, space().width(Length::FILL)]).width(Length::FILL))
                .look(ButtonLook::Custom {
                    fill: picked.then_some(PICKED),
                    edge: Some((if picked { EDGE } else { Color { a: 0, ..EDGE } }, 1.0)),
                    radius: 5.0,
                })
                .padding(Padding::hv(7.0, 12.0))
                .width(Length::FILL)
                .on_press(Msg::Page(p))
                .name(format!("page: {}", p.name()));
            // About is set apart at the foot: a page about the app, not a setting.
            let gap = if p == Page::About { 18.0 } else { 0.0 };
            items.push(panel(item).padding(Padding { top: gap, bottom: 2.0, ..Padding::all(0.0) }).width(Length::FILL));
        }
        column(items).width(Length::Fixed(168.0)).name("Settings pages")
    }

    fn high_contrast(&self) -> bool {
        sheet_theme(&self.desktop).high_contrast
    }

    fn page(&self, page: Page, t: &Theme) -> Element<Msg> {
        let body = match page {
            Page::Library => self.library_page(t),
            Page::Playback => self.playback_page(),
            Page::Appearance => self.appearance_page(t),
            Page::Keys => keys_page(),
            Page::About => about_page(),
        };
        // Each page scrolls on its own; the window never jumps from page to page.
        scroll(panel(body).padding(Padding { right: 14.0, ..Padding::all(0.0) }).width(Length::FILL))
            .name(format!("page {}", page.name()))
            .width(Length::FILL)
            .height(Length::FILL)
    }

    fn playback_page(&self) -> Element<Msg> {
        let s = &self.settings;
        let after = |a: AfterAlbum, words: &str, about: &str| {
            with_note(radio(s.after == a, words).on_press(Msg::After(a)), about)
        };
        // Until chosen, the choice shown is what an album plays at: by album.
        let gain = s.gain.unwrap_or(Loudness::Album);
        let gain_row =
            |g: Loudness, words: &str, about: &str| with_note(radio(gain == g, words).on_press(Msg::Gain(g)), about);
        column(vec![
            title("Playback"),
            heading("When AlbumWall opens"),
            with_note(
                checkbox(s.resume, "Pick up where I left off").on_press(Msg::Resume(!s.resume)),
                "Brings back the album you had open, the track, and how far into it you were.",
            ),
            with_note(
                {
                    let c = checkbox(s.auto_play, "…and start playing straight away");
                    if s.resume { c.on_press(Msg::AutoPlay(!s.auto_play)) } else { c }
                },
                "Off, it comes back paused and waits for you. Nothing is heard until you press play.",
            ),
            heading("When an album ends"),
            after(AfterAlbum::Stop, "Stop", "The music ends with the record, as it would on a turntable."),
            after(
                AfterAlbum::NextAlbum,
                "Play the next album",
                "The one after it on the wall, and on round to the first. With a search in the box, only the albums it shows.",
            ),
            after(
                AfterAlbum::RandomAlbum,
                "Play a random album",
                "Each album whole and in its own order; none comes round twice until every one on the wall has played.",
            ),
            after(
                AfterAlbum::ShuffleSongs,
                "Shuffle all songs",
                "Every song on the wall in a random order, each at its own level (by track, unless you have chosen a ReplayGain setting below).",
            ),
            note(
                "While the music goes on, the wall opens each album as it starts, and in a shuffle the album of each song.",
            ),
            heading("Loudness"),
            note(
                "Records are mastered at different levels. ReplayGain evens them out using the loudness a tagger has measured and written into each file; a file without that plays as it is. Until you choose, albums play by album and a shuffle plays by track.",
            ),
            gain_row(
                Loudness::Album,
                "By album",
                "A whole record at one level, so its quiet songs stay quiet. For playing an album through.",
            ),
            gain_row(Loudness::Track, "By track", "Every song to the same level. For shuffling across the collection."),
            gain_row(Loudness::Off, "Off", "Everything plays at exactly the level it was encoded at."),
            with_note(
                checkbox(s.meters_after_gain, "Meters read after ReplayGain")
                    .on_press(Msg::MetersAfterGain(!s.meters_after_gain)),
                "Off, the meters show the record as it was mastered, so a quiet master reads quiet. On, they show what ReplayGain sends on, closer to what you hear.",
            ),
        ])
        .spacing(14.0)
        .width(Length::FILL)
    }

    fn appearance_page(&self, t: &Theme) -> Element<Msg> {
        let s = &self.settings;
        let (light, tint) = self.held.unwrap_or((s.light, s.tint));
        let (hue, own) = {
            let l = self.link.borrow();
            (l.hue, l.hue_is_library)
        };
        let swatch = colors::from_hsl(hue, 0.60, 0.52);
        let swatch = Color::rgb(swatch.0, swatch.1, swatch.2);
        let (line, about) = if own {
            (
                format!("Your collection leans {}", hue_name(hue)),
                "AlbumWall has no color scheme of its own. It looks at the covers in your library, finds the color they lean toward, and builds the window from that — so it is different for every collection, and changes if yours does. The controls below do not pick a color. They decide what is done with yours.",
            )
        } else {
            (
                "Your covers do not lean any one way".to_string(),
                "AlbumWall builds its colors from the covers in your library. Yours are spread evenly around the color wheel, or there are none yet, so it is using a neutral warm amber instead. That will change by itself as the collection does. The controls below decide what is done with the color.",
            )
        };
        let found = panel(
            column(vec![
                row(vec![
                    panel(space()).color(swatch).radius(9.0).width(Length::Fixed(18.0)).height(Length::Fixed(18.0)),
                    label(line).size(14.0).weight(600),
                ])
                .spacing(10.0)
                .align(Align::Center),
                note(about),
            ])
            .spacing(8.0)
            .width(Length::FILL),
        )
        .color(t.page)
        .radius(6.0)
        .padding(Padding::hv(12.0, 14.0))
        .width(Length::FILL);
        let slider_row = |name: &str, min: u32, max: u32, at: u32, f: fn(f32) -> Msg, about: &str| {
            column(vec![
                row(vec![
                    label(name.to_string()).size(14.0).weight(600).width(Length::Fixed(70.0)),
                    slider(min as f32, max as f32, at as f32, f)
                        .step(1.0)
                        .on_release(Msg::LetGo)
                        .width(Length::FILL)
                        .name(name.to_string()),
                    panel(label(at.to_string()).size(12.0).dim()).align_x(Align::End).width(Length::Fixed(34.0)),
                ])
                .align(Align::Center)
                .width(Length::FILL),
                panel(note(about.to_string())).padding(Padding { left: 70.0, ..Padding::all(0.0) }).width(Length::FILL),
            ])
            .spacing(2.0)
            .width(Length::FILL)
        };
        let names: Vec<String> = colors::CHROMES.iter().map(|c| capital(c.name)).collect();
        column(vec![
            title("Appearance"),
            found,
            slider_row(
                "Light",
                colors::MIN_LIGHT,
                colors::MAX_LIGHT,
                light,
                Msg::Light,
                "How light or dark the window is. The color stays your collection's; only its brightness moves.",
            ),
            slider_row(
                "Tint",
                0,
                colors::MAX_TINT,
                tint,
                Msg::Tint,
                "How much of your collection's color comes through. At 0 the window is a plain neutral; turn it up and more of that color shows.",
            ),
            column(vec![
                row(vec![
                    label("Bars").size(14.0).weight(600).width(Length::Fixed(70.0)),
                    choice(names, Some(s.bars), Msg::Bars).width(Length::Fixed(170.0)).name("Bars"),
                ])
                .align(Align::Center),
                panel(note(about_bars(colors::CHROMES[s.bars].name)))
                    .padding(Padding { left: 70.0, ..Padding::all(0.0) })
                    .width(Length::FILL),
            ])
            .spacing(6.0)
            .width(Length::FILL),
            heading("Meters"),
            with_note(
                checkbox(s.meters, "Show meters in the play controls").on_press(Msg::Meters(!s.meters)),
                "The VU meters at the end of the play bar.",
            ),
            panel(panel_button("Back to the defaults").on_press(Msg::Defaults))
                .padding(Padding { left: 70.0, top: 2.0, ..Padding::all(0.0) }),
        ])
        .spacing(16.0)
        .width(Length::FILL)
    }

    fn library_page(&self, t: &Theme) -> Element<Msg> {
        let l = self.link.borrow();
        let mut lines: Vec<Element<Msg>> = Vec::new();
        let selected = l.selected.clone().filter(|p| l.libraries.iter().any(|x| &x.folder == p));
        let picked_folder = selected.or_else(|| l.libraries.get(l.current).map(|x| x.folder.clone()));
        if l.libraries.is_empty() {
            lines.push(note("No library yet. Add a folder, or answer the welcome in the main window."));
        }
        for (i, lib) in l.libraries.iter().enumerate() {
            let shown = i == l.current;
            let picked = picked_folder.as_ref() == Some(&lib.folder);
            let name = if shown { format!("{}  — on the wall", lib.name) } else { lib.name.clone() };
            let mut parts = vec![
                label(name).size(13.0).weight(if shown { 600 } else { 400 }),
                label(lib.server.clone().unwrap_or_else(|| lib.folder.to_string_lossy().into_owned()))
                    .size(12.0)
                    .dim()
                    .elide()
                    .width(Length::FILL),
            ];
            if shown && l.reading {
                parts.push(label("reading…").size(12.0).dim());
            }
            // A press anywhere on the line that is not a button selects it.
            let line = button(row(parts).spacing(10.0).align(Align::Center).width(Length::FILL))
                .look(ButtonLook::Custom { fill: None, edge: None, radius: 4.0 })
                .padding(Padding::all(0.0))
                .width(Length::FILL)
                .on_press(Msg::Select(lib.folder.clone()))
                .name(lib.name.clone());
            let mut buttons = vec![line];
            if !shown {
                buttons
                    .push(panel_button("Switch to").on_press(Msg::SwitchTo(i)).name(format!("Switch to {}", lib.name)));
            }
            buttons.push(
                panel_button("Settings…").on_press(Msg::LibrarySettings(i)).name(format!("Settings for {}", lib.name)),
            );
            lines.push(
                panel(row(buttons).spacing(10.0).align(Align::Center).width(Length::FILL))
                    .color(if picked { PICKED } else { t.page })
                    .edge(if picked { EDGE } else { t.page }, 1.0)
                    .radius(4.0)
                    .padding(Padding::hv(7.0, 12.0))
                    .width(Length::FILL),
            );
        }
        let mut page = vec![
            title("Libraries"),
            note(
                "The wall shows one library at a time and never mixes them. Switch to puts a library on the wall, as the name in the bottom bar does. Settings is everything about one library; click a line to see its numbers below.",
            ),
            column(lines).spacing(4.0).width(Length::FILL),
            row(vec![
                panel_button("Add a folder…").on_press(Msg::AddFolder),
                panel_button("Add a music server…").on_press(Msg::AddServer),
            ])
            .spacing(9.0),
        ];
        if let Some(folder) = picked_folder {
            let name = l.libraries.iter().find(|x| x.folder == folder).map(|x| x.name.clone()).unwrap_or_default();
            let live = l.live.as_ref().filter(|(f, _)| *f == folder).map(|(_, s)| s.clone());
            let (s, as_of) = match live {
                Some(s) => (Some(s), "as it is now".to_string()),
                None => match stats::load(&folder) {
                    Some((s, when)) => (Some(s), format!("as of {}", when_words(when))),
                    None => (None, "not read yet".to_string()),
                },
            };
            page.push(numbers(&name, &as_of, s.as_ref(), t));
        }
        column(page).spacing(12.0).width(Length::FILL)
    }
}

/// The selected library's numbers (PrefsWindow.Statistics.cs, LibraryNumbers.cs).
fn numbers(name: &str, as_of: &str, s: Option<&Stats>, t: &Theme) -> Element<Msg> {
    let mut out = vec![
        panel(space().height(Length::Fixed(1.0))).color(EDGE).width(Length::FILL).height(Length::Fixed(1.0)),
        label(format!("{name}  —  {as_of}")).size(14.0).weight(600),
    ];
    let Some(s) = s.filter(|s| s.tracks > 0) else {
        out.push(note("Nothing to count yet. This fills in once the library has been read."));
        return column(out).spacing(10.0).width(Length::FILL);
    };
    let total = |figure: String, words: &str, w: f32| {
        column(vec![label(figure).size(20.0).weight(600), label(words.to_string()).size(12.0).dim()])
            .spacing(1.0)
            .width(Length::Fill((w * 100.0) as u16))
    };
    out.push(
        panel(
            row(vec![
                total(group(s.albums), "albums", 1.0),
                total(group(s.tracks), "tracks", 1.0),
                total(group(s.artists), "artists", 1.0),
                total(stats::spoken(s.millis), "of music", 1.35),
                total(stats::bytes(s.bytes), "on disk", 1.0),
            ])
            .width(Length::FILL),
        )
        .padding(Padding { top: 2.0, bottom: 6.0, ..Padding::all(0.0) })
        .width(Length::FILL),
    );
    let ink = if t.high_contrast { t.text } else { t.accent };
    // A table row: the first cell, a bar for its share, then figures right-aligned.
    let figure = |text: String, w: f32| panel(label(text).size(12.0)).align_x(Align::End).width(Length::Fixed(w));
    let bar = |fraction: f32| -> Element<Msg> {
        let f = fraction.clamp(0.004, 1.0);
        panel(
            row(vec![
                panel(space())
                    .color(Color { a: 217, ..ink })
                    .radius(3.0)
                    .width(Length::Fill(((f * 1000.0) as u16).max(4)))
                    .height(Length::Fixed(6.0)),
                space().width(Length::Fill((((1.0 - f) * 1000.0) as u16).max(1))),
            ])
            .width(Length::FILL),
        )
        .color(EDGE)
        .radius(3.0)
        .padding(Padding::all(0.0))
        .width(Length::FILL)
        .height(Length::Fixed(6.0))
    };
    let table_row = |first: Element<Msg>, first_w: f32, fraction: Option<f32>, rest: Vec<Element<Msg>>| {
        let mut cells = vec![panel(first).width(Length::Fixed(first_w))];
        cells.push(match fraction {
            Some(f) => {
                panel(bar(f)).padding(Padding { left: 6.0, right: 10.0, ..Padding::all(0.0) }).width(Length::FILL)
            }
            None => space().width(Length::FILL),
        });
        cells.extend(rest);
        row(cells).align(Align::Center).width(Length::FILL).height(Length::Fixed(22.0))
    };
    let section = |heading_words: &str, rows: Vec<Element<Msg>>| {
        let mut v = vec![label(heading_words.to_string()).size(14.0).weight(600)];
        v.extend(rows);
        panel(column(v).spacing(4.0).width(Length::FILL))
            .padding(Padding { top: 6.0, ..Padding::all(0.0) })
            .width(Length::FILL)
    };
    let small_dim = |w: &str, width: f32| {
        panel(label(w.to_string()).size(11.0).dim()).align_x(Align::End).width(Length::Fixed(width))
    };
    let mut types = vec![table_row(
        space(),
        58.0,
        None,
        vec![
            small_dim("tracks", 62.0),
            small_dim("share", 52.0),
            small_dim("size", 74.0),
            small_dim("time", 92.0),
            small_dim("typical", 84.0),
        ],
    )];
    for r in &s.types {
        types.push(table_row(
            label(r.kind.clone()).size(13.0).weight(600),
            58.0,
            Some(r.tracks as f32 / s.tracks as f32),
            vec![
                figure(group(r.tracks), 62.0),
                figure(stats::share(r.tracks, s.tracks), 52.0),
                figure(stats::bytes(r.bytes), 74.0),
                figure(stats::spoken(r.millis), 92.0),
                figure(if r.bitrate > 0 { format!("{} kbps", r.bitrate) } else { "—".into() }, 84.0),
            ],
        ));
    }
    out.push(section("File types", types));
    if !s.lossy.is_empty() {
        let lossy: usize = s.lossy.iter().map(|b| b.tracks).sum();
        let lossless: usize = s.lossless.iter().map(|b| b.tracks).sum();
        let silent = s.tracks.saturating_sub(lossy + lossless);
        let mut words = format!(
            "The {} lossy tracks, by the rate each file reports for itself in kbps. For a variable-rate file that is its average. ",
            group(lossy)
        );
        if silent > 0 {
            words += &if silent == 1 {
                "One track reports no rate and is left out. ".to_string()
            } else {
                format!("{} tracks report no rate and are left out. ", group(silent))
            };
        }
        words += "“Typical” above is the middle track of its type, not the mean, so a few low-rate files do not drag it down.";
        let mut rows = vec![label(words).size(12.0).dim().wrap().width(Length::FILL)];
        for b in &s.lossy {
            rows.push(table_row(
                label(b.label.clone()).size(13.0),
                120.0,
                Some(b.tracks as f32 / lossy as f32),
                vec![figure(group(b.tracks), 62.0), figure(stats::share(b.tracks, lossy), 52.0)],
            ));
        }
        out.push(section("Bitrates", rows));
    }
    if !s.lossless.is_empty() {
        let lossless: usize = s.lossless.iter().map(|b| b.tracks).sum();
        let rows = s
            .lossless
            .iter()
            .map(|b| {
                table_row(
                    label(b.label.clone()).size(13.0),
                    120.0,
                    Some(b.tracks as f32 / lossless as f32),
                    vec![figure(group(b.tracks), 62.0), figure(stats::share(b.tracks, lossless), 52.0)],
                )
            })
            .collect();
        out.push(section("Lossless", rows));
    }
    let n = |count: usize, one: &str, many: &str| format!("{} {}", group(count), if count == 1 { one } else { many });
    let art_line = |words: String, query: Option<&str>| -> Element<Msg> {
        let mut v = vec![label(words).size(13.0)];
        if let Some(q) = query {
            v.push(
                button(label("show on the wall").size(13.0).color(ink))
                    .look(ButtonLook::Plain)
                    .padding(Padding::all(0.0))
                    .on_press(Msg::Search(q.to_string()))
                    .name(format!("Show on the wall: {q}")),
            );
        }
        row(v).spacing(10.0).align(Align::Center)
    };
    let mut art = vec![art_line(
        if s.embedded == 0 {
            "No album carries its cover inside the files".into()
        } else {
            n(s.embedded, "album carries its", "albums carry their") + " cover inside the files"
        },
        None,
    )];
    if s.sidecar > 0 {
        art.push(art_line(n(s.sidecar, "album uses", "albums use") + " a cover file beside the tracks", None));
    }
    art.push(if s.missing == 0 {
        art_line("No album is missing a cover".into(), None)
    } else {
        art_line(n(s.missing, "album has", "albums have") + " no cover at all", Some("art:missing"))
    });
    if s.small > 0 {
        art.push(art_line(
            n(s.small, "album has", "albums have") + &format!(" a cover smaller than {} px", stats::SMALL_COVER),
            Some("art:small"),
        ));
    }
    out.push(section("Cover art", art));
    column(out).spacing(10.0).width(Length::FILL)
}

fn keys_page() -> Element<Msg> {
    let rows = |list: &[(&str, &str)]| -> Vec<Element<Msg>> {
        list.iter()
            .map(|(keys, does)| {
                row(vec![
                    label(keys.to_string()).size(13.0).width(Length::Fixed(160.0)),
                    label(does.to_string()).size(13.0).wrap().width(Length::FILL),
                ])
                .width(Length::FILL)
            })
            .collect()
    };
    let head = |w: &str, first: bool| -> Element<Msg> {
        panel(label(w.to_string()).size(14.0).weight(600)).padding(Padding {
            top: if first { 0.0 } else { 12.0 },
            bottom: 5.0,
            ..Padding::all(0.0)
        })
    };
    let mut v = vec![head("Keys", true)];
    v.extend(rows(&[
        ("Space", "Play or pause."),
        ("Ctrl+→   Ctrl+←", "Next track, previous track."),
        ("Shift+→   Shift+←", "Forward or back ten seconds."),
        ("Ctrl+F   or   /", "Go to the search box."),
        ("Esc", "Back out of the tracks; or leave full screen; or clear the search; or fold the open album."),
        ("Ctrl+L", "Go to the album that is playing, and open it."),
        ("F11", "Full screen, and back."),
        ("Ctrl+,", "Settings."),
    ]));
    v.push(note(
        "The arrow keys move from cover to cover; Page Up and Page Down by a screen, Home and End to either end. Enter opens an album, ↓ from the open one goes into its tracks, Enter plays one and Esc comes back out. From the search box, ↓ goes to the first album found. How loud is the dock's mixer's: AlbumWall plays at full level, untouched.",
    ));
    v.push(head("Searching", false));
    v.push(note(
        "Type part of an artist's name or an album's title and the wall narrows as you type. Three searches look at the cover art instead:",
    ));
    v.extend(rows(&[
        ("art:missing", "Albums with no cover at all."),
        ("art:small", "Albums whose cover is smaller than 300 pixels."),
        ("art:nonsquare", "Albums whose cover is not square."),
    ]));
    v.push(note(
        "Covers are never fetched from the internet. Better art belongs in the files themselves, put there with a tagger; these searches say which albums to look at.",
    ));
    column(vec![title("Keys and search"), column(v).spacing(3.0).width(Length::FILL)]).spacing(16.0).width(Length::FILL)
}

fn about_page() -> Element<Msg> {
    let fact = |name: &str, words: &str| -> Element<Msg> {
        row(vec![label(name.to_string()).size(13.0).weight(600).width(Length::Fixed(132.0)), note(words.to_string())])
            .width(Length::FILL)
    };
    column(vec![
        column(vec![
            label("AlbumWall").size(34.0).weight(700),
            label(format!("AlbumWall for Vergence, version {}", env!("CARGO_PKG_VERSION"))).size(13.0).dim(),
        ])
        .spacing(2.0),
        note("A player for a music library you own, built around seeing the whole collection at once."),
        column(vec![
            fact("The license", "Apache-2.0 or GPL-2.0-or-later, as all of Vergence: use it, share it, change it."),
            fact(
                "The audio engine",
                "Hymie, AlbumWall's own: the decoders, each run confined in a helper of its own, and gapless playback at each file's own rate.",
            ),
            fact(
                "Accessibility",
                "Motion, text size and contrast are the desktop's: Settings › Accessibility. Everything can be reached from the keyboard (Keys and search lists it), every control has a name for a screen reader, and text is always held to a minimum contrast against whatever color the wall takes from your covers.",
            ),
            fact(
                "The original",
                "The AlbumWall this one is rebuilt from runs on the Mac, Windows and the Raspberry Pi, and looks the same.",
            ),
        ])
        .spacing(7.0)
        .width(Length::FILL),
    ])
    .spacing(12.0)
    .width(Length::FILL)
}

/// A plain word for a hue, in degrees (PrefsWindow.HueName).
fn hue_name(hue: f64) -> &'static str {
    match hue.rem_euclid(360.0) {
        h if h < 15.0 => "red",
        h if h < 45.0 => "orange",
        h if h < 70.0 => "yellow",
        h if h < 160.0 => "green",
        h if h < 200.0 => "teal",
        h if h < 255.0 => "blue",
        h if h < 290.0 => "violet",
        h if h < 335.0 => "pink",
        _ => "red",
    }
}

/// What each way of setting the bars apart does (PrefsWindow.AboutBars).
fn about_bars(name: &str) -> &'static str {
    match name {
        "recede" => "The bars sink below the wall and lose their color, so the wall is the lit thing in the room.",
        "lift" => "The bars rise as a neutral panel, like a toolbar laid over the wall.",
        "ink" => "Near-black and fully neutral: the most separation, and the wall floats.",
        "warm" => "The bars keep your library's warmth, pushed well apart from the wall in lightness.",
        _ => "",
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// 4581 as "4,581".
pub fn group(n: usize) -> String {
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

/// Seconds since 1970 as "Nov 26, 6:05 PM", local time (the Avalonia
/// app's "d MMM, HH:mm" in American form).
fn when_words(secs: i64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let Some(l) = vergence_widgets::files::local(secs) else { return "an earlier read".into() };
    let (h, half) = match l.hour {
        0 => (12, "AM"),
        1..=11 => (l.hour, "AM"),
        12 => (12, "PM"),
        _ => (l.hour - 12, "PM"),
    };
    format!("{} {}, {h}:{:02} {half}", MONTHS[l.month.clamp(0, 11) as usize], l.mday, l.min)
}

impl Program for Prefs {
    type Message = Msg;

    fn title(&self) -> String {
        "AlbumWall Settings".into()
    }

    fn app_id(&self) -> String {
        self.app_id.clone()
    }

    fn role(&self) -> String {
        "settings".into()
    }

    fn theme(&self) -> Theme {
        sheet_theme(&self.desktop)
    }

    fn bar_style(&self) -> Option<vergence_ui::BarStyle> {
        Some(vergence_ui::BarStyle::Light)
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        let page = self.link.borrow().page;
        panel(
            row(vec![
                self.sidebar(page),
                panel(self.page(page, t))
                    .color(t.card)
                    .edge(t.line, 1.0)
                    .radius(6.0)
                    .padding(Padding { top: 20.0, right: 8.0, bottom: 20.0, left: 22.0 })
                    .width(Length::FILL)
                    .height(Length::FILL),
            ])
            .spacing(12.0)
            .width(Length::FILL)
            .height(Length::FILL),
        )
        .color(t.page)
        .radius(0.0)
        .padding(12.0)
        .width(Length::FILL)
        .height(Length::FILL)
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        let s = self.settings.clone();
        match m {
            Msg::Page(p) => self.link.borrow_mut().page = p,
            Msg::Resume(b) => self.write(Settings { resume: b, ..s }),
            Msg::AutoPlay(b) => self.write(Settings { auto_play: b, ..s }),
            Msg::After(a) => self.write(Settings { after: a, ..s }),
            Msg::Gain(g) => self.write(Settings { gain: Some(g), ..s }),
            Msg::MetersAfterGain(b) => self.write(Settings { meters_after_gain: b, ..s }),
            Msg::Meters(b) => self.write(Settings { meters: b, ..s }),
            Msg::Bars(i) => self.write(Settings { bars: i.min(colors::CHROMES.len() - 1), ..s }),
            Msg::Light(v) | Msg::Tint(v) => {
                let (mut light, mut tint) = self.held.unwrap_or((s.light, s.tint));
                if matches!(m, Msg::Light(_)) {
                    light = v.round().clamp(colors::MIN_LIGHT as f32, colors::MAX_LIGHT as f32) as u32;
                } else {
                    tint = v.round().clamp(0.0, colors::MAX_TINT as f32) as u32;
                }
                self.held = Some((light, tint));
                // Shown on the wall as it moves; written when let go.
                let w = {
                    let mut l = self.link.borrow_mut();
                    l.preview = Some((light, tint));
                    l.wall.clone()
                };
                if let Some(w) = w {
                    w.wake();
                }
            }
            Msg::LetGo => {
                if let Some((light, tint)) = self.held.take() {
                    self.link.borrow_mut().preview = None;
                    self.write(Settings { light, tint, ..s });
                }
            }
            Msg::Defaults => {
                let d = Settings::default();
                self.write(Settings { light: d.light, tint: d.tint, bars: d.bars, meters: d.meters, ..s });
            }
            Msg::Select(folder) => self.link.borrow_mut().selected = Some(folder),
            Msg::SwitchTo(i) => {
                let folder = self.link.borrow().libraries.get(i).map(|l| l.folder.clone());
                self.link.borrow_mut().selected = folder;
                Link::ask(&self.link, Ask::SwitchTo(i));
            }
            Msg::LibrarySettings(i) => {
                let folder = self.link.borrow().libraries.get(i).map(|l| l.folder.clone());
                let already = {
                    let mut l = self.link.borrow_mut();
                    l.selected = folder;
                    let was = l.lib_open.is_some();
                    l.lib_open = Some(i);
                    was
                };
                if already {
                    self.link.borrow().tell_windows();
                } else {
                    window.open_window(vergence_widgets::window(LibrarySettings::new(self.link.clone(), &self.app_id)));
                }
            }
            Msg::AddFolder => Link::ask(&self.link, Ask::AddFolder),
            Msg::AddServer => window.open_window(vergence_widgets::window(crate::serverwin::ServerWindow::new(
                self.link.clone(),
                &self.app_id,
            ))),
            Msg::Search(q) => Link::ask(&self.link, Ask::Search(q)),
        }
        window.redraw();
    }

    /// The arrow keys move through the pages while one of them has the
    /// keyboard, and the page follows (the Avalonia ListBox's way).
    fn key_first(&mut self, input: &vergence_widgets::Input, _window: &mut Window) -> Option<bool> {
        let vergence_widgets::Input::Key { sym, pressed: true, .. } = input else { return None };
        if !self.focus.as_deref().is_some_and(|f| f.starts_with("page: ")) {
            return None;
        }
        let step: isize = match *sym {
            vergence_ui::sym::UP => -1,
            vergence_ui::sym::DOWN => 1,
            _ => return None,
        };
        let mut l = self.link.borrow_mut();
        let at = Page::ALL.iter().position(|p| *p == l.page).unwrap_or(0) as isize;
        let to = Page::ALL[(at + step).clamp(0, Page::ALL.len() as isize - 1) as usize];
        l.page = to;
        self.focus_wanted = Some(format!("page: {}", to.name()));
        Some(true)
    }

    fn focus_to(&mut self) -> Option<Option<String>> {
        self.focus_wanted.take().map(Some)
    }

    fn focus_moved(&mut self, name: Option<String>) {
        self.focus = name;
    }

    fn waker(&mut self, waker: Waker) {
        self.link.borrow_mut().prefs = Some(waker);
    }

    /// The wall changed (a library read, the hue measured), or desktop.toml did.
    fn woken(&mut self, _window: &mut Window) -> bool {
        self.reread();
        true
    }

    fn closed(&mut self) {
        let mut l = self.link.borrow_mut();
        l.open = false;
        l.prefs = None;
        l.preview = None;
    }

    fn resized(&mut self, width: f32, height: f32) {
        self.size = Some((width, height));
    }

    /// One size that never changes by itself from page to page (780 by 640;
    /// never smaller than 640 by 480, where the sidebar and a readable page
    /// still fit).
    fn opening_size(&self) -> Option<(u32, u32)> {
        Some((780, 640))
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style {
            kind: vergence_ui::WinKind::Dialog,
            modal: false,
            min: (640.0, 480.0),
            max: (0.0, 0.0),
            alone: false,
            frosted: false,
        }
    }
}

/// One library's settings (LibrarySettingsWindow.cs, milestone 1's part):
/// its name, its folder, reading it again, forgetting it.
pub struct LibrarySettings {
    link: Shared,
    app_id: String,
    desktop: String,
    /// The name as typed, until it is kept.
    name: Option<String>,
}

#[derive(Debug, Clone)]
pub enum LibMsg {
    Name(String),
    KeepName,
    Rescan,
    Forget,
    Close,
}

impl LibrarySettings {
    fn new(link: Shared, app_id: &str) -> LibrarySettings {
        let desktop = vergence_widgets::prefs::desktop_path()
            .map(|p| vergence_config::effective_text_with(Some(&p)))
            .unwrap_or_default();
        LibrarySettings { link, app_id: app_id.into(), desktop, name: None }
    }

    fn library(&self) -> Option<(usize, Line, bool)> {
        let l = self.link.borrow();
        let i = l.lib_open?;
        l.libraries.get(i).map(|x| (i, x.clone(), i == l.current))
    }
}

impl Program for LibrarySettings {
    type Message = LibMsg;

    fn title(&self) -> String {
        match self.library() {
            Some((_, l, _)) => format!("{}: settings", l.name),
            None => "Library settings".into(),
        }
    }

    fn app_id(&self) -> String {
        self.app_id.clone()
    }

    fn role(&self) -> String {
        "library".into()
    }

    /// A library's name is not prose: no spelling marks under it.
    fn theme(&self) -> Theme {
        Theme { spell: false, ..sheet_theme(&self.desktop) }
    }

    fn bar_style(&self) -> Option<vergence_ui::BarStyle> {
        Some(vergence_ui::BarStyle::Light)
    }

    fn view(&self, t: &Theme) -> Element<LibMsg> {
        let Some((_, lib, on_wall)) = self.library() else {
            return panel(note("This library has been forgotten."))
                .padding(20.0)
                .width(Length::FILL)
                .height(Length::FILL);
        };
        let (reading, rescanned) = {
            let l = self.link.borrow();
            (l.reading && on_wall, l.rescanned.clone())
        };
        let typed = self.name.clone().unwrap_or_else(|| lib.name.clone());
        let mut v = vec![
            label(if on_wall { format!("{}  — on the wall", lib.name) } else { lib.name.clone() })
                .size(18.0)
                .weight(600),
            row(vec![
                label("Name").size(13.0),
                field(typed, LibMsg::Name).on_submit(LibMsg::KeepName).width(Length::FILL).name("Name"),
            ])
            .spacing(10.0)
            .align(Align::Center)
            .width(Length::FILL),
            panel(
                label(lib.server.clone().unwrap_or_else(|| lib.folder.to_string_lossy().into_owned()))
                    .size(12.0)
                    .wrap()
                    .width(Length::FILL),
            )
            .color(t.card)
            .edge(t.line, 1.0)
            .radius(4.0)
            .padding(Padding::hv(6.0, 10.0))
            .width(Length::FILL),
            note(if lib.server.is_some() {
                "The albums are the server's, and so is the art. The files are played as they are, and nothing on the server is changed. Its album list and covers are kept on this computer so the wall opens at once."
            } else {
                "Everything under this folder is scanned. Album art is taken from the tags, or from a cover file beside the tracks if there is none."
            }),
        ];
        if on_wall {
            v.push(row(vec![
                panel_button(if reading { "Reading…" } else { "Rescan" }).on_press_if(!reading, LibMsg::Rescan),
            ]));
            v.push(note(
                "Reads every track again. Only needed if a tag edit hasn't shown up: files that look unchanged are not read again. Keep listening.",
            ));
            if let Some(r) = rescanned.filter(|_| !reading) {
                v.push(note(r));
            }
        } else {
            v.push(note("It can be read again once it is on the wall: Switch to, in Settings."));
        }
        v.push(panel(space().height(Length::Fixed(1.0))).color(t.line).width(Length::FILL).height(Length::Fixed(1.0)));
        v.push(row(vec![panel_button("Forget this library").on_press(LibMsg::Forget)]));
        v.push(note(if lib.server.is_some() {
            "Removes it from the list, with its sign-in and the covers kept here. Nothing on the server is touched."
        } else {
            "Removes it from the list. Nothing on disk is touched."
        }));
        v.push(
            row(vec![
                space().width(Length::FILL),
                button(label("Close").size(13.0))
                    .look(ButtonLook::Accent)
                    .padding(Padding::hv(5.0, 14.0))
                    .on_press(LibMsg::Close),
            ])
            .width(Length::FILL),
        );
        panel(column(v).spacing(12.0).width(Length::FILL))
            .color(t.page)
            .radius(0.0)
            .padding(Padding::hv(20.0, 22.0))
            .width(Length::FILL)
            .height(Length::FILL)
    }

    fn update(&mut self, m: LibMsg, window: &mut Window) {
        let Some((i, lib, _)) = self.library() else {
            window.close();
            return;
        };
        match m {
            LibMsg::Name(t) => self.name = Some(t),
            LibMsg::KeepName => {
                if let Some(n) = self.name.take() {
                    let n = n.trim().replace(['\t', '\n'], " ");
                    if !n.is_empty() && n != lib.name {
                        Link::ask(&self.link, Ask::Rename(i, n));
                    }
                }
            }
            LibMsg::Rescan => {
                self.link.borrow_mut().rescanned = None;
                Link::ask(&self.link, Ask::Rescan);
            }
            LibMsg::Forget => {
                self.link.borrow_mut().lib_open = None;
                Link::ask(&self.link, Ask::Forget(i));
                window.close();
            }
            LibMsg::Close => {
                self.update(LibMsg::KeepName, window);
                window.close();
            }
        }
        window.redraw();
    }

    fn waker(&mut self, waker: Waker) {
        self.link.borrow_mut().lib = Some(waker);
    }

    fn woken(&mut self, _window: &mut Window) -> bool {
        true
    }

    fn close_asked(&mut self, window: &mut Window) -> bool {
        // A name typed and not yet kept is kept as the window closes.
        if let Some((i, lib, _)) = self.library()
            && let Some(n) = self.name.take()
        {
            let n = n.trim().replace(['\t', '\n'], " ");
            if !n.is_empty() && n != lib.name {
                Link::ask(&self.link, Ask::Rename(i, n));
            }
        }
        let _ = window;
        true
    }

    fn closed(&mut self) {
        let mut l = self.link.borrow_mut();
        l.lib_open = None;
        l.lib = None;
    }

    fn opening_size(&self) -> Option<(u32, u32)> {
        Some((560, 460))
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style {
            kind: vergence_ui::WinKind::Dialog,
            modal: false,
            min: (460.0, 360.0),
            max: (0.0, 0.0),
            alone: false,
            frosted: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hue_words() {
        assert_eq!(hue_name(11.0), "red");
        assert_eq!(hue_name(22.0), "orange");
        assert_eq!(hue_name(220.0), "blue");
        assert_eq!(hue_name(-10.0), "red");
    }

    #[test]
    fn every_bars_choice_is_explained() {
        for c in colors::CHROMES {
            assert!(!about_bars(c.name).is_empty(), "{}", c.name);
        }
    }

    #[test]
    fn written_keys_are_the_sections() {
        // The window writes only [albumwall].
        let s = Settings { light: 30, ..Settings::default() };
        let text = s.write_into("");
        assert!(text.starts_with(vergence_albumwall::settings::SECTION));
    }
}
