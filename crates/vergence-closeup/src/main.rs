//! Close-Up (files milestone 1, step 5; docs/files-milestone-1.md §4):
//! the step after File Viewer's Preview, a file's contents in full. Dave's
//! words: "picture, video, and pdf viewer. Connected to file manager by file
//! type association, but a standalone app"; named Close-Up 2026-11-15.
//!
//! - **Pictures** (decoded apart by vergence-decode's helper, never in this
//!   process): fitted to the window, or at any zoom (scroll or the keys,
//!   about the pointer), panned by dragging, a double click between fitted
//!   and actual size; turned a quarter either way; GIF and WebP animations
//!   played; transparency over a checkerboard. Left and Right step through
//!   the pictures of the file's folder in File Viewer's order.
//! - **PDFs** go to Reader (Reader milestone 1, 2026-11-20): documents
//!   have their own app; a window with nothing else in it closes.
//! - **Text**: shown read-only in the toolkit's text area, with "Open in Text
//!   Editor".
//! - **Anything else**: says what it is and offers the app chosen for it.
//! - **Windows**: one process (hand.rs), one file a window, no tabs (Dave,
//!   2026-11-16; docs/files-milestone-2.md). A file dropped on a window
//!   takes its place; files handed over or chosen in Open… open in the
//!   window if it shows nothing, else each in a window of its own.
//! - **Preview** (`--preview FILE`): one file in a small window, for File
//!   Viewer's space bar; Space or Escape closes it, Enter opens it in full.
//! - **Its settings** (Ctrl+,): the ground around a picture, whether small
//!   pictures stay at actual size, animations; desktop.toml `[closeup]`.

mod hand;
mod pictures;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use vergence_scene::{Color, Image, Rect};
use vergence_ui::{AppCommand as C, Bar, BarStyle, Role};
use vergence_widgets::prefs::{Choice, Control, Part, Prefs, Row, Rows};
use vergence_widgets::{Align, AreaOptions, ButtonLook, Element, Input, Length, Padding, Program, TextDoc, Theme, Touch, Waker, Window, button, column, drawing, label, panel, row, rule, text_area};

const APP_ID: &str = "vergence-closeup";
const SECTION: &str = "[closeup]";
/// The largest text file shown (more is cut, and said so).
const MAX_TEXT: usize = 4 << 20;
/// Zoom limits, as image pixels per screen pixel.
const MIN_ZOOM: f32 = 0.02;
const MAX_ZOOM: f32 = 32.0;
/// The animation's timer.
const ANIM_TIMER: u64 = 1;

mod cmd {
    pub const NEW_WINDOW: u32 = 1;
    pub const OPEN: u32 = 2;
    pub const CLOSE_TAB: u32 = 3;
    pub const ZOOM_IN: u32 = 4;
    pub const ZOOM_OUT: u32 = 5;
    pub const FIT: u32 = 6;
    pub const ACTUAL: u32 = 7;
    pub const TURN_RIGHT: u32 = 8;
    pub const TURN_LEFT: u32 = 9;
    pub const NEXT: u32 = 10;
    pub const PREVIOUS: u32 = 11;
    pub const OPEN_WITH: u32 = 14;
    pub const CROP: u32 = 15;
    pub const SAVE: u32 = 16;
    pub const SETTINGS: u32 = 40;
    pub const ABOUT: u32 = 41;
}

/// What a file turned out to be, from the worker that read it.
enum Loaded {
    /// Width, height, and each frame's delay and levels (pictures.rs).
    Picture { width: u32, height: u32, frames: Vec<(u32, Vec<Arc<Image>>)> },
    Text { text: String, cut: bool },
    /// A PDF: handed to Reader, which shows documents.
    HandedOver,
    /// Why it cannot be shown here, and whether another app might.
    Problem { why: String, elsewhere: bool },
}

/// What all windows share (one thread).
struct Shared {
    /// Files other starts handed over (hand.rs), and from the Open dialog.
    inbox: Arc<Mutex<Vec<Vec<PathBuf>>>>,
    /// Files read by the workers: (tab id, what it was).
    loaded: Arc<Mutex<Vec<(u32, Loaded)>>>,
    next_tab: u32,
}

type SharedRef = Rc<RefCell<Shared>>;

/// A tab: one file.
struct Tab {
    id: u32,
    path: PathBuf,
    content: Content,
    /// None: fitted. Some: image pixels per screen pixel.
    zoom: Option<f32>,
    /// Offset of the picture's center from the place's, in points.
    pan: (f32, f32),
    /// The frame shown (animations).
    frame: usize,
    /// Turned or cropped since it was read or saved (step 6 of
    /// screenshots milestone 1: Close-Up's minor edits).
    edited: bool,
    /// Cropping: the part to keep, in the picture's pixels, while it is
    /// dragged out (None until it is).
    crop: Option<Option<(f32, f32, f32, f32)>>,
}

enum Content {
    Loading,
    Picture { width: u32, height: u32, frames: Vec<(u32, Vec<Arc<Image>>)> },
    Text { doc: TextDoc, cut: bool },
    Problem { why: String, elsewhere: bool },
}

impl Tab {
    fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.path.display().to_string())
    }
}

/// Reads a file in a worker: a picture through the helper (its pyramid made
/// here, off the window's thread), text, or why not.
fn load(path: PathBuf, id: u32, shared: &Shared, waker: Option<Waker>) {
    let out = shared.loaded.clone();
    std::thread::spawn(move || {
        let mime = vergence_types::of_file(&path);
        let loaded = if mime == "application/pdf" {
            // Documents are Reader's (Reader milestone 1).
            match std::process::Command::new("vergence-reader").arg(&path).spawn() {
                Ok(_) => Loaded::HandedOver,
                Err(e) => Loaded::Problem { why: format!("PDF documents open in Reader, which could not be started: {e}"), elsewhere: false },
            }
        } else if mime.starts_with("image/") {
            match vergence_decode::decode_file(&path, None) {
                Ok(p) => {
                    let (w, h) = (p.width, p.height);
                    let still = p.frames.len() == 1;
                    let frames = p
                        .frames
                        .into_iter()
                        .map(|f| {
                            let img = Image { width: w, height: h, rgba: f.rgba };
                            // Animations are small as a rule: one level each.
                            (f.delay_ms, if still { pictures::pyramid(img) } else { vec![Arc::new(img)] })
                        })
                        .collect();
                    Loaded::Picture { width: w, height: h, frames }
                }
                Err(e) => Loaded::Problem { why: format!("“{}” could not be shown: {e}", path.file_name().unwrap_or_default().to_string_lossy()), elsewhere: !matches!(e, vergence_decode::Error::Io(_)) },
            }
        } else if vergence_types::is_a(mime, vergence_types::TEXT) {
            use std::io::Read;
            let mut bytes = Vec::new();
            match vergence_types::open_regular(&path).and_then(|f| f.take(MAX_TEXT as u64 + 1).read_to_end(&mut bytes)) {
                Ok(_) => {
                    let cut = bytes.len() > MAX_TEXT;
                    bytes.truncate(MAX_TEXT);
                    match String::from_utf8(bytes) {
                        Ok(text) => Loaded::Text { text, cut },
                        Err(e) => {
                            let ok = e.utf8_error().valid_up_to();
                            if cut && ok + 4 >= MAX_TEXT {
                                Loaded::Text { text: String::from_utf8_lossy(&e.into_bytes()[..ok]).into_owned(), cut }
                            } else {
                                Loaded::Problem { why: "This text is not UTF-8.".into(), elsewhere: true }
                            }
                        }
                    }
                }
                Err(e) => Loaded::Problem { why: format!("It could not be read: {e}"), elsewhere: false },
            }
        } else if mime == vergence_types::FOLDER {
            Loaded::Problem { why: "This is a folder.".into(), elsewhere: true }
        } else {
            Loaded::Problem { why: format!("Close-Up does not show {} ({mime}) yet.", vergence_types::describe(mime)), elsewhere: true }
        };
        out.lock().unwrap_or_else(|e| e.into_inner()).push((id, loaded));
        if let Some(w) = waker {
            w.wake();
        }
    });
}

/// ~/.config/vergence/desktop.toml.
fn desktop_text() -> (String, Option<SystemTime>) {
    match vergence_widgets::prefs::desktop_path() {
        Some(p) => (std::fs::read_to_string(&p).unwrap_or_default(), std::fs::metadata(&p).and_then(|m| m.modified()).ok()),
        None => (String::new(), None),
    }
}

/// The picture's place and the zoom it is drawn at: (zoom, x, y, w, h) in
/// points, for a place `r` at `scale`.
fn placed(r: Rect, scale: f32, (w, h): (u32, u32), zoom: Option<f32>, pan: (f32, f32), small_actual: bool) -> (f32, f32, f32, f32, f32) {
    let scale = scale.max(0.25);
    let z = zoom.unwrap_or_else(|| {
        let fit = (r.w * scale / w as f32).min(r.h * scale / h as f32);
        if small_actual { fit.min(1.0) } else { fit }
    });
    let (dw, dh) = (w as f32 * z / scale, h as f32 * z / scale);
    (z, r.x + (r.w - dw) / 2.0 + pan.0, r.y + (r.h - dh) / 2.0 + pan.1, dw, dh)
}

/// A pan kept so the picture cannot be lost: centered on an axis where it
/// is smaller than the place, else no edge further in than the place's.
fn clamp_pan(r: Rect, dw: f32, dh: f32, pan: (f32, f32)) -> (f32, f32) {
    let axis = |room: f32, size: f32, p: f32| if size <= room { 0.0 } else { p.clamp(-(size - room) / 2.0, (size - room) / 2.0) };
    (axis(r.w, dw, pan.0), axis(r.h, dh, pan.1))
}

struct Viewer {
    shared: SharedRef,
    tabs: Vec<Tab>,
    front: u32,
    preview: bool,
    desktop: String,
    stamp: Option<SystemTime>,
    waker: Option<Waker>,
    scale: f32,
    /// The picture's place as last drawn, and the pointer in it.
    area: Rc<Cell<Rect>>,
    pointer: Rc<Cell<(f32, f32)>>,
    /// A drag: where it started and the pan then.
    drag: Option<((f32, f32), (f32, f32))>,
    last_press: Option<std::time::Instant>,
    prefs_open: Rc<Cell<bool>>,
    animating: bool,
    /// The close question, while it is asked (an edited picture).
    asking: Option<Rc<Cell<Option<Answer>>>>,
}

/// The close question's answers.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Answer {
    Save,
    Discard,
    Cancel,
}

/// "Save changes?", a modal dialog over the window, as Edit's. The answer
/// goes back through `answer` and a wake of the window.
struct SaveQuestion {
    name: String,
    copy: bool,
    answer: Rc<Cell<Option<Answer>>>,
    waker: Option<Waker>,
    theme: Theme,
}

impl Program for SaveQuestion {
    type Message = Answer;
    fn title(&self) -> String {
        "Save changes?".into()
    }
    fn app_id(&self) -> String {
        APP_ID.into()
    }
    fn theme(&self) -> Theme {
        self.theme.clone()
    }
    fn view(&self, _: &Theme) -> Element<Answer> {
        let b = |text: &str, a: Answer, look: ButtonLook| -> Element<Answer> { button(label(text).size(12.0)).look(look).padding(Padding::hv(4.0, 12.0)).on_press(a).name(text) };
        let save = if self.copy { "Save as PNG" } else { "Save" };
        let how = if self.copy { "Saved as a PNG beside it; the original is kept. " } else { "" };
        panel(
            column(vec![
                label(format!("Save the changes to “{}”?", self.name)).size(15.0).weight(600).wrap().width(Length::FILL),
                label(format!("{how}If you don't save them, they are lost.")).dim().wrap().width(Length::FILL),
                row(vec![b("Don't save", Answer::Discard, ButtonLook::Normal), vergence_widgets::space().width(Length::FILL), b("Cancel", Answer::Cancel, ButtonLook::Normal), b(save, Answer::Save, ButtonLook::Accent)]).spacing(8.0).align(Align::Center).width(Length::FILL),
            ])
            .spacing(10.0)
            .width(Length::FILL),
        )
        .padding(20.0)
        .radius(0.0)
        .width(Length::FILL)
        .height(Length::FILL)
    }
    fn update(&mut self, a: Answer, window: &mut Window) {
        self.answer.set(Some(a));
        window.close();
    }
    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { kind: vergence_ui::WinKind::Dialog, modal: true, min: (420.0, 150.0), max: (0.0, 0.0), alone: false }
    }
}

impl Drop for SaveQuestion {
    /// Answered or closed, the window hears of it.
    fn drop(&mut self) {
        if let Some(w) = &self.waker {
            w.wake();
        }
    }
}



#[derive(Debug, Clone)]
enum Msg {
    Touch(Touch),
    /// A command from the toolbar.
    Run(u32),
    OpenWith,
    OpenInEditor,
    /// The part dragged out kept.
    Crop,
}

impl Viewer {
    fn new(shared: SharedRef, preview: bool) -> Viewer {
        let (desktop, stamp) = desktop_text();
        Viewer {
            shared,
            tabs: Vec::new(),
            front: 0,
            preview,
            desktop,
            stamp,
            waker: None,
            scale: 1.0,
            area: Rc::new(Cell::new(Rect::new(0.0, 0.0, 1.0, 1.0))),
            pointer: Rc::new(Cell::new((0.0, 0.0))),
            drag: None,
            last_press: None,
            prefs_open: Rc::new(Cell::new(false)),
            animating: false,
            asking: None,
        }
    }

    fn setting(&self, key: &str) -> Option<String> {
        vergence_ui::table_value(&self.desktop, SECTION, key)
    }

    fn small_actual(&self) -> bool {
        self.setting("small-actual").as_deref() != Some("false")
    }

    fn reread(&mut self) -> bool {
        let (text, stamp) = desktop_text();
        if stamp == self.stamp {
            return false;
        }
        (self.desktop, self.stamp) = (text, stamp);
        true
    }

    /// Files to show: the first here if this window shows nothing (or
    /// `replace`, a drop), the rest each in a new window.
    fn take(&mut self, paths: &[PathBuf], replace: bool, window: &mut Window) {
        let mut rest = paths.iter();
        if (self.tabs.is_empty() || replace || self.preview)
            && let Some(first) = rest.next()
        {
            self.tabs.clear();
            self.open_paths(std::slice::from_ref(first));
        }
        for p in rest {
            if self.tabs.iter().any(|t| t.path == *p) {
                continue;
            }
            let mut w = Viewer::new(self.shared.clone(), false);
            w.queue(p.clone());
            window.open_window(vergence_widgets::window(w));
        }
    }

    /// A file shown once the window can be woken (its `waker` reads it).
    fn queue(&mut self, p: PathBuf) {
        let id = {
            let mut s = self.shared.borrow_mut();
            s.next_tab += 1;
            s.next_tab
        };
        self.tabs.push(Tab { id, path: p, content: Content::Loading, zoom: None, pan: (0.0, 0.0), frame: 0, edited: false, crop: None });
        self.front = id;
    }

    fn open_paths(&mut self, paths: &[PathBuf]) {
        for p in paths {
            let p = std::path::absolute(p).unwrap_or_else(|_| p.clone());
            // Already open: brought to the front.
            if let Some(t) = self.tabs.iter().find(|t| t.path == p) {
                self.front = t.id;
                continue;
            }
            let id = {
                let mut s = self.shared.borrow_mut();
                s.next_tab += 1;
                s.next_tab
            };
            load(p.clone(), id, &self.shared.borrow(), self.waker.clone());
            self.tabs.push(Tab { id, path: p, content: Content::Loading, zoom: None, pan: (0.0, 0.0), frame: 0, edited: false, crop: None });
            self.front = id;
            if self.preview {
                // A preview shows one file: the newest.
                self.tabs.retain(|t| t.id == id);
            }
        }
    }

    fn front_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == self.front).or(self.tabs.first())
    }

    fn front_mut(&mut self) -> Option<&mut Tab> {
        let id = self.front_tab()?.id;
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// The front picture's zoom as drawn now.
    fn zoom_now(&self) -> Option<f32> {
        let t = self.front_tab()?;
        let Content::Picture { width, height, .. } = &t.content else { return None };
        Some(placed(self.area.get(), self.scale, (*width, *height), t.zoom, t.pan, self.small_actual()).0)
    }

    /// Zoom to `z` keeping the point `at` (in the place) where it is.
    fn zoom_to(&mut self, z: f32, at: Option<(f32, f32)>) {
        let (r, scale, small) = (self.area.get(), self.scale, self.small_actual());
        let Some(now) = self.zoom_now() else { return };
        let Some(t) = self.front_mut() else { return };
        let Content::Picture { width, height, .. } = &t.content else { return };
        let (w, h) = (*width, *height);
        let z = z.clamp(MIN_ZOOM, MAX_ZOOM);
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let (px, py) = at.unwrap_or((cx, cy));
        let k = z / now;
        let pan = ((px - cx) - (px - cx - t.pan.0) * k, (py - cy) - (py - cy - t.pan.1) * k);
        t.zoom = Some(z);
        let (_, _, _, dw, dh) = placed(r, scale, (w, h), Some(z), (0.0, 0.0), small);
        t.pan = clamp_pan(r, dw, dh, pan);
    }











    fn fit(&mut self) {
        if let Some(t) = self.front_mut() {
            t.zoom = None;
            t.pan = (0.0, 0.0);
        }
    }

    fn turn(&mut self, left: bool) {
        let Some(t) = self.front_mut() else { return };
        if let Content::Picture { width, height, frames } = &mut t.content {
            // A still picture's turn can be saved; an animation's is for looking.
            t.edited |= frames.len() == 1;
            t.crop = None;
            for (_, levels) in frames.iter_mut() {
                for l in levels.iter_mut() {
                    *l = Arc::new(pictures::turn(l, left));
                }
            }
            std::mem::swap(width, height);
            t.pan = (0.0, 0.0);
        }
    }

    /// In crop mode, a point of the picture area as the picture's pixel
    /// (clamped to it); None when not cropping.
    fn crop_point(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let t = self.front_tab()?;
        t.crop?;
        let Content::Picture { width, height, .. } = &t.content else { return None };
        let r = self.area.get();
        let (_, px, py, dw, dh) = placed(r, self.scale, (*width, *height), t.zoom, t.pan, self.small_actual());
        let kx = *width as f32 / dw.max(1.0);
        let ky = *height as f32 / dh.max(1.0);
        Some((((r.x + x - px) * kx).clamp(0.0, *width as f32), ((r.y + y - py) * ky).clamp(0.0, *height as f32)))
    }

    /// Crop mode on (or off, letting go of a part dragged out).
    fn crop_toggle(&mut self) {
        let Some(t) = self.front_mut() else { return };
        let still = matches!(&t.content, Content::Picture { frames, .. } if frames.len() == 1);
        t.crop = if t.crop.is_some() || !still { None } else { Some(None) };
    }

    /// The part dragged out kept, the rest cut away (each level made again
    /// from the whole).
    fn crop_apply(&mut self) {
        let Some(t) = self.front_mut() else { return };
        let Some(Some((x0, y0, x1, y1))) = t.crop else { return };
        let Content::Picture { width, height, frames } = &mut t.content else { return };
        let (x0, y0) = (x0.max(0.0).round() as u32, y0.max(0.0).round() as u32);
        let (x1, y1) = ((x1.round() as u32).min(*width), (y1.round() as u32).min(*height));
        if x1 <= x0 + 1 || y1 <= y0 + 1 {
            return;
        }
        let Some(full) = frames.first().and_then(|f| f.1.first()).cloned() else { return };
        let cut = pictures::crop(&full, x0, y0, x1 - x0, y1 - y0);
        *width = cut.width;
        *height = cut.height;
        frames[0].1 = pictures::pyramid(cut);
        t.edited = true;
        t.crop = None;
        t.zoom = None;
        t.pan = (0.0, 0.0);
    }

    /// Saves the front picture's edits: a PNG back in its place; any other
    /// kind as a PNG copy beside it (a JPEG is not made again, losing more),
    /// which the window then shows. False (and why, in the log) if not.
    fn save(&mut self) -> bool {
        let Some(t) = self.front_mut() else { return false };
        let Content::Picture { frames, .. } = &t.content else { return false };
        let Some(full) = frames.first().and_then(|f| f.1.first()).cloned() else { return false };
        let png = t.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"));
        let to = if png { t.path.clone() } else { pictures::copy_name(&t.path) };
        match pictures::save_png(&full, &to) {
            Ok(()) => {
                t.path = to;
                t.edited = false;
                true
            }
            Err(e) => {
                eprintln!("vergence-closeup: not saved to {}: {e}", to.display());
                false
            }
        }
    }

    /// The next or previous picture in the front file's folder.
    fn step(&mut self, by: isize) {
        let Some(t) = self.front_tab() else { return };
        // Edits not saved: kept until saved or the window is closed (which asks).
        if t.edited {
            return;
        }
        if !matches!(t.content, Content::Picture { .. } | Content::Loading | Content::Problem { .. }) {
            return;
        }
        let list = pictures::siblings(&t.path);
        let Some(i) = list.iter().position(|p| *p == t.path) else { return };
        let j = (i as isize + by).rem_euclid(list.len() as isize) as usize;
        if j == i {
            return;
        }
        let next = list[j].clone();
        let waker = self.waker.clone();
        let shared = self.shared.clone();
        let Some(t) = self.front_mut() else { return };
        t.path = next.clone();
        t.content = Content::Loading;
        t.zoom = None;
        t.pan = (0.0, 0.0);
        t.frame = 0;
        let id = t.id;
        load(next, id, &shared.borrow(), waker);
    }

    fn open_dialog(&mut self, window: &mut Window) {
        let inbox = self.shared.borrow().inbox.clone();
        let waker = self.waker.clone();
        let dir = self.front_tab().and_then(|t| t.path.parent().map(Path::to_path_buf));
        let mut d = vergence_widgets::filedialog::FileDialog::open(APP_ID, "Open", self.theme(), move |paths| {
            if !paths.is_empty() {
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push(paths);
                if let Some(w) = &waker {
                    w.wake();
                }
            }
        })
        .multiple(true)
        .filters(vec![vergence_widgets::filedialog::Filter::of_types("Pictures", &["image/*"]), vergence_widgets::filedialog::Filter::of_types("Text", &["text/plain"]), vergence_widgets::filedialog::Filter::new("All files", &[])]);
        if let Some(dir) = dir {
            d = d.folder(&dir);
        }
        window.open_window(vergence_widgets::window(d));
    }

    fn open_settings(&mut self, window: &mut Window) {
        if self.prefs_open.get() {
            return;
        }
        let waker = self.waker.clone();
        let prefs = Prefs::new("Close-Up", APP_ID, SECTION, true, Box::new(CloseupRows), self.prefs_open.clone()).on_write(move || {
            if let Some(w) = &waker {
                w.wake();
            }
        });
        window.open_window(vergence_widgets::window(prefs));
    }

    /// The front file with the app chosen for it (by the server, as File Viewer will).
    fn open_with(&self, editor: bool) {
        let Some(t) = self.front_tab() else { return };
        let mut c = std::process::Command::new("vergence-open");
        if editor {
            c.args(["--with", "vergence-edit"]);
        }
        let _ = c.arg(&t.path).spawn();
    }

    /// Animations: the next frame after the front one's delay.
    fn arm_animation(&mut self, window: &mut Window) {
        let animate = self.setting("animate").as_deref() != Some("false");
        let delay = self.front_tab().and_then(|t| match &t.content {
            Content::Picture { frames, .. } if frames.len() > 1 && animate => frames.get(t.frame).map(|f| f.0.max(20)),
            _ => None,
        });
        match delay {
            Some(ms) if !self.animating => {
                self.animating = true;
                window.wake_after(ANIM_TIMER, std::time::Duration::from_millis(ms as u64));
            }
            None if self.animating => {
                self.animating = false;
                window.cancel_timer(ANIM_TIMER);
            }
            _ => {}
        }
    }

    fn run(&mut self, id: u32, window: &mut Window) {
        match id {
            cmd::NEW_WINDOW => {
                let w = Viewer::new(self.shared.clone(), false);
                window.open_window(vergence_widgets::window(w));
            }
            cmd::OPEN => self.open_dialog(window),
            cmd::CLOSE_TAB => window.close(),
            cmd::ZOOM_IN => {
                if let Some(z) = self.zoom_now() {
                    self.zoom_to(z * 1.25, None);
                }
            }
            cmd::ZOOM_OUT => {
                if let Some(z) = self.zoom_now() {
                    self.zoom_to(z / 1.25, None);
                }
            }
            cmd::FIT => self.fit(),
            cmd::ACTUAL => self.zoom_to(1.0, None),
            cmd::TURN_RIGHT => self.turn(false),
            cmd::TURN_LEFT => self.turn(true),
            cmd::CROP => self.crop_toggle(),
            cmd::SAVE => {
                self.save();
            }
            cmd::NEXT => self.step(1),
            cmd::PREVIOUS => self.step(-1),
            cmd::OPEN_WITH => self.open_with(false),
            cmd::SETTINGS => self.open_settings(window),
            cmd::ABOUT => {
                let about = vergence_widgets::about::About::new(APP_ID, "Close-Up", env!("CARGO_PKG_VERSION"), "A file's contents in full: pictures (PNG, JPEG, GIF, WebP, BMP, SVG), decoded apart from the app, and text. The step after FileView's Preview. PDF documents open in Reader.", self.theme());
                window.open_window(vergence_widgets::window(about));
            }
            _ => {}
        }
        self.arm_animation(window);
        window.redraw();
    }

    fn picture_view(&self, t: &Tab, width: u32, height: u32, frames: &[(u32, Vec<Arc<Image>>)]) -> Element<Msg> {
        let levels = frames.get(t.frame).or(frames.first()).map(|f| f.1.clone()).unwrap_or_default();
        let (zoom, pan, scale, small) = (t.zoom, t.pan, self.scale, self.small_actual());
        let crop = t.crop.flatten();
        let ground = match self.setting("ground").as_deref() {
            Some("light") => Some(Color::rgb(0xf2, 0xf2, 0xf2)),
            Some("window") => None,
            _ => Some(Color::rgb(0x1c, 0x1c, 0x1e)),
        };
        let area = self.area.clone();
        let waker = self.waker.clone();
        drawing(move |scene, _, r, theme| {
            // A new place (the first frame, a resize): the status line's zoom
            // is worked out from it, so the window looks again.
            if area.get() != r {
                area.set(r);
                if let Some(w) = &waker {
                    w.wake();
                }
            }
            scene.push_clip(r, 0.0);
            scene.fill(r, ground.unwrap_or(theme.page));
            let (_, x, y, dw, dh) = placed(r, scale, (width, height), zoom, pan, small);
            let dest = Rect::new(x, y, dw, dh);
            // Transparency shows over a checkerboard, under the picture only.
            let cell = 8.0;
            let vis = Rect::new(dest.x.max(r.x), dest.y.max(r.y), (dest.x + dest.w).min(r.x + r.w) - dest.x.max(r.x), (dest.y + dest.h).min(r.y + r.h) - dest.y.max(r.y));
            if vis.w > 0.0 && vis.h > 0.0 {
                scene.push_clip(vis, 0.0);
                scene.fill(vis, Color::rgb(0xcc, 0xcc, 0xcc));
                let (x0, y0) = (((vis.x - dest.x) / cell).floor() as i32, ((vis.y - dest.y) / cell).floor() as i32);
                let (x1, y1) = (((vis.x + vis.w - dest.x) / cell).ceil() as i32, ((vis.y + vis.h - dest.y) / cell).ceil() as i32);
                if ((x1 - x0) as i64 * (y1 - y0) as i64) < 40_000 {
                    for cy in y0..y1 {
                        for cx in x0..x1 {
                            if (cx + cy) % 2 == 0 {
                                let c = Rect::new(dest.x + cx as f32 * cell, dest.y + cy as f32 * cell, cell, cell);
                                scene.fill(c, Color::rgb(0xee, 0xee, 0xee));
                            }
                        }
                    }
                }
                scene.pop_clip();
            }
            if let Some(level) = pictures::level_for(&levels, dw * scale) {
                let id = scene.image(level.clone());
                scene.draw_image(id, dest, 1.0);
            }
            // The part to keep while cropping: the rest dimmed, an outline.
            if let Some((x0, y0, x1, y1)) = crop {
                let (kx, ky) = (dw / width as f32, dh / height as f32);
                let (ax, bx) = (dest.x + x0.min(x1) * kx, dest.x + x0.max(x1) * kx);
                let (ay, by) = (dest.y + y0.min(y1) * ky, dest.y + y0.max(y1) * ky);
                let dim = Color::rgba(0, 0, 0, 140);
                scene.fill(Rect::new(dest.x, dest.y, dest.w, ay - dest.y), dim);
                scene.fill(Rect::new(dest.x, by, dest.w, dest.y + dest.h - by), dim);
                scene.fill(Rect::new(dest.x, ay, ax - dest.x, by - ay), dim);
                scene.fill(Rect::new(bx, ay, dest.x + dest.w - bx, by - ay), dim);
                scene.stroke(Rect::new(ax, ay, bx - ax, by - ay), 0.0, 1.0, Color::rgb(255, 255, 255));
            }
            scene.pop_clip();
        })
        .on_touch(|t| Some(Msg::Touch(t)))
        .width(Length::FILL)
        .height(Length::FILL)
    }





    /// The toolbar: through the folder, zoom, turn; Open at the end.
    fn toolbar(&self, t: &Tab, th: &Theme) -> Element<Msg> {
        use vergence_widgets::toolbar::{Icon, flexible, group, tool, toggle, toolbar};
        let picture = matches!(t.content, Content::Picture { .. });
        let still = matches!(&t.content, Content::Picture { frames, .. } if frames.len() == 1);
        let on = |c: u32| picture.then_some(Msg::Run(c));
        let many = picture && pictures::siblings(&t.path).len() > 1;
        let step = |c: u32| many.then_some(Msg::Run(c));
        let items = vec![
            group(vec![tool(Icon::Previous, "Previous picture", step(cmd::PREVIOUS)), tool(Icon::Next, "Next picture", step(cmd::NEXT))]),
            group(vec![
                tool(Icon::ZoomOut, "Zoom out", on(cmd::ZOOM_OUT)),
                tool(Icon::ZoomIn, "Zoom in", on(cmd::ZOOM_IN)),
                if picture { toggle(Icon::Fit, "Fit to the window", t.zoom.is_none(), Msg::Run(cmd::FIT)) } else { tool(Icon::Fit, "Fit to the window", None) },
                tool(Icon::Actual, "Actual size", on(cmd::ACTUAL)),
            ]),
            group(vec![
                tool(Icon::TurnLeft, "Turn left", on(cmd::TURN_LEFT)),
                tool(Icon::TurnRight, "Turn right", on(cmd::TURN_RIGHT)),
                if still { toggle(Icon::Crop, "Crop", t.crop.is_some(), Msg::Run(cmd::CROP)) } else { tool(Icon::Crop, "Crop", None) },
            ]),
            flexible(),
            tool(Icon::Open, "Open…", Some(Msg::Run(cmd::OPEN))),
        ];
        toolbar(items, th)
    }

    fn status(&self, t: &Tab, th: &Theme) -> Element<Msg> {
        let mut items: Vec<Element<Msg>> = vec![label(t.name()).size(12.0).width(Length::FILL)];
        match &t.content {
            Content::Picture { width, height, frames } => {
                items.push(label(format!("{width} × {height}")).size(12.0).dim());
                if frames.len() > 1 {
                    items.push(label(format!("Frame {} of {}", t.frame + 1, frames.len())).size(12.0).dim());
                }
                items.push(label(vergence_types::describe(vergence_types::of_file(&t.path))).size(12.0).dim());
                if let Some(z) = self.zoom_now() {
                    items.push(label(format!("{:.0}%{}", z * 100.0, if t.zoom.is_none() { " (fitted)" } else { "" })).size(12.0).dim());
                }
                let list = pictures::siblings(&t.path);
                if list.len() > 1
                    && let Some(i) = list.iter().position(|p| *p == t.path)
                {
                    items.push(label(format!("{} of {}", i + 1, list.len())).size(12.0).dim());
                }
                match t.crop {
                    Some(None) => items.push(label("Drag over the part to keep").size(12.0).dim()),
                    Some(Some(_)) => items.push(button(label("Crop").size(12.0)).look(ButtonLook::Accent).padding(Padding::hv(2.0, 10.0)).on_press(Msg::Crop).name("Crop")),
                    None => {}
                }
                if t.edited {
                    let png = t.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"));
                    let what = if png { "Save" } else { "Save as PNG" };
                    items.push(button(label(what).size(12.0)).padding(Padding::hv(2.0, 10.0)).on_press(Msg::Run(cmd::SAVE)).name(what));
                }
            }
            Content::Text { cut, .. } => {
                if *cut {
                    items.push(label(format!("the first {} MB", MAX_TEXT >> 20)).size(12.0).dim());
                }
                items.push(button(label("Open in Edit").size(12.0)).padding(Padding::hv(2.0, 10.0)).on_press(Msg::OpenInEditor).name("Open in Edit"));
            }
            _ => {}
        }
        let _ = th;
        panel(row(items).spacing(14.0).align(Align::Center).width(Length::FILL)).radius(0.0).padding(Padding { top: 3.0, bottom: 3.0, left: 12.0, right: 12.0 }).width(Length::FILL)
    }
}

impl Program for Viewer {
    type Message = Msg;

    fn title(&self) -> String {
        self.front_tab().map(Tab::name).unwrap_or_else(|| "Close-Up".into())
    }

    fn app_id(&self) -> String {
        APP_ID.into()
    }

    /// Dark under App default: pictures read best on a dark ground.
    fn theme(&self) -> Theme {
        Theme::from_desktop(&self.desktop, true, SECTION)
    }

    fn bar_style(&self) -> Option<BarStyle> {
        Some(vergence_ui::bar_style_setting(&self.desktop, SECTION, BarStyle::Dark))
    }

    /// The toolbar reaches up under the bar: one band (toolbar.rs).
    fn under_bar(&self) -> bool {
        !self.preview
    }

    /// A plain title bar: one file a window.
    fn bar(&self) -> Option<Bar> {
        (!self.preview).then_some(Bar::Plain)
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        let Some(tab) = self.front_tab() else {
            return panel(label("Nothing open: Ctrl+O opens a picture.").size(13.0).dim()).padding(Padding { top: t.top + 24.0, bottom: 24.0, left: 24.0, right: 24.0 }).width(Length::FILL).height(Length::FILL);
        };
        let body: Element<Msg> = match &tab.content {
            Content::Loading => panel(label("Opening…").size(13.0).dim()).padding(24.0).width(Length::FILL).height(Length::FILL),
            Content::Picture { width, height, frames } => self.picture_view(tab, *width, *height, frames),
            Content::Text { doc, .. } => text_area(doc).key(tab.id as u64).name(tab.name()),
            Content::Problem { why, elsewhere } => {
                let mut items: Vec<Element<Msg>> = vec![label(why.clone()).size(13.0).wrap().width(Length::FILL)];
                if *elsewhere {
                    items.push(button(label("Open with its app").size(12.0)).look(ButtonLook::Accent).padding(Padding::hv(4.0, 12.0)).on_press(Msg::OpenWith).name("Open with its app"));
                }
                panel(column(items).spacing(12.0).width(Length::Fixed(420.0))).padding(24.0).width(Length::FILL).height(Length::FILL)
            }
        };
        let mut all = Vec::new();
        if !self.preview {
            all.push(self.toolbar(tab, t));
        }
        all.push(panel(body).padding(Padding { top: if self.preview { t.top } else { 0.0 }, ..Padding::default() }).radius(0.0).width(Length::FILL).height(Length::FILL));
        if !self.preview {
            all.push(rule());
            all.push(self.status(tab, t));
        }
        column(all).width(Length::FILL).height(Length::FILL)
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        match m {
            Msg::Touch(Touch::Press { x, y }) => {
                let now = std::time::Instant::now();
                let double = self.last_press.is_some_and(|t| now.duration_since(t) < std::time::Duration::from_millis(400));
                self.last_press = Some(now);
                let r = self.area.get();
                self.pointer.set((r.x + x, r.y + y));
                if double {
                    // Between fitted and actual size, about the point pressed.
                    let fitted = self.front_tab().is_some_and(|t| t.zoom.is_none());
                    if fitted {
                        self.zoom_to(1.0, Some((r.x + x, r.y + y)));
                    } else {
                        self.fit();
                    }
                    self.last_press = None;
                } else if let Some(at) = self.crop_point(x, y) {
                    if let Some(t) = self.front_mut() {
                        t.crop = Some(Some((at.0, at.1, at.0, at.1)));
                    }
                    self.last_press = None;
                } else if let Some(t) = self.front_tab() {
                    self.drag = Some(((x, y), t.pan));
                }
            }
            Msg::Touch(Touch::Move { x, y, pressed }) => {
                let r = self.area.get();
                self.pointer.set((r.x + x, r.y + y));
                if pressed
                    && let Some(at) = self.crop_point(x, y)
                    && let Some(t) = self.front_mut()
                    && let Some(Some(c)) = t.crop.as_mut()
                {
                    c.2 = at.0;
                    c.3 = at.1;
                } else if pressed && let Some(((sx, sy), start)) = self.drag {
                    let (scale, small) = (self.scale, self.small_actual());
                    if let Some(t) = self.front_mut()
                        && let Content::Picture { width, height, .. } = &t.content
                    {
                        let (_, _, _, dw, dh) = placed(r, scale, (*width, *height), t.zoom, (0.0, 0.0), small);
                        t.pan = clamp_pan(r, dw, dh, (start.0 + x - sx, start.1 + y - sy));
                    }
                }
            }
            Msg::Touch(Touch::Release { .. }) => {
                self.drag = None;
            }
            Msg::Touch(Touch::Leave) => self.drag = None,
            Msg::Run(c) => return self.run(c, window),
            Msg::OpenWith => self.open_with(false),
            Msg::OpenInEditor => self.open_with(true),
            Msg::Crop => self.crop_apply(),
        }
        window.redraw();
    }

    fn input(&mut self, input: &Input, window: &mut Window) -> bool {
        match input {
            // The wheel zooms about the pointer.
            Input::Scroll { x, y, dy, .. } if *dy != 0.0 => {
                let r = self.area.get();
                if *x >= r.x && *x <= r.x + r.w && *y >= r.y && *y <= r.y + r.h
                    && let Some(z) = self.zoom_now()
                {
                    let k = 1.0015f32.powf(-dy.clamp(-200.0, 200.0) * 10.0);
                    self.zoom_to(z * k, Some((*x, *y)));
                    return true;
                }
                false
            }
            Input::Key { sym, pressed: true, modifiers: m, text, .. } => {
                const LEFT: u32 = 0xff51;
                const RIGHT: u32 = 0xff53;
                const HOME: u32 = 0xff50;
                const END: u32 = 0xff57;
                const ESCAPE: u32 = 0xff1b;
                const RETURN: u32 = 0xff0d;
                const KP_ADD: u32 = 0xffab;
                const KP_SUB: u32 = 0xffad;
                let picture = self.front_tab().is_some_and(|t| !matches!(t.content, Content::Text { .. }));
                if m.ctrl {
                    match *sym {
                        0x6f | 0x4f => self.run(cmd::OPEN, window),
                        0x6e | 0x4e => self.run(cmd::NEW_WINDOW, window),
                        0x77 | 0x57 => self.run(cmd::CLOSE_TAB, window),
                        0x2c => self.run(cmd::SETTINGS, window),
                        0x3d | 0x2b | KP_ADD => self.run(cmd::ZOOM_IN, window),
                        0x2d | KP_SUB => self.run(cmd::ZOOM_OUT, window),
                        0x30 => self.run(cmd::FIT, window),
                        0x73 | 0x53 => self.run(cmd::SAVE, window),
                        // Ctrl+K crops, as the Mac's Preview.
                        0x6b | 0x4b => {
                            self.crop_apply();
                            window.redraw();
                        }
                        _ => return false,
                    }
                    return true;
                }
                if self.preview && (*sym == ESCAPE || *sym == 0x20) {
                    window.close();
                    return true;
                }
                if self.preview && *sym == RETURN {
                    // In full: a Close-Up of its own (or the one running) takes it.
                    if let (Some(t), Ok(me)) = (self.front_tab(), std::env::current_exe()) {
                        let _ = std::process::Command::new(me).arg(&t.path).spawn();
                    }
                    window.close();
                    return true;
                }
                if !picture {
                    return false;
                }
                let c = text.as_deref().unwrap_or("");
                // Cropping: Enter keeps the part dragged out, Escape lets go.
                if self.front_tab().is_some_and(|t| t.crop.is_some()) && (*sym == RETURN || *sym == ESCAPE) {
                    if *sym == RETURN {
                        self.crop_apply();
                    } else {
                        self.crop_toggle();
                    }
                    window.redraw();
                    return true;
                }
                match *sym {
                    RIGHT => self.run(cmd::NEXT, window),
                    LEFT => self.run(cmd::PREVIOUS, window),
                    HOME | END => {
                        if let Some(t) = self.front_tab() {
                            let list = pictures::siblings(&t.path);
                            if let Some(i) = list.iter().position(|p| *p == t.path) {
                                let to = if *sym == HOME { 0 } else { list.len() - 1 };
                                self.step(to as isize - i as isize);
                            }
                        }
                        window.redraw();
                    }
                    KP_ADD => self.run(cmd::ZOOM_IN, window),
                    KP_SUB => self.run(cmd::ZOOM_OUT, window),
                    _ => match c {
                        "+" | "=" => self.run(cmd::ZOOM_IN, window),
                        "-" => self.run(cmd::ZOOM_OUT, window),
                        "0" => self.run(cmd::FIT, window),
                        "1" => self.run(cmd::ACTUAL, window),
                        "r" => self.run(cmd::TURN_RIGHT, window),
                        "R" => self.run(cmd::TURN_LEFT, window),
                        "c" => self.run(cmd::CROP, window),
                        _ => return false,
                    },
                }
                true
            }
            _ => false,
        }
    }

    fn waker(&mut self, waker: Waker) {
        // The first window listens for other starts' files.
        let first = self.waker.is_none();
        self.waker = Some(waker.clone());
        if first && !self.preview {
            let inbox = self.shared.borrow().inbox.clone();
            let w = waker.clone();
            let _ = hand::listen(inbox, move || w.wake());
        }
        // More files from the command line wait for windows of their own.
        if !self.shared.borrow().inbox.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
            waker.wake();
        }
        // Files asked for before the window could wake (the command line's).
        let pending: Vec<(u32, PathBuf)> = self.tabs.iter().filter(|t| matches!(t.content, Content::Loading)).map(|t| (t.id, t.path.clone())).collect();
        for (id, path) in pending {
            load(path, id, &self.shared.borrow(), Some(waker.clone()));
        }
    }

    fn woken(&mut self, window: &mut Window) -> bool {
        // The close question answered.
        if let Some(a) = self.asking.as_ref().and_then(|c| c.take()) {
            self.asking = None;
            match a {
                Answer::Save if self.save() => window.close(),
                Answer::Discard => {
                    if let Some(t) = self.front_mut() {
                        t.edited = false;
                    }
                    window.close();
                }
                _ => {}
            }
            return true;
        }
        // Woken by the drawing (its place changed) or a PDF's page drawn: look again.
        let mut changed = self.reread() || self.front_tab().is_some_and(|t| matches!(t.content, Content::Picture { .. }));
        let asked: Vec<Vec<PathBuf>> = std::mem::take(&mut *self.shared.borrow().inbox.lock().unwrap_or_else(|e| e.into_inner()));
        for paths in asked {
            self.take(&paths, false, window);
            // Handed over by a start (or the Open dialog): forward, if the server agrees.
            window.activate();
            changed = true;
        }
        let done: Vec<(u32, Loaded)> = {
            let shared = self.shared.borrow();
            let mut all = shared.loaded.lock().unwrap_or_else(|e| e.into_inner());
            let (mine, rest): (Vec<_>, Vec<_>) = all.drain(..).partition(|(id, _)| self.tabs.iter().any(|t| t.id == *id));
            *all = rest;
            mine
        };
        for (id, loaded) in done {
            if matches!(loaded, Loaded::HandedOver) {
                self.tabs.retain(|t| t.id != id);
                if self.tabs.is_empty() {
                    window.close();
                    return false;
                }
                changed = true;
                continue;
            }
            if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
                t.content = match loaded {
                    Loaded::Picture { width, height, frames } => Content::Picture { width, height, frames },
                    Loaded::Text { text, cut } => {
                        let options = AreaOptions { editable: false, complete: false, spell: false, current_line: false, ..AreaOptions::default() };
                        Content::Text { doc: TextDoc::with_options(&text, options), cut }
                    }
                    Loaded::HandedOver => continue,
                    Loaded::Problem { why, elsewhere } => Content::Problem { why, elsewhere },
                };
                t.frame = 0;
                changed = true;
            }
        }
        if changed {
            self.arm_animation(window);
        }
        changed
    }

    fn timer(&mut self, id: u64, window: &mut Window) -> bool {
        if id != ANIM_TIMER {
            return false;
        }
        self.animating = false;
        if let Some(t) = self.front_mut()
            && let Content::Picture { frames, .. } = &t.content
            && frames.len() > 1
        {
            t.frame = (t.frame + 1) % frames.len();
        }
        self.arm_animation(window);
        true
    }

    /// A file dropped here takes this window's place (more, windows of their own).
    fn drag_over(&mut self, _x: f32, _y: f32, kinds: &[String], _ctrl: bool) -> u8 {
        if !self.preview && kinds.iter().any(|k| k == "text/uri-list") { vergence_ui::DROP_COPY } else { vergence_ui::DROP_NONE }
    }

    fn dropped(&mut self, _x: f32, _y: f32, _action: u8, items: &[(String, Vec<u8>)], window: &mut Window) {
        let paths: Vec<PathBuf> = items.iter().find(|(k, _)| k == "text/uri-list").map(|(_, v)| String::from_utf8_lossy(v).lines().filter_map(|l| vergence_types::path_of_link(l.trim())).collect()).unwrap_or_default();
        self.take(&paths, true, window);
        window.redraw();
    }

    fn scale(&mut self, scale: f32) -> bool {
        self.scale = scale;
        true
    }

    fn commands(&self) -> Option<(bool, Vec<C>)> {
        let picture = self.front_tab().is_some_and(|t| matches!(t.content, Content::Picture { .. }));
        let fitted = self.front_tab().is_some_and(|t| t.zoom.is_none());
        let mut list = vec![
            C::role(cmd::OPEN, Role::Open).featured(),
            C::role(cmd::NEW_WINDOW, Role::NewWindow),
            C::role(cmd::CLOSE_TAB, Role::CloseWindow),
            C::own(cmd::OPEN_WITH, "File", "Open with its app", ""),
        ];
        if picture {
            list.extend([
                C::role(cmd::ZOOM_IN, Role::ZoomIn).featured(),
                C::role(cmd::ZOOM_OUT, Role::ZoomOut).featured(),
                C::own(cmd::FIT, "View", "Fit to the window", "0").checked(fitted),
                C::own(cmd::ACTUAL, "View", "Actual size", "1"),
                C::own(cmd::TURN_RIGHT, "Picture", "Turn right", "R").featured(),
                C::own(cmd::TURN_LEFT, "Picture", "Turn left", "Shift+R"),
                C::own(cmd::CROP, "Picture", "Crop", "C").checked(self.front_tab().is_some_and(|t| t.crop.is_some())),
                C::role(cmd::SAVE, Role::Save),
                C::own(cmd::NEXT, "Go", "Next picture", "Right"),
                C::own(cmd::PREVIOUS, "Go", "Previous picture", "Left"),
            ]);
        }
        list.extend([C::role(cmd::SETTINGS, Role::Settings), C::role(cmd::ABOUT, Role::About)]);
        Some((false, list))
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        self.run(id, window);
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { min: (320.0, 240.0), ..Default::default() }
    }

    /// An edited picture asks before its window closes.
    fn close_asked(&mut self, window: &mut Window) -> bool {
        let Some(t) = self.front_tab().filter(|t| t.edited) else { return true };
        if self.asking.is_none() {
            let answer = Rc::new(Cell::new(None));
            let copy = !t.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"));
            let q = SaveQuestion { name: t.name(), copy, answer: answer.clone(), waker: self.waker.clone(), theme: self.theme() };
            self.asking = Some(answer);
            window.open_window(vergence_widgets::window(q));
        }
        false
    }
}

struct CloseupRows;

impl Rows for CloseupRows {
    fn parts(&self, text: &str) -> Vec<Part> {
        let opacity = vergence_ui::opacity_setting(text, "[appearance]");
        let locked = vergence_ui::desktop_pick(text).map(|light| format!("{} for every app: Settings > Appearance > Style", if light { "Light" } else { "Dark" }));
        let styles = vec![Choice::text("Dark", "dark"), Choice::text("Light", "light")];
        vec![
            Part {
                title: "Close-Up".into(),
                rows: vec![
                    Row::new("ground", "Around a picture", "What shows where the picture does not reach.", Control::Choose { choices: vec![Choice::text("Dark", "dark"), Choice::text("Light", "light"), Choice::text("The window's color", "window")], desktop: None, app_default: 0 }),
                    Row::new("small-actual", "Small pictures at actual size", "Fitting never makes a picture larger than it is.", Control::Switch { default: true }),
                    Row::new("animate", "Play animations", "GIF and WebP pictures that move.", Control::Switch { default: true }),
                ],
            },
            Part {
                title: "On this desktop".into(),
                rows: vec![
                    Row::new("style", "Style", "While the desktop's Style is App default.", Control::Choose { choices: styles.clone(), desktop: Some("Built-in (Dark)".into()), app_default: 0 }).locked(locked.clone()),
                    Row::new("bar-style", "Title bar", "", Control::Choose { choices: styles, desktop: Some("Built-in (Dark)".into()), app_default: 0 }).locked(locked),
                    Row::new("opacity", "Opacity", "30 to 100: how much of the window's ground shows.", Control::Percent { min: 30, desktop: format!("Desktop default ({:.0})", opacity * 100.0), now: (opacity * 100.0).round() as u32 }),
                    Row::new("", "Window places", "Where the desktop puts Close-Up's windows, for each set of screens.", Control::Button { label: "Forget…".into(), id: 1 }),
                ],
            },
        ]
    }

    fn button(&self, id: u32) {
        if id == 1 {
            let _ = std::process::Command::new("vergence-settings").args(["--page", "windows", "--forget", APP_ID]).spawn();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("vergence-closeup {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let preview = args.iter().any(|a| a == "--preview");
    let files: Vec<PathBuf> = args.iter().filter(|a| !a.starts_with("--")).map(|a| vergence_types::path_of_link(a).unwrap_or_else(|| PathBuf::from(a))).map(|p| std::path::absolute(&p).unwrap_or(p)).collect();
    // One Close-Up for all its windows: a running one takes the files.
    if !preview && hand::hand_over(&files) {
        return;
    }
    let shared = Rc::new(RefCell::new(Shared { inbox: Arc::default(), loaded: Arc::default(), next_tab: 0 }));
    let mut viewer = Viewer::new(shared, preview);
    // The first file here, read once the window can be woken (waker()); the
    // others go into windows of their own when it is (woken()).
    let mut files = files.into_iter();
    if let Some(p) = files.next() {
        viewer.queue(p);
    }
    let more: Vec<PathBuf> = files.collect();
    if !more.is_empty() {
        viewer.shared.borrow().inbox.lock().unwrap_or_else(|e| e.into_inner()).push(more);
    }
    let size = if preview { (720, 540) } else { (960, 680) };
    if let Err(e) = vergence_widgets::run(viewer, size) {
        eprintln!("vergence-closeup: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_fitted_never_enlarged_and_kept_in_reach() {
        let r = Rect::new(0.0, 0.0, 800.0, 600.0);
        // 4000 x 3000 at scale 2: fitted to 800 x 600 points, a zoom of 0.4.
        let (z, x, y, w, h) = placed(r, 2.0, (4000, 3000), None, (0.0, 0.0), true);
        assert!((z - 0.4).abs() < 1e-4 && x.abs() < 1e-3 && y.abs() < 1e-3 && (w - 800.0).abs() < 1e-3 && (h - 600.0).abs() < 1e-3);
        // A small picture stays at actual size (one pixel a pixel), centered.
        let (z, x, _, w, _) = placed(r, 2.0, (200, 100), None, (0.0, 0.0), true);
        assert_eq!((z, w), (1.0, 100.0));
        assert_eq!(x, 350.0);
        let (z, ..) = placed(r, 2.0, (200, 100), None, (0.0, 0.0), false);
        assert_eq!(z, 8.0, "unless asked to fill the window");
        // Panning: none on an axis it fits; no further than its edges.
        assert_eq!(clamp_pan(r, 400.0, 300.0, (50.0, -50.0)), (0.0, 0.0));
        assert_eq!(clamp_pan(r, 1000.0, 900.0, (500.0, -10.0)), (100.0, -10.0));
    }
}
