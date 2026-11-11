// iced spike: title bar with tabs, two panes of colored monospace text (iced's
// own pane grid, so splits come built in), a right-click menu.

use iced::widget::{button, column, container, mouse_area, pane_grid, rich_text, row, span, stack, text, Space};
use iced::{Color, Element, Font, Length, Padding, Point, Task, Theme};

#[derive(Debug, Clone)]
enum Message {
    Tab(usize),
    Drag,
    Close,
    Resized(pane_grid::ResizeEvent),
    Moved(Point),
    RightClick,
    Pick(&'static str),
}

struct Spike {
    panes: pane_grid::State<()>,
    tab: usize,
    cursor: Point,
    menu: Option<Point>,
}

impl Spike {
    fn new() -> Self {
        let (mut panes, first) = pane_grid::State::new(());
        let _ = panes.split(pane_grid::Axis::Vertical, first, ());
        eprintln!("iced-spike: state built");
        Spike { panes, tab: 0, cursor: Point::ORIGIN, menu: None }
    }

    fn update(&mut self, m: Message) -> Task<Message> {
        match m {
            Message::Tab(i) => self.tab = i,
            Message::Drag => return iced::window::latest().and_then(iced::window::drag),
            Message::Close => return iced::exit(),
            Message::Resized(e) => self.panes.resize(e.split, e.ratio),
            Message::Moved(p) => self.cursor = p,
            Message::RightClick => self.menu = Some(self.cursor),
            Message::Pick(_) => self.menu = None,
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let tab = |i: usize, t: &'static str| {
            button(text(t).size(12)).width(160).on_press(Message::Tab(i)).style(move |th: &Theme, s| {
                let mut st = button::secondary(th, s);
                if i != self.tab {
                    st.background = None;
                }
                st
            })
        };
        let bar = mouse_area(
            container(
                row![tab(0, "~/src/vergence"), tab(1, "htop"), button(text("+")).style(button::text), Space::new().width(Length::Fill), button(text("\u{2715}")).style(button::text).on_press(Message::Close)]
                    .spacing(4)
                    .padding(4),
            )
            .style(|_| container::background(Color::from_rgb8(0x18, 0x18, 0x25))),
        )
        .on_press(Message::Drag);
        let grid = pane_grid::PaneGrid::new(&self.panes, |_, _, _| pane_grid::Content::new(pane()))
            .on_resize(10, Message::Resized)
            .spacing(4);
        let body = mouse_area(grid).on_right_press(Message::RightClick).on_move(Message::Moved);
        let mut layers = stack![column![bar, body]];
        if let Some(p) = self.menu {
            let item = |t: &'static str| button(text(t).size(13)).width(200).style(button::text).on_press(Message::Pick(t));
            let menu = container(column![item("Copy"), item("Paste"), item("Split side by side"), item("Split top and bottom"), item("Close pane")])
                .padding(4)
                .style(container::rounded_box);
            layers = layers.push(container(menu).padding(Padding { top: p.y + 36.0, left: p.x, right: 0.0, bottom: 0.0 }));
        }
        layers.into()
    }
}

fn pane<'a>() -> Element<'a, Message> {
    let (g, b, y, w, m) = (
        Color::from_rgb8(0xa6, 0xe3, 0xa1),
        Color::from_rgb8(0x89, 0xb4, 0xfa),
        Color::from_rgb8(0xf9, 0xe2, 0xaf),
        Color::from_rgb8(0xcd, 0xd6, 0xf4),
        Color::from_rgb8(0xcb, 0xa6, 0xf7),
    );
    let s = |t: String, c: Color| -> iced::advanced::text::Span<'a, ()> { span(t).color(c).font(Font::MONOSPACE).size(13) };
    let prompt = |cmd: &str| rich_text![s("derickson".into(), g), s("@".into(), w), s("hubble".into(), g), s(" ~/src/vergence $ ".into(), b), s(cmd.into(), w)];
    let mut lines: Vec<Element<'a, Message>> = vec![prompt("ls -l crates").into()];
    for n in ["vergd", "vergence-dbus", "vergence-edid", "vergence-protocol", "vergence-server", "vergence-settings"] {
        lines.push(rich_text![s("drwxr-xr-x  4 derickson derickson  6 Nov  8 13:40 ".into(), w), s(n.into(), b)].into());
    }
    lines.push(prompt("cargo build --release").into());
    for i in 0..14 {
        lines.push(rich_text![s("   Compiling ".into(), g), s(format!("crate-number-{i} v0.{i}.0"), w)].into());
    }
    lines.push(rich_text![s("    Finished ".into(), g), s("`release` profile [optimized] target(s) in 41.02s".into(), w)].into());
    lines.push(rich_text![s("warning: ".into(), y), s("unused variable `x`".into(), w), s("  --> src/main.rs:12:9".into(), m)].into());
    container(column(lines))
        .padding(8)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::background(Color::from_rgb8(0x1e, 0x1e, 0x2e)))
        .into()
}

fn main() -> iced::Result {
    iced::application(Spike::new, Spike::update, Spike::view)
        .title("iced spike")
        .window(iced::window::Settings { size: iced::Size::new(900.0, 560.0), decorations: false, ..Default::default() })
        .theme(|_: &Spike| Theme::CatppuccinMocha)
        .run()
}
