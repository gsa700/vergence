// Closing a window asks first (settings milestone 2, step 7; Dave: "more than
// one tab open or an active tab present"): with more than one tab, or with a
// tab whose shell is running a program (its terminal's foreground process
// group is not the shell's own: a build, an SSH session, an editor). One idle
// tab closes without asking. The question is a modal dialog over the window
// (window styles milestone 1), on the toolkit's widgets.

use std::cell::Cell;
use std::rc::Rc;

use vergence_ui::{Waker, Window};
use vergence_widgets::{Align, ButtonLook, Element, Length, Padding, Program, Theme, button, column, label, panel, row, space};

/// What the question says: (heading, detail).
pub fn question(tabs: usize, running: &[String]) -> Option<(String, String)> {
    let first = running.first().map(String::as_str).unwrap_or("a program");
    match (tabs, running.len()) {
        (0 | 1, 0) => None,
        (0 | 1, _) => Some((format!("A program is running in this tab ({first}). Close it?"), "Closing the window ends it.".into())),
        (n, 0) => Some((format!("Close {n} tabs?"), "Closing the window closes every tab in it.".into())),
        (n, 1) => Some((format!("Close {n} tabs?"), format!("{first} is running in one of them; closing the window ends it."))),
        (n, k) => Some((format!("Close {n} tabs?"), format!("Programs are running in {k} of them ({first} and more); closing the window ends them."))),
    }
}

/// The dialog. Its answer goes back through `close` and a wake of the
/// terminal's window, which then closes itself; Cancel, or the dialog's own
/// close button, leaves it be.
pub struct CloseDialog {
    pub heading: String,
    pub detail: String,
    pub close: Rc<Cell<bool>>,
    /// Set while the dialog is open, so a second close asks no second question.
    pub open: Rc<Cell<bool>>,
    pub waker: Option<Waker>,
    pub theme: Theme,
}

#[derive(Debug, Clone)]
pub enum CloseMsg {
    Close,
    Cancel,
}

impl Program for CloseDialog {
    type Message = CloseMsg;
    fn title(&self) -> String {
        "Close this window?".into()
    }
    fn app_id(&self) -> String {
        "vergence-term".into()
    }
    fn theme(&self) -> Theme {
        self.theme.clone()
    }
    fn view(&self, _: &Theme) -> Element<CloseMsg> {
        panel(column(vec![
            label(self.heading.clone()).size(15.0).weight(600).wrap().width(Length::FILL),
            label(self.detail.clone()).dim().wrap().width(Length::FILL),
            row(vec![
                space().width(Length::FILL),
                button(label("Cancel").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(CloseMsg::Cancel).name("Cancel"),
                button(label("Close").size(12.0)).look(ButtonLook::Accent).padding(Padding::hv(4.0, 14.0)).on_press(CloseMsg::Close).name("Close"),
            ])
            .spacing(8.0)
            .align(Align::Center)
            .width(Length::FILL),
        ])
        .spacing(10.0)
        .width(Length::FILL))
        .padding(20.0)
        .radius(0.0)
        .width(Length::FILL)
        .height(Length::FILL)
        .into()
    }
    fn update(&mut self, m: CloseMsg, window: &mut Window) {
        if matches!(m, CloseMsg::Close) {
            self.close.set(true);
            if let Some(w) = &self.waker {
                w.wake();
            }
        }
        window.close();
    }
    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { kind: vergence_ui::WinKind::Dialog, modal: true, min: (400.0, 150.0), max: (0.0, 0.0), alone: false, frosted: false }
    }
    fn scale(&mut self, _: f32) -> bool {
        false
    }
}

impl Drop for CloseDialog {
    fn drop(&mut self) {
        self.open.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_question_says() {
        assert_eq!(question(1, &[]), None);
        assert_eq!(question(1, &["ssh".into()]).unwrap().0, "A program is running in this tab (ssh). Close it?");
        assert_eq!(question(3, &[]).unwrap().0, "Close 3 tabs?");
        assert!(question(3, &["make".into()]).unwrap().1.starts_with("make is running"));
        assert!(question(2, &["make".into(), "vi".into()]).unwrap().1.contains("2 of them"));
    }
}
