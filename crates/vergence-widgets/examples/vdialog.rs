//! Dialogs on the native road (window styles milestone 1, step 3): a window
//! with buttons that open a dialog of it, a modal one, and a palette.
//! `cargo run --release -p vergence-widgets --example vdialog`.
//!
//! A dialog opens over the window, has a slim bar with close only, goes where
//! the window goes, comes forward and is minimized with it, and is not listed
//! apart from it; a modal one dims the window and takes every press on it.
use vergence_widgets::{Align, Element, Length, Program, Theme, Window, button, column, label, run, window};

#[derive(Debug, Clone)]
enum Msg {
    Open { modal: bool },
    Palette,
}

struct Main {
    opened: u32,
}

impl Program for Main {
    type Message = Msg;
    fn title(&self) -> String {
        "Dialogs".into()
    }
    fn app_id(&self) -> String {
        "vdialog".into()
    }
    fn view(&self, _: &Theme) -> Element<Msg> {
        column(vec![
            label("A window, and dialogs of it.").size(16.0),
            label(format!("Opened so far: {}.", self.opened)).dim(),
            button(label("Open a dialog")).name("Open a dialog").on_press(Msg::Open { modal: false }),
            button(label("Open a modal dialog")).name("Open a modal dialog").on_press(Msg::Open { modal: true }),
            button(label("Open a palette")).name("Open a palette").on_press(Msg::Palette),
        ])
        .spacing(12.0)
        .padding(24.0)
        .align(Align::Start)
        .width(Length::FILL)
        .into()
    }
    fn update(&mut self, m: Msg, w: &mut Window) {
        match m {
            Msg::Open { modal } => {
                self.opened += 1;
                w.open_window(window(Dialog { modal }));
            }
            Msg::Palette => {
                self.opened += 1;
                w.open_window(window(Palette));
            }
        }
    }
}

struct Dialog {
    modal: bool,
}

impl Program for Dialog {
    type Message = Msg;
    fn title(&self) -> String {
        if self.modal { "A modal dialog".into() } else { "A dialog".into() }
    }
    fn app_id(&self) -> String {
        "vdialog".into()
    }
    fn view(&self, _: &Theme) -> Element<Msg> {
        column(vec![label(if self.modal { "Its window waits until this closes." } else { "Its window stays usable." }), label("Close it with its bar's button.").dim()])
            .spacing(8.0)
            .padding(20.0)
            .into()
    }
    fn update(&mut self, _: Msg, _: &mut Window) {}
    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { kind: vergence_ui::WinKind::Dialog, modal: self.modal, min: (320.0, 140.0), max: (0.0, 0.0), alone: false, frosted: false }
    }
}

/// A tool panel of the window: above its application's windows, hidden while
/// another application is in front.
struct Palette;

impl Program for Palette {
    type Message = Msg;
    fn title(&self) -> String {
        "Tools".into()
    }
    fn app_id(&self) -> String {
        "vdialog".into()
    }
    fn view(&self, _: &Theme) -> Element<Msg> {
        column(vec![label("A palette."), label("Hidden while another application is in front.").dim()]).spacing(6.0).padding(12.0).into()
    }
    fn update(&mut self, _: Msg, _: &mut Window) {}
    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { kind: vergence_ui::WinKind::Palette, modal: false, min: (200.0, 100.0), max: (0.0, 0.0), alone: false, frosted: false }
    }
}

fn main() -> std::io::Result<()> {
    run(Main { opened: 0 }, (520, 360))
}
