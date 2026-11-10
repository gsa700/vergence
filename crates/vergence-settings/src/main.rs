// Vergence settings, on iced (ported from Slint 2026-11-08: the tools share one
// toolkit, and Slint's licence question is gone; Dave's choice after the
// toolkit spike, spikes/toolkit/RESULTS.md).
//
// THE FORMAT (Dave, 2026-11-08: "similar to the macOS ... designed for desktop
// and not mobiles"; colours from his System Settings screenshot): a warm grey
// sidebar of pages, the selected one filled with the desktop's accent; the page
// on a lighter ground, its heading centred in a card, settings in soft grey
// cards with the name on the left and the control on the right. Every change
// applies at once.
//
// Pages: Appearance; Theme and fonts; Windows (title bar controls, either side,
// chosen buttons); Keyboard (key repeat: on, delay, rate, a box to try it); Energy
// (screens off and sleep after idle, the sleep command); Screens (several screens: an overview to arrange them and
// the size preference) with a page per screen (scale, wallpaper); About.
//
// Files: ~/.config/vergence/desktop.toml ([appearance], [keyboard], [wallpaper],
// [pictures], [displays]) and settings.toml (theme, fonts, the title bar
// layout); the server applies both within a second. The screens come from the
// session bus (crates/vergence-protocol), followed live.

mod disp;
mod theme;
mod thumbs;
mod wall;
mod winbar;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use iced::widget::canvas::{self, Canvas, Frame, Geometry, Program};
use iced::widget::{Space, button, column, container, image, pick_list, row, scrollable, slider, stack, text, text_input, toggler};
use iced::{Background, Border, Color, Element, Event, Length, Padding, Point, Rectangle, Renderer, Size, Subscription, Task, Theme, mouse};
use vergence_protocol::bus::{ClientMsg, OutputInfo, ServerMsg, Topic};

const SCHEMES: [&str; 3] = ["default", "dark", "light"];
const SCHEME_NAMES: [&str; 3] = ["App default", "Dark", "Light"];
const FITS: [&str; 4] = ["Fill", "Fit", "Centre", "Stretch"];
const SWATCHES: [&str; 8] = ["#3b82f6", "#8b5cf6", "#ec4899", "#ef4444", "#e05a00", "#eab308", "#22c55e", "#6b7280"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Appearance,
    Theme,
    Windows,
    Keyboard,
    Energy,
    Overview,
    Screen(usize),
    About,
}

#[derive(Debug, Clone)]
enum Message {
    Go(Page),
    // Appearance.
    Scheme(usize),
    Accent(Option<&'static str>),
    AccentText(String),
    AccentSubmit,
    Contrast(bool),
    Motion(bool),
    // Theme and fonts.
    Pick(&'static str, String),
    FontSize(bool, String),
    TermScheme(String),
    // Windows.
    Layout(winbar::Layout),
    // Keyboard.
    Repeat(bool),
    RepeatDelay(f32),
    RepeatRate(f32),
    RepeatDone,
    KeyTest(String),
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
    Arranged(usize, f32, f32),
    // Wallpaper.
    PickPicture(usize),
    Fit(usize),
    Background(String),
    BackgroundSubmit,
    SameAsPrimary(bool),
    MakePrimary,
    Thumb(PathBuf, Option<thumbs::Thumb>),
    WindowOpened(iced::window::Id),
    UiScale(f32),
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

struct App {
    page: Page,
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
    // Wallpaper.
    files: Vec<PathBuf>,
    thumbs: HashMap<PathBuf, image::Handle>,
    // The grid's previews at its exact pixel size for the window's scale, drawn
    // without filtering: resampling 38 full previews in software on every frame
    // made the wallpaper page crawl at scale 3 (Hubble, 2026-11-08).
    raw: HashMap<PathBuf, thumbs::Thumb>,
    grid: HashMap<PathBuf, image::Handle>,
    ui_scale: f32,
    asked: HashSet<PathBuf>,
    bg_text: String,
    browser: Option<Browser>,
    // Keyboard: the sliders while they move (saved on release), and the test box.
    repeat_delay: f32,
    repeat_rate: f32,
    key_test: String,
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

fn parse_hex(s: &str) -> Option<Color> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(rgb(v))
}

fn appearance_text(d: &toml::Table, k: &str) -> String {
    d.get("appearance").and_then(|a| a.get(k)).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

impl App {
    fn new() -> (App, Task<Message>) {
        let desktop = read_desktop();
        let mut app = App {
            page: Page::Appearance,
            accent_text: appearance_text(&desktop, "accent"),
            size: disp::size(&desktop) as f32,
            sleep_cmd: energy_cmd(&desktop),
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
        };
        app.repeat_delay = keyboard_int(&app.desktop, "repeat-delay", REPEAT_DELAY) as f32;
        app.repeat_rate = keyboard_int(&app.desktop, "repeat-rate", REPEAT_RATE) as f32;
        let task = app.load_pictures();
        (app, task)
    }

    fn save(&mut self, f: impl FnOnce(&mut toml::Table)) {
        match update_desktop(f) {
            Ok(t) => {
                self.desktop = t;
                self.status.clear();
            }
            Err(e) => self.status = format!("Could not save: {e}"),
        }
    }

    fn dark(&self) -> bool {
        appearance_text(&self.desktop, "color-scheme") == "dark"
    }

    fn accent(&self) -> Color {
        parse_hex(&appearance_text(&self.desktop, "accent")).unwrap_or(rgb(0x3b82f6))
    }

    /// The pictures on offer (the system's and the user's), with previews.
    fn load_pictures(&mut self) -> Task<Message> {
        let (folders, files) = wall::sources(&self.desktop);
        self.files = wall::pictures(&folders, &files);
        let list = self.files.clone();
        self.ask_thumbs(list)
    }

    /// Draw previews not yet asked for, each on its own thread (an 8K JPEG
    /// takes a second to decode; the cache makes later opens instant).
    fn ask_thumbs(&mut self, paths: Vec<PathBuf>) -> Task<Message> {
        let tasks: Vec<Task<Message>> = paths
            .into_iter()
            .filter(|p| self.asked.insert(p.clone()))
            .map(|p| {
                Task::perform(
                    async move {
                        let (tx, rx) = iced::futures::channel::oneshot::channel();
                        std::thread::spawn(move || {
                            let t = thumbs::get(&p);
                            let _ = tx.send((p, t));
                        });
                        rx.await.ok()
                    },
                    |r| match r {
                        Some((p, t)) => Message::Thumb(p, t),
                        None => Message::BrowseCancel,
                    },
                )
            })
            .collect();
        Task::batch(tasks)
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
        self.browser = None;
        let scope = self.scope();
        self.bg_text = wall::shown(&self.desktop, scope.as_ref().map(|(i, _)| i.as_str())).color;
        if let Some(o) = self.current_output() {
            self.scale = o.base_scale as f32;
        }
    }

    fn update(&mut self, m: Message) -> Task<Message> {
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
            Message::Pick(key, value) => self.set_x(key, &value),
            Message::FontSize(mono, size) => {
                let key = if mono { theme::MONO } else { theme::FONT };
                let (family, _) = theme::split_font(&theme::get(&self.xsettings, key));
                if let Ok(sz) = size.parse::<f32>() {
                    self.set_x(key, &theme::join_font(&family, sz));
                }
            }
            Message::Layout(l) => self.set_x(winbar::KEY, &winbar::format(l)),
            Message::Outputs(outputs, setup) => {
                let current = self.current_output().map(|o| o.id.clone());
                self.outputs = outputs;
                self.setup = setup;
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
                return self.ask_thumbs(wanted);
            }
            Message::Size(v) => self.size = if (v - 1.0).abs() < 0.035 { 1.0 } else { v },
            Message::Repeat(on) => self.save_keyboard(|t| {
                t.insert("repeat".into(), on.into());
            }),
            Message::RepeatDelay(v) => self.repeat_delay = v,
            Message::RepeatRate(v) => self.repeat_rate = v,
            Message::RepeatDone => {
                let (delay, rate) = (self.repeat_delay.round() as i64, self.repeat_rate.round() as i64);
                self.save_keyboard(|t| {
                    t.insert("repeat-delay".into(), delay.into());
                    t.insert("repeat-rate".into(), rate.into());
                });
            }
            Message::KeyTest(t) => self.key_test = t,
            Message::TermScheme(name) => {
                let v = name.to_ascii_lowercase();
                self.save_table("terminal", |t| {
                    t.insert("scheme".into(), v.into());
                })
            }
            Message::ScreensOff(label) => {
                let m = minutes_of(&OFF_CHOICES, &label);
                self.save_energy(|t| {
                    t.insert("screens-off".into(), m.into());
                })
            }
            Message::SleepAfter(label) => {
                let m = minutes_of(&SLEEP_CHOICES, &label);
                self.save_energy(|t| {
                    t.insert("sleep".into(), m.into());
                })
            }
            Message::SleepCommand(s) => self.sleep_cmd = s,
            Message::SleepCommandSubmit => {
                let v = self.sleep_cmd.trim().to_string();
                self.save_energy(|t| {
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
            Message::Arranged(i, fx, fy) => {
                let mut rects: Vec<disp::Rect> = self.outputs.iter().map(|o| disp::Rect { id: o.id.clone(), x: o.x, y: o.y, w: o.width, h: o.height }).collect();
                if i < rects.len() {
                    let (x0, y0, bw, bh) = bbox(&self.outputs);
                    let (x, y) = (x0 + (fx * bw as f32).round() as i32, y0 + (fy * bh as f32).round() as i32);
                    let (nx, ny) = disp::snap(&rects, i, x, y, (bw.max(bh) / 40).max(20));
                    rects[i].x = nx;
                    rects[i].y = ny;
                    let setup = self.setup.clone();
                    self.save(|d| disp::set_arrangement(d, &setup, &rects));
                }
            }
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
                    self.status = "Background colour: write it as #rrggbb".into();
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
                    self.grid.insert(p.clone(), thumbs::grid_handle(&t, self.ui_scale));
                    self.thumbs.insert(p.clone(), image::Handle::from_rgba(t.w, t.h, t.rgba.clone()));
                    self.raw.insert(p, t);
                }
            }
            Message::WindowOpened(id) => return iced::window::scale_factor(id).map(Message::UiScale),
            Message::UiScale(s) => {
                if s > 0.0 && (s - self.ui_scale).abs() > 0.001 {
                    self.ui_scale = s;
                    self.grid = self.raw.iter().map(|(p, t)| (p.clone(), thumbs::grid_handle(t, s))).collect();
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
                return self.load_pictures();
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
                    return self.load_pictures();
                }
            }
            Message::BrowseCancel => self.browser = None,
        }
        Task::none()
    }

    fn save_keyboard(&mut self, f: impl FnOnce(&mut toml::Table)) {
        self.save(|d| {
            if !d.get("keyboard").is_some_and(|a| a.is_table()) {
                d.insert("keyboard".into(), toml::Value::Table(toml::Table::new()));
            }
            if let Some(t) = d.get_mut("keyboard").and_then(|a| a.as_table_mut()) {
                f(t);
            }
        });
    }

    fn save_energy(&mut self, f: impl FnOnce(&mut toml::Table)) {
        self.save_table("energy", f);
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
        self.save(|d| {
            if !d.get("appearance").is_some_and(|a| a.is_table()) {
                d.insert("appearance".into(), toml::Value::Table(toml::Table::new()));
            }
            if let Some(t) = d.get_mut("appearance").and_then(|a| a.as_table_mut()) {
                f(t);
            }
        });
    }

    fn save_accent(&mut self) {
        let a = self.accent_text.trim().to_string();
        if !a.is_empty() && parse_hex(&a).is_none() {
            self.status = "Accent colour: write it as #rrggbb, or leave it empty".into();
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
                set_ui_family(&self.xsettings);
                self.status.clear();
            }
            Err(e) => self.status = format!("Could not save: {e}"),
        }
    }

    /// A picture, fit or colour chosen: for the open screen (which then has its
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

    fn title(&self) -> String {
        "Vergence Settings".into()
    }

    fn theme(&self) -> Theme {
        let base = if self.dark() { Theme::Dark } else { Theme::Light };
        Theme::custom("Vergence".to_string(), iced::theme::Palette { primary: self.accent(), background: self.colours().page, ..base.palette() })
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(bus_outputs),
            iced::window::open_events().map(Message::WindowOpened),
            iced::window::events().filter_map(|(_, e)| match e {
                iced::window::Event::Rescaled(s) => Some(Message::UiScale(s)),
                _ => None,
            }),
        ])
    }

    // ---- the view ----

    fn colours(&self) -> Colours {
        let accent = self.accent();
        if self.dark() {
            Colours { sidebar: rgb(0x262628), page: rgb(0x1c1c1e), card: rgb(0x2c2c2e), line: rgb(0x3a3a3c), dim: rgb(0xa1a1a6), text: rgb(0xf2f2f2), hover: Color { a: 0.08, ..Color::WHITE }, accent }
        } else {
            Colours { sidebar: rgb(0xe7e5e2), page: rgb(0xf7f7f6), card: rgb(0xecebea), line: rgb(0xd9d8d6), dim: rgb(0x6e6e73), text: rgb(0x1d1d1f), hover: Color { a: 0.06, ..Color::BLACK }, accent }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let c = self.colours();
        let page: Element<'_, Message> = match self.page {
            Page::Appearance => self.appearance(c),
            Page::Theme => self.theme_page(c),
            Page::Windows => self.windows(c),
            Page::Keyboard => self.keyboard(c),
            Page::Energy => self.energy(c),
            Page::Overview => self.overview(c),
            Page::Screen(_) if self.browser.is_some() => self.browser_page(c),
            Page::Screen(_) => self.screen(c),
            Page::About => self.about(c),
        };
        // No fill of its own: the page colour is the theme's background, which the
        // software renderer clears each redrawn region with directly. A container
        // fill the size of the page crossed the scrolled region's edge, so every
        // scroll step filled it again through a clip mask: ~45% of the frame
        // (dtrace on Hubble, 2026-11-08).
        let page = container(page).width(Length::Fill).height(Length::Fill);
        row![self.sidebar(c), container(Space::new().width(1.0).height(Length::Fill)).style(move |_| container::background(c.line)), page].into()
    }

    fn sidebar(&self, c: Colours) -> Element<'_, Message> {
        let item = move |label: String, page: Page, selected: bool, child: bool| -> Element<'_, Message> {
            let font = ui_font(if selected { iced::font::Weight::Semibold } else { iced::font::Weight::Normal });
            button(text(label).size(if child { 12 } else { 13 }).color(if selected { Color::WHITE } else { c.text }).font(font))
                .width(Length::Fill)
                .padding(Padding { top: 6.0, bottom: 6.0, left: if child { 26.0 } else { 12.0 }, right: 12.0 })
                .on_press(Message::Go(page))
                .style(move |_: &Theme, s| button::Style {
                    background: if selected {
                        Some(Background::Color(c.accent))
                    } else if s == button::Status::Hovered {
                        Some(Background::Color(c.hover))
                    } else {
                        None
                    },
                    border: Border { radius: 6.0.into(), ..Border::default() },
                    text_color: c.text,
                    ..button::Style::default()
                })
                .into()
        };
        let multi = self.outputs.len() > 1;
        let screens_page = if multi { Page::Overview } else { Page::Screen(0) };
        let screens_selected = if multi { self.page == Page::Overview } else { matches!(self.page, Page::Screen(_)) };
        let mut col = column![
            item("Appearance".into(), Page::Appearance, self.page == Page::Appearance, false),
            item("Theme and fonts".into(), Page::Theme, self.page == Page::Theme, false),
            item("Windows".into(), Page::Windows, self.page == Page::Windows, false),
            item("Keyboard".into(), Page::Keyboard, self.page == Page::Keyboard, false),
            item("Energy".into(), Page::Energy, self.page == Page::Energy, false),
            item(if self.outputs.is_empty() { "Wallpaper".into() } else { "Screens".into() }, screens_page, screens_selected, false),
        ]
        .spacing(2);
        if multi {
            for (i, o) in self.outputs.iter().enumerate() {
                col = col.push(item(screen_label(o), Page::Screen(i), self.page == Page::Screen(i), true));
            }
        }
        col = col.push(Space::new().height(16.0)).push(item("About".into(), Page::About, self.page == Page::About, false));
        container(col)
            .width(Length::Fixed(200.0))
            .height(Length::Fill)
            .padding(Padding { top: 12.0, bottom: 12.0, left: 10.0, right: 10.0 })
            .style(move |_| container::background(c.sidebar))
            .into()
    }

    fn appearance(&self, c: Colours) -> Element<'_, Message> {
        let scheme = SCHEMES.iter().position(|s| *s == appearance_text(&self.desktop, "color-scheme")).unwrap_or(0);
        let accent = appearance_text(&self.desktop, "accent");
        let none_selected = accent.is_empty();
        let mut swatches = row![
            button(text("\u{2715}").size(10).color(c.dim).center())
                .width(22)
                .height(22)
                .padding(0)
                .on_press(Message::Accent(None))
                .style(move |_: &Theme, _| button::Style {
                    background: None,
                    border: Border { color: if none_selected { c.text } else { c.line }, width: if none_selected { 2.0 } else { 1.0 }, radius: 11.0.into() },
                    ..button::Style::default()
                })
        ]
        .spacing(6)
        .align_y(iced::alignment::Vertical::Center);
        for hex in SWATCHES {
            let col = parse_hex(hex).unwrap_or(Color::BLACK);
            let selected = accent.eq_ignore_ascii_case(hex);
            swatches = swatches.push(button(Space::new()).width(22).height(22).padding(0).on_press(Message::Accent(Some(hex))).style(move |_: &Theme, _| button::Style {
                background: Some(Background::Color(col)),
                border: Border { color: c.text, width: if selected { 2.0 } else { 0.0 }, radius: 11.0.into() },
                ..button::Style::default()
            }));
        }
        let contrast = appearance_text(&self.desktop, "contrast") == "high";
        let motion = self.desktop.get("appearance").and_then(|a| a.get("reduced-motion")).and_then(|v| v.as_bool()).unwrap_or(false);
        page_layout(
            c,
            "Appearance",
            "Light or dark, the accent colour, contrast and motion. Every app that follows the desktop changes at once.",
            vec![
                group(c, None, vec![
                    setting(c, "Style", "App default lets each app choose.", pick_list(SCHEME_NAMES, Some(SCHEME_NAMES[scheme]), |s| Message::Scheme(SCHEME_NAMES.iter().position(|x| *x == s).unwrap_or(0))).text_size(13).width(160).into()),
                    setting(c, "Accent colour", if accent.is_empty() { "None: each app uses its own." } else { "" }, swatches.into()),
                    setting(c, "Custom accent", "Any colour as #rrggbb; Enter applies it.", text_input("#rrggbb", &self.accent_text).on_input(Message::AccentText).on_submit(Message::AccentSubmit).size(13).width(120).into()),
                ]),
                group(c, Some("Accessibility"), vec![
                    setting(c, "High contrast", "Apps that support it draw stronger edges and text.", toggler(contrast).on_toggle(Message::Contrast).into()),
                    setting(c, "Reduce motion", "Apps that support it turn off animations.", toggler(motion).on_toggle(Message::Motion).into()),
                ]),
                self.status_line(c),
            ],
        )
    }

    fn theme_page(&self, c: Colours) -> Element<'_, Message> {
        let with = |mut list: Vec<String>, cur: &str| {
            if !cur.is_empty() && !list.iter().any(|x| x == cur) {
                list.insert(0, cur.to_string());
            }
            list
        };
        let pick = |key: &'static str, list: &[String]| -> Element<'_, Message> {
            let cur = theme::get(&self.xsettings, key);
            let list = with(list.to_vec(), &cur);
            pick_list(list, (!cur.is_empty()).then_some(cur), move |v| Message::Pick(key, v)).text_size(13).width(220).into()
        };
        let num = |s: f32| if s.fract() == 0.0 { format!("{}", s as i32) } else { format!("{s}") };
        let font_row = |mono: bool| -> Element<'_, Message> {
            let key = if mono { theme::MONO } else { theme::FONT };
            let (family, size) = theme::split_font(&theme::get(&self.xsettings, key));
            let families = with(if mono { self.monos.clone() } else { self.fonts.clone() }, &family);
            let sizes: Vec<String> = theme::sizes(size).into_iter().map(num).collect();
            row![
                pick_list(families, Some(family.clone()), move |f| Message::Pick(key, theme::join_font(&f, size))).text_size(13).width(220),
                pick_list(sizes, Some(num(size)), move |s| Message::FontSize(mono, s)).text_size(13).width(76),
            ]
            .spacing(6)
            .into()
        };
        page_layout(
            c,
            "Theme and fonts",
            "How apps look: their theme, icons, pointer and fonts. GTK and X11 apps follow at once; the newest GNOME apps keep their own look.",
            vec![
                group(c, Some("Theme"), vec![
                    setting(c, "Applications", "Buttons, menus and windows in GTK apps.", pick(theme::THEME, &self.gtk_themes)),
                    setting(c, "Icons", "", pick(theme::ICONS, &self.icon_themes)),
                    setting(c, "Pointer", "The same size on every screen.", pick(theme::CURSOR, &self.cursor_themes)),
                ]),
                group(c, Some("Fonts"), vec![
                    setting(c, "Interface", "Menus, buttons, labels, and the title bars.", font_row(false)),
                    setting(c, "Monospace", "Terminals and code.", font_row(true)),
                ]),
                group(c, Some("Terminal"), vec![setting(
                    c,
                    "Colours",
                    "The terminal's 16 colours. Tango is the classic set that mc, htop and vim were tuned for; Catppuccin is softer.",
                    pick_list(TERM_SCHEMES.to_vec(), Some(term_scheme(&self.desktop)), |s| Message::TermScheme(s.to_string())).text_size(13).width(160).into(),
                )]),
                self.status_line(c),
            ],
        )
    }

    fn keyboard(&self, c: Colours) -> Element<'_, Message> {
        let on = self.desktop.get("keyboard").and_then(|a| a.get("repeat")).and_then(|v| v.as_bool()).unwrap_or(true);
        let value = |s: String| text(s).size(13).color(c.text).width(64).align_x(iced::alignment::Horizontal::Right);
        let delay = row![
            slider(150.0..=1000.0, self.repeat_delay, Message::RepeatDelay).step(10.0_f32).on_release(Message::RepeatDone).width(220),
            value(format!("{:.0} ms", self.repeat_delay)),
        ]
        .spacing(8)
        .align_y(iced::alignment::Vertical::Center);
        let rate = row![
            slider(5.0..=60.0, self.repeat_rate, Message::RepeatRate).step(1.0_f32).on_release(Message::RepeatDone).width(220),
            value(format!("{:.0} a second", self.repeat_rate)),
        ]
        .spacing(8)
        .align_y(iced::alignment::Vertical::Center);
        let mut rows = vec![setting(c, "Key repeat", "Holding a key down types it again and again.", toggler(on).on_toggle(Message::Repeat).into())];
        if on {
            rows.push(setting(c, "Delay", "How long a key is held before it repeats. Longer: fewer doubled letters.", delay.into()));
            rows.push(setting(c, "Speed", "How fast it repeats once it starts.", rate.into()));
        }
        page_layout(
            c,
            "Keyboard",
            "Key repeat for every app. Changes apply at once.",
            vec![
                group(c, Some("Key repeat"), rows),
                group(c, Some("Try it"), vec![setting(c, "Test", "Hold a key down here.", text_input("Type here", &self.key_test).on_input(Message::KeyTest).size(13).width(260).into())]),
                self.status_line(c),
            ],
        )
    }

    fn energy(&self, c: Colours) -> Element<'_, Message> {
        let off = energy_int(&self.desktop, "screens-off", 0);
        let sleep = energy_int(&self.desktop, "sleep", 0);
        let (off_list, off_now) = minute_choices(&OFF_CHOICES, off);
        let (sleep_list, sleep_now) = minute_choices(&SLEEP_CHOICES, sleep);
        let default_cmd = if Path::new("/run/systemd/system").is_dir() { "systemctl suspend" } else { "sudo -n zzz" };
        let detail = format!("Empty: {default_cmd}. zzz needs root: a sudoers line for it, without a password.");
        page_layout(
            c,
            "Energy",
            "When nothing has been touched for a while: the screens go dark, then the machine sleeps. Any key or a nudge of the mouse brings everything back.",
            vec![
                group(
                    c,
                    Some("After no input"),
                    vec![
                        setting(c, "Turn the screens off", "Every monitor is switched off; apps keep running.", pick_list(off_list, Some(off_now), Message::ScreensOff).text_size(13).width(160).into()),
                        setting(c, "Put the machine to sleep", "Suspend to memory. The screens go off first.", pick_list(sleep_list, Some(sleep_now), Message::SleepAfter).text_size(13).width(160).into()),
                    ],
                ),
                group(
                    c,
                    Some("How it sleeps"),
                    vec![setting(c, "Sleep command", &detail, text_input(default_cmd, &self.sleep_cmd).on_input(Message::SleepCommand).on_submit(Message::SleepCommandSubmit).size(13).width(260).into())],
                ),
                group(c, None, vec![setting(c, "Apps", "Apps cannot yet ask to keep the screens on while they play or present; that comes with the session bus.", Space::new().into())]),
                self.status_line(c),
            ],
        )
    }

    fn windows(&self, c: Colours) -> Element<'_, Message> {
        let l = winbar::parse(&theme::get(&self.xsettings, winbar::KEY));
        let side = |left: bool, label: &'static str| {
            let selected = l.left == left;
            button(text(label).size(13).color(if selected { Color::WHITE } else { c.text }))
                .padding(Padding::from([5, 16]))
                .on_press(Message::Layout(winbar::Layout { left, ..l }))
                .style(move |_: &Theme, _| button::Style {
                    background: Some(Background::Color(if selected { c.accent } else { c.page })),
                    border: Border { color: if selected { c.accent } else { c.line }, width: 1.0, radius: 6.0.into() },
                    ..button::Style::default()
                })
        };
        page_layout(
            c,
            "Windows",
            "The title bar: which side its controls are on, and which buttons it has. The title, tabs and everything else in the bar move to the other side.",
            vec![
                group(c, None, vec![preview(c, l)]),
                group(c, Some("Title bar"), vec![
                    setting(c, "Controls", "Left, as on a Mac; right, as on Windows.", row![side(true, "Left"), side(false, "Right")].spacing(6).into()),
                    setting(c, "Minimise", "Hides the window until it is chosen again.", toggler(l.minimize).on_toggle(move |on| Message::Layout(winbar::Layout { minimize: on, ..l })).into()),
                    setting(c, "Maximise", "Fills the screen. Double-clicking the title bar does too.", toggler(l.maximize).on_toggle(move |on| Message::Layout(winbar::Layout { maximize: on, ..l })).into()),
                    setting(c, "Close", "Always there.", text("On").size(13).color(c.dim).into()),
                ]),
                self.status_line(c),
            ],
        )
    }

    fn overview(&self, c: Colours) -> Element<'_, Message> {
        let arrangement = Canvas::new(Arrangement { app: self, highlight: None, interactive: true, colours: c }).width(Length::Fill).height(Length::Fixed(240.0));
        page_layout(
            c,
            "Screens",
            "Drag the screens to where they sit on your desk. Each one's scale is worked out from its size and how far away it is.",
            vec![
                group(c, None, vec![container(arrangement).padding(8).into()]),
                text("Double-click a screen for its own settings.").size(11).color(c.dim).into(),
                group(c, None, vec![setting(c, "Size", &format!("How big everything is, on every screen: {}%.", (self.size * 100.0).round()), size_slider(self.size, c))]),
                self.status_line(c),
            ],
        )
    }

    fn screen(&self, c: Colours) -> Element<'_, Message> {
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
        let mut items: Vec<Element<'_, Message>> = Vec::new();
        if multi && let Page::Screen(i) = self.page {
            let map = Canvas::new(Arrangement { app: self, highlight: Some(i), interactive: false, colours: c }).width(Length::Fill).height(Length::Fixed(110.0));
            items.push(group(c, None, vec![container(map).padding(6).into()]));
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
            let mut shown_row = row![text(format!("{:.0}%", o.scale * 100.0)).size(13).color(c.dim)].spacing(8).align_y(iced::alignment::Vertical::Center);
            if chosen {
                shown_row = shown_row.push(button(text("Default").size(12)).on_press(Message::ScaleDefault).padding(Padding::from([3, 10])));
            }
            let mut rows = vec![setting(c, "Scale", note, scale_slider(self.scale, o, &marks, c)), setting(c, "Shown at", &o.note, shown_row.into())];
            if !multi {
                rows.push(setting(c, "Size", &format!("How big everything is: {}%.", (self.size * 100.0).round()), size_slider(self.size, c)));
            }
            items.push(group(c, Some("Display"), rows));
        }
        let mut wp: Vec<Element<'_, Message>> = Vec::new();
        if multi && !is_primary {
            let detail = if shown.own { "Off: this screen has its own.".to_string() } else { format!("Shows {primary_label}'s wallpaper.") };
            wp.push(setting(c, "Same as the primary screen", &detail, toggler(!shown.own).on_toggle(Message::SameAsPrimary).into()));
        }
        if multi {
            wp.push(setting(
                c,
                "Primary screen",
                if is_primary { "Its wallpaper is the default for the others." } else { "Make this the primary: its wallpaper becomes the default." },
                if is_primary { text("Primary").size(13).color(c.dim).into() } else { button(text("Make primary").size(12)).on_press(Message::MakePrimary).padding(Padding::from([4, 12])).into() },
            ));
        }
        let fit = wall::MODES.iter().position(|m| *m == shown.mode).unwrap_or(0);
        wp.push(setting(c, "Fit", "", pick_list(FITS, Some(FITS[fit]), |f| Message::Fit(FITS.iter().position(|x| *x == f).unwrap_or(0))).text_size(13).width(140).into()));
        wp.push(setting(c, "Background colour", "Around a picture, or alone without one. Enter applies it.", text_input("#202020", &self.bg_text).on_input(Message::Background).on_submit(Message::BackgroundSubmit).size(13).width(120).into()));
        items.push(group(c, Some("Wallpaper"), wp));
        // The pictures.
        let mut grid = row![].spacing(8);
        for (i, p) in self.files.iter().enumerate() {
            let selected = shown.image.as_deref() == Some(p.as_path());
            let picture: Element<'_, Message> = match self.grid.get(p) {
                Some(h) => image(h.clone())
                    .width(Length::Fixed(thumbs::GRID_W))
                    .height(Length::Fixed(thumbs::GRID_H))
                    .content_fit(iced::ContentFit::Fill)
                    .filter_method(image::FilterMethod::Nearest)
                    .into(),
                None => container(Space::new().width(thumbs::GRID_W).height(thumbs::GRID_H)).style(move |_| container::background(c.card)).into(),
            };
            let name = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            // Long names are shortened to the tile's width instead of running into the
            // next one. Cut in the text, not clipped: a clipped caption made the software
            // renderer clear a window-sized mask for each one on every frame.
            let caption = text(short_name(&name, 25)).size(11).color(c.text).width(thumbs::GRID_W).wrapping(text::Wrapping::None);
            let tile = button(column![picture, caption].spacing(4))
                .padding(3)
                .on_press(Message::PickPicture(i))
                .style(move |_: &Theme, _| button::Style {
                    background: None,
                    // Unselected tiles draw no border at all (a transparent one is still drawn).
                    border: if selected { Border { color: c.accent, width: 3.0, radius: 6.0.into() } } else { Border::default() },
                    ..button::Style::default()
                });
            grid = grid.push(tile);
        }
        items.push(grid.wrap().into());
        // Your pictures.
        let (folders, files) = wall::sources(&self.desktop);
        let mut yours: Vec<Element<'_, Message>> = Vec::new();
        for (i, (p, kind)) in folders.iter().map(|f| (f, "Folder")).chain(files.iter().map(|f| (f, "Picture"))).enumerate() {
            yours.push(setting(c, &p.display().to_string(), kind, button(text("Remove").size(12)).on_press(Message::RemoveSource(i)).padding(Padding::from([3, 10])).into()));
        }
        yours.push(setting(
            c,
            if folders.is_empty() && files.is_empty() { "Add your own pictures" } else { "Add more" },
            "Pictures added to a folder later appear too.",
            row![
                button(text("Add a folder\u{2026}").size(12)).on_press(Message::Browse(true)).padding(Padding::from([4, 10])),
                button(text("Add pictures\u{2026}").size(12)).on_press(Message::Browse(false)).padding(Padding::from([4, 10])),
            ]
            .spacing(6)
            .into(),
        ));
        items.push(group(c, Some("Your pictures"), yours));
        items.push(self.status_line(c));
        page_layout(c, &title, &description, items)
    }

    fn browser_page(&self, c: Colours) -> Element<'_, Message> {
        let Some(b) = self.browser.as_ref() else { return Space::new().into() };
        let mut entries = column![].spacing(1);
        for (i, (p, folder)) in b.entries.iter().enumerate() {
            let picked = !b.folders && b.picked.contains(p);
            let mark = if *folder { "\u{25B8}" } else if picked { "\u{2713}" } else { "" };
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let fg = if picked { Color::WHITE } else { c.text };
            entries = entries.push(
                button(row![text(mark).size(13).width(18).color(fg), text(name).size(13).color(fg)].spacing(6))
                    .width(Length::Fill)
                    .padding(Padding::from([5, 10]))
                    .on_press(Message::BrowseOpen(i))
                    .style(move |_: &Theme, s| button::Style {
                        background: if picked {
                            Some(Background::Color(c.accent))
                        } else if s == button::Status::Hovered {
                            Some(Background::Color(c.hover))
                        } else {
                            None
                        },
                        border: Border { radius: 4.0.into(), ..Border::default() },
                        ..button::Style::default()
                    }),
            );
        }
        let add_label = if b.folders {
            "Add this folder".to_string()
        } else if b.picked.len() == 1 {
            "Add 1 picture".into()
        } else {
            format!("Add {} pictures", b.picked.len())
        };
        let mut add = button(text(add_label).size(13).color(Color::WHITE)).padding(Padding::from([5, 14])).style(move |_: &Theme, _| button::Style {
            background: Some(Background::Color(c.accent)),
            border: Border { radius: 6.0.into(), ..Border::default() },
            ..button::Style::default()
        });
        if b.folders || !b.picked.is_empty() {
            add = add.on_press(Message::BrowseAdd);
        }
        column![
            title_card(
                c,
                if b.folders { "Add a folder" } else { "Add pictures" },
                if b.folders { "Open the folder you want, then add it. Every picture in it and its folders is offered." } else { "Click pictures to pick them, in as many folders as you like, then add them." }
            ),
            row![button(text("\u{2191} Up").size(12)).on_press(Message::BrowseUp).padding(Padding::from([4, 10])), text(b.dir.display().to_string()).size(12).color(c.dim)].spacing(10).align_y(iced::alignment::Vertical::Center),
            container(scrollable(entries)).height(Length::Fill).width(Length::Fill).padding(4).style(move |_| card_style(c)),
            row![Space::new().width(Length::Fill), button(text("Cancel").size(13)).on_press(Message::BrowseCancel).padding(Padding::from([5, 14])), add].spacing(8),
        ]
        .spacing(12)
        .padding(22)
        .into()
    }

    fn about(&self, c: Colours) -> Element<'_, Message> {
        let session = match (std::env::var("VERGENCE_BUS"), std::env::var("WAYLAND_DISPLAY")) {
            (Ok(_), Ok(w)) => format!("Vergence on {w}"),
            (_, Ok(w)) => format!("another Wayland desktop ({w})"),
            _ => "an X11 desktop".to_string(),
        };
        page_layout(
            c,
            "About",
            "Vergence: every app the right size, in the right place and readable, on any monitor setup.",
            vec![group(c, None, vec![
                setting(c, "Settings app", "", text(format!("version {}", env!("CARGO_PKG_VERSION"))).size(13).color(c.text).into()),
                setting(c, "Session", "", text(session).size(13).color(c.text).into()),
                setting(c, "Built with", "", text("iced (MIT)").size(13).color(c.text).into()),
            ])],
        )
    }

    fn status_line(&self, c: Colours) -> Element<'_, Message> {
        if self.status.is_empty() { Space::new().into() } else { text(self.status.clone()).size(11).color(c.dim).into() }
    }
}

// ---- the look ----

#[derive(Debug, Clone, Copy)]
struct Colours {
    sidebar: Color,
    page: Color,
    card: Color,
    line: Color,
    dim: Color,
    text: Color,
    hover: Color,
    accent: Color,
}

fn rgb(v: u32) -> Color {
    Color::from_rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn card_style(c: Colours) -> container::Style {
    container::Style { background: Some(Background::Color(c.card)), border: Border { radius: 10.0.into(), ..Border::default() }, ..container::Style::default() }
}

fn bold() -> iced::Font {
    ui_font(iced::font::Weight::Bold)
}

/// The desktop's UI font family (Gtk/FontName in settings.toml), for every label.
/// iced's generic sans-serif plus a weight lets cosmic-text pick any family that
/// has that weight: on Void, Noto Sans has no Semibold, so semibold labels came
/// out in a script font with no Latin glyphs (boxes; Hubble, 2026-11-09). Naming
/// the family keeps the choice within it (the nearest weight it has).
static UI_FAMILY: std::sync::Mutex<&'static str> = std::sync::Mutex::new("Noto Sans");

fn set_ui_family(xsettings: &vergence_xsettings::Settings) {
    static NAMES: std::sync::Mutex<Option<HashSet<&'static str>>> = std::sync::Mutex::new(None);
    let (family, _) = theme::split_font(&theme::get(xsettings, theme::FONT));
    let family = family.trim();
    if family.is_empty() {
        return;
    }
    let mut names = NAMES.lock().unwrap();
    let names = names.get_or_insert_with(HashSet::new);
    let name = match names.get(family) {
        Some(n) => *n,
        None => {
            let n: &'static str = Box::leak(family.to_string().into_boxed_str());
            names.insert(n);
            n
        }
    };
    *UI_FAMILY.lock().unwrap() = name;
}

fn ui_font(weight: iced::font::Weight) -> iced::Font {
    iced::Font { family: iced::font::Family::Name(*UI_FAMILY.lock().unwrap()), weight, ..iced::Font::DEFAULT }
}

fn title_card<'a>(c: Colours, title: &str, description: &str) -> Element<'a, Message> {
    container(column![text(title.to_string()).size(20).color(c.text).font(bold()), text(description.to_string()).size(13).color(c.dim).center()].spacing(6).align_x(iced::alignment::Horizontal::Center))
        .width(Length::Fill)
        .padding(18)
        .style(move |_| card_style(c))
        .into()
}

/// A page: its heading card, then its groups, scrolling as one.
fn page_layout<'a>(c: Colours, title: &str, description: &str, items: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    let mut col = column![title_card(c, title, description)].spacing(16).padding(22);
    for i in items {
        col = col.push(i);
    }
    scrollable(col).height(Length::Fill).into()
}

/// A group of settings in a rounded card, a hairline between rows.
fn group<'a>(c: Colours, heading: Option<&str>, rows: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    let mut inner = column![];
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            inner = inner.push(container(container(Space::new().height(1.0).width(Length::Fill)).style(move |_| container::background(c.line))).padding(Padding::from([0, 12])));
        }
        inner = inner.push(r);
    }
    let card = container(inner).width(Length::Fill).padding(2).style(move |_| card_style(c));
    match heading {
        Some(h) => column![text(h.to_string()).size(12).color(c.dim).font(ui_font(iced::font::Weight::Semibold)), card].spacing(6).into(),
        None => card.into(),
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
fn setting<'a>(c: Colours, label: &str, detail: &str, control: Element<'a, Message>) -> Element<'a, Message> {
    let mut left = column![text(label.to_string()).size(13).color(c.text)];
    if !detail.is_empty() {
        left = left.push(text(detail.to_string()).size(11).color(c.dim));
    }
    row![left.width(Length::Fill), control].spacing(16).padding(Padding::from([8, 12])).align_y(iced::alignment::Vertical::Center).into()
}

fn scale_max(o: &OutputInfo) -> f64 {
    (o.default_scale.ceil() + 2.0).max(3.0)
}

/// Ticks under a slider of `width` over `lo..=hi`: (value, tall, lit).
fn ticks<'a>(width: f32, lo: f32, hi: f32, marks: &[(f32, bool, bool)], c: Colours) -> Element<'a, Message> {
    // iced's slider handle is 14 px wide by default: its centre travels 7 px in from each end.
    let mut s = stack![Space::new().width(width).height(7.0)];
    for &(m, tall, lit) in marks {
        let x = (width - 14.0) * ((m - lo) / (hi - lo)) + 7.0 - 1.0;
        s = s.push(row![
            Space::new().width(Length::Fixed(x.max(0.0))),
            container(Space::new().width(2.0).height(if tall { 7.0 } else { 5.0 })).style(move |_| container::background(if lit { c.accent } else { c.dim })),
        ]);
    }
    s.into()
}

/// The size slider, 75% to 150%, sticking to 100% within 3.5%.
fn size_slider<'a>(v: f32, c: Colours) -> Element<'a, Message> {
    row![
        column![slider(0.75..=1.5, v, Message::Size).step(0.01_f32).on_release(Message::SizeDone).width(220), ticks(220.0, 0.75, 1.5, &[(1.0, true, v == 1.0)], c)].spacing(2),
        text(format!("{:.0}%", v * 100.0)).size(13).color(c.text).width(48).align_x(iced::alignment::Horizontal::Right),
    ]
    .spacing(8)
    .align_y(iced::alignment::Vertical::Center)
    .into()
}

/// A screen's scale slider, with marks at the whole numbers and the default.
fn scale_slider<'a>(v: f32, o: &OutputInfo, marks: &[f64], c: Colours) -> Element<'a, Message> {
    let max = scale_max(o) as f32;
    let width = 260.0;
    let m: Vec<(f32, bool, bool)> = marks.iter().map(|&m| (m as f32, (m - o.default_scale).abs() < 1e-6, (v as f64 - m).abs() < 1e-6)).collect();
    row![
        column![slider(0.75..=max, v, Message::Scale).step(0.01_f32).on_release(Message::ScaleDone).width(width), ticks(width, 0.75, max, &m, c)].spacing(2),
        text(format!("{:.0}%", v * 100.0)).size(13).color(c.text).width(48).align_x(iced::alignment::Horizontal::Right),
    ]
    .spacing(8)
    .align_y(iced::alignment::Vertical::Center)
    .into()
}

/// A preview of a title bar in the chosen layout, in the server's colours.
fn preview<'a>(c: Colours, l: winbar::Layout) -> Element<'a, Message> {
    let dot = |col: Color| {
        container(Space::new().width(12.0).height(12.0)).style(move |_| container::Style { background: Some(Background::Color(col)), border: Border { radius: 6.0.into(), ..Border::default() }, ..container::Style::default() })
    };
    let close = rgb(0xc74d47);
    let other = Color { a: 0.55, ..Color::WHITE };
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
    let mut buttons = row![].spacing(6).align_y(iced::alignment::Vertical::Center);
    for col in order {
        buttons = buttons.push(dot(col));
    }
    let tab = |active: bool, t: &'static str| {
        container(text(t).size(11).color(Color::WHITE)).padding(Padding::from([4, 12])).style(move |_| container::Style {
            background: active.then_some(Background::Color(rgb(0x1e1e2e))),
            border: Border { radius: iced::border::Radius { top_left: 6.0, top_right: 6.0, bottom_left: 0.0, bottom_right: 0.0 }, ..Border::default() },
            ..container::Style::default()
        })
    };
    let tabs = row![tab(true, "~/src/vergence"), tab(false, "htop"), text("+").size(13).color(Color::WHITE)].spacing(4).align_y(iced::alignment::Vertical::Bottom);
    let bar = if l.left {
        row![buttons, Space::new().width(14.0), tabs, Space::new().width(Length::Fill)]
    } else {
        row![tabs, Space::new().width(Length::Fill), Space::new().width(14.0), buttons]
    };
    let window = column![
        container(bar.align_y(iced::alignment::Vertical::Bottom).padding(Padding { top: 6.0, bottom: 0.0, left: 10.0, right: 10.0 }))
            .height(32)
            .align_y(iced::alignment::Vertical::Bottom)
            .style(|_| container::background(Color::from_rgb(0.20, 0.24, 0.32))),
        container(Space::new().height(56.0).width(Length::Fill)).style(|_| container::background(rgb(0x1e1e2e))),
    ];
    container(container(window).width(Length::Fixed(420.0)).style(move |_| container::Style { border: Border { color: c.line, width: 1.0, radius: 4.0.into() }, ..container::Style::default() }))
        .center_x(Length::Fill)
        .padding(14)
        .into()
}

fn bbox(outputs: &[OutputInfo]) -> (i32, i32, i32, i32) {
    let x0 = outputs.iter().map(|o| o.x).min().unwrap_or(0);
    let y0 = outputs.iter().map(|o| o.y).min().unwrap_or(0);
    let x1 = outputs.iter().map(|o| o.x + o.width).max().unwrap_or(1);
    let y1 = outputs.iter().map(|o| o.y + o.height).max().unwrap_or(1);
    (x0, y0, (x1 - x0).max(1), (y1 - y0).max(1))
}

/// The screens as arranged, each showing its wallpaper and name. Interactive
/// (the overview): drag one to move it, double-click for its page. Otherwise
/// (a screen's page): the screen being edited framed in the accent; a click on
/// another goes to its page.
struct Arrangement<'a> {
    app: &'a App,
    highlight: Option<usize>,
    interactive: bool,
    colours: Colours,
}

#[derive(Default)]
struct DragState {
    dragging: Option<(usize, Point)>,
    offset: iced::Vector,
    last_click: Option<(usize, std::time::Instant)>,
}

/// Room around the arrangement in the canvas, and the gap between screens.
const MARGIN: f32 = 20.0;
const GAP: f32 = 3.0;

impl Arrangement<'_> {
    /// The arrangement's area in the canvas and its scale (canvas px per desktop px).
    fn area(&self, bounds: Rectangle) -> (Point, f32) {
        let (_, _, bw, bh) = bbox(&self.app.outputs);
        let k = ((bounds.width - 2.0 * MARGIN) / bw as f32).min((bounds.height - 2.0 * MARGIN) / bh as f32).max(0.0001);
        (Point::new((bounds.width - bw as f32 * k) / 2.0, (bounds.height - bh as f32 * k) / 2.0), k)
    }

    /// Each screen's rectangle in the canvas.
    fn tiles(&self, bounds: Rectangle) -> Vec<Rectangle> {
        let (x0, y0, _, _) = bbox(&self.app.outputs);
        let (origin, k) = self.area(bounds);
        self.app
            .outputs
            .iter()
            .map(|s| Rectangle::new(Point::new(origin.x + (s.x - x0) as f32 * k + GAP, origin.y + (s.y - y0) as f32 * k + GAP), Size::new(s.width as f32 * k - 2.0 * GAP, s.height as f32 * k - 2.0 * GAP)))
            .collect()
    }
}

impl Program<Message> for Arrangement<'_> {
    type State = DragState;

    fn update(&self, state: &mut DragState, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_in(bounds)?;
                let i = self.tiles(bounds).iter().position(|t| t.contains(p))?;
                if !self.interactive {
                    return Some(canvas::Action::publish(Message::Go(Page::Screen(i))).and_capture());
                }
                if let Some((j, t)) = state.last_click
                    && j == i
                    && t.elapsed() < std::time::Duration::from_millis(400)
                {
                    state.last_click = None;
                    state.dragging = None;
                    return Some(canvas::Action::publish(Message::Go(Page::Screen(i))).and_capture());
                }
                state.last_click = Some((i, std::time::Instant::now()));
                state.dragging = Some((i, p));
                state.offset = iced::Vector::new(0.0, 0.0);
                Some(canvas::Action::request_redraw().and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging.is_some() => {
                let (_, start) = state.dragging?;
                let p = cursor.position_in(bounds).unwrap_or(start);
                state.offset = p - start;
                Some(canvas::Action::request_redraw().and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.dragging.is_some() => {
                let (i, _) = state.dragging.take()?;
                let off = std::mem::replace(&mut state.offset, iced::Vector::new(0.0, 0.0));
                if off.x.abs() < 2.0 && off.y.abs() < 2.0 {
                    return Some(canvas::Action::request_redraw());
                }
                // Back to desktop pixels, as fractions of the arrangement's size.
                let (x0, y0, bw, bh) = bbox(&self.app.outputs);
                let (_, k) = self.area(bounds);
                let o = &self.app.outputs[i];
                let fx = ((o.x - x0) as f32 + off.x / k) / bw as f32;
                let fy = ((o.y - y0) as f32 + off.y / k) / bh as f32;
                Some(canvas::Action::publish(Message::Arranged(i, fx, fy)))
            }
            _ => None,
        }
    }

    fn draw(&self, state: &DragState, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let c = self.colours;
        let dragged = state.dragging.map(|(d, _)| d);
        for (i, mut t) in self.tiles(bounds).into_iter().enumerate() {
            if dragged == Some(i) {
                t.x += state.offset.x;
                t.y += state.offset.y;
            }
            let o = &self.app.outputs[i];
            frame.fill_rectangle(t.position(), t.size(), Color::BLACK);
            if let Some(path) = wall::shown(&self.app.desktop, Some(&o.id)).image
                && let Some(h) = self.app.thumbs.get(&path)
            {
                frame.draw_image(t, canvas::Image::new(h.clone()));
            }
            let band = 34.0_f32.min(t.height / 2.0);
            frame.fill_rectangle(Point::new(t.x, t.y + t.height - band), Size::new(t.width, band), Color { a: 0.6, ..Color::BLACK });
            frame.fill_text(canvas::Text {
                content: format!("{}\n{:.0}%", screen_label(o), o.scale * 100.0),
                position: Point::new(t.x + t.width / 2.0, t.y + t.height - band + 3.0),
                color: Color::WHITE,
                size: iced::Pixels(11.0),
                line_height: iced::widget::text::LineHeight::Relative(1.3),
                align_x: iced::widget::text::Alignment::Center,
                ..canvas::Text::default()
            });
            let framed = self.highlight == Some(i) || dragged == Some(i);
            frame.stroke(&canvas::Path::rectangle(t.position(), t.size()), canvas::Stroke::default().with_color(if framed { c.accent } else { c.line }).with_width(if framed { 3.0 } else { 1.0 }));
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &DragState, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if state.dragging.is_some() {
            return mouse::Interaction::Grabbing;
        }
        match cursor.position_in(bounds) {
            Some(p) if self.tiles(bounds).iter().any(|t| t.contains(p)) => {
                if self.interactive {
                    mouse::Interaction::Grab
                } else {
                    mouse::Interaction::Pointer
                }
            }
            _ => mouse::Interaction::default(),
        }
    }
}

/// A folder's sub-folders, then its pictures.
fn list(dir: &Path) -> Vec<(PathBuf, bool)> {
    let (dirs, pics) = wall::list_dir(dir);
    dirs.into_iter().map(|d| (d, true)).chain(pics.into_iter().map(|p| (p, false))).collect()
}

/// The screens from the session bus, followed (a monitor plugged in appears).
/// Outside a Vergence session there is no bus: the screen pages then offer
/// only the wallpaper.
fn bus_outputs() -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(8, async |tx: iced::futures::channel::mpsc::Sender<Message>| {
        let (done_tx, done_rx) = iced::futures::channel::oneshot::channel::<()>();
        std::thread::spawn(move || {
            let _done = done_tx;
            let mut tx = tx;
            let Ok(mut bus) = vergence_protocol::Client::connect("vergence-settings") else { return };
            if bus.request(|serial| ClientMsg::Subscribe { serial, topic: Topic::Outputs }).is_err() {
                return;
            }
            while let Ok(m) = bus.next_event() {
                if let ServerMsg::Outputs { outputs, setup, .. } = m
                    && iced::futures::executor::block_on(iced::futures::SinkExt::send(&mut tx, Message::Outputs(outputs, setup))).is_err()
                {
                    return;
                }
            }
        });
        let _ = done_rx.await;
        // The bus went away: stay quiet rather than reconnect in a loop.
        std::future::pending::<()>().await
    })
}

fn main() -> iced::Result {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("vergence-settings {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // Software drawing, as the terminal: the GPU renderer hung Hubble's GPU
    // (2026-11-08, /var/log/messages); a settings window is light work for the CPU.
    if std::env::var_os("ICED_BACKEND").is_none() {
        // SAFETY: no other threads yet.
        unsafe { std::env::set_var("ICED_BACKEND", "tiny-skia") };
    }
    set_ui_family(&theme::read());
    iced::application(App::new, App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .subscription(App::subscription)
        .default_font(ui_font(iced::font::Weight::Normal))
        .window(iced::window::Settings { size: Size::new(880.0, 640.0), min_size: Some(Size::new(680.0, 480.0)), ..Default::default() })
        .run()
}
