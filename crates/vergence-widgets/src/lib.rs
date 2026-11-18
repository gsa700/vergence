//! Widgets (whitepaper §6.10; toolkit milestone 2).
//!
//! A program says what its window holds as a tree of [`Element`]s built
//! from its state ([`Program::view`]), and what to do with the messages its
//! widgets send ([`Program::update`]). The tree is built again only when
//! something changed, laid out at the window's size, drawn into the scene
//! `vergence-ui` shows, and kept until the next build: input is judged
//! against the tree that was on the screen when it happened.
//!
//! What a widget remembers between builds (the pointer over it, a press on
//! it, the focus) is kept under its **id**: its place in the tree, or the
//! [key](Element::key) the program gave it.
//!
//! **Meaning is built in** (Contract 3): as the tree is laid out every
//! widget has a role, a name, a value, its states and its bounds
//! ([`Ui::meaning`]). `VERGENCE_UI_TREE=1` in the environment prints it
//! each time the tree is laid out; `VERGENCE_UI_FRAMES=1` prints how long
//! each frame took to draw, and with what.
//!
//! Timers: a program's own ids for [`Window::wake_at`] are below 2^63; the
//! toolkit keeps those above for its widgets.

mod element;
pub mod about;
pub mod chart;
pub mod edit;
pub mod filedialog;
pub mod files;
pub mod prefs;
pub mod textarea;
pub mod toolbar;
mod layout;
mod theme;
mod ui;

use std::io;
use std::time::Instant;

pub use element::{Align, ButtonLook, Element, Fit, Length, Padding, Touch, Weight, button, choice, column, drawing, field, label, panel, picture, row, rule, scroll, slider, space, stack, switch, vrule};
pub use layout::{Id, Line, Measure};
pub use textarea::{AreaOptions, TextDoc, WeakDoc, Words, text_area};
pub use theme::{Theme, parse_hex, tint_amount};
pub use ui::{Meaning, Response, Ui};
pub use vergence_ui::{Frame, Input, Waker, Window};

use vergence_scene::{Color, Scene};
use vergence_text::{FaceId, Fonts};

/// A program with one window of widgets.
pub trait Program {
    type Message: Clone;

    fn title(&self) -> String;

    /// The program's name to the desktop (window memory, the shelf).
    fn app_id(&self) -> String;

    /// The look to draw with. Asked each time the tree is built.
    fn theme(&self) -> Theme {
        Theme::default()
    }

    /// What the window holds, from the program's state.
    fn view(&self, theme: &Theme) -> Element<Self::Message>;

    /// A widget sent a message. The tree is built again afterwards.
    fn update(&mut self, message: Self::Message, window: &mut Window);

    /// Input no widget used (a key with no field focused, a press on the
    /// ground). True if the state changed.
    fn input(&mut self, _input: &Input, _window: &mut Window) -> bool {
        false
    }

    /// Every input, seen before the widgets get it, for what a widget's
    /// message does not carry (the Open dialog's Ctrl+click: the modifiers
    /// of the press that clicked a row). Nothing is taken.
    fn saw(&mut self, _input: &Input) {}

    /// A key before any widget sees it (a calculator's keys, which must not
    /// press the button last clicked): Some(changed) if the program took it.
    fn key_first(&mut self, _input: &Input, _window: &mut Window) -> Option<bool> {
        None
    }

    /// Called once, before the window opens, with the means for other
    /// threads to wake it.
    fn waker(&mut self, _waker: Waker) {}

    /// The window's kind and size limits (window styles milestone 1); a
    /// normal window with none by default.
    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style::default()
    }

    /// True if the window draws under the server's frosted bar: the strip's
    /// height is `Theme::top` when the view is built.
    fn under_bar(&self) -> bool {
        false
    }

    /// The bar the server draws: its own buttons and where the title starts
    /// (`Bar::Tools`), say. None: a plain bar with the title.
    fn bar(&self) -> Option<vergence_ui::Bar> {
        None
    }

    /// One of its own bar buttons was pressed (the id it gave it).
    fn bar_button(&mut self, _id: u32, _window: &mut Window) {}

    /// Something was done with a tab the server drew ([`Program::bar`]
    /// gave `Bar::Tabs`): chosen, closed, moved, torn off, joined.
    fn tab(&mut self, _event: vergence_ui::TabEvent, _window: &mut Window) {}

    /// The user asked to close the window. True closes it; false keeps it
    /// (the program may ask first and close it later with `Window::close`).
    fn close_asked(&mut self, _window: &mut Window) -> bool {
        true
    }

    /// The window has closed while the program runs on.
    fn closed(&mut self) {}

    /// The toolkit's number for this window, as tab events name another
    /// window of the program (`TabEvent::Join`'s `into`).
    fn opened(&mut self, _id: u32) {}

    /// The persistence token this window had on an earlier run, so window
    /// memory puts it back where it was; "new" asks for one; empty: the
    /// program keeps none.
    fn token(&self) -> String {
        String::new()
    }

    /// The server's token for this window, once it exists.
    fn token_given(&mut self, _token: &str) {}

    /// The size to open at, in points, if the program has one in mind (a
    /// settings window's last size). None: the toolkit's choice.
    fn opening_size(&self) -> Option<(u32, u32)> {
        None
    }

    /// The window is now this size, in points (it opened, or was resized).
    fn resized(&mut self, _width: f32, _height: f32) {}

    /// Its app menu's commands (app settings milestone 1): whether it is a
    /// terminal, and the commands. Asked after each frame; sent when changed.
    fn commands(&self) -> Option<(bool, Vec<vergence_ui::AppCommand>)> {
        None
    }

    /// A command of its app menu was chosen (its id).
    fn command(&mut self, _id: u32, _window: &mut Window) {}

    /// Drag and drop (files milestone 1, step 8; vergence_ui::App's):
    /// what a drop at (x, y) would do, the drag leaving, the drop with its
    /// data, and the end of a drag this window started.
    fn drag_over(&mut self, _x: f32, _y: f32, _kinds: &[String], _ctrl: bool) -> u8 {
        vergence_ui::DROP_NONE
    }
    fn drag_left(&mut self) {}
    fn dropped(&mut self, _x: f32, _y: f32, _action: u8, _items: &[(String, Vec<u8>)], _window: &mut Window) {}
    fn drag_ended(&mut self, _action: u8, _window: &mut Window) {}

    /// The style its bar is to be drawn in (app settings milestone 1).
    /// Asked after each frame; sent when changed.
    fn bar_style(&self) -> Option<vergence_ui::BarStyle> {
        None
    }

    /// The window's height follows its content when it opens: the toolkit
    /// measures what its outermost scrolling place holds and asks for that
    /// (the server keeps it on the screen). False by default.
    fn fit_content(&self) -> bool {
        false
    }

    /// Another thread used the [`Waker`]. True if the state changed.
    fn woken(&mut self, _window: &mut Window) -> bool {
        false
    }

    /// A timer the program set went off. True if the state changed.
    fn timer(&mut self, _id: u64, _window: &mut Window) -> bool {
        false
    }

    /// The window is now drawn at `scale` pixels to the reference unit (it
    /// opened, or moved to another monitor), for a program that prepares
    /// pictures at their exact pixel size. True if the state changed.
    fn scale(&mut self, _scale: f32) -> bool {
        false
    }
}

/// Text measured by the window's fonts, in the theme's family.
pub struct FontsMeasure<'a> {
    pub fonts: &'a mut Fonts,
    pub family: &'a str,
}

impl FontsMeasure<'_> {
    fn face(&mut self, weight: Weight) -> Option<FaceId> {
        self.fonts.face(self.family, weight, false).or_else(|| self.fonts.face("sans-serif", weight, false))
    }
}

impl Measure for FontsMeasure<'_> {
    fn width(&mut self, size: f32, weight: Weight, text: &str) -> f32 {
        match self.face(weight) {
            Some(f) => self.fonts.layout(f, size, text).1,
            None => text.chars().count() as f32 * size * 0.55,
        }
    }

    fn line(&mut self, size: f32, weight: Weight) -> (f32, f32) {
        match self.face(weight) {
            Some(f) => {
                let m = self.fonts.metrics(f, size);
                (m.ascent, m.ascent + m.descent + m.line_gap)
            }
            None => (size * 0.8, size * 1.2),
        }
    }
}

/// A [`Program`] as `vergence-ui` sees it.
#[derive(Clone, Copy, PartialEq, Debug)]
enum FitToContent {
    Wanted,
    Ask(u32, u32),
    Done,
}

struct Host<P: Program> {
    program: P,
    ui: Ui<P::Message>,
    theme: Theme,
    /// The tree must be built again; the size it was laid out at.
    stale: bool,
    laid_at: (f32, f32),
    scale: f32,
    print_tree: bool,
    print_frames: bool,
    /// How long the last scene took to build (laid out and described), in ms.
    scene_ms: f64,
    /// The meaning last handed to the server (Contract 3): sent again only
    /// when it differs.
    meaning_sent: Option<vergence_ui::MeaningTree>,
    /// The bar last handed to the server: sent again only when it differs.
    bar_sent: Option<vergence_ui::Bar>,
    /// The app menu and bar style last handed to the server.
    commands_sent: Option<(bool, Vec<vergence_ui::AppCommand>)>,
    bar_style_sent: Option<vergence_ui::BarStyle>,
    /// The tint the theme was last built with.
    tint: Option<Color>,
    /// The title last given to the window (at first, the one it opens with).
    title_sent: String,
    /// A finger down and still (touch and hold, 2026-11-15, Dave: "there is
    /// no way to get to the desktop context menu" by touch): its id and
    /// where; held HOLD it is a right click there.
    hold: Option<(i32, f32, f32)>,
    /// The finger whose hold became a right click: the rest of its touch
    /// is dropped (lifting it does not also tap).
    held: Option<i32>,
    /// Fitting the window's height to its content (Program::fit_content):
    /// wanted until the first layout, then the size to ask for, then done.
    fit: FitToContent,
    /// The means to come back with the window (for the fit's request).
    waker: Option<Waker>,
}

/// The toolkit's timer for touch and hold, and how long a finger is held.
const HOLD_TIMER: u64 = (1 << 63) + 1;
const HOLD: std::time::Duration = std::time::Duration::from_millis(500);

/// The toolkit's own timer (the caret's blink).
const TOOLKIT_TIMER: u64 = 1 << 63;

impl<P: Program> Host<P> {
    fn changed(&mut self, window: &mut Window) {
        self.stale = true;
        window.redraw();
        // The title follows the program's state (a document's name, a dot for changes).
        let title = self.program.title();
        if title != self.title_sent {
            window.set_title(&title);
            self.title_sent = title;
        }
    }

    /// Asks for the toolkit's next wake, or forgets it.
    fn rearm(&self, window: &mut Window) {
        match self.ui.next_timer() {
            Some(at) => window.wake_at(TOOLKIT_TIMER, at),
            None => window.cancel_timer(TOOLKIT_TIMER),
        }
    }
}

impl<P: Program> vergence_ui::App for Host<P> {
    fn title(&self) -> String {
        self.program.title()
    }

    fn app_id(&self) -> String {
        self.program.app_id()
    }

    fn style(&self) -> vergence_ui::Style {
        self.program.style()
    }

    /// A dialog or palette opens at its own smallest size, not at the
    /// toolkit's 820 x 520 for another window.
    fn opening_size(&self) -> Option<(u32, u32)> {
        if let Some(s) = self.program.opening_size() {
            return Some(s);
        }
        let st = self.program.style();
        let small = matches!(st.kind, vergence_ui::WinKind::Dialog | vergence_ui::WinKind::Palette);
        (small && st.min.0 > 0.0 && st.min.1 > 0.0).then(|| (st.min.0.ceil() as u32, st.min.1.ceil() as u32))
    }

    fn under_bar(&self) -> bool {
        self.program.under_bar()
    }

    fn tabs_by_server(&self) -> bool {
        self.program.bar().is_some()
    }

    fn bar(&mut self) -> Option<vergence_ui::Bar> {
        let bar = self.program.bar();
        if bar.is_some() && bar != self.bar_sent {
            self.bar_sent = bar.clone();
            bar
        } else {
            None
        }
    }

    fn commands(&mut self) -> Option<(bool, Vec<vergence_ui::AppCommand>)> {
        let c = self.program.commands();
        (c.is_some() && c != self.commands_sent).then(|| {
            self.commands_sent = c.clone();
            c
        })?
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        self.program.command(id, window);
        self.changed(window);
    }

    fn drag_over(&mut self, x: f32, y: f32, kinds: &[String], ctrl: bool) -> u8 {
        self.program.drag_over(x, y, kinds, ctrl)
    }

    fn drag_left(&mut self) {
        self.program.drag_left();
    }

    fn dropped(&mut self, x: f32, y: f32, action: u8, items: &[(String, Vec<u8>)], window: &mut Window) {
        self.program.dropped(x, y, action, items, window);
        self.changed(window);
    }

    fn drag_ended(&mut self, action: u8, window: &mut Window) {
        self.program.drag_ended(action, window);
        self.changed(window);
    }

    fn bar_style(&mut self) -> Option<vergence_ui::BarStyle> {
        let s = self.program.bar_style();
        (s.is_some() && s != self.bar_style_sent).then(|| {
            self.bar_style_sent = s;
            s
        })?
    }

    fn tab(&mut self, event: vergence_ui::TabEvent, window: &mut Window) {
        self.program.tab(event, window);
        self.changed(window);
    }

    fn close_asked(&mut self, window: &mut Window) -> bool {
        let close = self.program.close_asked(window);
        if !close {
            self.changed(window);
        }
        close
    }

    fn closed(&mut self) {
        self.program.closed();
    }

    fn opened(&mut self, id: u32) {
        self.program.opened(id);
    }

    fn token(&self) -> String {
        self.program.token()
    }

    fn token_given(&mut self, token: &str) {
        self.program.token_given(token);
    }

    fn bar_button(&mut self, id: u32, window: &mut Window) {
        self.program.bar_button(id, window);
        self.stale = true;
        // A new frame: the page it shows, and the bar's new description after it.
        window.redraw();
    }

    fn background(&self) -> Color {
        self.theme.ground()
    }

    fn scene(&mut self, frame: &Frame, fonts: &mut Fonts) -> Scene {
        let began = Instant::now();
        let scene = self.build(frame, fonts);
        self.scene_ms = began.elapsed().as_secs_f64() * 1e3;
        scene
    }

    fn animating(&self) -> bool {
        self.ui.animating()
    }

    /// The tree's meaning for the server (native road), when it has changed
    /// since the last frame: layout, focus, values and states all show in it.
    fn meaning(&mut self) -> Option<vergence_ui::MeaningTree> {
        let mut tree = vergence_ui::MeaningTree::default();
        for m in self.ui.meaning() {
            if m.states.contains(&"focused") {
                tree.focus = m.id;
            }
            tree.nodes.push(vergence_ui::MeaningNode {
                id: m.id,
                parent: m.parent,
                role: m.role.to_string(),
                name: m.name,
                value: m.value,
                states: m.states.iter().map(|s| s.to_string()).collect(),
                x: m.bounds.x,
                y: m.bounds.y,
                w: m.bounds.w,
                h: m.bounds.h,
            });
        }
        if self.meaning_sent.as_ref() == Some(&tree) {
            return None;
        }
        self.meaning_sent = Some(tree.clone());
        Some(tree)
    }

    fn shown(&mut self, frame: &Frame, draw_ms: f64) -> bool {
        if self.print_frames {
            // `drawn_by` is the frame before's; this one's is told with the next.
            eprintln!("frame {} {:.2} ms, scene {:.2} ms (before: {})", frame.number, draw_ms, self.scene_ms, frame.drawn_by);
        }
        true
    }

    fn input(&mut self, input: &Input, window: &mut Window) {
        // Touch and hold: a finger kept still for HOLD is a right click
        // (every app's context menu by touch); the finger's touch is then
        // taken back from the widgets, and the rest of it dropped.
        if let Input::Touch { id, phase, x, y } = input {
            use vergence_ui::touch::Phase;
            if self.held == Some(*id) {
                if matches!(phase, Phase::Up | Phase::Cancelled) {
                    self.held = None;
                }
                return;
            }
            match phase {
                Phase::Down => {
                    self.hold = Some((*id, *x, *y));
                    window.wake_after(HOLD_TIMER, HOLD);
                }
                Phase::Moved => {
                    if let Some((h, hx, hy)) = self.hold
                        && h == *id
                        && ((x - hx).powi(2) + (y - hy).powi(2)).sqrt() > vergence_ui::touch::SLOP
                    {
                        self.hold = None;
                        window.cancel_timer(HOLD_TIMER);
                    }
                }
                Phase::Up | Phase::Cancelled => {
                    if self.hold.is_some_and(|(h, ..)| h == *id) {
                        self.hold = None;
                        window.cancel_timer(HOLD_TIMER);
                    }
                }
            }
        }
        self.program.saw(input);
        if matches!(input, Input::Key { .. })
            && let Some(changed) = self.program.key_first(input, window)
        {
            if changed {
                self.changed(window);
            }
            self.rearm(window);
            return;
        }
        let r = self.ui.input(input);
        if let Some(c) = r.cursor {
            window.set_cursor(c);
        }
        if r.redraw {
            window.redraw();
        }
        if let Some(text) = &r.copy {
            window.copy(text);
        }
        if r.paste {
            window.paste();
        }
        let any = !r.messages.is_empty();
        for m in r.messages {
            self.program.update(m, window);
        }
        if any {
            self.changed(window);
        }
        if !r.used && self.program.input(input, window) {
            self.changed(window);
        }
        self.rearm(window);
    }

    fn waker(&mut self, waker: Waker) {
        self.waker = Some(waker.clone());
        self.program.waker(waker);
    }

    fn woken(&mut self, window: &mut Window) {
        if let FitToContent::Ask(w, h) = self.fit {
            self.fit = FitToContent::Done;
            window.request_size(w, h);
        }
        if self.program.woken(window) {
            self.changed(window);
        }
    }

    fn timer(&mut self, id: u64, window: &mut Window) {
        if id == HOLD_TIMER {
            if let Some((finger, x, y)) = self.hold.take() {
                use vergence_ui::touch::Phase;
                self.input(&Input::Touch { id: finger, phase: Phase::Cancelled, x, y }, window);
                self.held = Some(finger);
                let modifiers = vergence_ui::Modifiers::default();
                self.input(&Input::Button { button: vergence_ui::Button::Right, pressed: true, x, y, modifiers }, window);
                self.input(&Input::Button { button: vergence_ui::Button::Right, pressed: false, x, y, modifiers }, window);
            }
            return;
        }
        if id == TOOLKIT_TIMER {
            // The caret blinks.
            window.redraw();
            self.rearm(window);
        } else if id < TOOLKIT_TIMER && self.program.timer(id, window) {
            self.changed(window);
        }
    }
}

impl<P: Program> Host<P> {
    /// The frame's scene: the tree built again if anything changed, then drawn.
    fn build(&mut self, frame: &Frame, fonts: &mut Fonts) -> Scene {
        if frame.scale != self.scale {
            self.scale = frame.scale;
            self.stale |= self.program.scale(frame.scale);
        }
        if self.stale || self.laid_at != (frame.width, frame.height) || self.theme.top != frame.top || self.tint != frame.tint || self.theme.text_factor != frame.text {
            self.theme = self.program.theme();
            self.theme.top = frame.top;
            // Text as large on this screen as on any (Frame::text).
            self.theme.text_factor = frame.text;
            self.theme.text_size = vergence_look::text_size(self.theme.text_size, frame.text);
            self.ui.set_text_factor(frame.text);
            // The wallpaper's color behind the window (tint, window styles milestone 1).
            if let Some(c) = frame.tint {
                self.theme = self.theme.tinted(c);
            }
            self.tint = frame.tint;
            let root = self.program.view(&self.theme);
            let mut measure = FontsMeasure { fonts, family: &self.theme.font };
            self.ui.layout(root, frame.width, frame.height, &mut measure, self.theme.text_size);
            self.ui.set_under_bar(self.theme.top);
            self.ui.set_reduced_motion(self.theme.reduced_motion);
            self.ui.set_double_click(self.theme.double_click);
            self.stale = false;
            if self.laid_at != (frame.width, frame.height) {
                self.program.resized(frame.width, frame.height);
            }
            self.laid_at = (frame.width, frame.height);
            // As tall as its content, once, after the first layout (Dave,
            // 2026-11-16: "size to content, then window space"; the server
            // keeps it on the screen, where a long one scrolls).
            if self.fit == FitToContent::Wanted {
                self.fit = FitToContent::Done;
                if self.program.fit_content()
                    && let Some(extra) = self.ui.overflow()
                    && extra.abs() >= 2.0
                {
                    let h = (frame.height + extra).max(self.program.style().min.1 as f32).ceil();
                    self.fit = FitToContent::Ask(frame.width.round() as u32, h as u32);
                    if let Some(w) = self.waker.as_ref() {
                        w.wake();
                    }
                }
            }
            if self.print_tree {
                eprint!("{}", self.ui.meaning_text());
            }
        }
        self.ui.advance(Instant::now());
        let mut scene = Scene::new(frame.width, frame.height);
        self.ui.draw(&mut scene, fonts, &self.theme);
        scene
    }
}

/// The program as a window another window of the same program opens with
/// `Window::open_window` (its opener is its parent: a dialog's, say).
pub fn window<P: Program + 'static>(program: P) -> Box<dyn vergence_ui::App> {
    let theme = program.theme();
    let title_sent = program.title();
    Box::new(Host { program, ui: Ui::new(), theme, stale: true, laid_at: (0.0, 0.0), scale: 0.0, print_tree: false, print_frames: false, scene_ms: 0.0, meaning_sent: None, bar_sent: None, commands_sent: None, bar_style_sent: None, tint: None, title_sent, hold: None, held: None, fit: FitToContent::Wanted, waker: None })
}

/// Opens the program's window and runs until it is closed. `size` is the
/// size asked for, in reference units.
pub fn run<P: Program>(program: P, size: (u32, u32)) -> io::Result<()> {
    let theme = program.theme();
    let title_sent = program.title();
    let mut host = Host { program, ui: Ui::new(), theme, stale: true, laid_at: (0.0, 0.0), scale: 0.0, print_tree: std::env::var_os("VERGENCE_UI_TREE").is_some_and(|v| v == "1"), print_frames: std::env::var_os("VERGENCE_UI_FRAMES").is_some_and(|v| v == "1"), scene_ms: 0.0, meaning_sent: None, bar_sent: None, commands_sent: None, bar_style_sent: None, tint: None, title_sent, hold: None, held: None, fit: FitToContent::Wanted, waker: None };
    vergence_ui::run(&mut host, size)
}

#[cfg(test)]
mod tests;
