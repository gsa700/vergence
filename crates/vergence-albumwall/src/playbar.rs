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
    Align, ButtonLook, Element, Length, Padding, button, column, drawing, label, panel as boxed, picture, row, slider,
    space, stack,
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

pub fn glyph(g: Glyph, s: f32, c: Rgb) -> Element<Msg> {
    let c = color(c);
    drawing(move |scene, _, r: Rect, _| {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let u = s / 24.0;
        match g {
            Glyph::Play => tri(scene, cx - 7.0 * u, cy, 16.0 * u, 18.0 * u, true, c),
            Glyph::Pause => {
                scene.fill_rounded(Rect::new(cx - 7.0 * u, cy - 9.0 * u, 5.0 * u, 18.0 * u), 1.0, c);
                scene.fill_rounded(Rect::new(cx + 2.0 * u, cy - 9.0 * u, 5.0 * u, 18.0 * u), 1.0, c);
            }
            Glyph::Next => {
                tri(scene, cx - 8.0 * u, cy, 12.0 * u, 14.0 * u, true, c);
                scene.fill(Rect::new(cx + 4.0 * u, cy - 7.0 * u, 2.5 * u, 14.0 * u), c);
            }
            Glyph::Previous => {
                scene.fill(Rect::new(cx - 6.5 * u, cy - 7.0 * u, 2.5 * u, 14.0 * u), c);
                tri(scene, cx - 4.0 * u, cy, 12.0 * u, 14.0 * u, false, c);
            }
            Glyph::Ending(e) => {
                // Stop: a square; next album: two arrows; random: crossed lines.
                match e {
                    Ending::Stop => {
                        scene.fill_rounded(Rect::new(cx - 5.0 * u, cy - 5.0 * u, 10.0 * u, 10.0 * u), 1.5, c);
                    }
                    Ending::NextAlbum => {
                        tri(scene, cx - 9.0 * u, cy, 9.0 * u, 12.0 * u, true, c);
                        tri(scene, cx, cy, 9.0 * u, 12.0 * u, true, c);
                    }
                    Ending::RandomAlbum => {
                        scene.line(
                            Point::new(cx - 8.0 * u, cy - 6.0 * u),
                            Point::new(cx + 8.0 * u, cy + 6.0 * u),
                            2.0 * u,
                            c,
                        );
                        scene.line(
                            Point::new(cx - 8.0 * u, cy + 6.0 * u),
                            Point::new(cx + 8.0 * u, cy - 6.0 * u),
                            2.0 * u,
                            c,
                        );
                    }
                };
            }
        }
    })
    .width(Length::Fixed(s))
    .height(Length::Fixed(s))
}

fn press(g: Glyph, s: f32, c: Rgb, m: Msg) -> Element<Msg> {
    button(glyph(g, s, c)).look(ButtonLook::Plain).padding(4.0).on_press(m)
}

/// The VU meters: two needles swinging on arcs, as a pair of analog meters
/// (the levels already have VU ballistics: VuMeter.cs's 300 ms).
fn meters(levels: [f32; 2], c: Rgb) -> Element<Msg> {
    let (ink, face) = (color(c), Color::rgba(c.0, c.1, c.2, 40));
    drawing(move |scene, _, r: Rect, _| {
        let w = (r.w - 10.0) / 2.0;
        for (i, level) in levels.iter().enumerate() {
            let x = r.x + i as f32 * (w + 10.0);
            let face_r = Rect::new(x, r.y, w, r.h);
            scene.fill_rounded(face_r, 4.0, face);
            // The scale: -20 VU at the left, 0 at 70%, +3 at the right, in dB.
            let pivot = Point::new(x + w / 2.0, r.y + r.h - 4.0);
            let len = r.h - 12.0;
            let db = if *level > 0.0 { 20.0 * level.log10() + 18.0 } else { -40.0 };
            let t = ((db + 20.0) / 23.0).clamp(0.0, 1.0);
            let angle = (-50.0 + 100.0 * t).to_radians();
            for k in 0..=10 {
                let a = (-50.0 + 10.0 * k as f32).to_radians();
                let p1 = Point::new(pivot.x + a.sin() * (len - 4.0), pivot.y - a.cos() * (len - 4.0));
                let p2 = Point::new(pivot.x + a.sin() * len, pivot.y - a.cos() * len);
                scene.line(p1, p2, if k >= 7 { 1.6 } else { 1.0 }, ink);
            }
            let tip = Point::new(pivot.x + angle.sin() * (len - 2.0), pivot.y - angle.cos() * (len - 2.0));
            scene.line(pivot, tip, 1.4, ink);
        }
    })
    .width(Length::Fixed(METERS))
    .height(Length::Fixed(HEIGHT - 26.0))
}

/// What the bar shows.
pub struct Shown<'a> {
    pub album: &'a Album,
    pub track: usize,
    pub seconds: f64,
    pub length: f64,
    pub paused: bool,
    pub ending: Ending,
    pub levels: [f32; 2],
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
            .weight(600)
            .color(color(p.on_light))
            .elide()
            .width(Length::FILL),
        label(t.map_or(album.album_artist.clone(), |t| t.artist.clone()))
            .size(12.0)
            .color(color(p.on_light_dim))
            .elide()
            .width(Length::FILL),
    ])
    .spacing(2.0)
    .width(Length::Fixed(260.0));
    let controls = row(vec![
        press(Glyph::Previous, 24.0, p.on_light, Msg::Previous),
        press(if s.paused { Glyph::Play } else { Glyph::Pause }, 30.0, p.on_light, Msg::PlayPause),
        press(Glyph::Next, 24.0, p.on_light, Msg::Next),
        press(Glyph::Ending(s.ending), 22.0, p.on_light_dim, Msg::Ending),
    ])
    .spacing(2.0)
    .align(Align::Center);
    let at = s.scrub.unwrap_or(s.seconds).min(s.length.max(0.0));
    let length = s.length.max(0.001);
    let seek = row(vec![
        label(panel::duration((at * 1000.0) as u64)).size(11.0).color(color(p.on_light_dim)),
        slider(0.0, length as f32, at as f32, |v| Msg::Scrub(v as f64)).on_release(Msg::SeekDone).width(Length::FILL),
        label(format!("-{}", panel::duration(((length - at).max(0.0) * 1000.0) as u64)))
            .size(11.0)
            .color(color(p.on_light_dim)),
    ])
    .spacing(10.0)
    .align(Align::Center)
    .width(Length::FILL);
    boxed(
        row(vec![face, words, controls, seek, meters(s.levels, p.on_light)])
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
