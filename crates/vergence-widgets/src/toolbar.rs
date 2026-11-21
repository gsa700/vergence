//! The toolbar (files milestone 1, step 7; Dave, 2026-11-15: "b"): a strip
//! right under the title bar the server draws, the same in every app. It
//! reaches up under the bar (the window draws under its bar), so the bar's
//! frost and the strip are one band of one color; the app's content starts
//! below it. Its tools are icon buttons from one set drawn here ([`Icon`]),
//! a word on hover by their names, chosen ones shaded; groups sit together;
//! a flexible space pushes what follows to the end; any element (a path, a
//! search field) can stand in it.

use vergence_scene::{Color, Point, Rect};

use crate::{Align, ButtonLook, Element, Length, Padding, Theme, button, drawing, panel, row, space};

/// The strip's height below the bar, and a tool's square.
pub const HEIGHT: f32 = 40.0;
pub const TOOL: f32 = 30.0;

/// The toolbar's icons, drawn as lines on a 16-unit square.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Back,
    Forward,
    Up,
    List,
    Grid,
    /// The preview pane (a window with a pane at its right).
    Pane,
    Search,
    NewFolder,
    ZoomIn,
    ZoomOut,
    /// Fit to the window (four corners pointing in).
    Fit,
    /// Actual size (1:1).
    Actual,
    TurnLeft,
    TurnRight,
    Previous,
    Next,
    Info,
    Trash,
    Open,
    Hidden,
    /// Put back where it was (out of the Trash): an arrow turning back.
    PutBack,
    /// Remove for good: a cross.
    Remove,
    /// Fit to the width (arrows out to the sides).
    FitWidth,
    /// One page at a time.
    OnePage,
    /// Pages in a column, scrolled (continuous).
    Continuous,
    /// Two pages side by side (a book's spread).
    TwoPages,
    /// The hand: moving what is shown by dragging it.
    Hand,
    /// Selecting text: an I-beam.
    TextSelect,
}

/// An icon at `size` points, in the theme's text color (dim when `dim`).
pub fn icon<M: 'static>(which: Icon, size: f32, dim: bool) -> Element<M> {
    drawing(move |scene, _, r: Rect, t: &Theme| {
        let ink = if dim { Color { a: 90, ..t.text } } else { t.text };
        let k = size / 16.0;
        let (ox, oy) = (r.x + (r.w - size) / 2.0, r.y + (r.h - size) / 2.0);
        let p = |x: f32, y: f32| Point::new(ox + x * k, oy + y * k);
        let w = 1.4f32.max(1.25 * k);
        macro_rules! line {
            ($a:expr, $b:expr) => {{
                let (a, b): ((f32, f32), (f32, f32)) = ($a, $b);
                scene.line(p(a.0, a.1), p(b.0, b.1), w, ink);
            }};
        }
        let rect = |x: f32, y: f32, ww: f32, hh: f32| Rect::new(ox + x * k, oy + y * k, ww * k, hh * k);
        match which {
            Icon::Back | Icon::Previous => {
                line!((10.5, 3.0), (5.5, 8.0));
                line!((5.5, 8.0), (10.5, 13.0));
            }
            Icon::Forward | Icon::Next => {
                line!((5.5, 3.0), (10.5, 8.0));
                line!((10.5, 8.0), (5.5, 13.0));
            }
            Icon::Up => {
                line!((3.0, 10.5), (8.0, 5.5));
                line!((8.0, 5.5), (13.0, 10.5));
            }
            Icon::List => {
                for y in [4.0, 8.0, 12.0] {
                    line!((2.5, y), (3.5, y));
                    line!((5.5, y), (13.5, y));
                }
            }
            Icon::Grid => {
                for (x, y) in [(2.5, 2.5), (9.0, 2.5), (2.5, 9.0), (9.0, 9.0)] {
                    scene.stroke(rect(x, y, 4.5, 4.5), 1.0 * k, w, ink);
                }
            }
            Icon::Pane => {
                scene.stroke(rect(2.0, 3.0, 12.0, 10.0), 1.5 * k, w, ink);
                line!((9.5, 3.0), (9.5, 13.0));
            }
            Icon::Search => {
                scene.stroke(rect(2.5, 2.5, 8.0, 8.0), 4.0 * k, w, ink);
                line!((9.3, 9.3), (13.5, 13.5));
            }
            Icon::NewFolder => {
                line!((2.0, 4.0), (6.0, 4.0));
                line!((6.0, 4.0), (7.5, 5.5));
                line!((7.5, 5.5), (14.0, 5.5));
                line!((14.0, 5.5), (14.0, 13.0));
                line!((14.0, 13.0), (2.0, 13.0));
                line!((2.0, 13.0), (2.0, 4.0));
                line!((8.0, 7.5), (8.0, 11.5));
                line!((6.0, 9.5), (10.0, 9.5));
            }
            Icon::ZoomIn | Icon::ZoomOut => {
                scene.stroke(rect(2.0, 2.0, 9.0, 9.0), 4.5 * k, w, ink);
                line!((9.8, 9.8), (14.0, 14.0));
                line!((4.5, 6.5), (8.5, 6.5));
                if which == Icon::ZoomIn {
                    line!((6.5, 4.5), (6.5, 8.5));
                }
            }
            Icon::Fit => {
                for (sx, sy) in [(1.0f32, 1.0f32), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                    let (cx, cy) = (8.0 - sx * 5.5, 8.0 - sy * 5.5);
                    line!((cx, cy), (cx + sx * 3.0, cy));
                    line!((cx, cy), (cx, cy + sy * 3.0));
                }
            }
            Icon::Actual => {
                line!((4.0, 4.5), (5.5, 3.5));
                line!((5.5, 3.5), (5.5, 12.5));
                line!((10.0, 4.5), (11.5, 3.5));
                line!((11.5, 3.5), (11.5, 12.5));
                scene.fill_rounded(rect(7.6, 6.0, 1.1, 1.1), 0.55 * k, ink);
                scene.fill_rounded(rect(7.6, 9.5, 1.1, 1.1), 0.55 * k, ink);
            }
            Icon::TurnLeft | Icon::TurnRight => {
                // A rounded arrow: three sides of a square, the head at the open end.
                let m = if which == Icon::TurnRight { -1.0 } else { 1.0 };
                let x = |v: f32| if m < 0.0 { 16.0 - v } else { v };
                line!((x(12.5), 13.0), (x(12.5), 6.0));
                line!((x(12.5), 6.0), (x(10.5), 4.0));
                line!((x(10.5), 4.0), (x(5.0), 4.0));
                line!((x(5.0), 4.0), (x(7.5), 1.5));
                line!((x(5.0), 4.0), (x(7.5), 6.5));
                line!((x(3.5), 9.0), (x(3.5), 13.0));
                line!((x(3.5), 13.0), (x(9.5), 13.0));
                line!((x(9.5), 13.0), (x(9.5), 9.0));
                line!((x(9.5), 9.0), (x(3.5), 9.0));
            }
            Icon::Info => {
                scene.stroke(rect(1.5, 1.5, 13.0, 13.0), 6.5 * k, w, ink);
                line!((8.0, 7.0), (8.0, 11.5));
                scene.fill_rounded(rect(7.4, 4.0, 1.2, 1.2), 0.6 * k, ink);
            }
            Icon::Trash => {
                line!((2.5, 4.0), (13.5, 4.0));
                line!((6.0, 4.0), (6.5, 2.0));
                line!((6.5, 2.0), (9.5, 2.0));
                line!((9.5, 2.0), (10.0, 4.0));
                line!((4.0, 4.0), (5.0, 14.0));
                line!((5.0, 14.0), (11.0, 14.0));
                line!((11.0, 14.0), (12.0, 4.0));
            }
            Icon::Open => {
                line!((2.0, 4.0), (6.0, 4.0));
                line!((6.0, 4.0), (7.5, 5.5));
                line!((7.5, 5.5), (12.0, 5.5));
                line!((12.0, 5.5), (12.0, 7.0));
                line!((2.0, 4.0), (2.0, 13.0));
                line!((2.0, 13.0), (12.5, 13.0));
                line!((12.5, 13.0), (14.5, 7.0));
                line!((14.5, 7.0), (4.5, 7.0));
                line!((4.5, 7.0), (2.0, 13.0));
            }
            Icon::Hidden => {
                // An eye.
                line!((1.5, 8.0), (5.0, 4.5));
                line!((5.0, 4.5), (11.0, 4.5));
                line!((11.0, 4.5), (14.5, 8.0));
                line!((14.5, 8.0), (11.0, 11.5));
                line!((11.0, 11.5), (5.0, 11.5));
                line!((5.0, 11.5), (1.5, 8.0));
                scene.stroke(rect(6.0, 6.0, 4.0, 4.0), 2.0 * k, w, ink);
            }
            Icon::PutBack => {
                // The head pointing left at the top, the shaft round and down.
                line!((2.5, 6.0), (5.5, 3.0));
                line!((2.5, 6.0), (5.5, 9.0));
                line!((2.5, 6.0), (10.0, 6.0));
                line!((10.0, 6.0), (12.5, 7.5));
                line!((12.5, 7.5), (13.5, 10.0));
                line!((13.5, 10.0), (12.5, 12.5));
                line!((12.5, 12.5), (10.0, 13.5));
                line!((10.0, 13.5), (6.0, 13.5));
            }
            Icon::Remove => {
                line!((4.0, 4.0), (12.0, 12.0));
                line!((12.0, 4.0), (4.0, 12.0));
            }
            Icon::FitWidth => {
                line!((1.5, 8.0), (14.5, 8.0));
                line!((1.5, 8.0), (4.0, 5.5));
                line!((1.5, 8.0), (4.0, 10.5));
                line!((14.5, 8.0), (12.0, 5.5));
                line!((14.5, 8.0), (12.0, 10.5));
                line!((1.5, 3.0), (1.5, 13.0));
                line!((14.5, 3.0), (14.5, 13.0));
            }
            Icon::OnePage => {
                scene.stroke(rect(4.0, 2.0, 8.0, 12.0), 1.0 * k, w, ink);
            }
            Icon::Continuous => {
                scene.stroke(rect(4.0, 1.0, 8.0, 6.0), 1.0 * k, w, ink);
                scene.stroke(rect(4.0, 9.0, 8.0, 6.0), 1.0 * k, w, ink);
            }
            Icon::TwoPages => {
                scene.stroke(rect(1.5, 3.0, 6.0, 10.0), 1.0 * k, w, ink);
                scene.stroke(rect(8.5, 3.0, 6.0, 10.0), 1.0 * k, w, ink);
            }
            Icon::Hand => {
                // Four fingers up, the thumb out to the left, the palm below.
                for (x, top) in [(5.5, 4.0), (7.5, 2.5), (9.5, 3.0), (11.5, 4.5)] {
                    line!((x, top), (x, 9.0));
                }
                line!((5.5, 9.0), (3.0, 7.5));
                line!((3.0, 7.5), (5.0, 12.0));
                line!((5.0, 12.0), (7.0, 14.0));
                line!((7.0, 14.0), (11.0, 14.0));
                line!((11.0, 14.0), (12.5, 11.5));
                line!((12.5, 11.5), (11.5, 9.0));
                line!((11.5, 4.5), (12.5, 11.5));
            }
            Icon::TextSelect => {
                line!((8.0, 3.0), (8.0, 13.0));
                line!((5.5, 2.5), (7.0, 3.0));
                line!((7.0, 3.0), (9.0, 3.0));
                line!((9.0, 3.0), (10.5, 2.5));
                line!((5.5, 13.5), (7.0, 13.0));
                line!((7.0, 13.0), (9.0, 13.0));
                line!((9.0, 13.0), (10.5, 13.5));
            }
        }
    })
    .width(Length::Fixed(size))
    .height(Length::Fixed(size))
}

/// A tool: its icon in a square button, named for the screen reader and the
/// hover word; None for a tool that cannot be used now (drawn faint).
pub fn tool<M: Clone + 'static>(which: Icon, name: &str, on: Option<M>) -> Element<M> {
    let enabled = on.is_some();
    let b = button(icon(which, 16.0, !enabled)).look(ButtonLook::Plain).padding(Padding::all(0.0)).width(Length::Fixed(TOOL)).height(Length::Fixed(TOOL)).name(name);
    match on {
        Some(m) => b.on_press(m),
        None => b,
    }
}

/// A tool that is on or off (the list or icon view, the preview pane): shaded while on.
pub fn toggle<M: Clone + 'static>(which: Icon, name: &str, on: bool, press: M) -> Element<M> {
    button(icon(which, 16.0, false)).look(if on { ButtonLook::Chosen } else { ButtonLook::Plain }).padding(Padding::all(0.0)).width(Length::Fixed(TOOL)).height(Length::Fixed(TOOL)).on_press(press).name(name)
}

/// Tools that belong together, close side by side.
pub fn group<M: 'static>(items: Vec<Element<M>>) -> Element<M> {
    row(items).spacing(2.0).align(Align::Center)
}

/// The rest of the strip's width: what follows goes to the end.
pub fn flexible<M: 'static>() -> Element<M> {
    space().width(Length::FILL)
}

/// The strip itself, the first thing in a window that draws under its bar
/// (`Program::under_bar`): reaching up under the bar by `t.top`, so its
/// color and the bar's frost are one band, then `HEIGHT` of tools.
pub fn toolbar<M: 'static>(items: Vec<Element<M>>, t: &Theme) -> Element<M> {
    let shade = Color { a: if t.dark { 26 } else { 16 }, ..t.text };
    panel(row(items).spacing(10.0).align(Align::Center).width(Length::FILL).height(Length::Fixed(HEIGHT)))
        .color(shade)
        .radius(0.0)
        .padding(Padding { top: t.top, bottom: 0.0, left: 10.0, right: 10.0 })
        .width(Length::FILL)
}
