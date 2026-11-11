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
//! each time the tree is laid out.
//!
//! Timers: a program's own ids for [`Window::wake_at`] are below 2^63; the
//! toolkit keeps those above for its widgets.

mod element;
mod layout;
mod theme;
mod ui;

use std::io;

pub use element::{Align, ButtonLook, Element, Length, Padding, Weight, button, column, label, panel, row, rule, space, stack, vrule};
pub use layout::{Id, Line, Measure};
pub use theme::{Theme, parse_hex};
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

    /// Called once, before the window opens, with the means for other
    /// threads to wake it.
    fn waker(&mut self, _waker: Waker) {}

    /// Another thread used the [`Waker`]. True if the state changed.
    fn woken(&mut self, _window: &mut Window) -> bool {
        false
    }

    /// A timer the program set went off. True if the state changed.
    fn timer(&mut self, _id: u64, _window: &mut Window) -> bool {
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
struct Host<P: Program> {
    program: P,
    ui: Ui<P::Message>,
    theme: Theme,
    /// The tree must be built again; the size it was laid out at.
    stale: bool,
    laid_at: (f32, f32),
    print_tree: bool,
}

impl<P: Program> Host<P> {
    fn changed(&mut self, window: &mut Window) {
        self.stale = true;
        window.redraw();
    }
}

impl<P: Program> vergence_ui::App for Host<P> {
    fn title(&self) -> String {
        self.program.title()
    }

    fn app_id(&self) -> String {
        self.program.app_id()
    }

    fn background(&self) -> Color {
        self.theme.ground()
    }

    fn scene(&mut self, frame: &Frame, fonts: &mut Fonts) -> Scene {
        if self.stale || self.laid_at != (frame.width, frame.height) {
            self.theme = self.program.theme();
            let root = self.program.view(&self.theme);
            let mut measure = FontsMeasure { fonts, family: &self.theme.font };
            self.ui.layout(root, frame.width, frame.height, &mut measure, self.theme.text_size);
            self.stale = false;
            self.laid_at = (frame.width, frame.height);
            if self.print_tree {
                eprint!("{}", self.ui.meaning_text());
            }
        }
        let mut scene = Scene::new(frame.width, frame.height);
        self.ui.draw(&mut scene, fonts, &self.theme);
        scene
    }

    fn input(&mut self, input: &Input, window: &mut Window) {
        let r = self.ui.input(input);
        if let Some(c) = r.cursor {
            window.set_cursor(c);
        }
        if r.redraw {
            window.redraw();
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
    }

    fn waker(&mut self, waker: Waker) {
        self.program.waker(waker);
    }

    fn woken(&mut self, window: &mut Window) {
        if self.program.woken(window) {
            self.changed(window);
        }
    }

    fn timer(&mut self, id: u64, window: &mut Window) {
        if id < 1 << 63 && self.program.timer(id, window) {
            self.changed(window);
        }
    }
}

/// Opens the program's window and runs until it is closed. `size` is the
/// size asked for, in reference units.
pub fn run<P: Program>(program: P, size: (u32, u32)) -> io::Result<()> {
    let theme = program.theme();
    let mut host = Host { program, ui: Ui::new(), theme, stale: true, laid_at: (0.0, 0.0), print_tree: std::env::var_os("VERGENCE_UI_TREE").is_some_and(|v| v == "1") };
    vergence_ui::run(&mut host, size)
}

#[cfg(test)]
mod tests;
