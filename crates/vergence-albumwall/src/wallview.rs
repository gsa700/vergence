//! The wall on screen (step 4): its tiles in the toolkit's grid (only the
//! rows in view built), each cover from the cache or its placeholder color
//! until it comes, the title and artist under it; the bottom bar with the
//! status line, the counts and the library picker. The wall's ground and
//! the bars take the library's hue (colors.rs).

use std::rc::Rc;
use std::sync::Arc;

use vergence_albumwall::colors::{Ground, Rgb};
use vergence_albumwall::covers::{self, Covers};
use vergence_albumwall::library::Album;
use vergence_albumwall::wall;
use vergence_scene::Color;
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Theme, button, choice, column, grid, label, panel, picture, row,
    scroll, space, stack,
};

use crate::Msg;

pub fn color(c: Rgb) -> Color {
    Color::rgb(c.0, c.1, c.2)
}

/// The bottom bar's height (AlbumWall's 44).
pub const BAR: f32 = 44.0;

/// One tile: the cover (or its placeholder), the title and the artist.
fn tile(album: &Album, i: usize, w: f32, covers: Option<&Covers>, scale: f32, g: &Ground) -> Element<Msg> {
    let ph = color(wall::placeholder(&album.album_artist, &album.title));
    let px = (w * scale).ceil();
    let cover = album.front.as_ref().and_then(|art| covers?.get(&covers::Key::of(art, covers::bucket(px))));
    let ground = panel(space()).color(ph).radius(2.0).width(Length::Fixed(w)).height(Length::Fixed(w));
    let face = match cover {
        // The whole cover, never cropped: a small or oblong one sits centered on its color.
        Some(c) => stack(vec![ground, picture(c.image).radius(2.0).width(Length::Fixed(w)).height(Length::Fixed(w))]),
        // No art at all (or not yet): the tile is its own sleeve once it is
        // known there is none; until then its color alone.
        None if album.front.is_none() => stack(vec![
            ground,
            panel(
                column(vec![
                    space().height(Length::FILL),
                    label(album.title.clone()).size(14.0).weight(700).color(Color::rgba(255, 255, 255, 242)).wrap(),
                    label(album.album_artist.clone()).size(11.0).color(Color::rgba(255, 255, 255, 179)).wrap(),
                ])
                .spacing(3.0)
                .width(Length::FILL)
                .height(Length::FILL),
            )
            .padding(Padding { top: 0.0, right: 14.0, bottom: 14.0, left: 14.0 })
            .width(Length::Fixed(w))
            .height(Length::Fixed(w)),
        ]),
        None => ground,
    };
    let body = column(vec![
        face,
        label(album.title.clone()).size(wall::TITLE).weight(500).elide().width(Length::Fixed(w)).color(color(g.text)),
        label(album.album_artist.clone()).size(wall::ARTIST).elide().width(Length::Fixed(w)).color(color(g.text_dim)),
    ])
    .spacing(wall::UNDER_GAP);
    button(body).look(ButtonLook::Plain).padding(0.0).on_press(Msg::Open(i)).key(i as u64)
}

/// The wall: every album's tile, the rows in view built.
pub fn wall(albums: Rc<Vec<Album>>, covers: Option<Arc<Covers>>, scale: f32, t: &Theme, g: Ground) -> Element<Msg> {
    let text = t.text_factor;
    let count = albums.len();
    let tiles = grid(
        count,
        move |w| {
            let (cols, cover) = wall::columns(w);
            (cols, cover, wall::tile_height(cover, text))
        },
        wall::GAP,
        wall::GAP,
        move |i, w| tile(&albums[i], i, w, covers.as_deref(), scale, &g),
    );
    // Only panels and buttons take padding: the wall's margins are a panel's.
    scroll(
        panel(tiles)
            .padding(Padding { top: wall::PAD_TOP, right: wall::PAD_SIDE, bottom: wall::PAD_TOP, left: wall::PAD_SIDE })
            .radius(0.0)
            .width(Length::FILL),
    )
    .name("wall")
}

/// The bottom bar: the status line on the left, the library picker and its
/// counts on the right.
pub fn bar(status: String, counts: String, libraries: Vec<String>, current: usize, g: &Ground) -> Element<Msg> {
    let mut right: Vec<Element<Msg>> = vec![label(counts).size(12.0).color(color(g.text_dim))];
    if libraries.len() > 1 {
        right.push(choice(libraries, Some(current), Msg::Library));
    }
    panel(
        row(vec![
            label(status).size(12.0).color(color(g.text_dim)).elide().width(Length::FILL),
            row(right).spacing(12.0).align(Align::Center),
        ])
        .spacing(12.0)
        .align(Align::Center)
        .width(Length::FILL)
        .height(Length::FILL),
    )
    .padding(Padding::hv(0.0, 16.0))
    .height(Length::Fixed(BAR))
    .color(color(g.bar))
    .radius(0.0)
    .width(Length::FILL)
}
