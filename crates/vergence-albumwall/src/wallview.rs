//! The wall on screen (steps 4 and 5): its tiles in the toolkit's grid
//! (only the rows in view built), each cover from the cache, fading in on
//! its placeholder color when it first comes; the title and artist under
//! it; the open album's tile ringed in its accent and turned over to its
//! back, its panel under its row (panelview.rs); the bottom bar with the
//! status line, the counts and the library picker. The wall's ground and
//! the bars take the library's hue (colors.rs).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use vergence_albumwall::colors::{Ground, Palette, Rgb};
use vergence_albumwall::covers::{self, Covers};
use vergence_albumwall::library::Album;
use vergence_albumwall::wall;
use vergence_scene::Color;
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Theme, button, choice, column, field, grid, label, panel, picture,
    row, scroll, space, stack,
};

use crate::Msg;

pub fn color(c: Rgb) -> Color {
    Color::rgb(c.0, c.1, c.2)
}

/// The bottom bar's height (AlbumWall's 44).
pub const BAR: f32 = 44.0;
/// A cover fades in over this when it first comes, its placeholder under it.
pub const FADE: Duration = Duration::from_millis(220);

/// When each cover first came, for its fade-in, and the latest (the window
/// animates while one is fading).
#[derive(Default)]
pub struct Fades {
    first: RefCell<HashMap<covers::Key, Instant>>,
    last: Cell<Option<Instant>>,
    pub reduced: Cell<bool>,
}

impl Fades {
    /// How opaque the cover for `key` is now.
    fn opacity(&self, key: &covers::Key, now: Instant) -> f32 {
        if self.reduced.get() {
            return 1.0;
        }
        let mut first = self.first.borrow_mut();
        let at = *first.entry(key.clone()).or_insert_with(|| {
            self.last.set(Some(now));
            now
        });
        (now.saturating_duration_since(at).as_secs_f32() / FADE.as_secs_f32()).clamp(0.0, 1.0)
    }

    /// A cover is fading in (and a moment past, so the last frame is whole).
    pub fn fading(&self, now: Instant) -> bool {
        self.last.get().is_some_and(|t| now.saturating_duration_since(t) < FADE + Duration::from_millis(60))
    }
}

/// The open album as the wall shows it this frame.
#[derive(Clone)]
pub struct OpenView {
    pub index: usize,
    /// The panel's height now (it unfolds from nothing, or folds away).
    pub height: f32,
    /// How opaque its contents are (0 while it moves, then fading in).
    pub shown: f32,
    pub palette: Palette,
    /// How far its tile has turned over toward its back, 0 to 1.
    pub turn: f32,
    /// The track of it playing, if it is the album playing.
    pub playing: Option<usize>,
    /// The keyboard's track in it (the arrows), if the keyboard is in its tracks.
    pub cursor: Option<usize>,
}

/// One tile: the cover (or its placeholder), the title and the artist.
#[allow(clippy::too_many_arguments)]
fn tile(
    album: &Album,
    i: usize,
    w: f32,
    covers: Option<&Covers>,
    scale: f32,
    g: &Ground,
    open: Option<&OpenView>,
    fades: &Fades,
    keyed: bool,
) -> Element<Msg> {
    let now = Instant::now();
    let ph = color(wall::placeholder(&album.album_artist, &album.title));
    let px = (w * scale).ceil();
    // The open album's tile turns over to its back (back.jpg) and stays so
    // while it is open: a turn about its upright axis, shown as its width
    // narrowing to nothing and widening again with the other face.
    let turn = open.filter(|o| o.index == i && album.back.is_some()).map_or(0.0, |o| o.turn);
    let art = if turn >= 0.5 { album.back.as_ref() } else { album.front.as_ref() };
    let face_w = (w * (std::f32::consts::PI * turn).cos().abs()).max(1.0);
    let key = art.map(|a| covers::Key::of(a, covers::bucket(px)));
    let cover = key.as_ref().and_then(|k| covers?.get(k));
    let ground = panel(space()).color(ph).radius(2.0).width(Length::Fixed(w)).height(Length::Fixed(w));
    let mut face = match cover {
        // The whole cover, never cropped: a small or oblong one sits centered
        // on its color; it fades in when it first comes.
        Some(c) => {
            let o = key.as_ref().map_or(1.0, |k| fades.opacity(k, now));
            stack(vec![
                ground,
                row(vec![
                    space().width(Length::FILL),
                    picture(c.image).radius(2.0).opacity(o).width(Length::Fixed(face_w)).height(Length::Fixed(w)),
                    space().width(Length::FILL),
                ])
                .width(Length::Fixed(w)),
            ])
        }
        // No art at all: the tile is its own sleeve; until a cover comes, its color alone.
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
    // The open album's tile wears a ring in the album's own accent.
    if let Some(o) = open.filter(|o| o.index == i) {
        face = stack(vec![
            face,
            panel(space())
                .edge(color(o.palette.accent), 3.0)
                .radius(2.0)
                .width(Length::Fixed(w))
                .height(Length::Fixed(w)),
        ]);
    }
    // The keyboard's album: a light ring with a dark rim inside it, so it
    // shows on any cover (the Avalonia app's key ring).
    if keyed {
        face = stack(vec![
            face,
            panel(space())
                .edge(Color::rgba(0, 0, 0, 150), 4.0)
                .radius(3.0)
                .width(Length::Fixed(w))
                .height(Length::Fixed(w)),
            panel(space())
                .edge(Color::rgba(255, 255, 255, 235), 2.0)
                .radius(3.0)
                .width(Length::Fixed(w))
                .height(Length::Fixed(w)),
        ]);
    }
    let body = column(vec![
        face,
        label(album.title.clone()).size(wall::TITLE).weight(500).elide().width(Length::Fixed(w)).color(color(g.text)),
        label(album.album_artist.clone()).size(wall::ARTIST).elide().width(Length::Fixed(w)).color(color(g.text_dim)),
    ])
    .spacing(wall::UNDER_GAP);
    button(body).look(ButtonLook::Plain).padding(0.0).on_press(Msg::Open(i)).key(i as u64)
}

/// The wall: every album's tile, the rows in view built; the open album's
/// panel under its row.
#[allow(clippy::too_many_arguments)]
pub fn wall(
    albums: Rc<Vec<Album>>,
    order: Rc<Vec<usize>>,
    keyed: Option<usize>,
    covers: Option<Arc<Covers>>,
    scale: f32,
    t: &Theme,
    g: Ground,
    open: Option<OpenView>,
    fades: Rc<Fades>,
    extra_foot: f32,
) -> Element<Msg> {
    let text = t.text_factor;
    let count = order.len();
    // The open album's place on the wall as searched (none: not found).
    let open_at = open.as_ref().and_then(|o| order.iter().position(|&i| i == o.index));
    let panel_el =
        open.as_ref().zip(open_at).filter(|(o, _)| o.index < albums.len() && o.height > 0.5).map(|(o, at)| {
            let el = crate::panelview::open(
                o.index,
                at,
                o.cursor,
                &albums[o.index],
                &o.palette,
                covers.clone(),
                scale,
                text,
                o.shown,
                o.playing,
            );
            (at, o.height, el)
        });
    let mut tiles = grid(
        count,
        move |w| {
            let (cols, cover) = wall::columns(w);
            (cols, cover, wall::tile_height(cover, text))
        },
        wall::GAP,
        wall::GAP,
        move |at, w| {
            let i = order[at];
            tile(&albums[i], i, w, covers.as_deref(), scale, &g, open.as_ref(), &fades, keyed == Some(i))
        },
    );
    if let Some((i, h, el)) = panel_el {
        tiles = tiles.after_item(i, h, el);
    }
    // Only panels and buttons take padding: the wall's margins are a panel's.
    scroll(
        panel(tiles)
            .padding(Padding {
                top: wall::PAD_TOP,
                right: wall::PAD_SIDE,
                // The play bar floats over the wall's foot: room under the last row for it.
                bottom: wall::PAD_TOP + extra_foot,
                left: wall::PAD_SIDE,
            })
            .radius(0.0)
            .width(Length::FILL),
    )
    .name("wall")
}

/// The bottom bar: the status line on the left, the library picker and its
/// counts on the right.
/// The bottom bar: the status line, the search box (320 points, as in the
/// Avalonia app), the library picker and the counts.
pub fn bar(
    status: String,
    query: &str,
    counts: String,
    libraries: Vec<String>,
    current: usize,
    g: &Ground,
) -> Element<Msg> {
    let mut right: Vec<Element<Msg>> = vec![label(counts).size(12.0).color(color(g.text_dim))];
    if libraries.len() > 1 {
        right.push(choice(libraries, Some(current), Msg::Library));
    }
    panel(
        row(vec![
            label(status).size(12.0).color(color(g.text_dim)).elide().width(Length::FILL),
            field(query, Msg::Search).placeholder("Search artist or album…").name("search").width(Length::Fixed(320.0)),
            row(vec![space().width(Length::FILL), row(right).spacing(12.0).align(Align::Center)])
                .align(Align::Center)
                .width(Length::FILL),
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
