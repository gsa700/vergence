// Vergence settings, on the project's own toolkit (vergence-widgets; moved
// from iced 2026-11-10, toolkit milestone 2, which had replaced Slint on
// 2026-11-08).
//
// THE FORMAT (Dave, 2026-11-08: "similar to the macOS ... designed for desktop
// and not mobiles"; colors from his System Settings screenshot): a warm gray
// sidebar of pages, the selected one filled with the desktop's accent; the page
// on a lighter ground, its heading centered in a card, settings in soft gray
// cards with the name on the left and the control on the right. Every change
// applies at once.
//
// Pages: Appearance; Theme and fonts; Windows (title bar controls, either side,
// chosen buttons); Dock (place, gap, hiding, opacity, pinned applications);
// Keyboard (key repeat: on, delay, rate, a box to try it); Mouse and touchpad
// (speed, acceleration, scrolling lines, natural scrolling, primary button,
// middle click, double-click speed; the touchpad's tapping, scrolling and
// right click; the server's pointer.rs applies them); Energy
// (screens off and sleep after idle, the sleep command); Screens (several screens: an overview to arrange them and
// the size preference) with a page per screen (scale, wallpaper); About.
//
// Files: ~/.config/vergence/desktop.toml ([appearance], [keyboard], [mouse],
// [touchpad], [shelf], [wallpaper],
// [pictures], [displays]) and settings.toml (theme, fonts, the title bar
// layout); the server applies both within a second. The screens come from the
// session bus (crates/vergence-protocol), followed live, on a thread of their
// own; previews are drawn on threads too. Both reach the window through a
// channel and the toolkit's waker.

mod disp;
mod theme;
mod thumbs;
mod wall;
mod winbar;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use vergence_protocol::bus::{ClientMsg, OutputInfo, ServerMsg, Topic};
use vergence_render::text;
use vergence_scene::{Color, Image, Point, Rect, Scene};
use vergence_text::Fonts;
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Program, Theme, Touch, Waker, Window, button, choice, column, drawing, field, label, panel, parse_hex, picture, row, rule, scroll, slider, space, switch,
    vrule,
};

const SCHEMES: [&str; 3] = ["default", "dark", "light"];

// About: copyright, license and notices, compiled in so they always match this
// build (also installed under share/doc/vergence by tools/install.sh).
const COPYRIGHT: &str = "\u{a9} 2026 David Erickson";
const LICENSE_APACHE: &str = include_str!("../../../LICENSE-APACHE");
const LICENSE_GPL: &str = include_str!("../../../LICENSE-GPL");
const NOTICES: &str = include_str!("../../../THIRD-PARTY-NOTICES.md");

/// A license text in paragraphs: blank lines separate them, and the line breaks
/// inside one become spaces so the label can wrap it.
fn paragraphs(text: &str) -> Vec<String> {
    text.split("\n\n").map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|p| !p.is_empty()).collect()
}

/// (crate, version, license) from the notices file's summary table.
fn notices_summary() -> Vec<(String, String, String)> {
    let Some(start) = NOTICES.find("## Summary") else { return Vec::new() };
    NOTICES[start..]
        .lines()
        .skip_while(|l| !l.starts_with("|---"))
        .skip(1)
        .take_while(|l| l.starts_with('|'))
        .filter_map(|l| {
            let cells: Vec<&str> = l.trim_matches('|').split('|').map(str::trim).collect();
            (cells.len() == 3).then(|| (cells[0].to_string(), cells[1].to_string(), cells[2].to_string()))
        })
        .collect()
}

/// Where tools/install.sh put the license files: <prefix>/share/doc/vergence
/// beside <prefix>/bin. None when running from a checkout.
fn installed_doc_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.parent()?.join("share/doc/vergence");
    dir.join("THIRD-PARTY-NOTICES.md").is_file().then_some(dir)
}
const SCHEME_NAMES: [&str; 3] = ["App default", "Dark", "Light"];
/// The Frost choice's names and their values in desktop.toml (Standard when unset).
const FROST_NAMES: [&str; 3] = ["Clear", "Standard", "Heavy"];
const FROST_VALUES: [&str; 3] = ["clear", "standard", "heavy"];
/// The tab in front: the page's own color, or frosted (page when unset).
const FRONT_TAB_NAMES: [&str; 2] = ["Page color", "Frosted"];
const FRONT_TAB_VALUES: [&str; 2] = ["page", "frosted"];
/// The size of controls (Standard when unset); a screen's adds Automatic first.
const CONTROLS_NAMES: [&str; 3] = ["Compact", "Standard", "Large"];
const CONTROLS_VALUES: [&str; 3] = ["compact", "standard", "large"];
const SCREEN_CONTROLS_NAMES: [&str; 4] = ["Automatic", "Compact", "Standard", "Large"];
const FITS: [&str; 4] = ["Fill", "Fit", "Center", "Stretch"];
const SWATCHES: [&str; 8] = ["#3b82f6", "#8b5cf6", "#ec4899", "#ef4444", "#e05a00", "#eab308", "#22c55e", "#6b7280"];
/// The accent where the user names none: the settings app's own blue.
/// The dock's edges: as shown, and as desktop.toml writes them.
const DOCK_EDGES: [(&str, &str); 4] = [("Right edge", "right"), ("Left edge", "left"), ("Top edge", "top"), ("Bottom edge", "bottom")];
/// When the dock hides: as shown, and as desktop.toml writes it.
const DOCK_HIDE: [(&str, &str); 3] = [("Never", "never"), ("When a window covers it", "windows"), ("Always", "always")];
/// Snapping and switching (windows milestone 1), as shown and as desktop.toml
/// `[windows]` writes them; the first of each is the default unless marked.
const SNAP_MODES: [(&str, &str); 3] = [("Off", "off"), ("While Ctrl is held", "ctrl"), ("Always", "always")];
const TOP_EDGE: [(&str, &str); 2] = [("Maximizes", "maximize"), ("Top half", "half")];
const BETWEEN_SCREENS: [(&str, &str); 2] = [("Never snap", "never"), ("After a short press", "press")];
const ALT_TAB: [(&str, &str); 2] = [("Every screen", "all"), ("This screen", "screen")];
const DEFAULT_ACCENT: Color = Color::rgb(0x3b, 0x82, 0xf6);
const WHITE: Color = Color::rgb(255, 255, 255);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Appearance,
    Theme,
    Windows,
    Dock,
    Keyboard,
    Pointer,
    Energy,
    Sound,
    Overview,
    Screen(usize),
    About,
    /// About's sub-pages: the license texts, and the code Vergence is built with.
    LicenseText,
    Notices,
}

#[derive(Debug, Clone)]
enum Message {
    Go(Page),
    // Sound: an output's volume (as it slides), its mute, making it the
    // default, and its test tone.
    SoundVolume(String, f32),
    SoundMute(String, bool),
    SoundDefault(String),
    SoundTone(String),
    // Appearance.
    Scheme(usize),
    Accent(Option<&'static str>),
    AccentText(String),
    AccentSubmit,
    Contrast(bool),
    Motion(bool),
    /// The desktop's own look (look milestone 1): how much shows through
    /// the frost, the tab in front, and Reduce transparency.
    Frost(usize),
    FrontTab(usize),
    Transparency(bool),
    /// The size of controls: the desktop's (Appearance), a screen's own (Screens).
    Controls(usize),
    ScreenControls(usize),
    // Theme and fonts.
    Pick(&'static str, String),
    FontSize(bool, String),
    TermScheme(String),
    // Windows.
    Layout(winbar::Layout),
    /// X11 applications (Xwayland) on or off: desktop.toml [x11] enabled.
    X11(bool),
    /// desktop.toml `[windows]`: a word from one of the lists above, or a switch.
    WinWord(&'static str, &'static str),
    WinFlag(&'static str, bool),
    // The dock (desktop.toml [shelf]; the server reads it within a second).
    DockEdge(usize),
    DockScreen(usize),
    DockGap(f32),
    DockOpacity(f32),
    /// A dock slider let go: what it shows is saved.
    DockSave,
    DockKeepClear(bool),
    DockHide(usize),
    PinAdd(usize),
    PinMove(usize, bool),
    PinRemove(usize),
    // Keyboard.
    Repeat(bool),
    RepeatDelay(f32),
    RepeatRate(f32),
    RepeatDone,
    KeyTest(String),
    // Mouse and touchpad (desktop.toml [mouse] and [touchpad]; the server applies them).
    /// A slider moved: which (table, key), and its value; saved on release.
    PointerSlide(&'static str, &'static str, f32),
    PointerSave(&'static str, &'static str),
    PointerFlag(&'static str, &'static str, bool),
    PrimaryButton(usize),
    PadScroll(usize),
    PadClick(usize),
    /// A press on the double-click test.
    ClickTest,
    // Energy.
    ScreensOff(String),
    SleepAfter(String),
    SleepCommand(String),
    SleepCommandSubmit,
    // Screens.
    Outputs(Vec<OutputInfo>, String),
    Size(f32),
    SizeDone,
    Scale(f32),
    ScaleDone,
    ScaleDefault,
    /// The pointer or a finger on the arrangement: the overview's (drag,
    /// double-click) or a screen page's map (a press opens that screen).
    Arrange(Touch),
    Map(Touch),
    // Wallpaper.
    PickPicture(usize),
    Fit(usize),
    Background(String),
    BackgroundSubmit,
    SameAsPrimary(bool),
    MakePrimary,
    Thumb(PathBuf, Option<thumbs::Thumb>),
    RemoveSource(usize),
    Browse(bool),
    BrowseOpen(usize),
    BrowseUp,
    BrowseAdd,
    BrowseCancel,
}

/// The picture browser: folder mode (add the folder shown) or pictures mode
/// (pick pictures in any folder, then add them).
struct Browser {
    folders: bool,
    dir: PathBuf,
    entries: Vec<(PathBuf, bool)>,
    picked: Vec<PathBuf>,
}

/// How other threads (the bus, the preview drawers) reach the window.
#[derive(Clone)]
struct Mail {
    tx: Sender<Message>,
    waker: Arc<OnceLock<Waker>>,
}

impl Mail {
    fn send(&self, m: Message) -> bool {
        let ok = self.tx.send(m).is_ok();
        if let Some(w) = self.waker.get() {
            w.wake();
        }
        ok
    }
}

/// A screen being dragged on the overview: which, where the press was, how
/// far it has gone (in the drawing's units).
#[derive(Debug, Clone, Copy)]
struct ArrangeDrag {
    index: usize,
    start: (f32, f32),
    offset: (f32, f32),
}

struct App {
    page: Page,
    /// The Sound page (vergence-sound): the system, read when the page opens,
    /// and its outputs as last read.
    sound: Option<Box<dyn vergence_sound::Sound>>,
    sound_outputs: Vec<vergence_sound::Output>,
    desktop: toml::Table,
    xsettings: vergence_xsettings::Settings,
    status: String,
    accent_text: String,
    // Energy: the sleep command as typed, saved on Enter.
    sleep_cmd: String,
    // Theme and fonts: what is installed.
    gtk_themes: Vec<String>,
    icon_themes: Vec<String>,
    cursor_themes: Vec<String>,
    fonts: Vec<String>,
    monos: Vec<String>,
    // Screens.
    outputs: Vec<OutputInfo>,
    setup: String,
    size: f32,
    scale: f32,
    arrange_drag: Option<ArrangeDrag>,
    arrange_click: Option<(usize, Instant)>,
    /// The size the arrangement was last drawn at (its drawing sets it).
    arrange_size: Rc<Cell<(f32, f32)>>,
    // Wallpaper.
    files: Vec<PathBuf>,
    /// Full previews (the screens' tiles), and the grid's, made at the
    /// tile's exact pixel size for the window's scale so nothing is
    /// resampled frame after frame.
    thumbs: HashMap<PathBuf, Arc<Image>>,
    raw: HashMap<PathBuf, thumbs::Thumb>,
    grid: HashMap<PathBuf, Arc<Image>>,
    ui_scale: f32,
    asked: HashSet<PathBuf>,
    bg_text: String,
    browser: Option<Browser>,
    // Keyboard: the sliders while they move (saved on release), and the test box.
    repeat_delay: f32,
    repeat_rate: f32,
    key_test: String,
    /// The mouse and touchpad sliders while they move, by (table, key).
    sliding: HashMap<(&'static str, &'static str), f32>,
    /// The double-click test: the last press, and how many double-clicks.
    click_test: (Option<Instant>, u32),
    /// The dock's gap and opacity while their sliders move (saved on release).
    dock_gap: f32,
    dock_opacity: f32,
    /// What can be pinned: the installed applications, by name.
    apps: Vec<vergence_shell::desktop::Entry>,
    mail: Mail,
    inbox: Receiver<Message>,
}

/// Key repeat as GNOME's defaults have it (the server's too): after 500 ms, 30 a second.
const REPEAT_DELAY: i64 = 500;
const REPEAT_RATE: i64 = 30;

fn keyboard_int(d: &toml::Table, k: &str, default: i64) -> i64 {
    d.get("keyboard").and_then(|a| a.get(k)).and_then(|v| v.as_integer()).unwrap_or(default)
}

// ---- Terminal: desktop.toml [terminal] scheme (vergence-term reads it) ----
const TERM_SCHEMES: [&str; 2] = ["Catppuccin", "Tango"];

fn term_scheme(d: &toml::Table) -> &'static str {
    match d.get("terminal").and_then(|a| a.get("scheme")).and_then(|v| v.as_str()).map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("tango") => "Tango",
        _ => "Catppuccin",
    }
}

// ---- Energy: desktop.toml [energy] (the server's energy.rs reads the same keys) ----
const OFF_CHOICES: [(&str, i64); 11] = [
    ("Never", 0),
    ("1 minute", 1),
    ("2 minutes", 2),
    ("5 minutes", 5),
    ("10 minutes", 10),
    ("15 minutes", 15),
    ("20 minutes", 20),
    ("30 minutes", 30),
    ("45 minutes", 45),
    ("1 hour", 60),
    ("2 hours", 120),
];
const SLEEP_CHOICES: [(&str, i64); 9] =
    [("Never", 0), ("10 minutes", 10), ("15 minutes", 15), ("20 minutes", 20), ("30 minutes", 30), ("45 minutes", 45), ("1 hour", 60), ("2 hours", 120), ("3 hours", 180)];

fn energy_int(d: &toml::Table, k: &str, default: i64) -> i64 {
    d.get("energy").and_then(|a| a.get(k)).and_then(|v| v.as_integer()).unwrap_or(default)
}

fn energy_cmd(d: &toml::Table) -> String {
    d.get("energy").and_then(|a| a.get("sleep-command")).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

/// The labels for a minutes setting, and the current one; a value set by hand
/// that is not among the choices is shown as "N minutes" and kept.
fn minute_choices(choices: &[(&str, i64)], current: i64) -> (Vec<String>, String) {
    let mut list: Vec<String> = choices.iter().map(|(l, _)| l.to_string()).collect();
    let label = match choices.iter().find(|(_, m)| *m == current) {
        Some((l, _)) => l.to_string(),
        None => {
            let l = format!("{current} minutes");
            list.push(l.clone());
            l
        }
    };
    (list, label)
}

fn minutes_of(choices: &[(&str, i64)], label: &str) -> i64 {
    choices
        .iter()
        .find(|(l, _)| *l == label)
        .map(|(_, m)| *m)
        .or_else(|| label.split_whitespace().next().and_then(|n| n.parse().ok()))
        .unwrap_or(0)
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn desktop_path() -> PathBuf {
    home().join(".config/vergence/desktop.toml")
}

fn read_desktop() -> toml::Table {
    std::fs::read_to_string(desktop_path()).ok().and_then(|t| t.parse().ok()).unwrap_or_default()
}

/// Change desktop.toml in place: read it fresh, let `f` edit it, write it back
/// (beside it, then renamed, so the server never reads half a file).
fn update_desktop(f: impl FnOnce(&mut toml::Table)) -> Result<toml::Table, String> {
    let path = desktop_path();
    let mut table = match std::fs::read_to_string(&path) {
        Ok(text) => text.parse::<toml::Table>().map_err(|e| format!("{} is not valid TOML ({e}); not overwriting it", path.display()))?,
        Err(_) => toml::Table::new(),
    };
    f(&mut table);
    let text = toml::to_string(&table).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("toml.new");
    std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| e.to_string())?;
    Ok(table)
}

/// A screen's name: the EDID's, or the connector when the EDID only names the port type.
fn screen_label(o: &OutputInfo) -> String {
    let port_type = o.name.split('-').next().unwrap_or("");
    if o.description.is_empty() || o.description == port_type { o.name.clone() } else { o.description.clone() }
}

fn appearance_text(d: &toml::Table, k: &str) -> String {
    d.get("appearance").and_then(|a| a.get(k)).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn rgb(v: u32) -> Color {
    Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

impl App {
    fn new() -> App {
        let desktop = read_desktop();
        let (tx, inbox) = channel();
        let mail = Mail { tx, waker: Arc::new(OnceLock::new()) };
        let mut app = App {
            page: Page::Appearance,
            accent_text: appearance_text(&desktop, "accent"),
            size: disp::size(&desktop) as f32,
            sleep_cmd: energy_cmd(&desktop),
            sound: None,
            sound_outputs: Vec::new(),
            desktop,
            xsettings: theme::read(),
            status: String::new(),
            gtk_themes: theme::gtk_themes(),
            icon_themes: theme::icon_themes(),
            cursor_themes: theme::cursor_themes(),
            fonts: theme::font_families(false),
            monos: theme::font_families(true),
            outputs: Vec::new(),
            setup: String::new(),
            scale: 1.0,
            arrange_drag: None,
            arrange_click: None,
            arrange_size: Rc::new(Cell::new((1.0, 1.0))),
            files: Vec::new(),
            thumbs: HashMap::new(),
            raw: HashMap::new(),
            grid: HashMap::new(),
            ui_scale: 1.0,
            asked: HashSet::new(),
            bg_text: String::new(),
            browser: None,
            repeat_delay: 0.0,
            repeat_rate: 0.0,
            key_test: String::new(),
            sliding: HashMap::new(),
            click_test: (None, 0),
            dock_gap: 0.0,
            dock_opacity: 1.0,
            apps: vergence_shell::apps::installed(),
            mail,
            inbox,
        };
        app.repeat_delay = keyboard_int(&app.desktop, "repeat-delay", REPEAT_DELAY) as f32;
        app.repeat_rate = keyboard_int(&app.desktop, "repeat-rate", REPEAT_RATE) as f32;
        app.dock_from_file();
        app.load_pictures();
        bus_outputs(app.mail.clone());
        app
    }

    /// desktop.toml as it is now: the server writes it too (the dock dragged,
    /// an application pinned from the dock).
    fn reread(&mut self) {
        let fresh = read_desktop();
        if fresh != self.desktop {
            self.desktop = fresh;
            self.dock_from_file();
        }
    }

    fn save(&mut self, f: impl FnOnce(&mut toml::Table)) {
        match update_desktop(f) {
            Ok(t) => {
                self.desktop = t;
                self.status.clear();
                self.dock_from_file();
            }
            Err(e) => self.status = format!("Could not save: {e}"),
        }
    }

    fn dark(&self) -> bool {
        appearance_text(&self.desktop, "color-scheme") == "dark"
    }

    fn accent(&self) -> Color {
        parse_hex(&appearance_text(&self.desktop, "accent")).unwrap_or(DEFAULT_ACCENT)
    }

    /// The pictures on offer (the system's and the user's), with previews.
    fn load_pictures(&mut self) {
        let (folders, files) = wall::sources(&self.desktop);
        self.files = wall::pictures(&folders, &files);
        let list = self.files.clone();
        self.ask_thumbs(list);
    }

    /// Draw previews not yet asked for, each on its own thread (an 8K JPEG
    /// takes a second to decode; the cache makes later opens instant).
    fn ask_thumbs(&mut self, paths: Vec<PathBuf>) {
        for p in paths.into_iter().filter(|p| self.asked.insert(p.clone())) {
            let mail = self.mail.clone();
            std::thread::spawn(move || {
                let t = thumbs::get(&p);
                mail.send(Message::Thumb(p, t));
            });
        }
    }

    fn current_output(&self) -> Option<&OutputInfo> {
        match self.page {
            Page::Screen(i) => self.outputs.get(i),
            _ => None,
        }
    }

    fn primary(&self) -> Option<String> {
        let ids: Vec<String> = self.outputs.iter().map(|o| o.id.clone()).collect();
        wall::primary(&self.desktop, &ids)
    }

    /// Where the open screen page's wallpaper is kept: None for the primary
    /// (and the only) screen; else its id and label.
    fn scope(&self) -> Option<(String, String)> {
        let o = self.current_output()?;
        (self.primary().as_deref() != Some(o.id.as_str())).then(|| (o.id.clone(), screen_label(o)))
    }

    fn open(&mut self, p: Page) {
        self.page = p;
        self.reread();
        if p == Page::Sound {
            self.sound_read();
        }
        self.browser = None;
        self.arrange_drag = None;
        let scope = self.scope();
        self.bg_text = wall::shown(&self.desktop, scope.as_ref().map(|(i, _)| i.as_str())).color;
        if let Some(o) = self.current_output() {
            self.scale = o.base_scale as f32;
        }
    }

    fn handle(&mut self, m: Message) {
        match m {
            Message::Go(p) => self.open(p),
            Message::Scheme(i) => self.save_appearance(|t| {
                t.insert("color-scheme".into(), SCHEMES[i.min(2)].into());
            }),
            Message::Accent(hex) => {
                self.accent_text = hex.unwrap_or("").to_string();
                self.save_accent();
            }
            Message::AccentText(t) => self.accent_text = t,
            Message::AccentSubmit => self.save_accent(),
            Message::Contrast(on) => self.save_appearance(|t| {
                t.insert("contrast".into(), if on { "high" } else { "normal" }.into());
            }),
            Message::Motion(on) => self.save_appearance(|t| {
                t.insert("reduced-motion".into(), on.into());
            }),
            Message::Frost(i) => self.save_appearance(|t| {
                t.insert("frost".into(), FROST_VALUES[i.min(2)].into());
            }),
            Message::FrontTab(i) => self.save_appearance(|t| {
                t.insert("front-tab".into(), FRONT_TAB_VALUES[i.min(1)].into());
            }),
            Message::Transparency(on) => self.save_appearance(|t| {
                t.insert("reduce-transparency".into(), on.into());
            }),
            Message::Controls(i) => self.save_appearance(|t| {
                t.insert("controls".into(), CONTROLS_VALUES[i.min(2)].into());
            }),
            Message::ScreenControls(i) => {
                if let Some(o) = self.current_output() {
                    let (id, label) = (o.id.clone(), screen_label(o));
                    self.save(|d| disp::set_controls(d, &id, &label, i));
                }
            }
            Message::Pick(key, value) => self.set_x(key, &value),
            Message::FontSize(mono, size) => {
                let key = if mono { theme::MONO } else { theme::FONT };
                let (family, _) = theme::split_font(&theme::get(&self.xsettings, key));
                if let Ok(sz) = size.parse::<f32>() {
                    self.set_x(key, &theme::join_font(&family, sz));
                }
            }
            Message::Layout(l) => self.set_x(winbar::KEY, &winbar::format(l)),
            Message::X11(on) => self.save_table("x11", |t| {
                t.insert("enabled".into(), on.into());
            }),
            Message::WinWord(key, word) => self.save_table("windows", |t| {
                t.insert(key.into(), word.into());
            }),
            Message::WinFlag(key, on) => self.save_table("windows", |t| {
                t.insert(key.into(), on.into());
            }),
            Message::Outputs(outputs, setup) => {
                let current = self.current_output().map(|o| o.id.clone());
                self.outputs = outputs;
                self.setup = setup;
                self.arrange_drag = None;
                // Keep the same screen's page if it is still there.
                if let Page::Screen(_) = self.page {
                    let i = current.and_then(|id| self.outputs.iter().position(|o| o.id == id)).unwrap_or(0);
                    self.page = Page::Screen(i);
                }
                if self.page == Page::Overview && self.outputs.len() <= 1 {
                    self.page = Page::Screen(0);
                }
                if let Some(o) = self.current_output() {
                    self.scale = o.base_scale as f32;
                }
                // The primary's own entry (from before there was one) folds into the default.
                if let Some(p) = self.primary()
                    && wall::shown(&self.desktop, Some(&p)).own
                {
                    self.save(|d| wall::make_primary(d, None, &p));
                }
                let wanted: Vec<PathBuf> = self.outputs.iter().filter_map(|o| wall::shown(&self.desktop, Some(&o.id)).image).collect();
                self.ask_thumbs(wanted);
            }
            Message::DockEdge(i) => {
                // Choosing an edge docks it there; "where it was put" is only shown, not chosen.
                if let Some(edge) = DOCK_EDGES.get(i) {
                    let edge = edge.1;
                    self.save_table("shelf", |t| {
                        for k in ["x", "y", "upright"] {
                            t.remove(k);
                        }
                        t.insert("place".into(), "edge".into());
                        t.insert("edge".into(), edge.into());
                    });
                }
            }
            Message::DockScreen(i) => {
                let id = if i == 0 { None } else { self.outputs.get(i - 1).map(|o| o.id.clone()) };
                self.save_table("shelf", |t| match id {
                    Some(id) => {
                        t.insert("screen".into(), id.into());
                    }
                    None => {
                        t.remove("screen");
                    }
                });
            }
            Message::DockGap(v) => self.dock_gap = v.round(),
            Message::DockOpacity(v) => self.dock_opacity = (v * 20.0).round() / 20.0,
            Message::DockSave => {
                let (gap, opacity) = (self.dock_gap as i64, (self.dock_opacity as f64 * 100.0).round() / 100.0);
                self.save_table("shelf", |t| {
                    t.insert("gap".into(), gap.into());
                    t.insert("opacity".into(), opacity.into());
                });
            }
            Message::DockKeepClear(on) => self.save_table("shelf", |t| {
                t.insert("keep-clear".into(), on.into());
            }),
            Message::DockHide(i) => {
                let word = DOCK_HIDE.get(i).map_or("never", |h| h.1);
                self.save_table("shelf", |t| {
                    t.insert("hide".into(), word.into());
                });
            }
            Message::PinAdd(i) => {
                let unpinned = self.unpinned();
                if let Some(id) = unpinned.get(i).map(|e| e.id.clone()) {
                    let mut pinned = self.pinned();
                    pinned.push(id);
                    self.save_pinned(pinned);
                }
            }
            Message::PinMove(i, up) => {
                let mut pinned = self.pinned();
                let j = if up { i.checked_sub(1) } else { Some(i + 1).filter(|j| *j < pinned.len()) };
                if let Some(j) = j.filter(|_| i < pinned.len()) {
                    pinned.swap(i, j);
                    self.save_pinned(pinned);
                }
            }
            Message::PinRemove(i) => {
                let mut pinned = self.pinned();
                if i < pinned.len() {
                    pinned.remove(i);
                    self.save_pinned(pinned);
                }
            }
            Message::Size(v) => self.size = if (v - 1.0).abs() < 0.035 { 1.0 } else { v },
            Message::Repeat(on) => self.save_keyboard(|t| {
                t.insert("repeat".into(), on.into());
            }),
            Message::RepeatDelay(v) => self.repeat_delay = v,
            Message::RepeatRate(v) => self.repeat_rate = v,
            Message::SoundVolume(id, v) => self.sound_do(|s| s.set_volume(&id, v.round() as u8)),
            Message::SoundMute(id, on) => self.sound_do(|s| s.set_muted(&id, on)),
            Message::SoundDefault(id) => self.sound_do(|s| s.set_default(&id)),
            Message::SoundTone(id) => {
                // 1.5 s of tone on its own thread (the window keeps answering),
                // through a sound system of its own.
                std::thread::spawn(move || {
                    if let Some(mut s) = vergence_sound::open() {
                        let _ = s.tone(&id, 2, &[1, 2]);
                    }
                });
            }
            Message::RepeatDone => {
                let (delay, rate) = (self.repeat_delay.round() as i64, self.repeat_rate.round() as i64);
                self.save_keyboard(|t| {
                    t.insert("repeat-delay".into(), delay.into());
                    t.insert("repeat-rate".into(), rate.into());
                });
            }
            Message::KeyTest(t) => self.key_test = t,
            Message::PointerSlide(table, key, v) => {
                self.sliding.insert((table, key), v);
            }
            Message::PointerSave(table, key) => {
                if let Some(v) = self.sliding.remove(&(table, key)) {
                    let value: toml::Value = if key == "speed" { ((v as f64 * 100.0).round() / 100.0).into() } else { (v.round() as i64).into() };
                    self.save_table(table, |t| {
                        t.insert(key.into(), value);
                    });
                }
            }
            Message::PointerFlag(table, key, on) => self.save_table(table, |t| {
                t.insert(key.into(), on.into());
            }),
            Message::PrimaryButton(i) => self.save_table("mouse", |t| {
                t.insert("left-handed".into(), (i == 1).into());
            }),
            Message::PadScroll(i) => self.save_table("touchpad", |t| {
                t.insert("scroll".into(), if i == 1 { "edge" } else { "two-finger" }.into());
            }),
            Message::PadClick(i) => self.save_table("touchpad", |t| {
                t.insert("click".into(), if i == 1 { "fingers" } else { "areas" }.into());
            }),
            Message::ClickTest => {
                let limit = Duration::from_millis(self.pointer_int("mouse", "double-click", 400) as u64);
                let now = Instant::now();
                match self.click_test.0 {
                    Some(at) if now.duration_since(at) <= limit => {
                        self.click_test = (None, self.click_test.1 + 1);
                    }
                    _ => self.click_test.0 = Some(now),
                }
            }
            Message::TermScheme(name) => {
                let v = name.to_ascii_lowercase();
                self.save_table("terminal", |t| {
                    t.insert("scheme".into(), v.into());
                })
            }
            Message::ScreensOff(label) => {
                let m = minutes_of(&OFF_CHOICES, &label);
                self.save_table("energy", |t| {
                    t.insert("screens-off".into(), m.into());
                })
            }
            Message::SleepAfter(label) => {
                let m = minutes_of(&SLEEP_CHOICES, &label);
                self.save_table("energy", |t| {
                    t.insert("sleep".into(), m.into());
                })
            }
            Message::SleepCommand(s) => self.sleep_cmd = s,
            Message::SleepCommandSubmit => {
                let v = self.sleep_cmd.trim().to_string();
                self.save_table("energy", |t| {
                    t.insert("sleep-command".into(), v.into());
                })
            }
            Message::SizeDone => {
                let s = (self.size as f64).clamp(0.75, 1.5);
                self.save(|d| disp::set_size(d, s));
            }
            Message::Scale(v) => {
                let marks = self.scale_marks();
                self.scale = disp::stick(v as f64, &marks, 0.06).map(|m| m as f32).unwrap_or(v);
            }
            Message::ScaleDone => {
                if let Some(o) = self.current_output() {
                    let v = (self.scale as f64 * 120.0).round() / 120.0;
                    let choice = ((v - o.default_scale).abs() > 1e-6).then_some(v);
                    let (id, label) = (o.id.clone(), screen_label(o));
                    self.save(|d| disp::set_scale(d, &id, &label, choice));
                }
            }
            Message::ScaleDefault => {
                if let Some(o) = self.current_output() {
                    let (id, label, d) = (o.id.clone(), screen_label(o), o.default_scale);
                    self.scale = d as f32;
                    self.save(|d| disp::set_scale(d, &id, &label, None));
                }
            }
            Message::Arrange(t) => self.arrange_touch(t),
            Message::Map(Touch::Press { x, y }) => {
                let size = self.arrange_size.get();
                if let Some(i) = tiles(&self.outputs, size).iter().position(|r| inside(r, x, y)) {
                    self.open(Page::Screen(i));
                }
            }
            Message::Map(_) => {}
            Message::PickPicture(i) => {
                if let Some(p) = self.files.get(i).cloned() {
                    self.save_wallpaper(Some(Some(&p)), None, None);
                }
            }
            Message::Fit(i) => self.save_wallpaper(None, Some(wall::MODES[i.min(3)]), None),
            Message::Background(t) => self.bg_text = t,
            Message::BackgroundSubmit => {
                let c = self.bg_text.trim().to_string();
                if parse_hex(&c).is_some() {
                    self.save_wallpaper(None, None, Some(&c));
                } else {
                    self.status = "Background color: write it as #rrggbb".into();
                }
            }
            Message::SameAsPrimary(same) => {
                if let Some((id, label)) = self.scope() {
                    let shown = wall::shown(&self.desktop, Some(&id));
                    self.save(|d| {
                        if same {
                            wall::clear_own(d, &id);
                        } else {
                            wall::set(d, Some(&id), &label, shown.image.as_deref(), &shown.mode, &shown.color);
                        }
                    });
                }
            }
            Message::MakePrimary => {
                if let Some(new) = self.current_output().map(|o| o.id.clone()) {
                    let old = self.primary().and_then(|p| self.outputs.iter().find(|o| o.id == p).map(|o| (o.id.clone(), screen_label(o))));
                    self.save(|d| wall::make_primary(d, old.as_ref().map(|(i, l)| (i.as_str(), l.as_str())), &new));
                }
            }
            Message::Thumb(p, t) => {
                if let Some(t) = t {
                    self.grid.insert(p.clone(), Arc::new(thumbs::grid_image(&t, self.ui_scale)));
                    self.thumbs.insert(p.clone(), Arc::new(Image { width: t.w, height: t.h, rgba: t.rgba.clone() }));
                    self.raw.insert(p, t);
                }
            }
            Message::RemoveSource(i) => {
                let (mut folders, mut files) = wall::sources(&self.desktop);
                if i < folders.len() {
                    folders.remove(i);
                } else if i - folders.len() < files.len() {
                    files.remove(i - folders.len());
                }
                self.save(|d| wall::set_sources(d, &folders, &files));
                self.load_pictures();
            }
            Message::Browse(folders) => {
                let pics = home().join("Pictures");
                let dir = if pics.is_dir() { pics } else { home() };
                self.browser = Some(Browser { folders, entries: list(&dir), dir, picked: Vec::new() });
            }
            Message::BrowseOpen(i) => {
                if let Some(b) = self.browser.as_mut()
                    && let Some((p, folder)) = b.entries.get(i).cloned()
                {
                    if folder {
                        b.entries = list(&p);
                        b.dir = p;
                    } else if !b.folders {
                        match b.picked.iter().position(|x| *x == p) {
                            Some(k) => {
                                b.picked.remove(k);
                            }
                            None => b.picked.push(p),
                        }
                    }
                }
            }
            Message::BrowseUp => {
                if let Some(b) = self.browser.as_mut()
                    && let Some(parent) = b.dir.parent().map(Path::to_path_buf)
                {
                    b.entries = list(&parent);
                    b.dir = parent;
                }
            }
            Message::BrowseAdd => {
                if let Some(b) = self.browser.take() {
                    let (mut folders, mut files) = wall::sources(&self.desktop);
                    if b.folders {
                        if !folders.contains(&b.dir) {
                            folders.push(b.dir);
                        }
                    } else {
                        for p in b.picked {
                            if !files.contains(&p) {
                                files.push(p);
                            }
                        }
                    }
                    self.save(|d| wall::set_sources(d, &folders, &files));
                    self.load_pictures();
                }
            }
            Message::BrowseCancel => self.browser = None,
        }
    }

    /// The pointer or a finger on the overview's arrangement: a press on a
    /// screen picks it up (a second within 400 ms opens its page), a move
    /// drags it, letting go puts it down where it snaps.
    fn arrange_touch(&mut self, t: Touch) {
        let size = self.arrange_size.get();
        match t {
            Touch::Press { x, y } => {
                let Some(i) = tiles(&self.outputs, size).iter().position(|r| inside(r, x, y)) else { return };
                if let Some((j, at)) = self.arrange_click
                    && j == i
                    && at.elapsed() < Duration::from_millis(400)
                {
                    self.arrange_click = None;
                    self.arrange_drag = None;
                    self.open(Page::Screen(i));
                    return;
                }
                self.arrange_click = Some((i, Instant::now()));
                self.arrange_drag = Some(ArrangeDrag { index: i, start: (x, y), offset: (0.0, 0.0) });
            }
            Touch::Move { x, y, pressed: true } => {
                if let Some(d) = self.arrange_drag.as_mut() {
                    d.offset = (x - d.start.0, y - d.start.1);
                }
            }
            Touch::Release { .. } | Touch::Leave => {
                let Some(d) = self.arrange_drag.take() else { return };
                if d.offset.0.abs() < 2.0 && d.offset.1.abs() < 2.0 || d.index >= self.outputs.len() {
                    return;
                }
                // Back to desktop pixels, as fractions of the arrangement's size.
                let (x0, y0, bw, bh) = bbox(&self.outputs);
                let (_, k) = area(&self.outputs, size);
                let o = &self.outputs[d.index];
                let fx = ((o.x - x0) as f32 + d.offset.0 / k) / bw as f32;
                let fy = ((o.y - y0) as f32 + d.offset.1 / k) / bh as f32;
                let mut rects: Vec<disp::Rect> = self.outputs.iter().map(|o| disp::Rect { id: o.id.clone(), x: o.x, y: o.y, w: o.width, h: o.height }).collect();
                let (x, y) = (x0 + (fx * bw as f32).round() as i32, y0 + (fy * bh as f32).round() as i32);
                let (nx, ny) = disp::snap(&rects, d.index, x, y, (bw.max(bh) / 40).max(20));
                rects[d.index].x = nx;
                rects[d.index].y = ny;
                let setup = self.setup.clone();
                self.save(|dt| disp::set_arrangement(dt, &setup, &rects));
            }
            Touch::Move { .. } => {}
        }
    }

    fn save_keyboard(&mut self, f: impl FnOnce(&mut toml::Table)) {
        self.save_table("keyboard", f);
    }

    /// Change one table of desktop.toml, making it if it is missing.
    fn save_table(&mut self, name: &str, f: impl FnOnce(&mut toml::Table)) {
        self.save(|d| {
            if !d.get(name).is_some_and(|a| a.is_table()) {
                d.insert(name.into(), toml::Value::Table(toml::Table::new()));
            }
            if let Some(t) = d.get_mut(name).and_then(|a| a.as_table_mut()) {
                f(t);
            }
        });
    }

    fn save_appearance(&mut self, f: impl FnOnce(&mut toml::Table)) {
        self.save_table("appearance", f);
    }

    fn save_accent(&mut self) {
        let a = self.accent_text.trim().to_string();
        if !a.is_empty() && parse_hex(&a).is_none() {
            self.status = "Accent color: write it as #rrggbb, or leave it empty".into();
            return;
        }
        self.save_appearance(|t| {
            if a.is_empty() {
                t.remove("accent");
            } else {
                t.insert("accent".into(), a.into());
            }
        });
    }

    fn set_x(&mut self, key: &str, value: &str) {
        match theme::set(key, value) {
            Ok(()) => {
                self.xsettings = theme::read();
                self.status.clear();
            }
            Err(e) => self.status = format!("Could not save: {e}"),
        }
    }

    /// A picture, fit or color chosen: for the open screen (which then has its
    /// own wallpaper) or for the primary (the default).
    fn save_wallpaper(&mut self, image: Option<Option<&Path>>, mode: Option<&str>, color: Option<&str>) {
        let scope = self.scope();
        let shown = wall::shown(&self.desktop, scope.as_ref().map(|(i, _)| i.as_str()));
        let image = image.unwrap_or(shown.image.as_deref()).map(Path::to_path_buf);
        let mode = mode.unwrap_or(&shown.mode).to_string();
        let color = color.unwrap_or(&shown.color).to_string();
        self.save(|d| match &scope {
            Some((id, label)) => wall::set(d, Some(id), label, image.as_deref(), &mode, &color),
            None => wall::set(d, None, "", image.as_deref(), &mode, &color),
        });
    }

    fn scale_marks(&self) -> Vec<f64> {
        match self.current_output() {
            Some(o) => disp::detents(o.default_scale, scale_max(o)),
            None => vec![1.0],
        }
    }

    // ---- the view ----

    fn sidebar(&self) -> Element<Message> {
        let item = |text: String, page: Page, selected: bool, child: bool| -> Element<Message> {
            let l = label(text).size(if child { 12.0 } else { 13.0 }).weight(if selected { 600 } else { 400 });
            button(row(vec![l, space().width(Length::FILL)]).width(Length::FILL))
                .look(if selected { ButtonLook::Chosen } else { ButtonLook::Plain })
                .padding(Padding { top: 6.0, bottom: 6.0, left: if child { 26.0 } else { 12.0 }, right: 12.0 })
                .on_press(Message::Go(page))
                .width(Length::FILL)
        };
        let multi = self.outputs.len() > 1;
        let screens_page = if multi { Page::Overview } else { Page::Screen(0) };
        let screens_selected = if multi { self.page == Page::Overview } else { matches!(self.page, Page::Screen(_)) };
        let mut items = vec![
            item("Appearance".into(), Page::Appearance, self.page == Page::Appearance, false),
            item("Theme and fonts".into(), Page::Theme, self.page == Page::Theme, false),
            item("Windows".into(), Page::Windows, self.page == Page::Windows, false),
            item("Dock".into(), Page::Dock, self.page == Page::Dock, false),
            item("Keyboard".into(), Page::Keyboard, self.page == Page::Keyboard, false),
            item("Mouse and touchpad".into(), Page::Pointer, self.page == Page::Pointer, false),
            item("Sound".into(), Page::Sound, self.page == Page::Sound, false),
            item("Energy".into(), Page::Energy, self.page == Page::Energy, false),
            item(if self.outputs.is_empty() { "Wallpaper".into() } else { "Screens".into() }, screens_page, screens_selected, false),
        ];
        if multi {
            for (i, o) in self.outputs.iter().enumerate() {
                items.push(item(screen_label(o), Page::Screen(i), self.page == Page::Screen(i), true));
            }
        }
        items.push(space().height(Length::Fixed(14.0)));
        let about = matches!(self.page, Page::About | Page::LicenseText | Page::Notices);
        items.push(item("About".into(), Page::About, about, false));
        panel(column(items).spacing(2.0).width(Length::FILL))
            .padding(Padding { top: 12.0, bottom: 12.0, left: 10.0, right: 10.0 })
            .radius(0.0)
            .width(Length::Fixed(200.0))
            .height(Length::FILL)
            .name("Pages")
    }

    fn appearance(&self) -> Element<Message> {
        let scheme = SCHEMES.iter().position(|s| *s == appearance_text(&self.desktop, "color-scheme")).unwrap_or(0);
        let accent = appearance_text(&self.desktop, "accent");
        let none_selected = accent.is_empty();
        let text_c = self.text_color();
        let line_c = self.line_color();
        let mut swatches = vec![
            button(label("\u{2715}").size(10.0).dim())
                .padding(0.0)
                .width(Length::Fixed(22.0))
                .height(Length::Fixed(22.0))
                .look(ButtonLook::Custom { fill: None, edge: Some(if none_selected { (text_c, 2.0) } else { (line_c, 1.0) }), radius: 11.0 })
                .on_press(Message::Accent(None))
                .name("No accent"),
        ];
        for hex in SWATCHES {
            let col = parse_hex(hex).unwrap_or(Color::rgb(0, 0, 0));
            let selected = accent.eq_ignore_ascii_case(hex);
            swatches.push(
                button(space())
                    .padding(0.0)
                    .width(Length::Fixed(22.0))
                    .height(Length::Fixed(22.0))
                    .look(ButtonLook::Custom { fill: Some(col), edge: selected.then_some((text_c, 2.0)), radius: 11.0 })
                    .on_press(Message::Accent(Some(hex)))
                    .name(format!("Accent {hex}")),
            );
        }
        let contrast = appearance_text(&self.desktop, "contrast") == "high";
        let motion = self.desktop.get("appearance").and_then(|a| a.get("reduced-motion")).and_then(|v| v.as_bool()).unwrap_or(false);
        let names: Vec<String> = SCHEME_NAMES.iter().map(|s| s.to_string()).collect();
        let transparency = self.desktop.get("appearance").and_then(|a| a.get("reduce-transparency")).and_then(|v| v.as_bool()).unwrap_or(false);
        let frost = FROST_VALUES.iter().position(|v| *v == appearance_text(&self.desktop, "frost")).unwrap_or(1);
        let front = FRONT_TAB_VALUES.iter().position(|v| *v == appearance_text(&self.desktop, "front-tab")).unwrap_or(0);
        let frost_names: Vec<String> = FROST_NAMES.iter().map(|s| s.to_string()).collect();
        let front_names: Vec<String> = FRONT_TAB_NAMES.iter().map(|s| s.to_string()).collect();
        let controls = CONTROLS_VALUES.iter().position(|v| *v == appearance_text(&self.desktop, "controls")).unwrap_or(1);
        let controls_names: Vec<String> = CONTROLS_NAMES.iter().map(|s| s.to_string()).collect();
        self.page_layout(
            "Appearance",
            "Light or dark, the accent color, contrast and motion. Every app that follows the desktop changes at once.",
            vec![
                self.group(None, vec![
                    setting("Style", "App default lets each app choose.", choice(names, Some(scheme), Message::Scheme).width(Length::Fixed(160.0)).name("Style")),
                    setting("Accent color", if accent.is_empty() { "None: each app uses its own." } else { "" }, row(swatches).spacing(6.0).align(Align::Center)),
                    setting(
                        "Custom accent",
                        "Any color as #rrggbb; Enter applies it.",
                        field(&self.accent_text, Message::AccentText).placeholder("#rrggbb").on_submit(Message::AccentSubmit).width(Length::Fixed(120.0)).name("Custom accent"),
                    ),
                ]),
                self.group(Some("Desktop look"), vec![
                    setting("Frost", "How much of a window shows through its bar. Titles stay readable whatever is under them.", choice(frost_names, Some(frost), Message::Frost).width(Length::Fixed(160.0)).name("Frost")),
                    setting("Tab in front", "In the page's own color, joined to it, or frosted like the rest of the bar.", choice(front_names, Some(front), Message::FrontTab).width(Length::Fixed(160.0)).name("Tab in front")),
                    setting("Size of controls", "Window bars, tabs and buttons. A screen with a touch panel uses Large unless its own setting says otherwise (Screens).", choice(controls_names, Some(controls), Message::Controls).width(Length::Fixed(160.0)).name("Size of controls")),
                ]),
                self.group(Some("Accessibility"), vec![
                    setting("High contrast", "Apps that support it draw stronger edges and text. The desktop's bars are solid.", switch(contrast, Message::Contrast).name("High contrast")),
                    setting("Reduce motion", "Apps that support it turn off animations.", switch(motion, Message::Motion).name("Reduce motion")),
                    setting("Reduce transparency", "Bars, the dock and menus are solid: nothing shows through them.", switch(transparency, Message::Transparency).name("Reduce transparency")),
                ]),
                self.status_line(),
            ],
        )
    }

    fn theme_page(&self) -> Element<Message> {
        let with = |mut list: Vec<String>, cur: &str| {
            if !cur.is_empty() && !list.iter().any(|x| x == cur) {
                list.insert(0, cur.to_string());
            }
            list
        };
        let pick = |key: &'static str, list: &[String], name: &str| -> Element<Message> {
            let cur = theme::get(&self.xsettings, key);
            let list = with(list.to_vec(), &cur);
            let at = list.iter().position(|x| *x == cur);
            let options = list.clone();
            choice(options, at, move |i| Message::Pick(key, list[i].clone())).width(Length::Fixed(220.0)).name(name)
        };
        let num = |s: f32| if s.fract() == 0.0 { format!("{}", s as i32) } else { format!("{s}") };
        let font_row = |mono: bool| -> Element<Message> {
            let key = if mono { theme::MONO } else { theme::FONT };
            let (family, size) = theme::split_font(&theme::get(&self.xsettings, key));
            let families = with(if mono { self.monos.clone() } else { self.fonts.clone() }, &family);
            let at = families.iter().position(|f| *f == family);
            let sizes: Vec<String> = theme::sizes(size).into_iter().map(num).collect();
            let size_at = sizes.iter().position(|s| *s == num(size));
            let (fam_list, size_list) = (families.clone(), sizes.clone());
            row(vec![
                choice(families, at, move |i| Message::Pick(key, theme::join_font(&fam_list[i], size))).width(Length::Fixed(220.0)).name(if mono { "Monospace font" } else { "Interface font" }),
                choice(sizes, size_at, move |i| Message::FontSize(mono, size_list[i].clone())).width(Length::Fixed(76.0)).name("Size"),
            ])
            .spacing(6.0)
        };
        let schemes: Vec<String> = TERM_SCHEMES.iter().map(|s| s.to_string()).collect();
        let term_at = TERM_SCHEMES.iter().position(|s| *s == term_scheme(&self.desktop));
        self.page_layout(
            "Theme and fonts",
            "How apps look: their theme, icons, pointer and fonts. GTK and X11 apps follow at once; the newest GNOME apps keep their own look.",
            vec![
                self.group(Some("Theme"), vec![
                    setting("Applications", "Buttons, menus and windows in GTK apps.", pick(theme::THEME, &self.gtk_themes, "Applications theme")),
                    setting("Icons", "", pick(theme::ICONS, &self.icon_themes, "Icons")),
                    setting("Pointer", "The same size on every screen.", pick(theme::CURSOR, &self.cursor_themes, "Pointer")),
                ]),
                self.group(Some("Fonts"), vec![
                    setting("Interface", "Menus, buttons, labels, and the title bars.", font_row(false)),
                    setting("Monospace", "Terminals and code.", font_row(true)),
                ]),
                self.group(Some("Terminal"), vec![setting(
                    "Colors",
                    "The terminal's 16 colors. Tango is the classic set that mc, htop and vim were tuned for; Catppuccin is softer.",
                    choice(schemes.clone(), term_at, move |i| Message::TermScheme(schemes[i].clone())).width(Length::Fixed(160.0)).name("Terminal colors"),
                )]),
                self.status_line(),
            ],
        )
    }

    fn pointer_table(&self, table: &str) -> Option<&toml::Table> {
        self.desktop.get(table).and_then(|v| v.as_table())
    }

    fn pointer_float(&self, table: &'static str, key: &'static str, default: f32) -> f32 {
        if let Some(v) = self.sliding.get(&(table, key)) {
            return *v;
        }
        self.pointer_table(table).and_then(|t| t.get(key)).and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64))).map_or(default, |v| v as f32)
    }

    fn pointer_int(&self, table: &'static str, key: &'static str, default: i64) -> i64 {
        if let Some(v) = self.sliding.get(&(table, key)) {
            return v.round() as i64;
        }
        self.pointer_table(table).and_then(|t| t.get(key)).and_then(|v| v.as_integer()).unwrap_or(default)
    }

    fn pointer_flag(&self, table: &str, key: &str, default: bool) -> bool {
        self.pointer_table(table).and_then(|t| t.get(key)).and_then(|v| v.as_bool()).unwrap_or(default)
    }

    fn pointer_word(&self, table: &str, key: &str) -> String {
        self.pointer_table(table).and_then(|t| t.get(key)).and_then(|v| v.as_str()).unwrap_or("").to_string()
    }

    fn pointer(&self) -> Element<Message> {
        let speed = |table: &'static str| {
            let v = self.pointer_float(table, "speed", 0.0).clamp(-1.0, 1.0);
            let words = if v < -0.05 { "slower" } else if v > 0.05 { "faster" } else { "standard" };
            row(vec![
                slider(-1.0, 1.0, v, move |v| Message::PointerSlide(table, "speed", v)).step(0.05).marks(vec![0.0]).on_release(Message::PointerSave(table, "speed")).width(Length::Fixed(220.0)).name("Pointer speed"),
                label(words).width(Length::Fixed(72.0)).align(Align::End),
            ])
            .spacing(8.0)
            .align(Align::Center)
        };
        let lines = |table: &'static str| {
            let n = self.pointer_int(table, "scroll-lines", 3).clamp(1, 10);
            row(vec![
                slider(1.0, 10.0, n as f32, move |v| Message::PointerSlide(table, "scroll-lines", v)).step(1.0).marks(vec![3.0]).on_release(Message::PointerSave(table, "scroll-lines")).width(Length::Fixed(220.0)).name("Scrolling speed"),
                label(if n == 1 { "1 line".to_string() } else { format!("{n} lines") }).width(Length::Fixed(72.0)).align(Align::End),
            ])
            .spacing(8.0)
            .align(Align::Center)
        };
        let dc = self.pointer_int("mouse", "double-click", 400).clamp(200, 900);
        let tested = match self.click_test.1 {
            0 => "Double-click the button to try it.".to_string(),
            1 => "One double-click so far.".to_string(),
            n => format!("{n} double-clicks so far."),
        };
        let mouse = vec![
            setting("Pointer speed", "How far the pointer goes for a move of the hand.", speed("mouse")),
            setting(
                "Acceleration",
                "On: a fast move takes the pointer farther. Off: always the same distance, for precise work and games.",
                switch(self.pointer_flag("mouse", "acceleration", true), |on| Message::PointerFlag("mouse", "acceleration", on)).name("Acceleration"),
            ),
            setting("Scrolling speed", "How many lines a notch of the wheel moves, in every app.", lines("mouse")),
            setting(
                "Natural scrolling",
                "The content follows the wheel, as on a Mac.",
                switch(self.pointer_flag("mouse", "natural-scroll", false), |on| Message::PointerFlag("mouse", "natural-scroll", on)).name("Natural scrolling (mouse)"),
            ),
            setting(
                "Primary button",
                "Right, for the left hand.",
                choice(vec!["Left".into(), "Right".into()], Some(self.pointer_flag("mouse", "left-handed", false) as usize), Message::PrimaryButton).width(Length::Fixed(120.0)).name("Primary button"),
            ),
            setting(
                "Middle click",
                "Pressing both buttons together is a middle click, for mice without one.",
                switch(self.pointer_flag("mouse", "middle-click", false), |on| Message::PointerFlag("mouse", "middle-click", on)).name("Middle click"),
            ),
            setting(
                "Double-click speed",
                &format!("How quickly the two clicks must come: {dc} ms. {tested}"),
                row(vec![
                    slider(200.0, 900.0, dc as f32, |v| Message::PointerSlide("mouse", "double-click", v)).step(10.0).marks(vec![400.0]).on_release(Message::PointerSave("mouse", "double-click")).width(Length::Fixed(160.0)).name("Double-click speed"),
                    button(label("Try it")).on_press(Message::ClickTest).name("Double-click test"),
                ])
                .spacing(8.0)
                .align(Align::Center),
            ),
        ];
        let pad_scroll = (self.pointer_word("touchpad", "scroll") == "edge") as usize;
        let pad_click = (self.pointer_word("touchpad", "click") == "fingers") as usize;
        let pad = vec![
            setting("Pointer speed", "", speed("touchpad")),
            setting("Tap to click", "A tap is a click; two fingers, a right click.", switch(self.pointer_flag("touchpad", "tap-to-click", true), |on| Message::PointerFlag("touchpad", "tap-to-click", on)).name("Tap to click")),
            setting("Tap and drag", "Tap, then touch again and move: a drag.", switch(self.pointer_flag("touchpad", "tap-and-drag", true), |on| Message::PointerFlag("touchpad", "tap-and-drag", on)).name("Tap and drag")),
            setting(
                "Natural scrolling",
                "The content follows the fingers, as on a phone.",
                switch(self.pointer_flag("touchpad", "natural-scroll", true), |on| Message::PointerFlag("touchpad", "natural-scroll", on)).name("Natural scrolling (touchpad)"),
            ),
            setting(
                "Scrolling",
                "",
                choice(vec!["Two fingers".into(), "Along the edge".into()], Some(pad_scroll), Message::PadScroll).width(Length::Fixed(160.0)).name("Scrolling"),
            ),
            setting("Scrolling speed", "", lines("touchpad")),
            setting(
                "Right click",
                "On the pad's bottom right corner, or with two fingers anywhere.",
                choice(vec!["Bottom right corner".into(), "Two fingers".into()], Some(pad_click), Message::PadClick).width(Length::Fixed(180.0)).name("Right click"),
            ),
            setting(
                "Off while typing",
                "A palm on the pad does nothing while keys are being typed.",
                switch(self.pointer_flag("touchpad", "disable-while-typing", true), |on| Message::PointerFlag("touchpad", "disable-while-typing", on)).name("Off while typing"),
            ),
        ];
        self.page_layout(
            "Mouse and touchpad",
            "How the pointer moves and scrolls. Changes apply at once, to every mouse or touchpad plugged in.",
            vec![self.group(Some("Mouse"), mouse), self.group(Some("Touchpad"), pad), label("The touchpad settings apply when one is connected.").size(11.0).dim(), self.status_line()],
        )
    }

    fn keyboard(&self) -> Element<Message> {
        let on = self.desktop.get("keyboard").and_then(|a| a.get("repeat")).and_then(|v| v.as_bool()).unwrap_or(true);
        let value = |s: String| label(s).width(Length::Fixed(72.0)).align(Align::End);
        let delay = row(vec![
            slider(150.0, 1000.0, self.repeat_delay, Message::RepeatDelay).step(10.0).on_release(Message::RepeatDone).width(Length::Fixed(220.0)).name("Delay"),
            value(format!("{:.0} ms", self.repeat_delay)),
        ])
        .spacing(8.0)
        .align(Align::Center);
        let rate = row(vec![
            slider(5.0, 60.0, self.repeat_rate, Message::RepeatRate).step(1.0).on_release(Message::RepeatDone).width(Length::Fixed(220.0)).name("Speed"),
            value(format!("{:.0} a second", self.repeat_rate)),
        ])
        .spacing(8.0)
        .align(Align::Center);
        let mut rows = vec![setting("Key repeat", "Holding a key down types it again and again.", switch(on, Message::Repeat).name("Key repeat"))];
        if on {
            rows.push(setting("Delay", "How long a key is held before it repeats. Longer: fewer doubled letters.", delay));
            rows.push(setting("Speed", "How fast it repeats once it starts.", rate));
        }
        self.page_layout(
            "Keyboard",
            "Key repeat for every app. Changes apply at once.",
            vec![
                self.group(Some("Key repeat"), rows),
                self.group(Some("Try it"), vec![setting("Test", "Hold a key down here.", field(&self.key_test, Message::KeyTest).placeholder("Type here").width(Length::Fixed(260.0)).name("Test"))]),
                self.status_line(),
            ],
        )
    }

    fn energy(&self) -> Element<Message> {
        let off = energy_int(&self.desktop, "screens-off", 0);
        let sleep = energy_int(&self.desktop, "sleep", 0);
        let (off_list, off_now) = minute_choices(&OFF_CHOICES, off);
        let (sleep_list, sleep_now) = minute_choices(&SLEEP_CHOICES, sleep);
        let default_cmd = if Path::new("/run/systemd/system").is_dir() { "systemctl suspend" } else { "sudo -n zzz" };
        let detail = format!("Empty: {default_cmd}. zzz needs root: a sudoers line for it, without a password.");
        let minutes = |list: Vec<String>, now: &str, f: fn(String) -> Message, name: &str| {
            let at = list.iter().position(|l| l == now);
            let options = list.clone();
            choice(options, at, move |i| f(list[i].clone())).width(Length::Fixed(160.0)).name(name)
        };
        self.page_layout(
            "Energy",
            "When nothing has been touched for a while: the screens go dark, then the machine sleeps. Any key or a nudge of the mouse brings everything back.",
            vec![
                self.group(Some("After no input"), vec![
                    setting("Turn the screens off", "Every monitor is switched off; apps keep running.", minutes(off_list, &off_now, Message::ScreensOff, "Turn the screens off")),
                    setting("Put the machine to sleep", "Suspend to memory. The screens go off first.", minutes(sleep_list, &sleep_now, Message::SleepAfter, "Put the machine to sleep")),
                ]),
                self.group(Some("How it sleeps"), vec![setting(
                    "Sleep command",
                    &detail,
                    field(&self.sleep_cmd, Message::SleepCommand).placeholder(default_cmd).on_submit(Message::SleepCommandSubmit).width(Length::Fixed(260.0)).name("Sleep command"),
                )]),
                self.group(None, vec![setting("Apps", "Apps cannot yet ask to keep the screens on while they play or present; that comes with the session bus.", space())]),
                self.status_line(),
            ],
        )
    }

    /// The sound system and its outputs, read again.
    fn sound_read(&mut self) {
        if self.sound.is_none() {
            self.sound = vergence_sound::open();
        }
        self.sound_outputs = self.sound.as_mut().and_then(|s| s.outputs().ok()).unwrap_or_default();
    }

    /// A change to the sound system, then the outputs read again; a refusal
    /// goes to the status line in words.
    fn sound_do(&mut self, change: impl FnOnce(&mut dyn vergence_sound::Sound) -> std::io::Result<()>) {
        let Some(s) = self.sound.as_mut() else { return };
        match change(s.as_mut()) {
            Ok(()) => self.status.clear(),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                // FreeBSD's default output (hw.snd.default_unit) is root's;
                // the helper says why it could not do it.
                self.status = format!("Choosing the output: {e}");
            }
            Err(e) => self.status = format!("Sound: {e}"),
        }
        self.sound_read();
    }

    fn sound_page(&self) -> Element<Message> {
        let title = "Sound";
        let about = "Where sound goes, and how loud. A program that does not choose plays to the default output.";
        let Some(system) = self.sound.as_ref().map(|s| s.system()) else {
            #[cfg(target_os = "linux")]
            let why = if vergence_sound::pipewire_running() {
                "PipeWire manages sound on this machine. Vergence will speak to it in a later milestone; until then, use its own tools."
            } else {
                "No sound system Vergence knows was found."
            };
            #[cfg(not(target_os = "linux"))]
            let why = "No sound system Vergence knows was found.";
            return self.page_layout(title, about, vec![self.group(None, vec![setting(why, "", space())])]);
        };
        let mut groups = Vec::new();
        for o in &self.sound_outputs {
            let id = o.id.clone();
            let chosen = if o.default {
                label("Default").dim()
            } else {
                let id = id.clone();
                button(label("Use this").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(Message::SoundDefault(id)).name(&format!("Use {}", o.name))
            };
            let vol_id = id.clone();
            let mute_id = id.clone();
            let tone_id = id.clone();
            let volume = if o.adjustable {
                setting(
                    "Volume",
                    &format!("{}%", o.volume),
                    slider(0.0, 100.0, o.volume as f32, move |v| Message::SoundVolume(vol_id.clone(), v)).step(1.0).width(Length::Fixed(220.0)).name(&format!("{} volume", o.name)),
                )
            } else {
                setting("Volume", "Set on the monitor or device itself.", space())
            };
            groups.push(self.group(
                Some(&o.name),
                vec![
                    setting(if o.default { "Sound goes here" } else { "Not the default" }, &o.detail, chosen),
                    volume,
                    setting("Mute", "", switch(o.muted, move |on| Message::SoundMute(mute_id.clone(), on)).name(&format!("Mute {}", o.name))),
                    setting("Test sound", "A short tone, quiet.", button(label("Play").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(Message::SoundTone(tone_id)).name(&format!("Test {}", o.name))),
                ],
            ));
        }
        if groups.is_empty() {
            groups.push(self.group(None, vec![setting("No outputs", system, space())]));
        }
        groups.push(self.status_line());
        self.page_layout(title, about, groups)
    }

    fn shelf(&self) -> Option<&toml::Table> {
        self.desktop.get("shelf").and_then(|v| v.as_table())
    }

    /// The dock's sliders follow the file when it is read.
    fn dock_from_file(&mut self) {
        let shelf = self.shelf();
        let gap = shelf.and_then(|t| t.get("gap")).and_then(|v| v.as_integer()).unwrap_or(vergence_shell::GAP as i64).clamp(0, vergence_shell::MAX_GAP as i64) as f32;
        let opacity = shelf.and_then(|t| t.get("opacity")).and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64))).unwrap_or(1.0).clamp(0.3, 1.0) as f32;
        (self.dock_gap, self.dock_opacity) = (gap, opacity);
    }

    /// When the dock hides, as a place in DOCK_HIDE: `hide` is a word, or
    /// true ("windows") and false ("never").
    /// desktop.toml `[windows] key` as a place in `list` (`default` if unset or unknown).
    fn win_word(&self, key: &str, list: &[(&str, &str)], default: usize) -> usize {
        let word = match self.desktop.get("windows").and_then(|t| t.get(key)) {
            // `snap = true` / `false` from before the three choices.
            Some(toml::Value::Boolean(true)) => "always".to_string(),
            Some(toml::Value::Boolean(false)) => "off".to_string(),
            Some(toml::Value::String(w)) => w.trim().to_ascii_lowercase(),
            _ => return default,
        };
        list.iter().position(|(_, w)| *w == word).unwrap_or(default)
    }

    fn win_flag(&self, key: &str, default: bool) -> bool {
        self.desktop.get("windows").and_then(|t| t.get(key)).and_then(|v| v.as_bool()).unwrap_or(default)
    }

    fn dock_hide(&self) -> usize {
        let word = match self.shelf().and_then(|t| t.get("hide")) {
            Some(toml::Value::Boolean(true)) => "windows".to_string(),
            Some(toml::Value::String(w)) => w.trim().to_ascii_lowercase(),
            _ => "never".to_string(),
        };
        DOCK_HIDE.iter().position(|h| h.1 == word).unwrap_or(0)
    }

    /// The pinned applications' entry ids, in their order on the dock.
    fn pinned(&self) -> Vec<String> {
        self.shelf().and_then(|t| t.get("pinned")).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
    }

    fn save_pinned(&mut self, pinned: Vec<String>) {
        self.save_table("shelf", |t| {
            t.insert("pinned".into(), toml::Value::Array(pinned.into_iter().map(toml::Value::String).collect()));
        });
    }

    /// Installed applications not pinned, by name.
    fn unpinned(&self) -> Vec<&vergence_shell::desktop::Entry> {
        let pinned = self.pinned();
        self.apps.iter().filter(|e| !e.no_display && !e.hidden && !pinned.contains(&e.id)).collect()
    }

    fn app_name(&self, id: &str) -> String {
        self.apps.iter().find(|e| e.id == id).map(|e| e.name.clone()).unwrap_or_else(|| id.to_string())
    }

    fn dock(&self) -> Element<Message> {
        let shelf = self.shelf();
        let text = |k: &str| shelf.and_then(|t| t.get(k)).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let flag = |k: &str, default: bool| shelf.and_then(|t| t.get(k)).and_then(|v| v.as_bool()).unwrap_or(default);
        let free = text("place") == "free";
        let mut edges: Vec<String> = DOCK_EDGES.iter().map(|e| e.0.to_string()).collect();
        let at = if free {
            edges.push("Where you put it".into());
            Some(edges.len() - 1)
        } else {
            let e = text("edge");
            Some(DOCK_EDGES.iter().position(|d| d.1 == e).unwrap_or(0))
        };
        let mut place = vec![setting(
            "Where",
            if free { "Floating where it was dragged. Choose an edge, or drag it to one, to dock it there." } else { "Drag the dock away from its edge to float it anywhere." },
            choice(edges, at, Message::DockEdge).width(Length::Fixed(180.0)).name("Where"),
        )];
        if self.outputs.len() > 1 {
            let mut screens = vec!["The primary screen".to_string()];
            screens.extend(self.outputs.iter().map(screen_label));
            let id = text("screen");
            let at = self.outputs.iter().position(|o| o.id == id).map_or(0, |i| i + 1);
            place.push(setting("Screen", "", choice(screens, Some(at), Message::DockScreen).width(Length::Fixed(180.0)).name("Screen")));
        }
        place.push(setting(
            "Gap",
            "The room between the dock, its edge and the windows.",
            row(vec![
                slider(0.0, vergence_shell::MAX_GAP as f32, self.dock_gap, Message::DockGap).step(1.0).marks(vec![vergence_shell::GAP as f32]).on_release(Message::DockSave).width(Length::Fixed(200.0)).name("Gap"),
                label(format!("{:.0} pt", self.dock_gap)).width(Length::Fixed(48.0)).align(Align::End),
            ])
            .spacing(8.0)
            .align(Align::Center),
        ));
        place.push(setting(
            "Keep windows clear",
            "At an edge, maximized and new windows stop short of the dock.",
            switch(flag("keep-clear", true), Message::DockKeepClear).name("Keep windows clear"),
        ));
        let look = vec![
            setting(
                "Hide",
                "Out of the way, a sliver left at its edge; the pointer there brings it back. Windows then have the whole screen.",
                choice(DOCK_HIDE.iter().map(|h| h.0.to_string()).collect(), Some(self.dock_hide()), Message::DockHide).width(Length::Fixed(220.0)).name("Hide"),
            ),
            setting(
                "Opacity",
                "Less, and the desktop shows through the dock. Its icons stay solid.",
                row(vec![
                    slider(0.3, 1.0, self.dock_opacity, Message::DockOpacity).step(0.05).on_release(Message::DockSave).width(Length::Fixed(200.0)).name("Opacity"),
                    percent(self.dock_opacity),
                ])
                .spacing(8.0)
                .align(Align::Center),
            ),
        ];
        let pinned = self.pinned();
        let n = pinned.len();
        let mut pins: Vec<Element<Message>> = Vec::new();
        for (i, id) in pinned.iter().enumerate() {
            let small = |text: &str, m: Message, can: bool, name: String| {
                let b = button(label(text).size(12.0)).padding(Padding::hv(3.0, 9.0)).name(name);
                if can { b.on_press(m) } else { b }
            };
            let name = self.app_name(id);
            pins.push(
                setting(
                    &name,
                    id,
                    row(vec![
                        small("\u{2191}", Message::PinMove(i, true), i > 0, format!("Move {name} up")),
                        small("\u{2193}", Message::PinMove(i, false), i + 1 < n, format!("Move {name} down")),
                        small("Unpin", Message::PinRemove(i), true, format!("Unpin {name}")),
                    ])
                    .spacing(6.0),
                )
                .key(i as u64),
            );
        }
        let names: Vec<String> = self.unpinned().iter().map(|e| e.name.clone()).collect();
        pins.push(setting(
            if n == 0 { "Pin an application" } else { "Pin another" },
            "Pinned applications stay on the dock when they are not running. Right-click an icon on the dock to pin it too.",
            choice(names, None, Message::PinAdd).placeholder("Choose one\u{2026}").width(Length::Fixed(200.0)).name("Pin an application"),
        ));
        self.page_layout(
            "Dock",
            "Where the dock sits and how it looks, and the applications pinned to it. It follows at once.",
            vec![self.group(Some("Place"), place), self.group(Some("Look"), look), self.group(Some("Pinned"), pins), self.status_line()],
        )
    }

    fn windows(&self) -> Element<Message> {
        let l = winbar::parse(&theme::get(&self.xsettings, winbar::KEY));
        let side = |left: bool, text: &'static str| {
            let selected = l.left == left;
            button(label(text)).look(if selected { ButtonLook::Chosen } else { ButtonLook::Normal }).padding(Padding::hv(5.0, 16.0)).on_press(Message::Layout(winbar::Layout { left, ..l }))
        };
        self.page_layout(
            "Windows",
            "The title bar: which side its controls are on, and which buttons it has. Snapping windows to the screen's edges, the window keys, and Alt+Tab.",
            vec![
                self.group(None, vec![preview(l, self.line_color())]),
                self.group(Some("Title bar"), vec![
                    setting("Controls", "Left, as on a Mac; right, as on Windows.", row(vec![side(true, "Left"), side(false, "Right")]).spacing(6.0)),
                    setting("Minimize", "Hides the window until it is chosen again.", switch(l.minimize, move |on| Message::Layout(winbar::Layout { minimize: on, ..l })).name("Minimize")),
                    setting("Maximize", "Fills the screen. Double-clicking the title bar does too.", switch(l.maximize, move |on| Message::Layout(winbar::Layout { maximize: on, ..l })).name("Maximize")),
                    setting("Close", "Always there.", label("On").dim()),
                ]),
                self.snapping(),
                self.group(Some("Older applications"), vec![setting(
                    "X11 applications",
                    "Through Xwayland, for programs with no Wayland support. Turning it on starts it at once; turning it off takes effect the next time Vergence starts.",
                    switch(self.desktop.get("x11").and_then(|t| t.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(false), Message::X11).name("X11 applications"),
                )]),
                self.status_line(),
            ],
        )
    }

    /// Windows > Snapping and switching (windows milestone 1, step 6).
    fn snapping(&self) -> Element<Message> {
        let names = |list: &[(&str, &str)]| list.iter().map(|(n, _)| n.to_string()).collect::<Vec<_>>();
        let pick = |key: &'static str, list: &'static [(&'static str, &'static str)], default: usize, width: f32, name: &str, enabled: bool| {
            let c = choice(names(list), Some(self.win_word(key, list, default)), move |i| Message::WinWord(key, list[i].1)).width(Length::Fixed(width)).name(name);
            if enabled { c } else { c.disabled() }
        };
        let flag = |key: &'static str, default: bool, name: &str, enabled: bool| {
            let s = switch(self.win_flag(key, default), move |on| Message::WinFlag(key, on)).name(name);
            if enabled { s } else { s.disabled() }
        };
        let snapping = self.win_word("snap", &SNAP_MODES, 1) != 0;
        self.group(Some("Snapping and switching"), vec![
            setting(
                "Snap a window dragged to an edge",
                "Left or right: half the screen. A corner: a quarter. The top: all of it. With Ctrl, only when Ctrl is held during the drag.",
                pick("snap", &SNAP_MODES, 1, 180.0, "Snap a window dragged to an edge", true),
            ),
            setting("The top edge", "What dragging a window to the top of the screen does.", pick("top-edge", &TOP_EDGE, 0, 180.0, "The top edge", snapping)),
            setting(
                "Edges between screens",
                "Where one screen meets another, a window crosses; it can snap there after the pointer has pressed against the edge.",
                pick("between-screens", &BETWEEN_SCREENS, 0, 180.0, "Edges between screens", snapping),
            ),
            setting("Show where it will go", "A tint over the part of the screen the window will take.", flag("preview", true, "Show where it will go", snapping)),
            setting("Room between snapped windows", "A little space round them and between them.", flag("gaps", true, "Room between snapped windows", true)),
            setting(
                "Layouts on the maximize button",
                "Resting the pointer on the button shows them. A right click on it, or on the title bar, always does.",
                flag("layouts-hover", true, "Layouts on the maximize button", true),
            ),
            setting(
                "Window keys",
                "Super with the arrows: left and right halves, Up to maximize, Down back or to minimize. Whether or not the buttons are shown.",
                flag("window-keys", true, "Window keys", true),
            ),
            setting("Alt+Tab shows", "The windows it steps through, the most recently used first.", pick("alt-tab", &ALT_TAB, 0, 180.0, "Alt+Tab shows", true)),
            setting("Alt+` (the key above Tab)", "The same, among the focused application's windows only.", flag("app-switch", true, "Alt+backtick", true)),
        ])
    }

    /// The screens as arranged, each with its wallpaper and name.
    /// `interactive`: the overview's (drag; double-click for a screen's
    /// page); else a screen page's map, the screen being edited framed in the
    /// accent and a press on another going to its page.
    fn arrangement(&self, highlight: Option<usize>, interactive: bool, height: f32) -> Element<Message> {
        let screens: Vec<(OutputInfo, Option<Arc<Image>>)> =
            self.outputs.iter().map(|o| (o.clone(), wall::shown(&self.desktop, Some(&o.id)).image.and_then(|p| self.thumbs.get(&p).cloned()))).collect();
        let drag = if interactive { self.arrange_drag } else { None };
        let size_out = self.arrange_size.clone();
        let (accent, line) = (self.accent(), self.line_color());
        let d = drawing(move |scene: &mut Scene, fonts: &mut Fonts, r: Rect, t: &Theme| {
            size_out.set((r.w, r.h));
            let outputs: Vec<OutputInfo> = screens.iter().map(|s| s.0.clone()).collect();
            let face = fonts.face(&t.font, 400, false).or_else(|| fonts.face("sans-serif", 400, false));
            for (i, mut tile) in tiles(&outputs, (r.w, r.h)).into_iter().enumerate() {
                if let Some(d) = drag.filter(|d| d.index == i) {
                    tile.x += d.offset.0;
                    tile.y += d.offset.1;
                }
                let tile = Rect::new(r.x + tile.x, r.y + tile.y, tile.w, tile.h);
                let (o, thumb) = &screens[i];
                scene.fill(tile, Color::rgb(0, 0, 0));
                if let Some(img) = thumb {
                    let id = scene.image(img.clone());
                    scene.draw_image(id, tile, 1.0);
                }
                let band = 34.0f32.min(tile.h / 2.0);
                scene.fill(Rect::new(tile.x, tile.y + tile.h - band, tile.w, band), Color::rgba(0, 0, 0, 153));
                if let Some(f) = face {
                    let m = fonts.metrics(f, 11.0);
                    let lines = [screen_label(o), format!("{:.0}%", o.scale * 100.0)];
                    for (k, s) in lines.iter().enumerate() {
                        let w = fonts.layout(f, 11.0, s).1;
                        let base = tile.y + tile.h - band + 3.0 + m.ascent + k as f32 * 14.0;
                        scene.push_clip(tile, 0.0);
                        text(scene, fonts, f, 11.0, WHITE, Point::new((tile.x + (tile.w - w) / 2.0).round(), base.round()), s);
                        scene.pop_clip();
                    }
                }
                let framed = highlight == Some(i) || drag.is_some_and(|d| d.index == i);
                let (c, w) = if framed { (accent, 3.0) } else { (line, 1.0) };
                scene.stroke(tile, 0.0, w, c);
            }
        })
        .height(Length::Fixed(height))
        .name(if interactive { "Screen arrangement" } else { "Screens" });
        if interactive {
            d.on_touch(|t| (!matches!(t, Touch::Move { pressed: false, .. })).then_some(Message::Arrange(t)))
        } else {
            d.on_touch(|t| matches!(t, Touch::Press { .. }).then_some(Message::Map(t)))
        }
    }

    fn overview(&self) -> Element<Message> {
        self.page_layout(
            "Screens",
            "Drag the screens to where they sit on your desk. Each one's scale is worked out from its size and how far away it is.",
            vec![
                self.group(None, vec![panel(self.arrangement(None, true, 240.0)).padding(8.0).width(Length::FILL)]),
                label("Double-click a screen for its own settings.").size(11.0).dim(),
                self.group(None, vec![setting("Size", &format!("How big everything is, on every screen: {}%.", (self.size * 100.0).round()), size_slider(self.size))]),
                self.status_line(),
            ],
        )
    }

    fn screen(&self) -> Element<Message> {
        let multi = self.outputs.len() > 1;
        let o = self.current_output();
        let scope = self.scope();
        let shown = wall::shown(&self.desktop, scope.as_ref().map(|(i, _)| i.as_str()));
        let is_primary = scope.is_none();
        let primary_label = self.primary().and_then(|p| self.outputs.iter().find(|o| o.id == p)).map(screen_label).unwrap_or_default();
        let title = o.map(screen_label).unwrap_or_else(|| "Wallpaper".into());
        let description = if !multi {
            "Its scale and wallpaper, drawn at the screen's own pixels.".to_string()
        } else if is_primary {
            "The primary screen: its wallpaper is the default, shown on every other screen that has none of its own.".to_string()
        } else {
            format!("Shows the primary screen's wallpaper ({primary_label}) unless it has its own.")
        };
        let mut items: Vec<Element<Message>> = Vec::new();
        if multi && let Page::Screen(i) = self.page {
            items.push(self.group(None, vec![panel(self.arrangement(Some(i), false, 110.0)).padding(6.0).width(Length::FILL)]));
        }
        if let Some(o) = o {
            let marks = disp::detents(o.default_scale, scale_max(o));
            let note = if (self.scale as f64 - o.default_scale).abs() < 1e-6 {
                "The default: the sharpest for this screen."
            } else if self.scale.fract() == 0.0 {
                "Whole number: sharp in every app."
            } else {
                "In between: soft in some older apps."
            };
            let chosen = disp::screen(&self.desktop, &o.id).1.is_some();
            let mut shown_row = vec![label(format!("{:.0}%", o.scale * 100.0)).dim()];
            if chosen {
                shown_row.push(button(label("Default").size(12.0)).padding(Padding::hv(3.0, 10.0)).on_press(Message::ScaleDefault));
            }
            let screen_controls: Vec<String> = SCREEN_CONTROLS_NAMES.iter().map(|s| s.to_string()).collect();
            let mut rows = vec![
                setting("Scale", note, scale_slider(self.scale, o, &marks)),
                setting("Shown at", &o.note, row(shown_row).spacing(8.0).align(Align::Center)),
                setting("Size of controls", "Automatic: Large with a touch panel, else the desktop's (Appearance).", choice(screen_controls, Some(disp::controls(&self.desktop, &o.id)), Message::ScreenControls).width(Length::Fixed(160.0)).name("Size of controls")),
            ];
            if !multi {
                rows.push(setting("Size", &format!("How big everything is: {}%.", (self.size * 100.0).round()), size_slider(self.size)));
            }
            items.push(self.group(Some("Display"), rows));
        }
        let mut wp: Vec<Element<Message>> = Vec::new();
        if multi && !is_primary {
            let detail = if shown.own { "Off: this screen has its own.".to_string() } else { format!("Shows {primary_label}'s wallpaper.") };
            wp.push(setting("Same as the primary screen", &detail, switch(!shown.own, Message::SameAsPrimary).name("Same as the primary screen")));
        }
        if multi {
            wp.push(setting(
                "Primary screen",
                if is_primary { "Its wallpaper is the default for the others." } else { "Make this the primary: its wallpaper becomes the default." },
                if is_primary { label("Primary").dim() } else { button(label("Make primary").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(Message::MakePrimary) },
            ));
        }
        let fit = wall::MODES.iter().position(|m| *m == shown.mode).unwrap_or(0);
        let fits: Vec<String> = FITS.iter().map(|s| s.to_string()).collect();
        wp.push(setting("Fit", "", choice(fits, Some(fit), Message::Fit).width(Length::Fixed(140.0)).name("Fit")));
        wp.push(setting(
            "Background color",
            "Around a picture, or alone without one. Enter applies it.",
            field(&self.bg_text, Message::Background).placeholder("#202020").on_submit(Message::BackgroundSubmit).width(Length::Fixed(120.0)).name("Background color"),
        ));
        items.push(self.group(Some("Wallpaper"), wp));
        // The pictures.
        let (accent, card) = (self.accent(), self.card_color());
        let mut tiles_out = Vec::new();
        for (i, p) in self.files.iter().enumerate() {
            let selected = shown.image.as_deref() == Some(p.as_path());
            let name = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let pic: Element<Message> = match self.grid.get(p) {
                Some(img) => picture(img.clone()).width(Length::Fixed(thumbs::GRID_W)).height(Length::Fixed(thumbs::GRID_H)),
                None => panel(space()).color(card).radius(0.0).width(Length::Fixed(thumbs::GRID_W)).height(Length::Fixed(thumbs::GRID_H)),
            };
            // Long names are shortened to the tile's width instead of running into the next.
            let caption = label(short_name(&name, 25)).size(11.0).width(Length::Fixed(thumbs::GRID_W));
            tiles_out.push(
                button(column(vec![pic, caption]).spacing(4.0))
                    .padding(3.0)
                    .look(ButtonLook::Custom { fill: None, edge: selected.then_some((accent, 3.0)), radius: 6.0 })
                    .on_press(Message::PickPicture(i))
                    .name(name)
                    .key(i as u64),
            );
        }
        items.push(row(tiles_out).spacing(8.0).wrap().width(Length::FILL));
        // Your pictures.
        let (folders, files) = wall::sources(&self.desktop);
        let mut yours: Vec<Element<Message>> = Vec::new();
        for (i, (p, kind)) in folders.iter().map(|f| (f, "Folder")).chain(files.iter().map(|f| (f, "Picture"))).enumerate() {
            yours.push(setting(&p.display().to_string(), kind, button(label("Remove").size(12.0)).padding(Padding::hv(3.0, 10.0)).on_press(Message::RemoveSource(i))));
        }
        yours.push(setting(
            if folders.is_empty() && files.is_empty() { "Add your own pictures" } else { "Add more" },
            "Pictures added to a folder later appear too.",
            row(vec![
                button(label("Add a folder\u{2026}").size(12.0)).padding(Padding::hv(4.0, 10.0)).on_press(Message::Browse(true)),
                button(label("Add pictures\u{2026}").size(12.0)).padding(Padding::hv(4.0, 10.0)).on_press(Message::Browse(false)),
            ])
            .spacing(6.0),
        ));
        items.push(self.group(Some("Your pictures"), yours));
        items.push(self.status_line());
        self.page_layout(&title, &description, items)
    }

    fn browser_page(&self) -> Element<Message> {
        let Some(b) = self.browser.as_ref() else { return space() };
        let mut entries = Vec::new();
        for (i, (p, folder)) in b.entries.iter().enumerate() {
            let picked = !b.folders && b.picked.contains(p);
            let mark = if *folder { "\u{25B8}" } else if picked { "\u{2713}" } else { "" };
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            entries.push(
                button(row(vec![label(mark).width(Length::Fixed(18.0)), label(name.clone()), space().width(Length::FILL)]).spacing(6.0).width(Length::FILL))
                    .padding(Padding::hv(5.0, 10.0))
                    .look(if picked { ButtonLook::Chosen } else { ButtonLook::Plain })
                    .on_press(Message::BrowseOpen(i))
                    .width(Length::FILL)
                    .name(name)
                    .key(i as u64),
            );
        }
        let add_label = if b.folders {
            "Add this folder".to_string()
        } else if b.picked.len() == 1 {
            "Add 1 picture".into()
        } else {
            format!("Add {} pictures", b.picked.len())
        };
        let mut add = button(label(add_label)).look(ButtonLook::Accent).padding(Padding::hv(5.0, 14.0));
        if b.folders || !b.picked.is_empty() {
            add = add.on_press(Message::BrowseAdd);
        }
        panel(
            column(vec![
                self.title_card(
                    if b.folders { "Add a folder" } else { "Add pictures" },
                    if b.folders { "Open the folder you want, then add it. Every picture in it and its folders is offered." } else { "Click pictures to pick them, in as many folders as you like, then add them." },
                ),
                row(vec![button(label("\u{2191} Up").size(12.0)).padding(Padding::hv(4.0, 10.0)).on_press(Message::BrowseUp), label(b.dir.display().to_string()).size(12.0).dim()]).spacing(10.0).align(Align::Center),
                panel(scroll(column(entries).spacing(1.0).width(Length::FILL)).name("Folder"))
                    .color(self.card_color())
                    .radius(10.0)
                    .padding(4.0)
                    .width(Length::FILL)
                    .height(Length::FILL),
                row(vec![space().width(Length::FILL), button(label("Cancel")).padding(Padding::hv(5.0, 14.0)).on_press(Message::BrowseCancel), add]).spacing(8.0).width(Length::FILL),
            ])
            .spacing(12.0)
            .width(Length::FILL)
            .height(Length::FILL),
        )
        .padding(22.0)
        .width(Length::FILL)
        .height(Length::FILL)
    }

    fn about(&self) -> Element<Message> {
        let session = match (std::env::var("VERGENCE_BUS"), std::env::var("WAYLAND_DISPLAY")) {
            (Ok(_), Ok(w)) => format!("Vergence on {w}"),
            (_, Ok(w)) => format!("another Wayland desktop ({w})"),
            _ => "an X11 desktop".to_string(),
        };
        self.page_layout(
            "About",
            "Vergence: every app the right size, in the right place and readable, on any monitor setup.",
            vec![
                self.group(None, vec![
                    setting("Settings app", "", label(format!("version {}", env!("CARGO_PKG_VERSION")))),
                    setting("Session", "", label(session)),
                    setting("Built with", "", label("vergence-ui, Vergence's own toolkit")),
                ]),
                self.group(None, vec![
                    setting("Copyright", "", label(COPYRIGHT)),
                    setting(
                        "License",
                        "Apache License 2.0, or GNU GPL version 2 or later, at your choice. No warranty.",
                        button(label("Read").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(Message::Go(Page::LicenseText)).name("Read the license"),
                    ),
                    setting(
                        "Third-party software",
                        &format!("{} crates, each under its own license", notices_summary().len()),
                        button(label("List").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(Message::Go(Page::Notices)).name("List third-party software"),
                    ),
                ]),
            ],
        )
    }

    fn back_to_about(&self) -> Element<Message> {
        row(vec![button(label("\u{2039} About").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(Message::Go(Page::About)).name("Back to About"), space().width(Length::FILL)])
            .width(Length::FILL)
    }

    /// Both license texts, a paragraph to a label so they wrap to the window.
    fn license_page(&self) -> Element<Message> {
        let text = |t: &'static str| -> Element<Message> {
            let paras: Vec<Element<Message>> = paragraphs(t).into_iter().map(|p| label(p).size(11.0).wrap().width(Length::FILL)).collect();
            panel(column(paras).spacing(8.0).width(Length::FILL)).padding(Padding::hv(12.0, 14.0)).width(Length::FILL)
        };
        self.page_layout(
            "License",
            "Vergence is yours to use under either license below, at your choice: Apache-2.0 OR GPL-2.0-or-later.",
            vec![
                self.back_to_about(),
                self.group(Some("Apache License 2.0 (LICENSE-APACHE)"), vec![text(LICENSE_APACHE)]),
                self.group(Some("GNU General Public License, version 2 (LICENSE-GPL)"), vec![text(LICENSE_GPL)]),
            ],
        )
    }

    /// The crates Vergence is built with, from THIRD-PARTY-NOTICES.md's summary.
    fn notices_page(&self) -> Element<Message> {
        let rows: Vec<Element<Message>> = notices_summary()
            .into_iter()
            .map(|(name, version, license)| setting(&format!("{name} {version}"), "", label(license).size(11.0).dim()))
            .collect();
        let full = match installed_doc_dir() {
            Some(d) => format!("Their full license and notice texts are in {}.", d.join("THIRD-PARTY-NOTICES.md").display()),
            None => "Their full license and notice texts are in THIRD-PARTY-NOTICES.md, beside the source.".to_string(),
        };
        self.page_layout(
            "Third-party software",
            &format!("Vergence is built with this code from other projects, each under its own license. {full}"),
            vec![self.back_to_about(), self.group(None, rows)],
        )
    }

    fn status_line(&self) -> Element<Message> {
        if self.status.is_empty() { space() } else { label(self.status.clone()).size(11.0).dim().wrap().width(Length::FILL) }
    }

    // The theme's colors, for the few places that are drawn by hand.
    fn palette(&self) -> vergence_look::Palette {
        if self.dark() { vergence_look::PALETTE_DARK } else { vergence_look::PALETTE_LIGHT }
    }

    fn text_color(&self) -> Color {
        let c = self.palette().text;
        Color::rgb(c.0, c.1, c.2)
    }

    fn line_color(&self) -> Color {
        let c = self.palette().line;
        Color::rgb(c.0, c.1, c.2)
    }

    fn card_color(&self) -> Color {
        let c = self.palette().card;
        Color::rgb(c.0, c.1, c.2)
    }
}

impl Program for App {
    type Message = Message;

    fn title(&self) -> String {
        "Vergence Settings".into()
    }

    fn app_id(&self) -> String {
        "vergence-settings".into()
    }

    fn theme(&self) -> Theme {
        let text = toml::to_string(&self.desktop).unwrap_or_default();
        let mut t = Theme::new(self.dark(), self.accent(), appearance_text(&self.desktop, "contrast") == "high");
        t.reduced_motion = self.desktop.get("appearance").and_then(|a| a.get("reduced-motion")).and_then(|v| v.as_bool()).unwrap_or(false);
        t.double_click = self.desktop.get("mouse").and_then(|a| a.get("double-click")).and_then(|v| v.as_integer()).map_or(400, |v| v.clamp(100, 2000) as u32);
        t.opacity = vergence_ui::opacity_setting(&text, "[settings]");
        let (family, _) = theme::split_font(&theme::get(&self.xsettings, theme::FONT));
        if !family.trim().is_empty() {
            t.font = family.trim().to_string();
        }
        t
    }

    fn view(&self, t: &Theme) -> Element<Message> {
        let page = match self.page {
            Page::Appearance => self.appearance(),
            Page::Theme => self.theme_page(),
            Page::Windows => self.windows(),
            Page::Dock => self.dock(),
            Page::Keyboard => self.keyboard(),
            Page::Pointer => self.pointer(),
            Page::Energy => self.energy(),
            Page::Sound => self.sound_page(),
            Page::Overview => self.overview(),
            Page::Screen(_) if self.browser.is_some() => self.browser_page(),
            Page::Screen(_) => self.screen(),
            Page::About => self.about(),
            Page::LicenseText => self.license_page(),
            Page::Notices => self.notices_page(),
        };
        row(vec![self.sidebar().color(t.sidebar), vrule().color(t.line), page.width(Length::FILL).height(Length::FILL)]).width(Length::FILL).height(Length::FILL)
    }

    fn update(&mut self, m: Message, _: &mut Window) {
        self.handle(m);
    }

    fn waker(&mut self, waker: Waker) {
        let _ = self.mail.waker.set(waker.clone());
        // Whatever arrived before the window opened is read now.
        waker.wake();
    }

    fn input(&mut self, input: &vergence_widgets::Input, _: &mut Window) -> bool {
        // Back to the window: what changed meanwhile is shown.
        if *input == vergence_widgets::Input::Focus(true) {
            self.reread();
            return true;
        }
        false
    }

    fn woken(&mut self, _: &mut Window) -> bool {
        let mut any = false;
        while let Ok(m) = self.inbox.try_recv() {
            self.handle(m);
            any = true;
        }
        any
    }

    fn scale(&mut self, s: f32) -> bool {
        if s <= 0.0 || (s - self.ui_scale).abs() < 0.001 {
            return false;
        }
        self.ui_scale = s;
        self.grid = self.raw.iter().map(|(p, t)| (p.clone(), Arc::new(thumbs::grid_image(t, s)))).collect();
        true
    }
}

// ---- the look ----

impl App {
    fn title_card(&self, title: &str, description: &str) -> Element<Message> {
        panel(
            column(vec![label(title).size(20.0).weight(700), label(description).dim().wrap().align(Align::Center).width(Length::FILL)])
                .spacing(6.0)
                .align(Align::Center)
                .width(Length::FILL),
        )
        .color(self.card_color())
        .radius(10.0)
        .padding(18.0)
        .width(Length::FILL)
    }

    /// A page: its heading card, then its groups, scrolling as one.
    fn page_layout(&self, title: &str, description: &str, items: Vec<Element<Message>>) -> Element<Message> {
        let mut all = vec![self.title_card(title, description)];
        all.extend(items);
        scroll(panel(column(all).spacing(16.0).width(Length::FILL)).padding(22.0).width(Length::FILL)).name("Page")
    }

    /// A group of settings in a rounded card, a hairline between rows.
    fn group(&self, heading: Option<&str>, rows: Vec<Element<Message>>) -> Element<Message> {
        let mut inner = Vec::new();
        for (i, r) in rows.into_iter().enumerate() {
            if i > 0 {
                inner.push(panel(rule()).padding(Padding::hv(0.0, 12.0)).width(Length::FILL));
            }
            inner.push(r);
        }
        let card = panel(column(inner).width(Length::FILL)).color(self.card_color()).radius(10.0).padding(2.0).width(Length::FILL);
        match heading {
            Some(h) => column(vec![label(h).size(12.0).dim().weight(600), card]).spacing(6.0).width(Length::FILL),
            None => card,
        }
    }
}




/// `name` cut to at most `max` characters, with an ellipsis when cut (about a
/// 152-point tile's width at 11 points).
fn short_name(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_string();
    }
    let mut s: String = name.chars().take(max - 1).collect();
    s.push('\u{2026}');
    s
}

/// One setting: its name and a line about it on the left, its control on the right.
fn setting(name: &str, detail: &str, control: Element<Message>) -> Element<Message> {
    let mut left = vec![label(name)];
    if !detail.is_empty() {
        left.push(label(detail).size(11.0).dim().wrap().width(Length::FILL));
    }
    panel(row(vec![column(left).spacing(2.0).width(Length::FILL), control]).spacing(16.0).align(Align::Center).width(Length::FILL)).padding(Padding::hv(8.0, 12.0)).width(Length::FILL)
}

fn scale_max(o: &OutputInfo) -> f64 {
    (o.default_scale.ceil() + 2.0).max(3.0)
}

fn percent(v: f32) -> Element<Message> {
    label(format!("{:.0}%", v * 100.0)).width(Length::Fixed(48.0)).align(Align::End)
}

/// The size slider, 75% to 150%, sticking to 100% within 3.5%.
fn size_slider(v: f32) -> Element<Message> {
    row(vec![slider(0.75, 1.5, v, Message::Size).step(0.01).marks(vec![1.0]).on_release(Message::SizeDone).width(Length::Fixed(220.0)).name("Size"), percent(v)]).spacing(8.0).align(Align::Center)
}

/// A screen's scale slider, with marks at the whole numbers and the default.
fn scale_slider(v: f32, o: &OutputInfo, marks: &[f64]) -> Element<Message> {
    let max = scale_max(o) as f32;
    let marks: Vec<f32> = marks.iter().map(|m| *m as f32).collect();
    row(vec![slider(0.75, max, v, Message::Scale).step(0.01).marks(marks).on_release(Message::ScaleDone).width(Length::Fixed(260.0)).name("Scale"), percent(v)]).spacing(8.0).align(Align::Center)
}

/// A preview of a title bar in the chosen layout, in the server's colors.
fn preview(l: winbar::Layout, line: Color) -> Element<Message> {
    let d = drawing(move |scene: &mut Scene, fonts: &mut Fonts, r: Rect, t: &Theme| {
        let w = 420.0f32.min(r.w - 28.0);
        let win = Rect::new(r.x + (r.w - w) / 2.0, r.y + 14.0, w, 88.0);
        let bar = Rect::new(win.x, win.y, win.w, 32.0);
        scene.fill(bar, Color::rgb(51, 61, 82));
        scene.fill(Rect::new(win.x, win.y + 32.0, win.w, 56.0), rgb(0x1e1e2e));
        scene.stroke(win, 0.0, 1.0, line);
        // The buttons: close in red, the others gray; on the chosen side.
        let close = rgb(0xc74d47);
        let other = Color::rgba(255, 255, 255, 140);
        let mut order = Vec::new();
        if l.minimize {
            order.push(other);
        }
        if l.maximize {
            order.push(other);
        }
        if l.left {
            order.insert(0, close);
        } else {
            order.push(close);
        }
        let dots_w = order.len() as f32 * 18.0 - 6.0;
        let mut x = if l.left { bar.x + 10.0 } else { bar.x + bar.w - 10.0 - dots_w };
        for c in order {
            scene.fill_rounded(Rect::new(x, bar.y + 14.0, 12.0, 12.0), 6.0, c);
            x += 18.0;
        }
        // The tabs: on the other side.
        let Some(f) = fonts.face(&t.font, 400, false).or_else(|| fonts.face("sans-serif", 400, false)) else { return };
        let ascent = fonts.metrics(f, 11.0).ascent;
        let tabs = ["~/src/vergence", "htop"];
        let widths: Vec<f32> = tabs.iter().map(|s| fonts.layout(f, 11.0, s).1 + 24.0).collect();
        let total: f32 = widths.iter().sum::<f32>() + 4.0 * tabs.len() as f32 + 12.0;
        let mut tx = if l.left { bar.x + 10.0 + dots_w + 14.0 } else { bar.x + bar.w - 10.0 - dots_w - 14.0 - total };
        for (k, s) in tabs.iter().enumerate() {
            let tab = Rect::new(tx, bar.y + 6.0, widths[k], 26.0);
            if k == 0 {
                scene.fill_rounded(Rect::new(tab.x, tab.y, tab.w, tab.h + 6.0), 6.0, rgb(0x1e1e2e));
            }
            text(scene, fonts, f, 11.0, WHITE, Point::new(tab.x + 12.0, (tab.y + 4.0 + ascent).round()), s);
            tx += widths[k] + 4.0;
        }
        text(scene, fonts, f, 13.0, WHITE, Point::new(tx + 2.0, (bar.y + 10.0 + ascent).round()), "+");
    });
    d.height(Length::Fixed(116.0)).name("Title bar preview")
}

fn bbox(outputs: &[OutputInfo]) -> (i32, i32, i32, i32) {
    let x0 = outputs.iter().map(|o| o.x).min().unwrap_or(0);
    let y0 = outputs.iter().map(|o| o.y).min().unwrap_or(0);
    let x1 = outputs.iter().map(|o| o.x + o.width).max().unwrap_or(1);
    let y1 = outputs.iter().map(|o| o.y + o.height).max().unwrap_or(1);
    (x0, y0, (x1 - x0).max(1), (y1 - y0).max(1))
}

/// Room around the arrangement in its drawing, and the gap between screens.
const MARGIN: f32 = 20.0;
const GAP: f32 = 3.0;

/// The arrangement's place in a drawing `size` big: its corner, and drawing
/// units to the desktop pixel.
fn area(outputs: &[OutputInfo], size: (f32, f32)) -> ((f32, f32), f32) {
    let (_, _, bw, bh) = bbox(outputs);
    let k = ((size.0 - 2.0 * MARGIN) / bw as f32).min((size.1 - 2.0 * MARGIN) / bh as f32).max(0.0001);
    (((size.0 - bw as f32 * k) / 2.0, (size.1 - bh as f32 * k) / 2.0), k)
}

/// Each screen's rectangle in a drawing `size` big, in its own coordinates.
fn tiles(outputs: &[OutputInfo], size: (f32, f32)) -> Vec<Rect> {
    let (x0, y0, _, _) = bbox(outputs);
    let (origin, k) = area(outputs, size);
    outputs.iter().map(|s| Rect::new(origin.0 + (s.x - x0) as f32 * k + GAP, origin.1 + (s.y - y0) as f32 * k + GAP, s.width as f32 * k - 2.0 * GAP, s.height as f32 * k - 2.0 * GAP)).collect()
}

fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h
}

/// A folder's sub-folders, then its pictures.
fn list(dir: &Path) -> Vec<(PathBuf, bool)> {
    let (dirs, pics) = wall::list_dir(dir);
    dirs.into_iter().map(|d| (d, true)).chain(pics.into_iter().map(|p| (p, false))).collect()
}

/// The screens from the session bus, followed (a monitor plugged in appears),
/// on a thread of its own. Outside a Vergence session there is no bus: the
/// screen pages then offer only the wallpaper.
fn bus_outputs(mail: Mail) {
    std::thread::spawn(move || {
        let Ok(mut bus) = vergence_protocol::Client::connect("vergence-settings") else { return };
        if bus.request(|serial| ClientMsg::Subscribe { serial, topic: Topic::Outputs }).is_err() {
            return;
        }
        // The bus going away ends this; nothing reconnects in a loop.
        while let Ok(m) = bus.next_event() {
            if let ServerMsg::Outputs { outputs, setup, .. } = m
                && !mail.send(Message::Outputs(outputs, setup))
            {
                return;
            }
        }
    });
}

fn main() -> std::io::Result<()> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("vergence-settings {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // Vergence's settings are for a Vergence session only (Dave, 2026-11-12):
    // other desktops have settings of their own. The server gives every
    // program it starts its bus, nested or not.
    if std::env::var_os(vergence_protocol::BUS_ENV).is_none() {
        eprintln!("vergence-settings: runs in a Vergence session only ({} is not set)", vergence_protocol::BUS_ENV);
        std::process::exit(1);
    }
    vergence_widgets::run(App::new(), (880, 640))
}

#[cfg(test)]
mod about_tests {
    use super::*;

    #[test]
    fn notices_summary_reads_the_table() {
        let rows = notices_summary();
        assert!(rows.len() > 100, "{} rows", rows.len());
        assert!(rows.iter().any(|(n, _, _)| n == "smithay"));
        assert!(rows.iter().all(|(n, v, l)| !n.is_empty() && !v.is_empty() && !l.is_empty()));
    }

    #[test]
    fn license_texts_split_into_paragraphs() {
        let a = paragraphs(LICENSE_APACHE);
        let g = paragraphs(LICENSE_GPL);
        assert!(a.len() > 20 && g.len() > 20);
        assert!(a.iter().any(|p| p.starts_with("Apache License")));
        assert!(g.iter().any(|p| p.starts_with("GNU GENERAL PUBLIC LICENSE")));
        assert!(a.iter().chain(&g).all(|p| !p.contains('\n')));
    }
}
