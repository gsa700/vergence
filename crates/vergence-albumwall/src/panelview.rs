//! The open album on screen (step 5): its panel under its row, in its own
//! colors: the arrow pointing at its tile, the body fading into the wall
//! at its foot, the title, artist and header line, the track list in two
//! columns (a track pressed plays from it, step 6), the 260-point sleeve
//! that is the play button (a translucent play overlay under the pointer).

use std::sync::Arc;

use vergence_albumwall::colors::{Palette, Rgb};
use vergence_albumwall::covers::{self, Covers};
use vergence_albumwall::library::Album;
use vergence_albumwall::panel::{self, Line};
use vergence_albumwall::wall;
use vergence_scene::{Color, Point, Rect};
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, button, column, drawing, label, panel as boxed, picture, row, space,
    stack,
};

use crate::Msg;
use crate::wallview::color;

fn alpha(c: Rgb, a: u8) -> Color {
    Color::rgba(c.0, c.1, c.2, a)
}

/// The panel's ground: the arrow over the tile at `item`'s column (worked
/// out from the width the grid gives it, as the grid does), the body in
/// the album's color, solid to 72% of its height, then fading to nothing.
fn ground(item: usize, p: &Palette) -> Element<Msg> {
    let c = p.panel;
    drawing(move |scene, _, r: Rect, _| {
        let (cols, cover) = wall::columns(r.w);
        let col = item % cols.max(1);
        let cx = r.x + col as f32 * (cover + wall::GAP) + cover / 2.0;
        // The arrow, a triangle point up, as thin bands.
        let bands = 22;
        for k in 0..bands {
            let t = (k as f32 + 0.5) / bands as f32;
            let half = panel::ARROW_W / 2.0 * t;
            let y = r.y + panel::MARGIN - panel::ARROW_H + t * panel::ARROW_H;
            scene.fill(Rect::new(cx - half, y, half * 2.0, panel::ARROW_H / bands as f32 + 0.1), color(c));
        }
        // The body: solid, then fading to the wall over its last 28%.
        let top = r.y + panel::MARGIN;
        let h = (r.h - panel::MARGIN).max(0.0);
        let solid = h * 0.72;
        scene.fill_rounded(Rect::new(r.x, top, r.w, solid.min(h)), 6.0, color(c));
        let steps = 24;
        let fade = h - solid;
        for k in 0..steps {
            let a = (255.0 * (1.0 - (k as f32 + 0.5) / steps as f32)) as u8;
            let y = top + solid + fade * k as f32 / steps as f32;
            scene.fill(Rect::new(r.x, y, r.w, fade / steps as f32 + 0.1), alpha(c, a));
        }
    })
    .width(Length::FILL)
    .height(Length::FILL)
}

/// One line of the track list.
fn line(album_index: usize, album: &Album, l: &Line, p: &Palette, text: f32, playing: Option<usize>) -> Element<Msg> {
    match l {
        Line::Disc(words) => boxed(label(words.clone()).size(11.0).weight(700).color(color(p.accent)))
            .padding(Padding { top: 14.0, right: 0.0, bottom: 5.0, left: 0.0 })
            .radius(0.0),
        Line::Track(i) => {
            let t = &album.tracks[*i];
            let number = if t.number > 0 { t.number.to_string() } else { String::new() };
            let num_w = (panel::NUMBER * text).ceil() - 10.0;
            // The track coming out of the speakers: a ▶ where its number was,
            // pulsing slowly, and its title in semibold.
            let here = playing == Some(*i);
            let mark: Element<Msg> = if here {
                let pulse = 0.55
                    + 0.45
                        * (std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0.0, |d| d.as_secs_f32())
                            * std::f32::consts::PI)
                            .sin()
                            .abs();
                let c = color(p.accent);
                row(vec![
                    space().width(Length::FILL),
                    drawing(move |scene, _, r: Rect, _| {
                        let (h, w) = (9.0, 8.0);
                        let cy = r.y + r.h / 2.0;
                        for k in 0..18 {
                            let t = (k as f32 + 0.5) / 18.0;
                            let len = w * (1.0 - (2.0 * t - 1.0).abs());
                            scene.fill(
                                Rect::new(r.x + r.w - w, cy - h / 2.0 + t * h, len, h / 18.0 + 0.05),
                                Color::rgba(c.r, c.g, c.b, (255.0 * pulse) as u8),
                            );
                        }
                    })
                    .width(Length::Fixed(10.0))
                    .height(Length::Fixed(12.0)),
                ])
                .width(Length::Fixed(num_w))
            } else {
                label(number).size(12.0).color(color(p.text_dim)).align(Align::End).width(Length::Fixed(num_w))
            };
            button(
                row(vec![
                    mark,
                    space().width(Length::Fixed(10.0)),
                    label(t.title.clone())
                        .size(14.0)
                        .weight(if here { 600 } else { 400 })
                        .color(color(p.text))
                        .elide()
                        .width(Length::FILL),
                    label(panel::duration(t.millis)).size(12.0).color(color(p.text_dim)),
                ])
                .align(Align::Center)
                .width(Length::FILL)
                .height(Length::Fixed((panel::LINE * text).ceil())),
            )
            .look(ButtonLook::Plain)
            .padding(0.0)
            .width(Length::FILL)
            .on_press(Msg::Play(album_index, *i))
        }
    }
}

/// The sleeve: the cover at 260 points, the play button; under the pointer
/// a translucent wash, a disc and a triangle.
fn sleeve(album_index: usize, album: &Album, covers: Option<&Covers>, scale: f32) -> Element<Msg> {
    let s = panel::SLEEVE;
    let ph = color(wall::placeholder(&album.album_artist, &album.title));
    let px = (s * scale).ceil();
    let cover = album.front.as_ref().and_then(|art| covers?.get(&covers::Key::of(art, covers::bucket(px))));
    let mut layers = vec![boxed(space()).color(ph).radius(3.0).width(Length::Fixed(s)).height(Length::Fixed(s))];
    if let Some(c) = cover {
        layers.push(picture(c.image).radius(3.0).width(Length::Fixed(s)).height(Length::Fixed(s)));
    }
    layers.push(
        drawing(move |scene, _, r: Rect, _| {
            scene.fill_rounded(r, 3.0, Color::rgba(0, 0, 0, 46));
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            scene.fill_rounded(Rect::new(cx - 44.0, cy - 44.0, 88.0, 88.0), 44.0, Color::rgba(0, 0, 0, 89));
            // The triangle, 27 wide and 32 tall, nudged right as the eye expects.
            let bands = 32;
            for k in 0..bands {
                let y = cy - 16.0 + k as f32;
                let t = 1.0 - ((k as f32 + 0.5) - 16.0).abs() / 16.0;
                scene.fill(Rect::new(cx - 13.5 + 3.5, y, 27.0 * t, 1.05), Color::rgba(255, 255, 255, 232));
            }
            let _ = Point::new(0.0, 0.0);
        })
        .width(Length::Fixed(s))
        .height(Length::Fixed(s))
        .shown_on_hover(),
    );
    button(stack(layers)).look(ButtonLook::Plain).padding(0.0).on_press(Msg::Play(album_index, 0))
}

/// The panel's contents: the header, the track list, the sleeve.
#[allow(clippy::too_many_arguments)]
fn contents(
    i: usize,
    album: &Album,
    p: &Palette,
    covers: Option<&Covers>,
    scale: f32,
    text: f32,
    playing: Option<usize>,
) -> Element<Msg> {
    let (left, right) = panel::columns(album);
    let col = |lines: &[Line]| -> Element<Msg> {
        column(lines.iter().map(|l| line(i, album, l, p, text, playing)).collect()).width(Length::FILL)
    };
    let words = column(vec![
        label(album.title.clone()).size(21.0).weight(700).color(color(p.text)).elide().width(Length::FILL),
        label(album.album_artist.clone()).size(14.0).color(color(p.text_dim)).elide().width(Length::FILL),
        boxed(label(panel::header_line(album)).size(11.0).color(color(p.text_dim)).elide().width(Length::FILL))
            .padding(Padding { top: 4.0, right: 0.0, bottom: 0.0, left: 0.0 })
            .radius(0.0)
            .width(Length::FILL),
        space().height(Length::Fixed(panel::BUTTON_GAP + (panel::BUTTON_ROW * text).round())),
        boxed(row(vec![col(&left), col(&right)]).spacing(panel::COLUMN_GAP).width(Length::FILL))
            .padding(Padding { top: panel::LIST_GAP, right: 0.0, bottom: 0.0, left: 0.0 })
            .radius(0.0)
            .width(Length::FILL),
    ])
    .spacing(3.0)
    .width(Length::FILL);
    boxed(row(vec![words, sleeve(i, album, covers, scale)]).spacing(panel::GAP).width(Length::FILL))
        .padding(Padding { top: panel::MARGIN + panel::PAD, right: panel::PAD, bottom: panel::FOOT, left: panel::PAD })
        .radius(0.0)
        .width(Length::FILL)
        .height(Length::FILL)
}

/// The open album's panel: its ground always, its contents at `shown`
/// opacity (0 while it unfolds, then fading in; and while contents swap).
#[allow(clippy::too_many_arguments)]
pub fn open(
    i: usize,
    album: &Album,
    p: &Palette,
    covers: Option<Arc<Covers>>,
    scale: f32,
    text: f32,
    shown: f32,
    playing: Option<usize>,
) -> Element<Msg> {
    let mut layers = vec![ground(i, p)];
    if shown > 0.0 {
        layers.push(contents(i, album, p, covers.as_deref(), scale, text, playing).opacity(shown));
    }
    stack(layers).width(Length::FILL).height(Length::FILL)
}
