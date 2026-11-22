//! Space's preview (preview milestone 1, docs/preview-milestone-1.md): a
//! window of its own over FileView's, shaped for what it shows, as macOS's
//! Quick Look (Dave, 2026-11-21, with screenshots of it). A picture fills a
//! window of its proportions, a PDF's pages scroll beside their thumbnails,
//! text is shown whole in the fixed-width font, and anything else as a large
//! icon and its facts. Its title is the file's name; its bar's one button
//! opens the file with the app chosen for its type.
//!
//! What decodes the data (whitepaper §6.10): never this process. Pictures
//! are decoded by `vergence-decode`'s confined helper and PDFs drawn by
//! `vergence-pdf`'s; text is only read and checked for UTF-8.
//!
//! The window and FileView's share a [`Link`]: FileView puts the file its
//! cursor is on there, and the preview passes back the keys that move the
//! cursor or open the file (the arrows and Enter go to FileView even while
//! the preview has the keyboard, as in Finder). Space or Escape closes it.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use vergence_scene::{Image, Rect};
use vergence_ui::keyboard::sym;
use vergence_widgets::files::{self, Entry, Kind};
use vergence_widgets::textarea::{AreaOptions, TextDoc, text_area};
use vergence_widgets::{Element, Input, Length, Padding, Program, Theme, Waker, Window, button, column, drawing, label, panel, row, scroll, space};

/// What the preview and the window it came from share.
pub struct Link {
    /// The file to show (FileView's cursor), and a count that changes with it.
    pub entry: Option<Entry>,
    pub serial: u64,
    /// Keys pressed in the preview for FileView: the arrows and Enter.
    pub keys: Vec<u32>,
    /// The preview is open; FileView asks it to close with `close`.
    pub open: bool,
    pub close: bool,
}

pub type LinkRef = Rc<RefCell<Link>>;

pub fn link(entry: Entry) -> LinkRef {
    Rc::new(RefCell::new(Link { entry: Some(entry), serial: 1, keys: Vec::new(), open: true, close: false }))
}

/// The most of a text file shown, and of a PDF's pages drawn.
const TEXT_MAX: u64 = 1 << 20;
const PAGES_MAX: usize = 24;
/// A PDF page's width when drawn, in pixels, and the thumbnail strip's width.
const PAGE_PX: f32 = 900.0;
const STRIP_W: f32 = 112.0;
const PAD: f32 = 12.0;
/// The frost shown around the content card, below the bar too, and the
/// card's corners (preview milestone 1, step 6b: Quick Look's look; Dave:
/// "let's make the preview windows look similar to the mac. We have frost in
/// our toolkit"). The window has no ground of its own: the server frosts
/// what is behind it, and the content stands on that as an inset card.
const MARGIN: f32 = 8.0;
const CARD_R: f32 = 8.0;

/// What the preview shows of the file.
enum Shows {
    Facts,
    Picture,
    Pages,
    Text(TextDoc),
    /// A sound file, through Hymie's helper (listen.rs).
    Sound(crate::listen::Listen),
}

/// The waveform's slices, and its height.
const SLICES: u32 = 1000;
const WAVE_H: f32 = 150.0;

/// A picture decoded apart: for which file, and the picture (None: it would not decode).
type Decoded = Arc<Mutex<Option<(PathBuf, Option<Arc<Image>>)>>>;

/// A PDF's pages as the helper draws them: for which file, each page's size
/// in points, the pages drawn so far, and how many the document has.
#[derive(Default)]
struct Pages {
    /// Which drawing this is: a drawing started again (another file, or the
    /// same once the waker came) stops the one before.
    run: u64,
    sizes: Vec<(f32, f32)>,
    drawn: Vec<Arc<Image>>,
    count: usize,
}

pub struct Peek {
    link: LinkRef,
    seen: u64,
    entry: Option<Entry>,
    shows: Shows,
    picture: Decoded,
    pages: Arc<Mutex<Pages>>,
    /// The app chosen for the file's type: its desktop id and name.
    app: Option<(String, String)>,
    waker: Option<Waker>,
    theme: Theme,
    /// The window's size, and the last size asked for.
    size: (f32, f32),
    asked: Option<(u32, u32)>,
    /// The size the server gave for the last ask (None: an ask is under
    /// way); and whether the person has resized the window since, which
    /// keeps their size for the files that follow (Dave, 2026-11-21: "macOS
    /// allows these preview windows to be resized").
    given: Option<(f32, f32)>,
    by_hand: bool,
    /// A thumbnail clicked: the page scroll's place to go to.
    jump: Option<f32>,
    /// What had come from the workers when last drawn: whether the picture
    /// had, how many pages, and the sound's state (a wake with nothing new
    /// draws nothing).
    had: (bool, usize, (bool, usize, bool, u64, bool)),
}

#[derive(Clone, Debug)]
pub enum Msg {
    Page(usize),
    /// Play or pause the sound; the waveform clicked at this x.
    PlayPause,
    Seek(f32),
}

impl Peek {
    pub fn new(link: LinkRef, theme: Theme) -> Peek {
        let mut p = Peek {
            link,
            seen: 0,
            entry: None,
            shows: Shows::Facts,
            picture: Arc::default(),
            pages: Arc::default(),
            app: None,
            waker: None,
            theme,
            size: (0.0, 0.0),
            asked: None,
            given: None,
            by_hand: false,
            jump: None,
            had: (false, 0, (false, 0, false, 0, false)),
        };
        p.follow();
        p
    }

    /// Takes the link's file if it changed: what to show, and its decoding started.
    fn follow(&mut self) -> bool {
        let (serial, entry) = {
            let l = self.link.borrow();
            (l.serial, l.entry.clone())
        };
        if serial == self.seen {
            return false;
        }
        self.seen = serial;
        self.entry = entry;
        self.jump = None;
        self.app = self.entry.as_ref().filter(|e| !e.dir).and_then(|e| app_for(&e.path));
        self.shows = match &self.entry {
            None => Shows::Facts,
            Some(e) if e.dir => Shows::Facts,
            Some(e) if e.mime == Some("application/pdf") => {
                self.draw_pages(e.path.clone());
                Shows::Pages
            }
            Some(e) if vergence_thumbs::has_thumbnail(e.mime) => {
                self.decode(e.path.clone());
                Shows::Picture
            }
            Some(e) if e.kind() == Kind::Sound => self.listen(e.path.clone()),
            Some(e) if e.kind() == Kind::Text => match read_text(&e.path) {
                Some(text) => {
                    let options = AreaOptions { editable: false, wrap: true, line_numbers: false, current_line: false, complete: false, spell: false, ..AreaOptions::default() };
                    Shows::Text(TextDoc::with_options(&text, options))
                }
                None => Shows::Facts,
            },
            Some(_) => Shows::Facts,
        };
        true
    }

    /// A sound file's worker, playing at once (as Quick Look does; Dave,
    /// 2026-11-21: "Ideally it would start playing when you open it") unless
    /// `[files] preview-autoplay = false`. Not before the waker has come:
    /// the worker started without it is replaced at once, and would click.
    fn listen(&self, path: PathBuf) -> Shows {
        let l = crate::listen::Listen::start(path, SLICES, self.waker.clone());
        let off = vergence_ui::table_value(&crate::desktop_text().0, crate::SECTION, "preview-autoplay").as_deref() == Some("false");
        if self.waker.is_some() && !off {
            l.ask(crate::listen::Ask::Play);
        }
        Shows::Sound(l)
    }

    fn decode(&self, path: PathBuf) {
        let (out, waker) = (self.picture.clone(), self.waker.clone());
        if out.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|(p, _)| *p == path) {
            return;
        }
        std::thread::spawn(move || {
            let img = vergence_decode::decode_file(&path, Some((2400, 1800))).ok().and_then(|p| {
                let f = p.frames.into_iter().next()?;
                Some(Arc::new(Image { width: p.width, height: p.height, rgba: f.rgba }))
            });
            *out.lock().unwrap_or_else(|e| e.into_inner()) = Some((path, img));
            if let Some(w) = waker {
                w.wake();
            }
        });
    }

    /// The PDF's pages drawn by the helper, one after another, each told as it comes.
    fn draw_pages(&self, path: PathBuf) {
        let (out, waker) = (self.pages.clone(), self.waker.clone());
        let run = {
            let mut p = out.lock().unwrap_or_else(|e| e.into_inner());
            let run = p.run + 1;
            *p = Pages { run, ..Pages::default() };
            run
        };
        std::thread::spawn(move || {
            let Ok(mut v) = vergence_pdf::helper::Viewer::open(&path) else { return };
            if v.info.needs_password {
                return;
            }
            let sizes = v.info.pages.clone();
            {
                let mut p = out.lock().unwrap_or_else(|e| e.into_inner());
                if p.run != run {
                    return;
                }
                p.count = sizes.len();
                p.sizes = sizes.iter().take(PAGES_MAX).copied().collect();
            }
            for (i, (w, _)) in sizes.iter().take(PAGES_MAX).enumerate() {
                let Ok(t) = v.render(i, (PAGE_PX / w.max(1.0)).clamp(0.05, 8.0), None) else { break };
                let mut p = out.lock().unwrap_or_else(|e| e.into_inner());
                // Started again meanwhile (another file): stop.
                if p.run != run {
                    return;
                }
                p.drawn.push(Arc::new(Image { width: t.width, height: t.height, rgba: t.rgba }));
                drop(p);
                if let Some(w) = &waker {
                    w.wake();
                }
            }
        });
    }

    fn picture(&self) -> Option<Arc<Image>> {
        let path = &self.entry.as_ref()?.path;
        self.picture.lock().unwrap_or_else(|e| e.into_inner()).as_ref().filter(|(p, _)| p == path).and_then(|(_, i)| i.clone())
    }

    /// The size the window wants for what it shows, in points (the server
    /// keeps it within 80% of the screen, both sides alike).
    fn wanted(&self) -> (u32, u32) {
        let (w, h) = match &self.shows {
            Shows::Facts => (440.0, 300.0),
            Shows::Picture => match self.picture() {
                Some(img) => {
                    let (w, h) = (img.width as f32, img.height as f32);
                    // Small pictures shown at least this big; large ones as they are, the server shrinks them.
                    let up = (360.0 / w.max(h)).max(1.0);
                    (w * up, h * up)
                }
                None => (640.0, 480.0),
            },
            Shows::Pages => {
                let first = self.pages.lock().unwrap_or_else(|e| e.into_inner()).sizes.first().copied().unwrap_or((612.0, 792.0));
                let page_w = 720.0;
                (page_w + STRIP_W + 3.0 * PAD, page_w * first.1 / first.0.max(1.0) + 2.0 * PAD)
            }
            Shows::Sound(_) => (720.0, 2.0 * PAD + 160.0 + 12.0 + WAVE_H + 12.0 + 46.0),
            Shows::Text(doc) => {
                let lines = doc.borrow().buffer().line_count() as f32;
                let line_h = self.theme.mono_size * 1.45;
                (self.theme.mono_size * 0.6 * 100.0 + 2.0 * PAD + 16.0, (lines * line_h + 2.0 * PAD).clamp(200.0, 1100.0))
            }
        };
        // The frost's margin around the card.
        ((w + 2.0 * MARGIN).round() as u32, (h + 2.0 * MARGIN).round() as u32)
    }

    /// The card's width: the window's less the frost's margins.
    fn card_w(&self) -> f32 {
        self.size.0 - 2.0 * MARGIN
    }

    fn pages_view(&self) -> Element<Msg> {
        let p = self.pages.lock().unwrap_or_else(|e| e.into_inner());
        if p.drawn.is_empty() {
            return panel(label("…").size(14.0).dim()).width(Length::FILL).height(Length::FILL);
        }
        let page_w = (self.card_w() - STRIP_W - 3.0 * PAD).max(100.0);
        let mut pages: Vec<Element<Msg>> = Vec::new();
        let mut thumbs: Vec<Element<Msg>> = Vec::new();
        for (i, img) in p.drawn.iter().enumerate() {
            let (w, h) = p.sizes.get(i).copied().unwrap_or((img.width as f32, img.height as f32));
            pages.push(page(img.clone(), page_w, page_w * h / w.max(1.0)));
            let tw = STRIP_W - 16.0;
            thumbs.push(button(page(img.clone(), tw, tw * h / w.max(1.0))).on_press(Msg::Page(i)).name(format!("Page {}", i + 1)));
        }
        if p.count > p.drawn.len() && p.drawn.len() == PAGES_MAX {
            let more = p.count - PAGES_MAX;
            pages.push(label(format!("{more} more page{} in the document", if more == 1 { "" } else { "s" })).size(12.0).dim());
        }
        row(vec![
            scroll(column(thumbs).spacing(8.0).width(Length::FILL)).width(Length::Fixed(STRIP_W)),
            scroll(column(pages).spacing(PAD).width(Length::FILL)).name("Pages"),
        ])
        .spacing(PAD)
        .width(Length::FILL)
        .height(Length::FILL)
    }

    /// A sound: its cover and words, the waveform (played in the accent
    /// color, a click plays from there), Play or Pause and the time.
    fn sound_view(&self, l: &crate::listen::Listen, theme: &Theme) -> Element<Msg> {
        let s = l.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(e) = &self.entry else { return space() };
        let cover: Element<Msg> = match &s.cover {
            Some(img) => fitted(img.clone()).width(Length::Fixed(160.0)).height(Length::Fixed(160.0)),
            None => panel(files::entry_icon_sized(e, 8.0).width(Length::Fixed(150.0)).height(Length::Fixed(150.0))).width(Length::Fixed(160.0)).height(Length::Fixed(160.0)),
        };
        let mut words = vec![label(s.info.as_ref().and_then(|i| i.title.clone()).unwrap_or_else(|| e.name.clone())).size(17.0).weight(600).wrap().width(Length::FILL)];
        if let Some(i) = &s.info {
            for w in [&i.artist, &i.album].into_iter().flatten() {
                words.push(label(w.clone()).size(13.0).wrap().width(Length::FILL));
            }
            let ch = match i.channels {
                1 => "mono".to_string(),
                2 => "stereo".to_string(),
                n => format!("{n} channels"),
            };
            let bits = if i.bits > 0 { format!(" · {}-bit", i.bits) } else { String::new() };
            words.push(label(format!("{} · {} kHz{bits} · {ch}", clock(i.seconds()), i.rate as f32 / 1000.0)).size(12.0).dim());
        }
        if let Some(why) = &s.error {
            words.push(label(why.clone()).size(12.0).dim().wrap().width(Length::FILL));
        }
        let (frames, at) = (s.info.as_ref().map_or(0, |i| i.frames), s.at);
        let played = if frames > 0 { (at as f64 / frames as f64) as f32 } else { 0.0 };
        let peaks = s.peaks.clone();
        let stereo = s.info.as_ref().is_some_and(|i| i.channels >= 2);
        let (on, off, edge) = (theme.accent, theme.dim, theme.line);
        let wave = drawing(move |scene, _, r: Rect, _t: &Theme| {
            let lanes = if stereo { 2 } else { 1 };
            let lane_h = r.h / lanes as f32;
            for lane in 0..lanes {
                let mid = r.y + lane_h * (lane as f32 + 0.5);
                scene.fill(Rect::new(r.x, mid, r.w, 1.0), edge);
                for (i, p) in peaks.iter().enumerate() {
                    let x = r.x + r.w * i as f32 / SLICES as f32;
                    let w = (r.w / SLICES as f32).max(1.0);
                    let (lo, hi) = p[lane];
                    let (top, bottom) = (mid - hi.clamp(-1.0, 1.0) * lane_h * 0.48, mid - lo.clamp(-1.0, 1.0) * lane_h * 0.48);
                    let color = if (i as f32 / SLICES as f32) < played { on } else { off };
                    scene.fill(Rect::new(x, top, w, (bottom - top).max(1.0)), color);
                }
            }
            scene.fill(Rect::new(r.x + r.w * played - 1.0, r.y, 2.0, r.h), on);
        })
        .on_touch(|t| match t {
            vergence_widgets::Touch::Press { x, .. } => Some(Msg::Seek(x)),
            _ => None,
        })
        .width(Length::FILL)
        .height(Length::Fixed(WAVE_H))
        .name("Waveform");
        let seconds = s.info.as_ref().map_or(0.0, |i| i.seconds());
        let now = s.info.as_ref().map_or(0.0, |i| at as f64 / i.rate.max(1) as f64);
        // Play at the bottom, centered and large (Dave, 2026-11-21), the time
        // played on its left and the length on its right.
        let play = button(label(if s.playing { "Pause" } else { "Play" }).size(16.0).weight(600))
            .look(vergence_widgets::ButtonLook::Accent)
            .padding(vergence_widgets::Padding::hv(10.0, 36.0))
            .on_press(Msg::PlayPause)
            .name(if s.playing { "Pause" } else { "Play" });
        column(vec![
            row(vec![cover, column(words).spacing(4.0).width(Length::FILL)]).spacing(PAD).width(Length::FILL),
            wave,
            row(vec![
                label(clock(now)).size(13.0).dim().width(Length::FILL),
                play,
                label(clock(seconds)).size(13.0).dim().align(vergence_widgets::Align::End).width(Length::FILL),
            ])
            .spacing(PAD)
            .align(vergence_widgets::Align::Center)
            .width(Length::FILL),
        ])
        .spacing(12.0)
        .width(Length::FILL)
        .height(Length::FILL)
    }

    fn facts_view(&self) -> Element<Msg> {
        let Some(e) = &self.entry else { return space() };
        let kind = if e.dir { "Folder".to_string() } else { vergence_types::describe(e.mime.unwrap_or(vergence_types::of_file(&e.path))) };
        let mut lines = vec![label(e.name.clone()).size(15.0).weight(600).wrap().width(Length::FILL), label(kind).size(12.0).dim()];
        if e.dir {
            let n = std::fs::read_dir(&e.path).map(|r| r.count()).unwrap_or(0);
            lines.push(label(format!("{n} item{}", if n == 1 { "" } else { "s" })).size(12.0).dim());
        } else {
            lines.push(label(files::size_words(e.size)).size(12.0).dim());
        }
        if let Some(c) = e.changed {
            lines.push(label(format!("Changed {}", files::when(c))).size(12.0).dim());
        }
        row(vec![
            panel(files::entry_icon_sized(e, 8.0).width(Length::Fixed(150.0)).height(Length::Fixed(150.0))).width(Length::Fixed(170.0)),
            column(lines).spacing(4.0).width(Length::FILL),
        ])
        .spacing(PAD)
        .width(Length::FILL)
        .height(Length::FILL)
    }
}

/// A page (or a picture) drawn at a fixed size.
fn page<M: 'static>(img: Arc<Image>, w: f32, h: f32) -> Element<M> {
    drawing(move |scene, _, r: Rect, _t: &Theme| {
        let id = scene.image(img.clone());
        scene.draw_image(id, Rect::new(r.x, r.y, r.w, r.h), 1.0);
    })
    .width(Length::Fixed(w))
    .height(Length::Fixed(h))
}

/// A picture fitted in its place, at its proportions.
fn fitted<M: 'static>(img: Arc<Image>) -> Element<M> {
    drawing(move |scene, _, r: Rect, _t: &Theme| {
        let s = (r.w / img.width as f32).min(r.h / img.height as f32);
        let (w, h) = (img.width as f32 * s, img.height as f32 * s);
        let id = scene.image(img.clone());
        scene.draw_image(id, Rect::new(r.x + (r.w - w) / 2.0, r.y + (r.h - h) / 2.0, w, h), 1.0);
    })
    .width(Length::FILL)
    .height(Length::FILL)
}

/// A picture fitted in its place, at its proportions, its corners rounded
/// to `radius` (the card's: preview milestone 1, step 6b).
fn rounded<M: 'static>(img: Arc<Image>, radius: f32) -> Element<M> {
    drawing(move |scene, _, r: Rect, _t: &Theme| {
        let s = (r.w / img.width as f32).min(r.h / img.height as f32);
        let (w, h) = (img.width as f32 * s, img.height as f32 * s);
        let at = Rect::new(r.x + (r.w - w) / 2.0, r.y + (r.h - h) / 2.0, w, h);
        let id = scene.image(img.clone());
        scene.push_clip(at, radius);
        scene.draw_image(id, at, 1.0);
        scene.pop_clip();
    })
    .width(Length::FILL)
    .height(Length::FILL)
}

/// A text file's text, the first [`TEXT_MAX`] bytes, if it is text: no NUL,
/// UTF-8 (cut at a character's edge where the read ended).
fn read_text(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut buf = Vec::new();
    std::fs::File::open(path).ok()?.take(TEXT_MAX).read_to_end(&mut buf).ok()?;
    if buf.contains(&0) {
        return None;
    }
    match std::str::from_utf8(&buf) {
        Ok(s) => Some(s.to_string()),
        Err(e) if e.valid_up_to() + 4 >= buf.len() => Some(String::from_utf8_lossy(&buf[..e.valid_up_to()]).into_owned()),
        Err(_) => None,
    }
}

/// The app chosen for a file's type: its desktop id and name.
fn app_for(path: &Path) -> Option<(String, String)> {
    let mime = vergence_types::of_file(path);
    let entries = vergence_shell::apps::installed();
    let id = vergence_shell::open::apps_for(mime, &entries, &vergence_types::lists()).into_iter().next()?;
    let name = entries.iter().find(|e| vergence_shell::open::desktop_id(&e.id) == id).map(|e| e.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| id.trim_end_matches(".desktop").to_string());
    Some((id, name))
}

/// The bar button's id.
const OPEN_WITH: u32 = 1;

impl Program for Peek {
    type Message = Msg;

    fn title(&self) -> String {
        self.entry.as_ref().map(|e| e.name.clone()).unwrap_or_default()
    }

    fn app_id(&self) -> String {
        crate::APP_ID.into()
    }

    fn theme(&self) -> Theme {
        self.theme.clone()
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { kind: vergence_ui::WinKind::Dialog, modal: false, min: (240.0, 160.0), max: (0.0, 0.0), alone: false, frosted: true }
    }

    /// Its own place in window memory, apart from FileView's windows.
    fn role(&self) -> String {
        "preview".into()
    }

    fn opening_size(&self) -> Option<(u32, u32)> {
        Some(self.wanted())
    }

    fn resized(&mut self, width: f32, height: f32) {
        self.size = (width, height);
        match self.given {
            // The answer to the window's own ask (or its opening size).
            None => self.given = Some((width, height)),
            // Another size with no ask under way: pulled by its edges.
            Some(g) if (g.0 - width).abs() > 1.0 || (g.1 - height).abs() > 1.0 => self.by_hand = true,
            Some(_) => {}
        }
    }

    fn bar(&self) -> Option<vergence_ui::Bar> {
        let buttons = self.app.iter().map(|(_, name)| vergence_ui::BarButton { id: OPEN_WITH, icon: vergence_ui::BarIcon::Label, name: format!("Open with {name}"), enabled: true }).collect();
        Some(vergence_ui::Bar::Tools { buttons, from: 0.0 })
    }

    fn bar_button(&mut self, id: u32, _window: &mut Window) {
        if id == OPEN_WITH
            && let (Some(e), Some((app, _))) = (&self.entry, &self.app)
        {
            let _ = crate::open_files_with(std::slice::from_ref(&e.path), Some(app));
        }
    }

    fn waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
        // The first file's decoding started before the waker came: again, to be woken.
        if let Some(e) = self.entry.clone() {
            match self.shows {
                Shows::Picture => {
                    *self.picture.lock().unwrap_or_else(|x| x.into_inner()) = None;
                    self.decode(e.path);
                }
                Shows::Pages => self.draw_pages(e.path),
                Shows::Sound(_) => self.shows = self.listen(e.path),
                _ => {}
            }
        }
    }

    fn key_first(&mut self, input: &Input, window: &mut Window) -> Option<bool> {
        let Input::Key { sym: key, pressed: true, modifiers, .. } = input else { return None };
        if modifiers.ctrl || modifiers.alt {
            return None;
        }
        match *key {
            sym::ESCAPE | 0x20 => {
                window.close();
                Some(false)
            }
            sym::UP | sym::DOWN | sym::LEFT | sym::RIGHT | sym::RETURN => {
                self.link.borrow_mut().keys.push(*key);
                if let Some(w) = &self.waker {
                    w.wake();
                }
                Some(false)
            }
            _ => None,
        }
    }

    fn woken(&mut self, window: &mut Window) -> bool {
        if self.link.borrow().close {
            window.close();
            return false;
        }
        let sound = match &self.shows {
            Shows::Sound(l) => {
                let s = l.state.lock().unwrap_or_else(|e| e.into_inner());
                let rate = s.info.as_ref().map_or(1, |i| i.rate.max(1)) as u64;
                (s.info.is_some(), s.peaks.len(), s.playing, s.at * 10 / rate, s.error.is_some())
            }
            _ => (false, 0, false, 0, false),
        };
        let now = (self.picture().is_some(), self.pages.lock().unwrap_or_else(|e| e.into_inner()).drawn.len(), sound);
        let changed = self.follow() | (now != self.had);
        self.had = now;
        let want = self.wanted();
        if !self.by_hand && self.asked != Some(want) {
            self.asked = Some(want);
            self.given = None;
            window.request_size(want.0, want.1);
        }
        changed
    }

    fn closed(&mut self) {
        self.link.borrow_mut().open = false;
        if let Some(w) = &self.waker {
            w.wake();
        }
    }

    fn scroll_to(&mut self) -> Vec<(String, f32)> {
        self.jump.take().map(|y| vec![("Pages".to_string(), y)]).unwrap_or_default()
    }

    fn update(&mut self, message: Msg, _window: &mut Window) {
        let i = match message {
            Msg::Page(i) => i,
            Msg::PlayPause => {
                if let Shows::Sound(l) = &self.shows {
                    let playing = l.state.lock().unwrap_or_else(|e| e.into_inner()).playing;
                    l.ask(if playing { crate::listen::Ask::Pause } else { crate::listen::Ask::Play });
                }
                return;
            }
            Msg::Seek(x) => {
                if let Shows::Sound(l) = &self.shows {
                    let frames = l.state.lock().unwrap_or_else(|e| e.into_inner()).info.as_ref().map_or(0, |i| i.frames);
                    let w = (self.card_w() - 2.0 * PAD).max(1.0);
                    l.ask(crate::listen::Ask::Seek(((x / w).clamp(0.0, 1.0) as f64 * frames as f64) as u64));
                    l.ask(crate::listen::Ask::Play);
                }
                return;
            }
        };
        let p = self.pages.lock().unwrap_or_else(|e| e.into_inner());
        let page_w = (self.card_w() - STRIP_W - 3.0 * PAD).max(100.0);
        let y: f32 = p.sizes.iter().take(i).map(|(w, h)| page_w * h / w.max(1.0) + PAD).sum();
        drop(p);
        self.jump = Some(y);
    }

    fn view(&self, theme: &Theme) -> Element<Msg> {
        // The card: the content on a ground of its own with rounded corners,
        // inset in the frost (no ground around it: the window's is clear).
        let card = match &self.shows {
            Shows::Facts => panel(self.facts_view()).color(theme.card).padding(PAD),
            // A picture is the card itself, its own corners rounded; the frost
            // shows where the window is not of its proportions.
            Shows::Picture => match self.picture() {
                Some(img) => panel(rounded(img, CARD_R)),
                None => panel(label("…").size(14.0).dim()).color(theme.page).padding(PAD),
            },
            Shows::Pages => panel(self.pages_view()).color(theme.card).padding(PAD),
            // The text area has no ground of its own: the page color under it.
            Shows::Text(doc) => panel(text_area(doc)).color(theme.page).padding(Padding::all(CARD_R / 2.0)),
            Shows::Sound(l) => panel(self.sound_view(l, theme)).color(theme.card).padding(PAD),
        };
        let card = card.radius(CARD_R).width(Length::FILL).height(Length::FILL);
        panel(card).radius(0.0).padding(MARGIN).width(Length::FILL).height(Length::FILL)
    }
}

/// Seconds as m:ss (h:mm:ss past an hour).
fn clock(seconds: f64) -> String {
    let t = seconds.max(0.0) as u64;
    if t >= 3600 { format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60) } else { format!("{}:{:02}", t / 60, t % 60) }
}
