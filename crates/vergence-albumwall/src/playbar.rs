//! The play bar (step 6; the Avalonia app's transport): 82 points, floating
//! over the wall's foot while something is loaded, in the light tone taken
//! from the playing album's cover. The cover (pressed: back to the playing
//! album), title and artist; previous, play or pause, next, and what
//! happens when the album ends; elapsed, the seek bar, remaining; the VU
//! meters.

use std::sync::Arc;

use vergence_albumwall::colors::{Palette, Rgb};
use vergence_albumwall::covers::{self, Covers};
use vergence_albumwall::library::Album;
use vergence_albumwall::panel;
use vergence_scene::{Color, Point, Rect};
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, SliderColors, button, column, drawing, label, panel as boxed, picture,
    row, slider, space, stack,
};

use crate::Msg;
use crate::wallview::color;

/// The bar's height and its cover's size.
pub const HEIGHT: f32 = 82.0;
const COVER: f32 = 58.0;
/// The meters' width.
const METERS: f32 = 214.0;

/// What happens when an album ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    Stop,
    NextAlbum,
    RandomAlbum,
}

impl Ending {
    pub fn next(self) -> Ending {
        match self {
            Ending::Stop => Ending::NextAlbum,
            Ending::NextAlbum => Ending::RandomAlbum,
            Ending::RandomAlbum => Ending::Stop,
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Ending::Stop => "Stop when the album ends",
            Ending::NextAlbum => "Then the next album",
            Ending::RandomAlbum => "Then a random album",
        }
    }
}

/// A glyph drawn in `c`, `s` points square.
#[derive(Clone, Copy)]
pub enum Glyph {
    Previous,
    Play,
    Pause,
    Next,
    Ending(Ending),
}

fn tri(scene: &mut vergence_scene::Scene, x: f32, cy: f32, w: f32, h: f32, right: bool, c: Color) {
    let bands = (h * 2.0).max(8.0) as usize;
    for k in 0..bands {
        let t = (k as f32 + 0.5) / bands as f32;
        let len = w * (1.0 - (2.0 * t - 1.0).abs());
        let y = cy - h / 2.0 + t * h;
        let x0 = if right { x } else { x + w - len };
        scene.fill(Rect::new(x0, y - h / bands as f32 / 2.0, len, h / bands as f32 + 0.05), c);
    }
}

/// The Avalonia app's glyphs (BuildTransport, ShowPlayMode), point for
/// point: paths in a small box, stroked 1.6 wide, the triangles filled.
pub fn glyph(g: Glyph, s: f32, c: Rgb) -> Element<Msg> {
    let c = color(c);
    drawing(move |scene, _, r: Rect, _| {
        // Each path's box, centered in the button.
        let (bw, bh) = match g {
            Glyph::Previous | Glyph::Next => (10.0, 12.0),
            Glyph::Play => (12.0, 14.0),
            Glyph::Pause => (11.0, 14.0),
            Glyph::Ending(Ending::RandomAlbum) => (14.0, 14.0),
            Glyph::Ending(_) => (14.0, 12.0),
        };
        let (x0, y0) = (r.x + (r.w - bw) / 2.0, r.y + (r.h - bh) / 2.0);
        let p = |x: f32, y: f32| Point::new(x0 + x, y0 + y);
        let w = 1.6;
        let line = |scene: &mut vergence_scene::Scene, a: (f32, f32), b: (f32, f32)| {
            scene.line(p(a.0, a.1), p(b.0, b.1), w, c);
        };
        match g {
            Glyph::Previous => {
                line(scene, (1.0, 0.0), (1.0, 12.0));
                tri(scene, x0 + 1.0, y0 + 6.0, 8.0, 12.0, false, c);
            }
            Glyph::Next => {
                line(scene, (9.0, 0.0), (9.0, 12.0));
                tri(scene, x0 + 1.0, y0 + 6.0, 8.0, 12.0, true, c);
            }
            Glyph::Play => tri(scene, x0 + 2.0, y0 + 7.0, 10.0, 14.0, true, c),
            Glyph::Pause => {
                line(scene, (2.0, 0.0), (2.0, 14.0));
                line(scene, (9.0, 0.0), (9.0, 14.0));
            }
            // Stop shows the next album's arrow, dimmed (the caller's color).
            Glyph::Ending(Ending::Stop | Ending::NextAlbum) => {
                line(scene, (0.0, 6.0), (11.0, 6.0));
                line(scene, (7.0, 2.0), (11.0, 6.0));
                line(scene, (11.0, 6.0), (7.0, 10.0));
                line(scene, (14.0, 1.0), (14.0, 11.0));
            }
            Glyph::Ending(Ending::RandomAlbum) => {
                // Crossed arrows, a box 14 tall from y -1 to 13.
                let d = 1.0;
                let pts = [
                    [(0.0, 2.0), (3.0, 2.0), (10.0, 10.0), (14.0, 10.0)],
                    [(0.0, 10.0), (3.0, 10.0), (10.0, 2.0), (14.0, 2.0)],
                ];
                for path in pts {
                    for k in 0..3 {
                        line(scene, (path[k].0, path[k].1 + d), (path[k + 1].0, path[k + 1].1 + d));
                    }
                }
                line(scene, (11.0, 7.0 + d), (14.0, 10.0 + d));
                line(scene, (14.0, 10.0 + d), (11.0, 13.0 + d));
                line(scene, (11.0, -1.0 + d), (14.0, 2.0 + d));
                line(scene, (14.0, 2.0 + d), (11.0, 5.0 + d));
            }
        }
    })
    .width(Length::Fixed(s))
    .height(Length::Fixed(s))
}

/// A round transport button (Button.transport: 32 across, 40 for play, a
/// faint dark disc that darkens under the pointer, dark on the light bar).
fn press(g: Glyph, s: f32, c: Rgb, m: Msg) -> Element<Msg> {
    let look = ButtonLook::Custom { fill: Some(Color::rgba(0, 0, 0, 0x18)), edge: None, radius: s / 2.0 };
    button(glyph(g, s, c)).look(look).padding(0.0).on_press(m)
}

/// `c` at 40% over the bar's ground `g` (the Avalonia button's opacity 0.4).
fn faded(c: Rgb, g: Rgb) -> Rgb {
    let m = |a: u8, b: u8| (a as f32 * 0.4 + b as f32 * 0.6).round() as u8;
    Rgb(m(c.0, g.0), m(c.1, g.1), m(c.2, g.2))
}

/// What the bar shows.
pub struct Shown<'a> {
    pub album: &'a Album,
    pub track: usize,
    pub seconds: f64,
    pub length: f64,
    pub paused: bool,
    pub ending: Ending,
    /// The meters' needles (0 to about 1.05 of their arcs).
    pub needles: [f32; 2],
    /// The seek bar while it is dragged (seconds).
    pub scrub: Option<f64>,
}

/// The bar, `height` tall (it slides in: less while it does).
pub fn bar(s: &Shown, p: &Palette, covers: Option<&Arc<Covers>>, scale: f32) -> Element<Msg> {
    let album = s.album;
    let t = album.tracks.get(s.track);
    let ph = color(vergence_albumwall::wall::placeholder(&album.album_artist, &album.title));
    let cover =
        album.front.as_ref().and_then(|art| covers?.get(&covers::Key::of(art, covers::bucket((COVER * scale).ceil()))));
    let mut face = vec![boxed(space()).color(ph).radius(3.0).width(Length::Fixed(COVER)).height(Length::Fixed(COVER))];
    if let Some(c) = cover {
        face.push(picture(c.image).radius(3.0).width(Length::Fixed(COVER)).height(Length::Fixed(COVER)));
    }
    let face = button(stack(face)).look(ButtonLook::Plain).padding(0.0).on_press(Msg::GoToPlaying);
    let words = column(vec![
        label(t.map_or(String::new(), |t| t.title.clone()))
            .size(16.0)
            .weight(500)
            .color(color(p.on_light))
            .elide()
            .width(Length::FILL),
        label(t.map_or(album.album_artist.clone(), |t| t.artist.clone()))
            .size(11.0)
            .color(color(p.on_light_dim))
            .elide()
            .width(Length::FILL),
    ])
    .spacing(2.0)
    .width(Length::Fixed(260.0));
    let controls = row(vec![
        press(Glyph::Previous, 32.0, p.on_light, Msg::Previous),
        press(if s.paused { Glyph::Play } else { Glyph::Pause }, 40.0, p.on_light, Msg::PlayPause),
        press(Glyph::Next, 32.0, p.on_light, Msg::Next),
        // Dimmed at Stop, lit while the music would go on (ShowPlayMode).
        press(
            Glyph::Ending(s.ending),
            32.0,
            if s.ending == Ending::Stop { faded(p.on_light, p.light) } else { p.on_light },
            Msg::Ending,
        ),
    ])
    .spacing(4.0)
    .align(Align::Center);
    let at = s.scrub.unwrap_or(s.seconds).min(s.length.max(0.0));
    let length = s.length.max(0.001);
    let seek = row(vec![
        // Fixed places for the times (46 and 52, as in the Avalonia bar): a
        // label as wide as its digits moved the seek bar every second.
        label(panel::duration((at * 1000.0) as u64))
            .size(11.0)
            .color(color(p.on_light_dim))
            .align(Align::End)
            .width(Length::Fixed(46.0)),
        slider(0.0, length as f32, at as f32, |v| Msg::Scrub(v as f64))
            .on_release(Msg::SeekDone)
            // Slider.onlight: dark on the light bar (#CC000000 played,
            // #33000000 to come, a near-black thumb with a light edge).
            .slider_colors(SliderColors {
                filled: Color::rgba(0, 0, 0, 0xCC),
                track: Color::rgba(0, 0, 0, 0x33),
                thumb: Color::rgb(0x1A, 0x15, 0x12),
                edge: Color::rgba(0xFF, 0xFF, 0xFF, 0x55),
            })
            .width(Length::FILL),
        label(format!("-{}", panel::duration(((length - at).max(0.0) * 1000.0) as u64)))
            .size(11.0)
            .color(color(p.on_light_dim))
            .width(Length::Fixed(52.0)),
    ])
    .spacing(12.0)
    .align(Align::Center)
    .width(Length::FILL);
    boxed(
        row(vec![face, words, controls, seek, crate::vumeter::pair(s.needles, METERS, HEIGHT - 20.0)])
            .spacing(18.0)
            .align(Align::Center)
            .width(Length::FILL)
            .height(Length::FILL),
    )
    .padding(Padding::hv(12.0, 18.0))
    .color(color(p.light))
    .radius(0.0)
    .width(Length::FILL)
    .height(Length::Fixed(HEIGHT))
}
