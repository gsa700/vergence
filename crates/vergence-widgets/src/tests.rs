//! Layout, input, focus and meaning, with text measured by a stand-in:
//! every character half its size wide.

use super::*;
use vergence_scene::Rect;
use vergence_ui::{Button as MouseButton, Modifiers, sym, touch::Phase};

struct Fixed;

impl Measure for Fixed {
    fn width(&mut self, size: f32, _: Weight, text: &str) -> f32 {
        text.chars().count() as f32 * size * 0.5
    }

    fn line(&mut self, size: f32, _: Weight) -> (f32, f32) {
        (size * 0.8, size * 1.25)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    A,
    B,
    C,
}

fn laid(root: Element<Msg>, w: f32, h: f32) -> Ui<Msg> {
    let mut ui = Ui::new();
    ui.layout(root, w, h, &mut Fixed, 10.0);
    ui
}

/// The bounds of the widgets with a meaning, by name.
fn bounds(ui: &Ui<Msg>, name: &str) -> Rect {
    ui.meaning().into_iter().find(|m| m.name == name).unwrap_or_else(|| panic!("no {name:?} in\n{}", ui.meaning_text())).bounds
}

fn press(ui: &mut Ui<Msg>, x: f32, y: f32, pressed: bool) -> Vec<Msg> {
    ui.input(&Input::Button { button: MouseButton::Left, pressed, x, y, modifiers: Modifiers::default() }).messages
}

fn key(ui: &mut Ui<Msg>, sym: u32, shift: bool) -> Vec<Msg> {
    let modifiers = Modifiers { shift, ..Modifiers::default() };
    let r = ui.input(&Input::Key { sym, text: None, pressed: true, repeat: false, composing: false, modifiers }).messages;
    ui.input(&Input::Key { sym, text: None, pressed: false, repeat: false, composing: false, modifiers });
    r
}

#[test]
fn a_row_shares_its_width() {
    // 100 wide: a fixed 20, a label of 4 characters at 10 (20 wide), and two
    // fills weighted 1 and 2 sharing what is left (100 - 20 - 20 - 3 gaps of 4 = 48).
    let ui = laid(
        row(vec![space().width(Length::Fixed(20.0)), label("abcd"), label("one").width(Length::Fill(1)), label("two").width(Length::Fill(2))]).spacing(4.0).width(Length::FILL),
        100.0,
        50.0,
    );
    assert_eq!(bounds(&ui, "abcd"), Rect::new(24.0, 0.0, 20.0, 13.0));
    assert_eq!(bounds(&ui, "one"), Rect::new(48.0, 0.0, 16.0, 13.0));
    assert_eq!(bounds(&ui, "two"), Rect::new(68.0, 0.0, 32.0, 13.0));
}

#[test]
fn a_column_centres_and_fills() {
    let ui = laid(
        column(vec![label("top"), space().height(Length::FILL), label("bottom")]).align(Align::Center).width(Length::FILL).height(Length::FILL),
        100.0,
        100.0,
    );
    assert_eq!(bounds(&ui, "top"), Rect::new(43.0, 0.0, 15.0, 13.0));
    assert_eq!(bounds(&ui, "bottom"), Rect::new(35.0, 87.0, 30.0, 13.0), "the space takes the rest");
}

#[test]
fn a_row_s_children_on_its_middle() {
    let ui = laid(row(vec![label("big").size(20.0), label("small")]).align(Align::Center), 200.0, 50.0);
    assert_eq!(bounds(&ui, "big").h, 25.0);
    assert_eq!(bounds(&ui, "small"), Rect::new(30.0, 6.0, 25.0, 13.0));
}

#[test]
fn panels_pad_and_pages_keep_readable() {
    let ui = laid(panel(label("inside")).padding(Padding::hv(5.0, 8.0)), 300.0, 300.0);
    assert_eq!(bounds(&ui, "inside"), Rect::new(8.0, 5.0, 30.0, 13.0));
    let ui = laid(column(vec![label("x").width(Length::FILL)]).width(Length::FILL).max_width(120.0), 300.0, 300.0);
    assert_eq!(bounds(&ui, "x").w, 120.0);
}

#[test]
fn text_wraps_at_its_width() {
    let ui = laid(label("one two three four").wrap().width(Length::Fixed(50.0)), 300.0, 300.0);
    let m = &ui.meaning()[0];
    // "one two three" would be 65 wide: "one two", then "three four" (exactly 50).
    assert_eq!((m.bounds.w, m.bounds.h), (50.0, 25.0), "two lines of 12.5: {}", ui.meaning_text());
    assert_eq!(m.name, "one two three four");
}

fn two_buttons() -> Element<Msg> {
    row(vec![button(label("A")).on_press(Msg::A), button(label("B")).on_press(Msg::B), button(label("off"))]).spacing(10.0)
}

#[test]
fn a_click_is_judged_where_it_happened() {
    let mut ui = laid(two_buttons(), 300.0, 100.0);
    let a = bounds(&ui, "A");
    let b = bounds(&ui, "B");
    let (ax, ay) = (a.x + 3.0, a.y + 3.0);
    // The pointer is reported over B, but the press and release came over A.
    ui.input(&Input::PointerMoved { x: b.x + 2.0, y: b.y + 2.0 });
    assert!(press(&mut ui, ax, ay, true).is_empty());
    assert_eq!(press(&mut ui, ax, ay, false), vec![Msg::A]);
    // Pressed on A, let go on B: nothing.
    press(&mut ui, ax, ay, true);
    assert!(press(&mut ui, b.x + 2.0, b.y + 2.0, false).is_empty());
    // A button that is off takes nothing.
    let off = bounds(&ui, "off");
    press(&mut ui, off.x + 2.0, off.y + 2.0, true);
    assert!(press(&mut ui, off.x + 2.0, off.y + 2.0, false).is_empty());
}

#[test]
fn a_finger_taps() {
    let mut ui = laid(two_buttons(), 300.0, 100.0);
    let b = bounds(&ui, "B");
    let at = |id, phase| Input::Touch { id, phase, x: b.x + 4.0, y: b.y + 4.0 };
    ui.input(&at(7, Phase::Down));
    assert_eq!(ui.input(&at(7, Phase::Up)).messages, vec![Msg::B]);
    ui.input(&at(8, Phase::Down));
    assert!(ui.input(&at(8, Phase::Cancelled)).messages.is_empty(), "taken back by the system");
}

#[test]
fn tab_walks_the_controls_and_enter_presses() {
    let mut ui = laid(column(vec![two_buttons(), button(label("C")).on_press(Msg::C)]), 300.0, 100.0);
    assert_eq!(ui.focused(), None);
    key(&mut ui, sym::TAB, false);
    assert_eq!(key(&mut ui, sym::RETURN, false), vec![Msg::A]);
    key(&mut ui, sym::TAB, false);
    key(&mut ui, sym::TAB, false);
    assert_eq!(key(&mut ui, ' ' as u32, false), vec![Msg::C], "the button that is off is passed over");
    key(&mut ui, sym::TAB, false);
    assert_eq!(key(&mut ui, sym::RETURN, false), vec![Msg::A], "round again");
    key(&mut ui, sym::TAB, true);
    assert_eq!(key(&mut ui, sym::RETURN, false), vec![Msg::C], "and back");
    assert!(ui.meaning().iter().any(|m| m.name == "C" && m.states == vec!["focused"]));
}

#[test]
fn every_widget_has_its_own_id() {
    let ui = laid(column(vec![row(vec![button(label("A")).on_press(Msg::A), button(label("B")).on_press(Msg::B)]), panel(label("x"))]), 300.0, 100.0);
    let mut ids: Vec<Id> = ui.ids();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n);
}

#[test]
fn what_is_remembered_follows_the_key() {
    let list = |order: &[(u64, &str, Msg)]| column(order.iter().map(|(k, n, m)| button(label(*n)).on_press(m.clone()).key(*k)).collect());
    let mut ui = laid(list(&[(1, "one", Msg::A), (2, "two", Msg::B)]), 300.0, 100.0);
    key(&mut ui, sym::TAB, false);
    key(&mut ui, sym::TAB, false);
    // "two" has the focus; the list is built again in the other order.
    ui.layout(list(&[(2, "two", Msg::B), (1, "one", Msg::A)]), 300.0, 100.0, &mut Fixed, 10.0);
    assert_eq!(key(&mut ui, sym::RETURN, false), vec![Msg::B]);
    // Gone from the tree: the focus goes with it.
    ui.layout(list(&[(1, "one", Msg::A)]), 300.0, 100.0, &mut Fixed, 10.0);
    assert_eq!(ui.focused(), None);
}

#[test]
fn the_meaning_of_a_page() {
    let page = column(vec![
        label("Appearance").size(20.0),
        rule(),
        row(vec![label("Accent"), button(label("\u{2715}")).name("No accent").on_press(Msg::A), button(label("Apply"))]),
    ]);
    let ui = laid(page, 300.0, 200.0);
    let text = ui.meaning_text();
    let lines: Vec<&str> = text.lines().map(|l| l.split(" at ").next().unwrap_or("")).collect();
    assert_eq!(lines, vec!["label \"Appearance\"", "separator \"\"", "label \"Accent\"", "button \"No accent\"", "button \"Apply\" [off]"], "{text}");
}

#[test]
fn drawn_through_the_cpu_renderer() {
    use vergence_render::Cpu;
    use vergence_text::db::FontDb;
    let theme = Theme::default();
    let mut fonts = Fonts::new(FontDb::default());
    let mut ui = laid(column(vec![button(space().width(Length::Fixed(40.0)).height(Length::Fixed(20.0))).padding(0.0).on_press(Msg::A).look(ButtonLook::Chosen)]), 100.0, 100.0);
    let shot = |ui: &Ui<Msg>, fonts: &mut Fonts| {
        let mut scene = Scene::new(100.0, 100.0);
        ui.draw(&mut scene, fonts, &theme);
        Cpu::new().render(&scene, fonts, 2.0, theme.page).expect("renders")
    };
    let p = shot(&ui, &mut fonts);
    let a = theme.accent;
    assert_eq!(p.pixel(40, 20), [a.r, a.g, a.b, 255], "the button, at scale 2");
    assert_eq!(p.pixel(90, 20), [theme.page.r, theme.page.g, theme.page.b, 255]);
    // The ring shows only after Tab.
    assert_eq!(p.pixel(40, 43), [theme.page.r, theme.page.g, theme.page.b, 255]);
    key(&mut ui, sym::TAB, false);
    let p = shot(&ui, &mut fonts);
    assert_eq!(p.pixel(40, 43), [a.r, a.g, a.b, 255], "the focus ring below it");
}
