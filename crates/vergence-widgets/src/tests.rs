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
    Toggle(bool),
    Value(f32),
    Done,
    Text(String),
    Submit,
    Pick(usize),
    Touched(Touch),
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
    let shot = |ui: &mut Ui<Msg>, fonts: &mut Fonts| {
        let mut scene = Scene::new(100.0, 100.0);
        ui.draw(&mut scene, fonts, &theme);
        Cpu::new().render(&scene, fonts, 2.0, theme.page).expect("renders")
    };
    let p = shot(&mut ui, &mut fonts);
    let a = theme.accent;
    assert_eq!(p.pixel(40, 20), [a.r, a.g, a.b, 255], "the button, at scale 2");
    assert_eq!(p.pixel(90, 20), [theme.page.r, theme.page.g, theme.page.b, 255]);
    // The ring shows only after Tab.
    assert_eq!(p.pixel(40, 43), [theme.page.r, theme.page.g, theme.page.b, 255]);
    key(&mut ui, sym::TAB, false);
    let p = shot(&mut ui, &mut fonts);
    assert_eq!(p.pixel(40, 43), [a.r, a.g, a.b, 255], "the focus ring below it");
}

fn type_text(ui: &mut Ui<Msg>, text: &str) -> Vec<Msg> {
    let mut out = Vec::new();
    for c in text.chars() {
        out.extend(ui.input(&Input::Key { sym: c as u32, text: Some(c.to_string()), pressed: true, repeat: false, composing: false, modifiers: Modifiers::default() }).messages);
    }
    out
}

fn chord(ui: &mut Ui<Msg>, c: char, ctrl: bool) -> Response<Msg> {
    let modifiers = Modifiers { ctrl, ..Modifiers::default() };
    ui.input(&Input::Key { sym: c as u32, text: None, pressed: true, repeat: false, composing: false, modifiers })
}

fn centre(r: Rect) -> (f32, f32) {
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

fn click(ui: &mut Ui<Msg>, x: f32, y: f32) -> Vec<Msg> {
    let mut m = press(ui, x, y, true);
    m.extend(press(ui, x, y, false));
    m
}

#[test]
fn a_switch_flips() {
    let page = |on: bool| column(vec![switch(on, Msg::Toggle).name("Repeat"), switch(on, Msg::Toggle).name("Fixed").disabled()]);
    let mut ui = laid(page(false), 300.0, 100.0);
    let (x, y) = centre(bounds(&ui, "Repeat"));
    assert_eq!(click(&mut ui, x, y), vec![Msg::Toggle(true)]);
    ui.layout(page(true), 300.0, 100.0, &mut Fixed, 10.0);
    key(&mut ui, sym::TAB, false);
    assert_eq!(key(&mut ui, ' ' as u32, false), vec![Msg::Toggle(false)]);
    let (x, y) = centre(bounds(&ui, "Fixed"));
    assert!(click(&mut ui, x, y).is_empty(), "one that cannot be changed");
    let m = ui.meaning();
    assert_eq!((m[0].value.as_deref(), m[1].states.clone()), (Some("on"), vec!["off"]));
}

#[test]
fn a_slider_is_dragged_and_keyed() {
    let mut ui = laid(column(vec![slider(0.0, 100.0, 50.0, Msg::Value).step(10.0).on_release(Msg::Done).width(Length::Fixed(116.0)).name("Delay")]), 300.0, 100.0);
    let r = bounds(&ui, "Delay");
    // The thumb travels 100 points, from 8 in (its radius).
    let y = r.y + r.h / 2.0;
    assert_eq!(press(&mut ui, r.x + 8.0 + 23.0, y, true), vec![Msg::Value(20.0)], "kept to the step");
    ui.input(&Input::PointerMoved { x: r.x + 8.0 + 77.0, y });
    let moved = ui.input(&Input::PointerMoved { x: r.x + 500.0, y }).messages;
    assert_eq!(moved, vec![Msg::Value(100.0)], "held at the end");
    assert_eq!(press(&mut ui, r.x + 500.0, y, false), vec![Msg::Done]);
    // The keys: from the value the program gave (50).
    assert_eq!(key(&mut ui, sym::RIGHT, false), vec![Msg::Value(60.0), Msg::Done]);
    assert_eq!(key(&mut ui, sym::HOME, false), vec![Msg::Value(0.0), Msg::Done]);
    // A finger drags it too.
    let f = |phase, x| Input::Touch { id: 1, phase, x, y };
    assert_eq!(ui.input(&f(Phase::Down, r.x + 8.0)).messages, vec![Msg::Value(0.0)]);
    assert_eq!(ui.input(&f(Phase::Moved, r.x + 8.0 + 40.0)).messages, vec![Msg::Value(40.0)]);
    assert_eq!(ui.input(&f(Phase::Up, r.x + 8.0 + 40.0)).messages, vec![Msg::Done]);
}

fn form(text: &str) -> Element<Msg> {
    column(vec![field(text, Msg::Text).on_submit(Msg::Submit).name("Accent"), field("hunter2", Msg::Text).secret().name("Password")])
}

#[test]
fn typing_into_a_field() {
    let mut ui = laid(form(""), 300.0, 100.0);
    key(&mut ui, sym::TAB, false);
    // Keys that come before the tree is built again are not lost.
    assert_eq!(type_text(&mut ui, "#ab"), vec![Msg::Text("#".into()), Msg::Text("#a".into()), Msg::Text("#ab".into())]);
    ui.layout(form("#ab"), 300.0, 100.0, &mut Fixed, 10.0);
    assert_eq!(key(&mut ui, sym::BACKSPACE, false), vec![Msg::Text("#a".into())]);
    key(&mut ui, sym::LEFT, false);
    assert_eq!(type_text(&mut ui, "x"), vec![Msg::Text("#xa".into())]);
    assert_eq!(key(&mut ui, sym::RETURN, false), vec![Msg::Submit]);
    // Shift+Home chooses back to the start; typing replaces it.
    ui.input(&Input::Key { sym: sym::HOME, text: None, pressed: true, repeat: false, composing: false, modifiers: Modifiers { shift: true, ..Modifiers::default() } });
    assert_eq!(chord(&mut ui, 'c', true).copy.as_deref(), Some("#x"));
    assert_eq!(type_text(&mut ui, "ff"), vec![Msg::Text("fa".into()), Msg::Text("ffa".into())]);
    // The program sets the text: its value wins.
    ui.layout(form("#123456"), 300.0, 100.0, &mut Fixed, 10.0);
    assert_eq!(ui.field_text().map(|t| t.0), Some("#123456".into()));
    assert!(chord(&mut ui, 'v', true).paste);
    assert!(chord(&mut ui, 'a', true).messages.is_empty());
    assert_eq!(ui.input(&Input::Paste("red\nignored\u{7}".into())).messages, vec![Msg::Text("red".into())], "one line");
    assert_eq!(ui.input(&Input::Paste("a\u{7}b".into())).messages, vec![Msg::Text("redab".into())], "no control characters");
    // A secret is never copied nor told.
    key(&mut ui, sym::TAB, false);
    assert_eq!(ui.field_text().map(|t| (t.1, t.2)), Some((0, 7)), "all chosen on arrival");
    assert_eq!(chord(&mut ui, 'c', true).copy, None);
    let m = ui.meaning();
    assert_eq!((m[1].value.clone(), m[1].states.clone()), (None, vec!["secret", "focused"]));
}

#[test]
fn a_field_holds_so_much_and_no_more() {
    let mut ui = laid(form(""), 300.0, 100.0);
    key(&mut ui, sym::TAB, false);
    let long = "\u{e9}".repeat(3000);
    let m = ui.input(&Input::Paste(long));
    let Some(Msg::Text(t)) = m.messages.last() else { panic!("no text") };
    assert_eq!(t.len(), crate::ui::MAX_FIELD, "cut on a character's edge");
    assert!(type_text(&mut ui, "z").is_empty(), "full");
}

fn picker(sel: Option<usize>) -> Element<Msg> {
    column(vec![choice((0..30).map(|i| format!("item {i}")).collect(), sel, Msg::Pick).name("Fit"), button(label("A")).on_press(Msg::A)])
}

fn is_open(ui: &Ui<Msg>) -> bool {
    ui.meaning()[0].states.contains(&"open")
}

#[test]
fn a_choice_opens_and_chooses() {
    let mut ui = laid(picker(Some(2)), 300.0, 400.0);
    let (x, y) = centre(bounds(&ui, "Fit"));
    assert!(click(&mut ui, x, y).is_empty());
    assert!(is_open(&ui));
    // The list is below it: the fourth item is chosen by a click.
    let fit = bounds(&ui, "Fit");
    let item = (10.0f32 * 1.25 + 8.0).ceil();
    let at = fit.y + fit.h + 2.0 + 4.0 + 3.5 * item;
    assert_eq!(press(&mut ui, x, at, true), vec![Msg::Pick(3)]);
    press(&mut ui, x, at, false);
    assert!(!is_open(&ui));
    // Keys: Down opens, Down moves, Enter chooses; Escape closes.
    key(&mut ui, sym::TAB, false);
    key(&mut ui, sym::DOWN, false);
    assert!(is_open(&ui));
    key(&mut ui, sym::DOWN, false);
    assert_eq!(key(&mut ui, sym::RETURN, false), vec![Msg::Pick(3)]);
    key(&mut ui, sym::DOWN, false);
    key(&mut ui, sym::ESCAPE, false);
    assert!(!is_open(&ui));
    // A click elsewhere closes the list and does nothing else.
    click(&mut ui, x, y);
    assert!(is_open(&ui));
    let (ax, ay) = centre(bounds(&ui, "A"));
    assert!(click(&mut ui, ax + 250.0, ay + 300.0).is_empty());
    assert!(!is_open(&ui));
}

fn long_page() -> Element<Msg> {
    scroll(column((0..40).map(|i| button(label(format!("b{i}"))).on_press(if i == 39 { Msg::C } else { Msg::A }).height(Length::Fixed(20.0))).collect::<Vec<_>>()).width(Length::FILL))
}

#[test]
fn a_page_scrolls() {
    // 40 rows of 20: 800 tall in 100.
    let mut ui = laid(long_page(), 200.0, 100.0);
    ui.input(&Input::Scroll { x: 20.0, y: 50.0, dx: 0.0, dy: 15.0 });
    assert_eq!(ui.scroll_offset(), 30.0);
    ui.input(&Input::Scroll { x: 20.0, y: 50.0, dx: 0.0, dy: 15000.0 });
    assert_eq!(ui.scroll_offset(), 700.0, "held at the end");
    // What is under a click is what is shown there.
    assert_eq!(click(&mut ui, 20.0, 95.0), vec![Msg::C]);
    ui.input(&Input::Scroll { x: 20.0, y: 50.0, dx: 0.0, dy: -15000.0 });
    // Tab back from nothing reaches the last, and brings it into view.
    key(&mut ui, sym::TAB, true);
    assert!(ui.scroll_offset() >= 700.0, "{}", ui.scroll_offset());
    assert_eq!(key(&mut ui, sym::RETURN, false), vec![Msg::C]);
    key(&mut ui, sym::HOME, false);
    assert_eq!(ui.scroll_offset(), 0.0);
    // Laid out again: the place is kept.
    key(&mut ui, sym::PAGE_DOWN, false);
    let kept = ui.scroll_offset();
    assert!(kept > 0.0);
    ui.layout(long_page(), 200.0, 100.0, &mut Fixed, 10.0);
    assert_eq!(ui.scroll_offset(), kept);
    // The scrollbar's thumb is dragged to the bottom.
    let at_bar = 200.0 - 2.0 - 3.0;
    press(&mut ui, at_bar, 10.0, true);
    ui.input(&Input::PointerMoved { x: at_bar, y: 500.0 });
    press(&mut ui, at_bar, 500.0, false);
    assert_eq!(ui.scroll_offset(), 700.0);
}

#[test]
fn a_finger_scrolls_and_flicks() {
    let mut ui = laid(long_page(), 200.0, 100.0);
    let t = |phase, y| Input::Touch { id: 3, phase, x: 20.0, y };
    // Down on a button, then dragged up: the page moves and the button is not pressed.
    ui.input(&t(Phase::Down, 90.0));
    ui.input(&t(Phase::Moved, 80.0));
    ui.input(&t(Phase::Moved, 60.0));
    assert_eq!(ui.scroll_offset(), 30.0, "from where it passed the slop");
    std::thread::sleep(std::time::Duration::from_millis(10));
    ui.input(&t(Phase::Moved, 40.0));
    assert!(ui.input(&t(Phase::Up, 40.0)).messages.is_empty());
    assert!(ui.animating(), "it goes on by itself");
    let before = ui.scroll_offset();
    let now = std::time::Instant::now();
    ui.advance(now);
    ui.advance(now + std::time::Duration::from_millis(50));
    assert!(ui.scroll_offset() > before);
    for k in 1..200 {
        ui.advance(now + std::time::Duration::from_millis(50 + 16 * k));
    }
    assert!(!ui.animating(), "and stops");
    // A tap that does not move presses.
    ui.input(&Input::Scroll { x: 20.0, y: 50.0, dx: 0.0, dy: -15000.0 });
    ui.input(&t(Phase::Down, 5.0));
    assert_eq!(ui.input(&t(Phase::Up, 5.0)).messages, vec![Msg::A]);
}

#[test]
fn a_drawing_hears_in_its_own_coordinates() {
    let page = column(vec![space().height(Length::Fixed(30.0)), drawing(|_, _, _, _| {}).on_touch(|t| Some(Msg::Touched(t))).name("Screens")]).width(Length::FILL);
    let mut ui = laid(page, 300.0, 200.0);
    let r = bounds(&ui, "Screens");
    assert_eq!((r.y, r.h), (30.0, 100.0));
    assert_eq!(press(&mut ui, 10.0, 40.0, true), vec![Msg::Touched(Touch::Press { x: 10.0, y: 10.0 })]);
    // Held: it hears the pointer even outside.
    assert_eq!(ui.input(&Input::PointerMoved { x: 50.0, y: 0.0 }).messages, vec![Msg::Touched(Touch::Move { x: 50.0, y: -30.0, pressed: true })]);
    assert_eq!(press(&mut ui, 50.0, 0.0, false), vec![Msg::Touched(Touch::Release { x: 50.0, y: -30.0 })]);
    ui.input(&Input::PointerMoved { x: 20.0, y: 50.0 });
    assert_eq!(ui.input(&Input::PointerLeft).messages, vec![Msg::Touched(Touch::Leave)]);
}

#[test]
fn a_picture_is_drawn_to_fit() {
    use std::sync::Arc;
    use vergence_render::Cpu;
    use vergence_scene::Image;
    use vergence_text::db::FontDb;
    // Two pixels, red then blue; contained in 40 x 40 it is 40 x 20, in the middle.
    let image = Arc::new(Image { width: 2, height: 1, rgba: vec![255, 0, 0, 255, 0, 0, 255, 255] });
    let mut ui = laid(column(vec![picture::<Msg>(image).width(Length::Fixed(40.0)).height(Length::Fixed(40.0))]), 40.0, 40.0);
    let mut scene = Scene::new(40.0, 40.0);
    let mut fonts = Fonts::new(FontDb::default());
    ui.draw(&mut scene, &mut fonts, &Theme::default());
    let p = Cpu::new().render(&scene, &mut fonts, 1.0, vergence_scene::Color::rgb(0, 0, 0)).expect("renders");
    assert!(p.pixel(2, 20)[0] > 200, "red on the left: {:?}", p.pixel(2, 20));
    assert!(p.pixel(37, 20)[2] > 200, "blue on the right: {:?}", p.pixel(37, 20));
    assert_eq!(p.pixel(20, 5), [0, 0, 0, 255], "nothing above it");
}

fn fuzz_page() -> Element<Msg> {
    scroll(
        column(vec![
            label("Title").size(20.0),
            row(vec![button(label("A")).on_press(Msg::A), switch(true, Msg::Toggle), slider(0.0, 1.0, 0.5, Msg::Value).on_release(Msg::Done)]),
            field("text", Msg::Text).on_submit(Msg::Submit),
            choice(vec!["one".into(), "two".into()], Some(0), Msg::Pick),
            drawing(|s, _, r, t| {
                s.fill(r, t.card);
            })
            .on_touch(|t| Some(Msg::Touched(t))),
            scroll(column((0..20).map(|i| label(format!("line {i}"))).collect()).width(Length::FILL)).height(Length::Fixed(60.0)),
            button(label("C")).on_press(Msg::C),
        ])
        .spacing(6.0)
        .width(Length::FILL),
    )
}

#[test]
fn every_widget_drawn_without_fault() {
    use vergence_render::Cpu;
    use vergence_text::db::FontDb;
    let mut ui = laid(fuzz_page(), 400.0, 300.0);
    key(&mut ui, sym::TAB, false);
    key(&mut ui, sym::TAB, false);
    let mut fonts = Fonts::new(FontDb::default());
    for theme in [Theme::default(), Theme::new(false, vergence_scene::Color::rgb(200, 0, 0), true)] {
        let mut scene = Scene::new(400.0, 300.0);
        ui.draw(&mut scene, &mut fonts, &theme);
        scene.validate().expect("a valid scene");
        Cpu::new().render(&scene, &mut fonts, 1.5, theme.page).expect("renders");
    }
}

/// Random input of every kind on a page of every widget: nothing panics,
/// what is remembered stays in bounds, every scene is valid.
/// `FUZZ_ITERS` for a long run.
#[test]
fn fuzz_input_on_every_widget() {
    use vergence_text::db::FontDb;
    let rounds: u64 = std::env::var("FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(20_000);
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut fonts = Fonts::new(FontDb::default());
    let mut ui = laid(fuzz_page(), 400.0, 300.0);
    let keys = [sym::TAB, sym::RETURN, ' ' as u32, sym::LEFT, sym::RIGHT, sym::UP, sym::DOWN, sym::HOME, sym::END, sym::PAGE_UP, sym::PAGE_DOWN, sym::BACKSPACE, sym::DELETE, sym::ESCAPE, 'a' as u32, 'c' as u32, 'v' as u32, 'x' as u32];
    let pastes = ["", "pasted", "two\nlines", "\u{1b}[31m", "e\u{301}\u{e9}"];
    for round in 0..rounds {
        let x = (next() % 500) as f32 - 50.0;
        let y = (next() % 400) as f32 - 50.0;
        let m = Modifiers { shift: next() % 4 == 0, ctrl: next() % 6 == 0, alt: next() % 10 == 0, logo: false };
        let input = match next() % 10 {
            0 => Input::PointerMoved { x, y },
            1 | 2 => Input::Button { button: MouseButton::Left, pressed: next() % 2 == 0, x, y, modifiers: m },
            3 => Input::Scroll { x, y, dx: 0.0, dy: (next() % 200) as f32 - 100.0 },
            4 => Input::Touch { id: (next() % 3) as i32, phase: [Phase::Down, Phase::Moved, Phase::Up, Phase::Cancelled][(next() % 4) as usize], x, y },
            5 | 6 => {
                let k = keys[(next() % keys.len() as u64) as usize];
                Input::Key { sym: k, text: char::from_u32(k).filter(|c| !c.is_control() && k < 0x100).map(|c| c.to_string()), pressed: next() % 3 != 0, repeat: next() % 5 == 0, composing: false, modifiers: m }
            }
            7 => Input::Paste(pastes[(next() % pastes.len() as u64) as usize].into()),
            8 => Input::PointerLeft,
            _ => Input::Focus(next() % 2 == 0),
        };
        let r = ui.input(&input);
        // The program does as it was told, now and then building the tree again.
        if !r.messages.is_empty() && round % 3 == 0 {
            ui.layout(fuzz_page(), 400.0, 300.0, &mut Fixed, 10.0);
        }
        if round % 97 == 0 {
            ui.advance(std::time::Instant::now());
            let _ = ui.meaning();
            let mut scene = Scene::new(400.0, 300.0);
            ui.draw(&mut scene, &mut fonts, &Theme::default());
            scene.validate().expect("a valid scene");
        }
        if let Some((t, a, c)) = ui.field_text() {
            assert!(a <= t.len() && c <= t.len() && t.is_char_boundary(a) && t.is_char_boundary(c));
        }
    }
}

#[test]
fn a_wrapping_row_makes_lines() {
    // Tiles 40 wide with gaps of 10 in 100: two to a line, three lines.
    let tiles = (0..5).map(|i| label(format!("t{i}")).width(Length::Fixed(40.0)).height(Length::Fixed(20.0))).collect();
    let ui = laid(column(vec![row(tiles).spacing(10.0).wrap().width(Length::FILL), label("after")]).width(Length::FILL), 100.0, 300.0);
    assert_eq!(bounds(&ui, "t0"), Rect::new(0.0, 0.0, 40.0, 20.0));
    assert_eq!(bounds(&ui, "t1"), Rect::new(50.0, 0.0, 40.0, 20.0));
    assert_eq!(bounds(&ui, "t2"), Rect::new(0.0, 30.0, 40.0, 20.0));
    assert_eq!(bounds(&ui, "t4"), Rect::new(0.0, 60.0, 40.0, 20.0));
    assert_eq!(bounds(&ui, "after").y, 80.0, "the row is as tall as its lines");
}
