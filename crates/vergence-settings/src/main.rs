//! Vergence's settings app (System Settings): every desktop setting in one window, each change
//! applied at once.
//!
//! On the project's own toolkit (vergence-widgets, toolkit milestone 2). Like macOS's System
//! Settings, made for a desktop and not for phones, with its colors (see
//! docs/settings-milestone-1.md): a warm gray sidebar of pages, the selected one filled with the
//! desktop's accent; the page on a lighter ground, its heading centered in a card, settings in
//! soft gray cards with the name on the left and the control on the right. Only one runs at a
//! time (single.rs).
//!
//! Pages (a module each): Appearance, Theme and fonts, Windows, Dock, Keyboard, Mouse, Touchpad
//! and Touchscreen (each shown once the machine has had one), Wallpaper, Accessibility (Zoom
//! among it), Energy, Battery, Sound, Wi-Fi, Network (cables and a phone by USB),
//! Notifications, Screenshots, Screens (an overview to arrange them, a page per screen),
//! Default apps, Apps, Session, Privacy, Lock Screen and Login, Date & Time, About.
//!
//! Files: ~/.config/vergence/desktop.toml (a section per page) and settings.toml (theme, fonts,
//! the title bar layout); the server applies both within a second. The screens come from the
//! session bus (vergence-protocol), followed live, on a thread of their own; previews are drawn
//! on threads too. Both reach the window through a channel and the toolkit's waker.
//!
//! Uses vergence-widgets and vergence-ui (the window), vergence-protocol (the bus),
//! vergence-profile (screens), vergence-net (Wi-Fi), vergence-power, vergence-sound,
//! vergence-dm (lock screen and login), vergence-xsettings, vergence-shell and vergence-types
//! (apps and default apps), vergence-decode (wallpaper previews).
//!
//! Start reading: this file (the app, its pages and messages), then sidebar.rs and look.rs (the
//! layout every page shares), then one page module (keyboard.rs is a small one).
//!
//! Rule: every change applies at once (the file is written, or the bus asked, as the control
//! moves); there is no Apply button.

mod battery;
mod datetime;
mod disp;
mod lockscreen;
mod modes;
mod notify;
mod shots;
mod single;
mod theme;
mod thumbs;
mod wall;
use notify::NotifyMsg;
use shots::ShotsMsg;
/// Where the app was, for the next start (session milestone 1).
mod keep;
mod places;
use places::PlacesMsg;
mod privacy;
use privacy::PrivacyMsg;
mod zoom;
use zoom::ZoomMsg;
mod apps;
use apps::AppsMsg;
mod defaults;
use defaults::DefaultsMsg;
mod wifi;
mod winbar;

mod about;
mod accessibility;
mod appearance;
mod dock;
mod energy;
mod fonts;
mod icons;
mod keyboard;
mod look;
mod mouse;
mod screens;
mod session;
mod sidebar;
mod sound;
mod wallpaper;
mod windows;
use dock::read_applets;
use energy::energy_cmd;
use icons::{Icon, chevron, icon_mark};
use keyboard::{REPEAT_DELAY, REPEAT_RATE, keyboard_int};
use look::{percent, setting, short_name};
use screens::ArrangeDrag;
use wallpaper::Browser;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use vergence_protocol::bus::{ClientMsg, InputInfo, InputKind, OutputInfo, ServerMsg, Topic};
use vergence_render::text;
use vergence_scene::{Color, Image, Point, Rect, Scene};
use vergence_text::Fonts;
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Program, Theme, Touch, Waker, Window, button, choice, column, drawing,
    field, label, panel, parse_hex, picture, row, rule, scroll, slider, space, switch, vrule,
};

/// The accent where the user names none: the settings app's own blue.
const DEFAULT_ACCENT: Color = Color::rgb(0x3b, 0x82, 0xf6);
const WHITE: Color = Color::rgb(255, 255, 255);
/// The sidebar's width, in points.
const SIDEBAR_W: f32 = 210.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    /// Wireless networks (settings milestone 1, step 4; wifi.rs).
    WiFi,
    /// Cables and a phone by USB (step 5; wifi.rs).
    Network,
    Appearance,
    Theme,
    Windows,
    Dock,
    Keyboard,
    Mouse,
    /// Shown once the machine has had one (the server remembers: bus `inputs`).
    Touchpad,
    Touchscreen,
    Energy,
    /// Do not disturb, how long, which corner (notify.rs).
    Notifications,
    /// What Super+Shift+3 takes, where screenshots go, the programs allowed (shots.rs).
    Screenshots,
    /// The dock's clock and the time zone (datetime.rs; dock milestone 1).
    DateTime,
    LockScreen,
    Login,
    /// What comes back when Vergence starts, and when an app opens again
    /// (session milestone 1; keep.rs, vergence-widgets session.rs).
    Session,
    /// Privacy > Agents: what agents may use (privacy.rs; meaning milestone 1).
    Agents,
    /// The battery and the GPUs (battery.rs); only where there is a battery.
    Battery,
    Sound,
    /// High contrast, motion, transparency, size (settings milestone 1).
    Accessibility,
    Overview,
    Screen(usize),
    /// Every screen's picture in one place (settings milestone 1): the
    /// screen whose wallpaper is open, by index (0 with one screen or none).
    Wallpaper(usize),
    About,
    /// Every app the desktop knows, and one app's page (`App::app_open`;
    /// apps.rs, app settings milestone 1).
    Apps,
    AppPage,
    /// Which app opens each kind of file (defaults.rs, files milestone 1).
    DefaultApps,
    /// About's sub-pages: the license texts, and the code Vergence is built with.
    LicenseText,
    Notices,
}

#[derive(Debug, Clone)]
enum Message {
    /// The Notifications page (notify.rs).
    Notify(NotifyMsg),
    Shots(ShotsMsg),
    Clock(datetime::ClockMsg),
    Lock(lockscreen::LockMsg),
    /// Another start's arguments (single.rs): its page, then forward.
    HandedOver(Vec<String>),
    /// Window places on the Windows page (places.rs).
    Places(PlacesMsg),
    /// Privacy > Agents (privacy.rs).
    Privacy(PrivacyMsg),
    Zoom(ZoomMsg),
    /// The Apps page (apps.rs).
    Apps(AppsMsg),
    /// The Default apps page (defaults.rs).
    Defaults(DefaultsMsg),
    /// desktop.toml changed under the app (its own settings window wrote it).
    Reread,
    Go(Page),
    /// The Wi-Fi page (wifi.rs).
    Wifi(wifi::WifiMsg),
    /// Time to read the battery again (battery.rs).
    BatterySample,
    /// A screen's resolution or refresh rate picked (modes.rs), and the
    /// keep-or-revert dialog's answer.
    Resolution(usize),
    Refresh(usize),
    KeepMode(bool),
    /// The battery history's range: the last day or week.
    BatteryRange(usize),
    /// The Advanced part of a page opened or closed, by its key.
    Advanced(&'static str),
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
    /// Title bar shade: an index into SHADE_CHOICES.
    BarShade(usize),
    FrontTab(usize),
    Transparency(bool),
    /// The sidebar's search field: typed, and Enter (the first page found).
    Search(String),
    SearchGo,
    /// Windows tinted with the wallpaper's color (window styles milestone 1).
    /// The tint's amount being slid (percent), then let go.
    TintSlide(f32),
    TintDone,
    /// Under the bar: 0 the titles' contrast first, 1 more of the window.
    UnderBar(usize),
    /// Words spelled wrong marked in every native app's text (editor milestone 1).
    Spellcheck(bool),
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
    /// Icons on the desktop (files milestone 1, step 9): desktop.toml `[desktop] icons`.
    DesktopIcons(bool),
    Restore(usize),
    /// `[session] places`: reopened documents go back to their place.
    SessionPlaces(bool),
    /// A second after a scroll that was not written yet: write it (keep.rs).
    KeepPlace,
    /// `[session] settings-keep`, an index into KEEP_CHOICES.
    SettingsKeep(usize),
    DockScreen(usize),
    DockGap(f32),
    DockOpacity(f32),
    DockSize(f32),
    /// A dock slider let go: what it shows is saved.
    DockSave,
    DockKeepClear(bool),
    DockHide(usize),
    /// An applet switched on or off (by its tile id), and whether applets in
    /// trouble come out of the drawer (dock milestone 1, step 9).
    Applet(String, bool),
    TroubleOut(bool),
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
    /// The same two on battery ([energy.battery]).
    BatteryScreensOff(String),
    BatterySleepAfter(String),
    SleepCommand(String),
    SleepCommandSubmit,
    // Screens.
    Outputs(Vec<OutputInfo>, String),
    /// The input devices present and the kinds seen (bus `inputs`).
    Inputs(Vec<InputInfo>, Vec<InputKind>),
    /// A touch panel (by its name) put on a screen (by id; None: the primary).
    PanelScreen(String, Option<String>),
    Size(f32),
    SizeDone,
    /// "Windows the same size on every screen" (settings milestone 2).
    SameSize(bool),
    Scale(f32),
    ScaleDone,
    ScaleDefault,
    /// The Screens overview's scale slider of screen `i`.
    ScaleOf(usize, f32),
    ScaleOfDone(usize),
    /// Make screen `i` the main screen (the overview's choice).
    MainScreen(usize),
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

struct App {
    /// The strip under the server's frosted bar (Theme::top), for the pages' layout.
    top: std::cell::Cell<f32>,
    /// What is typed in the sidebar's search field (settings milestone 1).
    search: String,
    /// The pages whose Advanced part is open (settings milestone 1, step 2):
    /// closed at first, remembered while the app runs.
    advanced: HashSet<&'static str>,
    /// The Wi-Fi page's state (wifi.rs).
    wifi: wifi::Wifi,
    /// The Battery page's state (battery.rs).
    battery: battery::BatteryPage,
    /// Apps that have sent cards, newest first (notify.rs, read when the page opens).
    notify_apps: Vec<(String, u64)>,
    /// The programs allowed to take screenshots (shots.rs), read when the page opens.
    shot_grants: Vec<(String, String)>,
    /// What vdm said about logging in (the Login page), or why it could not be asked.
    login: Result<lockscreen::LoginState, String>,
    /// Every applet the desktop has seen: tile id, name, who supplies it
    /// (the server's applets.toml), read when the Dock page opens.
    applets: Vec<(String, String, String)>,
    /// A new mode waiting to be kept, and whether to ask about it now (modes.rs).
    mode_pending: Option<modes::Pending>,
    mode_ask: bool,
    /// Window places from the server (places.rs); None until read.
    places: Option<Vec<(String, u32)>>,
    /// The agents' grants (Privacy > Agents; privacy.rs), once read.
    grants: Option<Vec<vergence_protocol::bus::Grant>>,
    /// Forget pressed: the dialog to open ("" for every app).
    places_ask: Option<String>,
    /// `--forget <app>` from the dock: asked once the list has come.
    places_from_dock: Option<String>,
    /// The app whose page is open (apps.rs).
    app_open: Option<String>,
    page: Page,
    /// A place to scroll the page to, kept from the last run (keep.rs):
    /// taken by the toolkit at the next layout.
    kept_scroll: Option<f32>,
    /// How far the page is scrolled, as the toolkit last told.
    scrolled_to: f32,
    /// What keep.rs last wrote, and when.
    kept_written: Option<(keep::Kept, std::time::Instant)>,
    /// When the session said it is ending (the close after it is the session's).
    ending: Option<std::time::Instant>,
    /// A KeepPlace is on its way (a scroll not yet written).
    keep_waiting: bool,
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
    /// The settings app's own settings window, if open (app settings milestone 1).
    prefs_open: std::rc::Rc<std::cell::Cell<bool>>,
    // Screens.
    outputs: Vec<OutputInfo>,
    /// The input devices present, and the kinds the machine has had (bus `inputs`).
    inputs: Vec<InputInfo>,
    inputs_seen: Vec<InputKind>,
    setup: String,
    size: f32,
    scale: f32,
    /// A scale slider on the Screens overview being dragged: which screen, the value.
    scale_drag: Option<(usize, f32)>,
    /// A `--page` named before its page was listed (the Screens pages wait
    /// for the screens): opened when they come.
    page_wanted: Option<String>,
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
    /// The wallpaper tint's amount in percent, as the slider shows it (0: off).
    tint_pct: f32,
    repeat_rate: f32,
    key_test: String,
    /// The mouse and touchpad sliders while they move, by (table, key).
    sliding: HashMap<(&'static str, &'static str), f32>,
    /// The double-click test: the last press, and how many double-clicks.
    click_test: (Option<Instant>, u32),
    /// The dock's gap and opacity while their sliders move (saved on release).
    dock_gap: f32,
    dock_opacity: f32,
    /// The dock's size in points ([shelf] size), while its slider moves.
    dock_size: f32,
    /// What can be pinned: the installed applications, by name.
    apps: Vec<vergence_shell::desktop::Entry>,
    /// The mimeapps.list files, read when Default apps opens (defaults.rs).
    default_lists: Vec<vergence_types::MimeApps>,
    mail: Mail,
    inbox: Receiver<Message>,
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
        Ok(text) => text
            .parse::<toml::Table>()
            .map_err(|e| format!("{} is not valid TOML ({e}); not overwriting it", path.display()))?,
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
            top: std::cell::Cell::new(0.0),
            search: String::new(),
            advanced: HashSet::new(),
            wifi: Default::default(),
            battery: Default::default(),
            notify_apps: Vec::new(),
            shot_grants: Vec::new(),
            login: Err(String::new()),
            applets: Vec::new(),
            mode_pending: None,
            mode_ask: false,
            places: None,
            grants: None,
            places_ask: None,
            places_from_dock: None,
            app_open: None,
            kept_scroll: None,
            scrolled_to: 0.0,
            kept_written: None,
            ending: None,
            keep_waiting: false,
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
            prefs_open: std::rc::Rc::new(std::cell::Cell::new(false)),
            outputs: Vec::new(),
            inputs: Vec::new(),
            inputs_seen: Vec::new(),
            setup: String::new(),
            scale: 1.0,
            scale_drag: None,
            page_wanted: None,
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
            tint_pct: 0.0,
            repeat_rate: 0.0,
            key_test: String::new(),
            sliding: HashMap::new(),
            click_test: (None, 0),
            dock_gap: 0.0,
            dock_opacity: 1.0,
            dock_size: vergence_shell::STRIP_W as f32,
            apps: vergence_shell::apps::installed(),
            default_lists: Vec::new(),
            mail,
            inbox,
        };
        app.repeat_delay = keyboard_int(&app.desktop, "repeat-delay", REPEAT_DELAY) as f32;
        app.tint_pct = {
            let a = app.desktop.get("appearance");
            let on = a.and_then(|a| a.get("wallpaper-tint")).and_then(|v| v.as_bool()).unwrap_or(true);
            let amount = a
                .and_then(|a| a.get("tint-amount"))
                .map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()));
            if on { ((vergence_widgets::tint_amount(amount.as_deref()) * 200.0).round() / 2.0).min(10.0) } else { 0.0 }
        };
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

    /// The settings app's own settings window: its look and window places.
    fn open_own_settings(&mut self, window: &mut Window) {
        if self.prefs_open.get() {
            return;
        }
        // Its changes (opacity) shown in the main window at once, not when it next has the keyboard.
        let mail = self.mail.clone();
        let prefs = vergence_widgets::prefs::Prefs::new(
            "Settings",
            "vergence-settings",
            "[settings]",
            false,
            Box::new(OwnRows),
            self.prefs_open.clone(),
        )
        .on_write(move || {
            mail.send(Message::Reread);
        });
        window.open_window(vergence_widgets::window(prefs));
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

    /// The screen whose wallpaper the page shows: a screen page's, or the
    /// Wallpaper page's chosen screen.
    fn wall_output(&self) -> Option<&OutputInfo> {
        match self.page {
            Page::Screen(i) | Page::Wallpaper(i) => self.outputs.get(i),
            _ => None,
        }
    }

    /// The primary screen's index, for the Wallpaper page's sidebar entry.
    fn primary_index(&self) -> usize {
        self.primary().and_then(|p| self.outputs.iter().position(|o| o.id == p)).unwrap_or(0)
    }

    fn primary(&self) -> Option<String> {
        let ids: Vec<String> = self.outputs.iter().map(|o| o.id.clone()).collect();
        wall::primary(&self.desktop, &ids)
    }

    /// Make `new` the main screen (desktop.toml `[displays] primary`): the
    /// default wallpaper moves without anything on screen changing, and the
    /// dock, if it was put on a screen by name, goes back to following the
    /// main screen (otherwise a dock pinned by name to an old screen stays
    /// there, and nothing on the Screens page can move it).
    fn make_main(&mut self, new: &str) {
        let old = self
            .primary()
            .and_then(|p| self.outputs.iter().find(|o| o.id == p).map(|o| (o.id.clone(), screen_label(o))));
        self.save(|d| {
            wall::make_primary(d, old.as_ref().map(|(i, l)| (i.as_str(), l.as_str())), new);
            if let Some(shelf) = d.get_mut("shelf").and_then(|s| s.as_table_mut()) {
                shelf.remove("screen");
            }
        });
    }

    /// Where the open screen page's wallpaper is kept: None for the primary
    /// (and the only) screen; else its id and label.
    fn scope(&self) -> Option<(String, String)> {
        let o = self.wall_output()?;
        (self.primary().as_deref() != Some(o.id.as_str())).then(|| (o.id.clone(), screen_label(o)))
    }

    /// The start's arguments (or another start's, handed over): `--forget
    /// <app>` (the dock's "Forget window places...": the question, asked once
    /// the Windows page's list has come from the server), `--page <name>`,
    /// `--app <id>` (the app menu's Settings… for an app not ours).
    fn apply_args(&mut self, args: &[String]) {
        if let Some(i) = args.iter().position(|a| a == "--forget")
            && let Some(name) = args.get(i + 1)
        {
            self.places_from_dock = Some(name.clone());
        }
        if let Some(i) = args.iter().position(|a| a == "--page")
            && let Some(name) = args.get(i + 1)
        {
            match self.page_named(name) {
                Some(page) => self.open(page),
                // The Screens pages are listed once the server has told the
                // screens (until then, `--page screens` would open Appearance).
                None if self.outputs.is_empty() => self.page_wanted = Some(name.clone()),
                None => {}
            }
        }
        if let Some(i) = args.iter().position(|a| a == "--app")
            && let Some(id) = args.get(i + 1).filter(|id| !id.is_empty() && id.len() < 256)
        {
            self.apps_message(AppsMsg::Open(id.to_lowercase()));
        }
    }

    /// Where the app is now, for keep.rs: the page by its sidebar name, or
    /// the app whose page is open, and how far it is scrolled.
    fn kept_now(&self, closed: Option<(u64, String)>) -> keep::Kept {
        let app = (self.page == Page::AppPage).then(|| self.app_open.clone()).flatten();
        let page = self
            .page_groups()
            .into_iter()
            .flat_map(|(_, rows)| rows)
            .find(|(_, p, ..)| *p == self.page)
            .map(|(n, ..)| n);
        keep::Kept { page, app, scroll: self.scrolled_to, closed }
    }

    /// Writes where the app is, if it changed: at once for a new page, at
    /// most once a second while it is only scrolled.
    fn keep_place(&mut self) {
        let now = self.kept_now(None);
        let due = match &self.kept_written {
            None => true,
            Some((k, _)) if k.page != now.page || k.app != now.app => true,
            Some((k, at)) => (k.scroll - now.scroll).abs() >= 1.0 && at.elapsed() >= std::time::Duration::from_secs(1),
        };
        if due {
            keep::save(&now);
            self.kept_written = Some((now, std::time::Instant::now()));
        } else if !self.keep_waiting
            && self.kept_written.as_ref().is_some_and(|(k, _)| (k.scroll - now.scroll).abs() >= 1.0)
        {
            // Scrolled again within the second: written once it has passed,
            // so where the scrolling stopped is what is kept.
            self.keep_waiting = true;
            let mail = self.mail.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(1100));
                mail.send(Message::KeepPlace);
            });
        }
    }

    /// Opens where the last run was, if that is to be used now (keep.rs).
    fn open_kept(&mut self, k: keep::Kept) {
        let args: Vec<String> = match (&k.app, &k.page) {
            (Some(a), _) => vec!["--app".into(), a.clone()],
            (None, Some(p)) => vec!["--page".into(), p.clone()],
            _ => return,
        };
        self.apply_args(&args);
        if k.scroll > 0.0 {
            self.kept_scroll = Some(k.scroll);
        }
    }

    fn open(&mut self, p: Page) {
        self.page = p;
        self.reread();
        if p == Page::Sound {
            self.sound_read();
        }
        if matches!(p, Page::WiFi | Page::Network) {
            self.wifi_read();
        }
        if matches!(p, Page::Windows | Page::Apps | Page::AppPage) {
            places::fetch(self.mail.clone());
        }
        if p == Page::Agents {
            privacy::fetch(self.mail.clone());
        }
        if p == Page::Notifications {
            self.notify_apps = notify::read_apps();
        }
        if p == Page::Screenshots {
            self.shot_grants = shots::read_grants();
        }
        if p == Page::Dock {
            self.applets = read_applets();
        }
        if p == Page::DefaultApps {
            self.defaults_read();
        }
        if p == Page::Login {
            self.login_read();
        }
        if matches!(p, Page::Battery | Page::Energy) {
            self.battery_read();
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
            Message::Wifi(m) => self.wifi_handle(m),
            Message::BatterySample => self.battery_sample(),
            Message::Notify(m) => self.notify_message(m),
            Message::Shots(m) => self.shots_message(m),
            Message::Clock(m) => self.clock_message(m),
            Message::Lock(m) => self.lock_message(m),
            Message::HandedOver(args) => self.apply_args(&args),
            Message::Places(m) => self.places_message(m),
            Message::Privacy(m) => self.privacy_message(m),
            Message::Zoom(m) => self.zoom_message(m),
            Message::Apps(m) => self.apps_message(m),
            Message::Defaults(m) => self.defaults_message(m),
            Message::Reread => self.reread(),
            Message::KeepPlace => {
                self.keep_waiting = false;
                self.keep_place();
            }
            Message::Resolution(i) => self.pick_mode(Some(i), None),
            Message::Refresh(i) => self.pick_mode(None, Some(i)),
            Message::KeepMode(keep) => self.settle_mode(keep),
            Message::BatteryRange(i) => self.battery_range(i),
            Message::Advanced(key) => {
                if !self.advanced.remove(key) {
                    self.advanced.insert(key);
                }
            }
            Message::Search(s) => self.search = s.chars().take(64).collect(),
            Message::SearchGo => {
                if let Some(p) = self.found_pages().first().copied() {
                    self.open(p);
                }
            }
            Message::Inputs(devices, seen) => {
                self.inputs = devices;
                self.inputs_seen = seen;
            }
            Message::Outputs(outputs, setup) => {
                let current = self.wall_output().map(|o| o.id.clone());
                self.outputs = outputs;
                self.setup = setup;
                self.arrange_drag = None;
                // Keep the same screen's page if it is still there.
                if let Page::Screen(_) = self.page {
                    let i = current.clone().and_then(|id| self.outputs.iter().position(|o| o.id == id)).unwrap_or(0);
                    self.page = Page::Screen(i);
                }
                if let Page::Wallpaper(_) = self.page {
                    let i = current.clone().and_then(|id| self.outputs.iter().position(|o| o.id == id)).unwrap_or(0);
                    self.page = Page::Wallpaper(i);
                }
                if self.page == Page::Overview && self.outputs.len() <= 1 {
                    self.page = Page::Screen(0);
                }
                if let Some(name) = self.page_wanted.take()
                    && let Some(page) = self.page_named(&name)
                {
                    self.open(page);
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
                let wanted: Vec<PathBuf> =
                    self.outputs.iter().filter_map(|o| wall::shown(&self.desktop, Some(&o.id)).image).collect();
                self.ask_thumbs(wanted);
            }
            Message::Thumb(p, t) => {
                if let Some(t) = t {
                    self.grid.insert(p.clone(), Arc::new(thumbs::grid_image(&t, self.ui_scale)));
                    self.thumbs.insert(p.clone(), Arc::new(Image { width: t.w, height: t.h, rgba: t.rgba.clone() }));
                    self.raw.insert(p, t);
                }
            }
            m @ (Message::Scheme(..)
            | Message::Accent(..)
            | Message::AccentText(..)
            | Message::AccentSubmit
            | Message::BarShade(..)
            | Message::Frost(..)
            | Message::FrontTab(..)
            | Message::UnderBar(..)
            | Message::TintSlide(..)
            | Message::TintDone
            | Message::Controls(..)) => self.appearance_message(m),
            m @ (Message::Pick(..) | Message::FontSize(..) | Message::TermScheme(..)) => self.fonts_message(m),
            m @ (Message::Contrast(..)
            | Message::Motion(..)
            | Message::Transparency(..)
            | Message::Size(..)
            | Message::SizeDone) => self.accessibility_message(m),
            m @ (Message::Layout(..) | Message::X11(..) | Message::WinWord(..) | Message::WinFlag(..)) => {
                self.windows_message(m)
            }
            m @ (Message::DesktopIcons(..)
            | Message::DockEdge(..)
            | Message::DockScreen(..)
            | Message::DockGap(..)
            | Message::DockOpacity(..)
            | Message::DockSize(..)
            | Message::DockSave
            | Message::DockKeepClear(..)
            | Message::DockHide(..)
            | Message::Applet(..)
            | Message::TroubleOut(..)) => self.dock_message(m),
            m @ (Message::SessionPlaces(..) | Message::SettingsKeep(..) | Message::Restore(..)) => {
                self.session_message(m)
            }
            m @ (Message::Spellcheck(..)
            | Message::Repeat(..)
            | Message::RepeatDelay(..)
            | Message::RepeatRate(..)
            | Message::RepeatDone
            | Message::KeyTest(..)) => self.keyboard_message(m),
            m @ (Message::PanelScreen(..)
            | Message::PointerSlide(..)
            | Message::PointerSave(..)
            | Message::PointerFlag(..)
            | Message::PrimaryButton(..)
            | Message::PadScroll(..)
            | Message::PadClick(..)
            | Message::ClickTest) => self.mouse_message(m),
            m @ (Message::ScreensOff(..)
            | Message::SleepAfter(..)
            | Message::BatteryScreensOff(..)
            | Message::BatterySleepAfter(..)
            | Message::SleepCommand(..)
            | Message::SleepCommandSubmit) => self.energy_message(m),
            m @ (Message::SoundVolume(..)
            | Message::SoundMute(..)
            | Message::SoundDefault(..)
            | Message::SoundTone(..)) => self.sound_message(m),
            m @ (Message::ScreenControls(..)
            | Message::SameSize(..)
            | Message::Scale(..)
            | Message::ScaleDone
            | Message::ScaleOf(..)
            | Message::ScaleOfDone(..)
            | Message::MainScreen(..)
            | Message::ScaleDefault
            | Message::Arrange(..)
            | Message::Map(..)) => self.screens_message(m),
            m @ (Message::PickPicture(..)
            | Message::Fit(..)
            | Message::Background(..)
            | Message::BackgroundSubmit
            | Message::SameAsPrimary(..)
            | Message::MakePrimary
            | Message::RemoveSource(..)
            | Message::Browse(..)
            | Message::BrowseOpen(..)
            | Message::BrowseUp
            | Message::BrowseAdd
            | Message::BrowseCancel) => self.wallpaper_message(m),
        }
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

    // ---- the view ----
}

impl Program for App {
    type Message = Message;

    /// The page's name, shown in the bar over the page (as macOS's).
    fn title(&self) -> String {
        match self.page {
            Page::Appearance => "Appearance".into(),
            Page::Theme => "Theme and fonts".into(),
            Page::Windows => "Windows".into(),
            Page::Dock => "Dock".into(),
            Page::Keyboard => "Keyboard".into(),
            Page::WiFi => "Wi-Fi".into(),
            Page::Network => "Network".into(),
            Page::Battery => "Battery".into(),
            Page::Mouse => "Mouse".into(),
            Page::Touchpad => "Touchpad".into(),
            Page::Touchscreen => "Touchscreen".into(),
            Page::Sound => "Sound".into(),
            Page::Energy => "Energy".into(),
            Page::Notifications => "Notifications".into(),
            Page::Screenshots => "Screenshots".into(),
            Page::DateTime => "Date & Time".into(),
            Page::LockScreen => "Lock Screen".into(),
            Page::Session => "Session".into(),
            Page::Agents => "Agents".into(),
            Page::Login => "Login".into(),
            Page::Apps => "Apps".into(),
            Page::DefaultApps => "Default apps".into(),
            Page::AppPage => self.app_open.as_deref().map(|a| self.known_app_name(a)).unwrap_or_else(|| "Apps".into()),
            Page::Accessibility => "Accessibility".into(),
            Page::Overview => "Screens".into(),
            Page::Screen(i) => self.outputs.get(i).map(screen_label).unwrap_or_else(|| "Screens".into()),
            Page::Wallpaper(_) => "Wallpaper".into(),
            Page::About | Page::LicenseText | Page::Notices => "About".into(),
        }
    }

    fn app_id(&self) -> String {
        "vergence-settings".into()
    }

    fn theme(&self) -> Theme {
        let text = toml::to_string(&self.desktop).unwrap_or_default();
        // Light, since its small text is hard to read dark, unless the
        // desktop's Style forces Dark (see docs/app-settings-milestone-1.md).
        // Its own Style choice does not apply.
        let dark = vergence_ui::desktop_pick(&text) == Some(false);
        let mut t = Theme::new(dark, self.accent(), appearance_text(&self.desktop, "contrast") == "high");
        t.reduced_motion = self
            .desktop
            .get("appearance")
            .and_then(|a| a.get("reduced-motion"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        t.double_click = self
            .desktop
            .get("mouse")
            .and_then(|a| a.get("double-click"))
            .and_then(|v| v.as_integer())
            .map_or(400, |v| v.clamp(100, 2000) as u32);
        // The wallpaper tint's amount and the spelling switch, as every other
        // app reads them (Theme::from_desktop), so the slider is obeyed here
        // too: even a 1% tint shows, and a fixed 8% was far too strong.
        t.tint_mix =
            vergence_widgets::tint_amount(vergence_ui::table_value(&text, "[appearance]", "tint-amount").as_deref());
        t.spell = vergence_ui::table_value(&text, "[settings]", "spellcheck")
            .or_else(|| vergence_ui::table_value(&text, "[text]", "spellcheck"))
            .is_none_or(|v| v != "false");
        t.opacity = vergence_ui::opacity_setting(&text, "[settings]");
        // The desktop's fonts as every app reads them (points; text milestone 1, step 5).
        let fonts = vergence_ui::DesktopFonts::from_text(&format!(
            "\"Gtk/FontName\" = \"{}\"\n\"Gtk/MonospaceFontName\" = \"{}\"\n",
            theme::get(&self.xsettings, theme::FONT),
            theme::get(&self.xsettings, theme::MONO)
        ));
        t.set_fonts(&fonts);
        t
    }

    /// Its bar: light as its content, unless the desktop's Style forces one.
    fn bar_style(&self) -> Option<vergence_ui::BarStyle> {
        let text = toml::to_string(&self.desktop).unwrap_or_default();
        Some(vergence_ui::bar_style_setting(&text, "[settings window]", vergence_ui::BarStyle::Light))
    }

    fn view(&self, t: &Theme) -> Element<Message> {
        self.top.set(t.top);
        let page = match self.page {
            Page::Appearance => self.appearance(),
            Page::Theme => self.theme_page(),
            Page::Windows => self.windows(),
            Page::Dock => self.dock(),
            Page::Keyboard => self.keyboard(),
            Page::WiFi => self.wifi_page(),
            Page::Network => self.network_page(),
            Page::Battery => self.battery_page(),
            Page::Mouse => self.mouse_page(),
            Page::Touchpad => self.touchpad_page(),
            Page::Touchscreen => self.touchscreen_page(),
            Page::Energy => self.energy(),
            Page::Notifications => self.notifications_page(),
            Page::Screenshots => self.screenshots_page(),
            Page::DateTime => self.datetime_page(),
            Page::LockScreen => self.lock_page(),
            Page::Session => self.session_page(),
            Page::Agents => self.agents_page(),
            Page::Login => self.login_page(),
            Page::Apps => self.apps_page(),
            Page::DefaultApps => self.defaults_page(),
            Page::AppPage => self.app_page(),
            Page::Sound => self.sound_page(),
            Page::Accessibility => self.accessibility(),
            Page::Overview => self.overview(),
            Page::Screen(_) => self.screen(),
            Page::Wallpaper(_) if self.browser.is_some() => self.browser_page(),
            Page::Wallpaper(_) => self.wallpaper(),
            Page::About => self.about(),
            Page::LicenseText => self.license_page(),
            Page::Notices => self.notices_page(),
        };
        row(vec![self.sidebar().color(t.sidebar), vrule().color(t.line), page.width(Length::FILL).height(Length::FILL)])
            .width(Length::FILL)
            .height(Length::FILL)
    }

    /// A sidebar window, as macOS's System Settings (see docs/window-styles-milestone-1.md):
    /// its content runs up under the server's
    /// frosted bar, 760 x 480 to 1100 wide, no maximize.
    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style {
            kind: vergence_ui::WinKind::Sidebar,
            min: (760.0, 480.0),
            max: (1100.0, 0.0),
            ..Default::default()
        }
    }

    fn under_bar(&self) -> bool {
        true
    }

    /// No buttons and no title in the bar over the page, since the page shows
    /// its own name; the bar still starts where the sidebar ends
    /// (see docs/settings-milestone-1.md).
    fn bar(&self) -> Option<vergence_ui::Bar> {
        Some(vergence_ui::Bar::Tools { buttons: Vec::new(), from: (SIDEBAR_W + 1.0) as f64 })
    }

    fn scroll_to(&mut self) -> Vec<(String, f32)> {
        // The page's scroll is named by its title (page_layout).
        self.kept_scroll.take().map(|s| vec![(self.title(), s)]).unwrap_or_default()
    }

    fn scrolled(&mut self, name: &str, offset: f32) {
        if name == self.title() {
            self.scrolled_to = offset;
            self.keep_place();
        }
    }

    fn session_ending(&mut self) {
        self.ending = Some(std::time::Instant::now());
        // Where it is, as it is: no "closed", so the next session opens here.
        let now = self.kept_now(None);
        keep::save(&now);
        self.kept_written = Some((now, std::time::Instant::now()));
    }

    fn closed(&mut self) {
        if vergence_widgets::session::ending(self.ending) {
            return;
        }
        // Closed by the user: kept for a while in this session (keep.rs).
        keep::save(&self.kept_now(Some((vergence_widgets::session::now(), vergence_widgets::session::id()))));
    }

    fn update(&mut self, m: Message, window: &mut Window) {
        let was = self.page;
        if matches!(m, Message::KeepMode(_)) {
            window.cancel_timer(modes::REVERT_TIMER);
        }
        self.handle(m);
        // A new mode: ask, and go back by itself unless kept.
        if std::mem::take(&mut self.mode_ask) {
            let until = std::time::Instant::now() + modes::KEEP_FOR;
            window.open_window(vergence_widgets::window(modes::KeepDialog {
                mail: self.mail.clone(),
                until,
                theme: Program::theme(self),
            }));
            window.wake_after(modes::REVERT_TIMER, modes::KEEP_FOR);
        }
        self.places_dialog(window);
        // The window is named after its page (Alt+Tab, the dock; the bar draws no title).
        if self.page != was {
            window.set_title(&self.title());
            // A new page starts at its top (the toolkit tells otherwise).
            self.scrolled_to = 0.0;
        }
        self.keep_place();
    }

    fn waker(&mut self, waker: Waker) {
        let _ = self.mail.waker.set(waker.clone());
        // Whatever arrived before the window opened is read now.
        waker.wake();
    }

    fn input(&mut self, input: &vergence_widgets::Input, window: &mut Window) -> bool {
        // Back to the window: what changed meanwhile is shown.
        if *input == vergence_widgets::Input::Focus(true) {
            self.reread();
            return true;
        }
        // Ctrl+, : the settings app's own settings, as every app's.
        if let vergence_widgets::Input::Key { sym: 0x2c, pressed: true, modifiers, .. } = input
            && modifiers.ctrl
            && !modifiers.alt
            && !modifiers.shift
        {
            self.open_own_settings(window);
        }
        false
    }

    /// Its app menu (app settings milestone 1): Settings… is its own window.
    fn commands(&self) -> Option<(bool, Vec<vergence_ui::AppCommand>)> {
        Some((false, vec![vergence_ui::AppCommand::role(1, vergence_ui::Role::Settings)]))
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        if id == 1 {
            self.open_own_settings(window);
        }
    }

    fn woken(&mut self, window: &mut Window) -> bool {
        let mut any = false;
        while let Ok(m) = self.inbox.try_recv() {
            // Another start asked for this window: forward (the server judges).
            if matches!(m, Message::HandedOver(_)) {
                window.activate();
            }
            // The keep-or-revert dialog answered: its window's timer is not needed.
            if matches!(m, Message::KeepMode(_)) {
                window.cancel_timer(modes::REVERT_TIMER);
            }
            self.handle(m);
            any = true;
        }
        // The dock's question, once the list it waited for has come.
        self.places_dialog(window);
        any
    }

    fn timer(&mut self, id: u64, _: &mut Window) -> bool {
        // 15 s and not kept (the dialog closed, or no answer): back it goes.
        if id == modes::REVERT_TIMER && self.mode_pending.is_some() {
            self.settle_mode(false);
            return true;
        }
        false
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

/// The screens from the session bus, followed (a monitor plugged in appears),
/// on a thread of its own. Outside a Vergence session there is no bus: the
/// screen pages then offer only the wallpaper.
fn bus_outputs(mail: Mail) {
    std::thread::spawn(move || {
        let Ok(mut bus) = vergence_protocol::Client::connect("vergence-settings") else { return };
        if bus.request(|serial| ClientMsg::Subscribe { serial, topic: Topic::Outputs }).is_err() {
            return;
        }
        // The input devices (protocol v7): which input pages to show.
        let _ = bus.request(|serial| ClientMsg::Subscribe { serial, topic: Topic::Inputs });
        // The bus going away ends this; nothing reconnects in a loop.
        while let Ok(m) = bus.next_event() {
            let sent = match m {
                ServerMsg::Outputs { outputs, setup, .. } => mail.send(Message::Outputs(outputs, setup)),
                ServerMsg::Inputs { devices, seen, .. } => mail.send(Message::Inputs(devices, seen)),
                _ => true,
            };
            if !sent {
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
    // Vergence's settings are for a Vergence session only: other desktops have
    // settings of their own (see docs/bar-milestone-1.md). The server gives every
    // program it starts its bus, nested or not.
    if std::env::var_os(vergence_protocol::BUS_ENV).is_none() {
        eprintln!("vergence-settings: runs in a Vergence session only ({} is not set)", vergence_protocol::BUS_ENV);
        std::process::exit(1);
    }
    // Portrait, as macOS's System Settings (see docs/window-styles-milestone-1.md): the server fits
    // the height to the screen; window memory decides after the first time.
    // `--page wifi`: the page to open with, by its sidebar name in lower case
    // ("wifi", "energy", "windows", "theme and fonts"); the server's "no network
    // cable" notification opens Wi-Fi.
    // Only one at a time: one is running already, so it turns to what this
    // start asks for and comes forward (single.rs).
    let args: Vec<String> = std::env::args().skip(1).collect();
    if single::hand_over(&args) {
        return Ok(());
    }
    let mut app = App::new();
    // Whether there is a battery, for the sidebar (the page reads it again when opened).
    app.battery.power = vergence_power::power();
    // Whether there is Wi-Fi, for the sidebar.
    app.read_interfaces();
    // Where the last run was, unless this start asks for a page (keep.rs).
    if !args.iter().any(|a| a == "--page" || a == "--app")
        && let Some(k) = keep::load()
    {
        app.open_kept(k);
    }
    app.apply_args(&args);
    // Only one System Settings (single.rs): this one now takes later starts'.
    let mail = app.mail.clone();
    single::listen(move |args| {
        mail.send(Message::HandedOver(args));
    });
    vergence_widgets::run(app, (840, 1100))
}

/// The settings app's own settings (app settings milestone 1): only what the
/// desktop does with it, since everything else here is the desktop's.
struct OwnRows;

impl vergence_widgets::prefs::Rows for OwnRows {
    fn parts(&self, text: &str) -> Vec<vergence_widgets::prefs::Part> {
        use vergence_widgets::prefs::{Control, Part, Row};
        // Always light (see docs/app-settings-milestone-1.md): no Style or Title bar of its own to choose.
        let opacity = vergence_ui::opacity_setting(text, "[appearance]");
        vec![Part {
            title: "On this desktop".into(),
            rows: vec![
                Row::new(
                    "opacity",
                    "Opacity",
                    "30 to 100: how much of the window's ground shows; the text stays as it is.",
                    Control::Percent {
                        min: 30,
                        desktop: format!("Desktop default ({:.0})", opacity * 100.0),
                        now: (opacity * 100.0).round() as u32,
                    },
                ),
                Row::new(
                    "",
                    "Window places",
                    "Where the desktop puts the settings window, for each set of screens.",
                    Control::Button { label: "Forget…".into(), id: 1 },
                ),
            ],
        }]
    }

    fn button(&self, id: u32) {
        if id == 1
            && let Err(e) = std::process::Command::new("vergence-settings")
                .args(["--page", "windows", "--forget", "vergence-settings"])
                .spawn()
        {
            eprintln!("vergence-settings: {e}");
        }
    }
}
