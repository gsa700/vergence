//! The words of the song playing, in the open album (milestone 2, step 4;
//! the Avalonia app's LyricsView.cs). In place of the track list when "Show
//! Lyrics" is pressed in the playing album's panel: a window onto the lines
//! that the song moves through by itself.
//!
//! - **Timed lyrics** (LRC): the line being sung held in the middle, bright
//!   and a little heavier, the others the panel's dim text; the first line
//!   lit from the start, through an intro before its time ("start at the
//!   first line whether or not the time is yet synced", Dave, 2026-11-01);
//!   lit 150 ms early, so a line lights as it is sung. A click on a line
//!   takes the song there. A gap in the singing shows a note.
//! - **Plain lyrics** (most of them for now): every line bright, the window
//!   drifting down through them as the song goes, from the first line a
//!   fifth of the way down to the last a fifth from the foot; a song that
//!   fits sits centered and does not move.
//! - **Held**: the left button held on the words stops them, to be read;
//!   let go, they go on from there (Dave, 2026-11-01).
//! - It glides a share of the way each frame; with motion reduced it jumps.
//!   New words (another song) start where they belong. Lines fade out at the
//!   head and foot instead of being cut by the edge.
//!
//! Not here, against the Avalonia view: the wheel nudging plain lyrics (a
//! drawing is told presses and moves, not the wheel, in this toolkit yet).

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use vergence_albumwall::colors::Rgb;
use vergence_albumwall::lyrics::Lyrics;
use vergence_scene::{Color, Point, Rect};
use vergence_widgets::{Element, Length, Touch, drawing};

use crate::Msg;

/// The view's motion, kept between frames (the window keeps one).
#[derive(Default)]
pub struct Motion {
    /// The column's offset from its top, in points.
    y: Cell<f32>,
    /// Which words it was placed for (another song starts in place).
    words: Cell<usize>,
    /// The last frame, for the glide's share of the way.
    at: Cell<Option<Instant>>,
    /// Plain lyrics: how far the hand moved the drift, kept after it lets go.
    nudge: Cell<f32>,
    /// The left button is down on the words: nothing moves.
    held: Cell<bool>,
    /// The press: when, and on which line.
    pressed: Cell<Option<(Instant, usize)>>,
    /// Still gliding: the window asks for frames while it does.
    pub moving: Cell<bool>,
}

/// What the view is given each frame.
pub struct Shown {
    pub lyrics: Arc<Lyrics>,
    pub seconds: f64,
    pub length: f64,
    pub bright: Rgb,
    pub dim: Rgb,
    pub text: f32,
    pub height: f32,
}

/// A line's height for a text size of 14 at `text` (the Avalonia view's pitch).
pub fn pitch(text: f32) -> f32 {
    (14.0 * text * 1.2 * 1.75).ceil()
}

/// A quick press: a click (takes a timed song to its line), not a hold.
const CLICK_MS: u128 = 350;
/// The share of the window, top and bottom, over which lines fade.
const FADE_SHARE: f32 = 0.14;

/// Where plain lyrics may be: from the first line a fifth of the way down
/// to the last line a fifth from the foot; a song that fits sits centered.
fn plain_range(lines: usize, pitch: f32, view: f32) -> (f32, f32) {
    let buffer = view * 0.2;
    let total = lines as f32 * pitch;
    if total + 2.0 * buffer <= view {
        let middle = (total - view) / 2.0;
        return (middle, middle);
    }
    (-buffer, total - view + buffer)
}

/// The line being sung (timed), 150 ms early; the first from the start.
fn current(lyrics: &Lyrics, seconds: f64) -> usize {
    lyrics.line_at(seconds + 0.15).unwrap_or(0)
}

/// Where the column should be now, and the line lit (timed only).
fn target(s: &Shown, m: &Motion, pitch: f32) -> (f32, Option<usize>) {
    let view = s.height;
    if s.lyrics.timed {
        let now = current(&s.lyrics, s.seconds);
        return (now as f32 * pitch + pitch / 2.0 - view / 2.0, Some(now));
    }
    let (lo, hi) = plain_range(s.lyrics.lines.len(), pitch, view);
    let share = if s.length > 0.0 { (s.seconds / s.length).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let base = lo + share * (hi - lo);
    if m.held.get() {
        // The hand has the words: the drift takes up from here when it lets go.
        m.nudge.set(m.y.get() - base);
    }
    ((base + m.nudge.get()).clamp(lo, hi), None)
}

/// The view, `s.height` tall and the width it is given.
pub fn view(s: Shown, motion: Rc<Motion>, reduced: bool) -> Element<Msg> {
    let height = s.height;
    let lyrics = s.lyrics.clone();
    let pitch = pitch(s.text);
    let m = motion.clone();
    let words_key = Arc::as_ptr(&s.lyrics) as usize;
    drawing(move |scene, fonts, r: Rect, theme| {
        let now = Instant::now();
        let (want, lit) = target(&s, &m, pitch);
        let lit = if m.held.get() { lit.map(|_| current(&s.lyrics, s.seconds)) } else { lit };
        // New words start where they belong, not gliding in from the last song's place.
        if m.words.get() != words_key {
            m.words.set(words_key);
            m.y.set(want);
            m.nudge.set(0.0);
        }
        let dt = m.at.get().map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32());
        m.at.set(Some(now));
        let y = m.y.get();
        let y = if m.held.get() {
            y
        } else if reduced || (want - y).abs() < 0.5 {
            want
        } else {
            // 18% of the way each 50 ms, whatever the frame rate.
            y + (want - y) * (1.0 - 0.82f32.powf(dt / 0.05))
        };
        m.y.set(y);
        m.moving.set((want - y).abs() >= 0.5 && !m.held.get());
        let size = 14.0 * s.text * 1.2;
        let regular = fonts.face(&theme.font, 400, false);
        let heavy = fonts.face(&theme.font, 600, false).or(regular);
        let edge = (r.h * FADE_SHARE).max(1.0);
        let ellipsis_w = regular.map_or(0.0, |f| {
            vergence_render::text(scene, fonts, f, size, Color::rgba(0, 0, 0, 0), Point::new(-1e5, -1e5), "…")
        });
        scene.push_clip(r, 0.0);
        for (i, line) in s.lyrics.lines.iter().enumerate() {
            let middle = i as f32 * pitch + pitch / 2.0 - y;
            if middle < -pitch || middle > r.h + pitch {
                continue;
            }
            let fade = (middle.min(r.h - middle) / edge).clamp(0.0, 1.0);
            if fade <= 0.0 {
                continue;
            }
            let here = lit == Some(i);
            let c = if !s.lyrics.timed || here { s.bright } else { s.dim };
            let Some(face) = (if here { heavy } else { regular }) else { continue };
            let words: &str = if !line.text.is_empty() {
                &line.text
            } else if s.lyrics.timed {
                "♪"
            } else {
                continue;
            };
            let color = Color::rgba(c.0, c.1, c.2, (255.0 * fade) as u8);
            let hidden = Color::rgba(0, 0, 0, 0);
            let off = Point::new(-1e5, -1e5);
            // One line each, centered; too long for the window, cut with "…".
            let mut shown = words.to_string();
            let mut w = vergence_render::text(scene, fonts, face, size, hidden, off, &shown);
            if w > r.w {
                let mut chars: Vec<char> = words.chars().collect();
                while !chars.is_empty() && w + ellipsis_w > r.w {
                    chars.pop();
                    shown = chars.iter().collect::<String>().trim_end().to_string();
                    w = vergence_render::text(scene, fonts, face, size, hidden, off, &shown);
                }
                shown.push('…');
                w += ellipsis_w;
            }
            let baseline = r.y + middle + size * 0.35;
            vergence_render::text(scene, fonts, face, size, color, Point::new(r.x + (r.w - w) / 2.0, baseline), &shown);
        }
        scene.pop_clip();
    })
    .on_touch(move |t| {
        let m = &motion;
        let line_at = |y: f32| ((y + m.y.get()) / pitch).floor();
        match t {
            Touch::Press { y, .. } => {
                m.held.set(true);
                let l = line_at(y);
                m.pressed.set((l >= 0.0).then(|| (Instant::now(), l as usize)));
                None
            }
            Touch::Release { .. } => {
                m.held.set(false);
                let quick = m.pressed.take().filter(|(at, _)| at.elapsed().as_millis() < CLICK_MS);
                // A quick click on a timed line: the song there.
                let (_, line) = quick?;
                if !lyrics.timed {
                    return None;
                }
                lyrics.lines.get(line).and_then(|l| l.at).map(Msg::LyricLine)
            }
            Touch::Leave => {
                m.held.set(false);
                m.pressed.set(None);
                None
            }
            Touch::Move { .. } => None,
        }
    })
    .width(Length::FILL)
    .height(Length::Fixed(height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_lyrics_that_fit_sit_still_and_long_ones_travel_with_room_at_both_ends() {
        let (lo, hi) = plain_range(3, 30.0, 300.0);
        assert_eq!(lo, hi, "fits: no travel");
        assert_eq!(lo, (90.0 - 300.0) / 2.0);
        let (lo, hi) = plain_range(40, 30.0, 150.0);
        assert_eq!((lo, hi), (-30.0, 1200.0 - 150.0 + 30.0));
    }

    #[test]
    fn the_first_timed_line_is_lit_from_the_start_and_lines_light_a_touch_early() {
        let l = Lyrics::parse("[00:05.00]One\n[00:10.00]Two").unwrap();
        assert_eq!(current(&l, 0.0), 0);
        assert_eq!(current(&l, 9.80), 0);
        assert_eq!(current(&l, 9.86), 1, "150 ms early");
    }
}
