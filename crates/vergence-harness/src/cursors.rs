// `vergence-harness cursors <dir>`: Vergence's pointers as an Xcursor theme
// (direction D, Dave 2026-11-18: "I like the D mouse pointers"; the design
// canvas https://claude.ai/artifact/KE2ECDjA9NM5YR2yCEYyQ3). The classic
// black arrow with a white rim, with color only where it means something:
// accent heads on the resize arrows, a six-color disc of our own that turns
// for busy, badges for copy, not allowed and help.
//
// Each shape is an SVG on a 32-unit square, drawn by the picture helper's
// own SVG code (vergence-decode, no new dependency) at 24, 32, 48, 64 and 96
// pixels, and written as an Xcursor file (the format every X11 and Wayland
// toolkit reads; the server takes the largest image and shrinks it for each
// screen, cursor.rs). The busy shapes turn: 12 frames. Names are the CSS
// ones (wp-cursor-shape-v1, as the server asks) with the X11 names as links.
// install.sh writes the theme to share/icons/Vergence; it inherits Adwaita
// for the rare shapes not drawn here.

use std::error::Error;
use std::path::Path;

const SIZES: [u32; 5] = [24, 32, 48, 64, 96];
const BUSY_FRAMES: usize = 12;
const BUSY_DELAY_MS: u32 = 80;

const ARROW: &str = r##"<path d="M3 2 L3 25 L9 19 L13 29 L17 27.5 L13 18 L21 18 Z" fill="#000" stroke="#fff" stroke-width="1.5" stroke-linejoin="miter"/>"##;

/// The six-color disc, its center at (cx, cy), radius r, turned by `turn` degrees.
fn disc(cx: f32, cy: f32, r: f32, turn: f32, rim: &str) -> String {
    let colors = ["#2f7ae6", "#14a39a", "#4caf72", "#f0b43c", "#e0574a", "#8a63d2"];
    let mut s = format!(r#"<g transform="translate({cx} {cy}) rotate({turn})">"#);
    for (i, c) in colors.iter().enumerate() {
        let a0 = (i as f32 * 60.0 - 90.0).to_radians();
        let a1 = ((i + 1) as f32 * 60.0 - 90.0).to_radians();
        s += &format!(r#"<path d="M0 0 L{:.3} {:.3} A{r} {r} 0 0 1 {:.3} {:.3} Z" fill="{c}"/>"#, r * a0.cos(), r * a0.sin(), r * a1.cos(), r * a1.sin());
    }
    s += &format!(r#"<circle r="{r}" fill="none" stroke="{rim}" stroke-width="1.4"/></g>"#);
    s
}

/// A shape: its SVG body on the 32-unit square, its hotspot, and how many frames.
struct Shape {
    names: &'static [&'static str],
    hot: (f32, f32),
    body: fn(usize) -> String,
    frames: usize,
}

fn arrow(_: usize) -> String {
    ARROW.into()
}

fn hand(_: usize) -> String {
    r##"<path d="M11 3 Q11 1 13 1 Q15 1 15 3 L15 13 L17 12 Q19 12 19 14 L21 13 Q23 13 23 15 L25 15 Q27 15 27 18 L27 24 Q27 30 21 30 L15 30 Q11 30 9 26 L4 18 Q3 16 5 15 Q7 14 8 16 L11 20 Z" fill="#000" stroke="#fff" stroke-width="1.5" stroke-linejoin="round"/>"##.into()
}

fn ibeam(_: usize) -> String {
    let p = "M11 3 L15 3 L16 4 L17 3 L21 3 M16 4 L16 28 M11 29 L15 29 L16 28 L17 29 L21 29";
    format!(r##"<path d="{p}" fill="none" stroke="#fff" stroke-width="4"/><path d="{p}" fill="none" stroke="#000" stroke-width="2"/>"##)
}

/// The two-headed arrow across, turned by `deg` (0 across, 90 up and down).
fn resize(deg: f32) -> String {
    format!(
        r##"<g transform="rotate({deg} 16 16)"><path d="M2 16 L9 9 L9 13 L23 13 L23 9 L30 16 L23 23 L23 19 L9 19 L9 23 Z" fill="#000" stroke="#fff" stroke-width="1.5"/><path d="M3.8 16 L8 11.8 L8 20.2 Z M28.2 16 L24 11.8 L24 20.2 Z" fill="#2f7ae6"/></g>"##
    )
}

fn ew(_: usize) -> String {
    resize(0.0)
}
fn ns(_: usize) -> String {
    resize(90.0)
}
fn nwse(_: usize) -> String {
    resize(45.0)
}
fn nesw(_: usize) -> String {
    resize(-45.0)
}

fn moving(_: usize) -> String {
    r##"<path d="M16 2 L21 7 L18 7 L18 14 L25 14 L25 11 L30 16 L25 21 L25 18 L18 18 L18 25 L21 25 L16 30 L11 25 L14 25 L14 18 L7 18 L7 21 L2 16 L7 11 L7 14 L14 14 L14 7 L11 7 Z" fill="#000" stroke="#fff" stroke-width="1.4" stroke-linejoin="miter"/>"##.into()
}

fn crosshair(_: usize) -> String {
    let p = "M16 3 L16 13 M16 19 L16 29 M3 16 L13 16 M19 16 L29 16";
    format!(r##"<path d="{p}" stroke="#fff" stroke-width="4" stroke-linecap="square"/><path d="{p}" stroke="#000" stroke-width="2"/><circle cx="16" cy="16" r="1.5" fill="#e0574a" stroke="#fff" stroke-width="0.8"/>"##)
}

fn busy(i: usize) -> String {
    disc(16.0, 16.0, 12.0, i as f32 * 360.0 / BUSY_FRAMES as f32, "#000") + r##"<circle cx="16" cy="16" r="13.2" fill="none" stroke="#fff" stroke-width="1"/><circle cx="16" cy="16" r="2.6" fill="#fff" stroke="#000" stroke-width="1"/>"##
}

fn progress(i: usize) -> String {
    format!("{ARROW}{}", disc(24.0, 24.0, 6.5, i as f32 * 360.0 / BUSY_FRAMES as f32, "#000"))
}

fn copy(_: usize) -> String {
    format!(r##"{ARROW}<circle cx="24" cy="24" r="6.5" fill="#2e9e5b" stroke="#fff" stroke-width="1.2"/><path d="M24 20.5 L24 27.5 M20.5 24 L27.5 24" stroke="#fff" stroke-width="2" stroke-linecap="round"/>"##)
}

fn not_allowed(_: usize) -> String {
    format!(r##"{ARROW}<circle cx="24" cy="24" r="6.5" fill="#fff" stroke="#d23c30" stroke-width="2.2"/><line x1="19.6" y1="28.4" x2="28.4" y2="19.6" stroke="#d23c30" stroke-width="2.2"/>"##)
}

fn help(_: usize) -> String {
    // The question mark as strokes (no fonts in the picture helper).
    format!(r##"{ARROW}<circle cx="24" cy="24" r="7" fill="#2f7ae6" stroke="#fff" stroke-width="1.2"/><path d="M21.6 22 Q21.6 19.6 24 19.6 Q26.4 19.6 26.4 21.8 Q26.4 23.4 24.6 24.2 Q24 24.6 24 25.6" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round"/><circle cx="24" cy="28.2" r="1.1" fill="#fff"/>"##)
}

fn context(_: usize) -> String {
    format!(r##"{ARROW}<rect x="19" y="18" width="11" height="12" rx="2" fill="#fff" stroke="#000" stroke-width="1.2"/><path d="M21.5 21.5 L27.5 21.5 M21.5 24 L27.5 24 M21.5 26.5 L25.5 26.5" stroke="#2f7ae6" stroke-width="1.4"/>"##)
}

fn shapes() -> Vec<Shape> {
    vec![
        Shape { names: &["default", "left_ptr", "arrow", "top_left_arrow"], hot: (3.0, 2.0), body: arrow, frames: 1 },
        Shape { names: &["pointer", "hand2", "hand1", "pointing_hand"], hot: (13.0, 1.5), body: hand, frames: 1 },
        Shape { names: &["grab", "openhand", "grabbing", "closedhand", "dnd-move", "dnd-none"], hot: (13.0, 1.5), body: hand, frames: 1 },
        Shape { names: &["text", "xterm", "ibeam", "vertical-text"], hot: (16.0, 16.0), body: ibeam, frames: 1 },
        Shape { names: &["ew-resize", "e-resize", "w-resize", "col-resize", "sb_h_double_arrow", "h_double_arrow", "size_hor", "split_h"], hot: (16.0, 16.0), body: ew, frames: 1 },
        Shape { names: &["ns-resize", "n-resize", "s-resize", "row-resize", "sb_v_double_arrow", "v_double_arrow", "size_ver", "split_v"], hot: (16.0, 16.0), body: ns, frames: 1 },
        Shape { names: &["nwse-resize", "nw-resize", "se-resize", "bd_double_arrow", "size_fdiag", "top_left_corner", "bottom_right_corner"], hot: (16.0, 16.0), body: nwse, frames: 1 },
        Shape { names: &["nesw-resize", "ne-resize", "sw-resize", "fd_double_arrow", "size_bdiag", "top_right_corner", "bottom_left_corner"], hot: (16.0, 16.0), body: nesw, frames: 1 },
        Shape { names: &["move", "all-scroll", "fleur", "size_all"], hot: (16.0, 16.0), body: moving, frames: 1 },
        Shape { names: &["crosshair", "cross", "tcross", "cell", "plus"], hot: (16.0, 16.0), body: crosshair, frames: 1 },
        Shape { names: &["wait", "watch"], hot: (16.0, 16.0), body: busy, frames: BUSY_FRAMES },
        Shape { names: &["progress", "left_ptr_watch", "half-busy"], hot: (3.0, 2.0), body: progress, frames: BUSY_FRAMES },
        Shape { names: &["copy", "dnd-copy", "alias", "dnd-link", "link"], hot: (3.0, 2.0), body: copy, frames: 1 },
        Shape { names: &["not-allowed", "no-drop", "crossed_circle", "forbidden", "dnd-no-drop", "circle"], hot: (3.0, 2.0), body: not_allowed, frames: 1 },
        Shape { names: &["help", "question_arrow", "whats_this", "left_ptr_help"], hot: (3.0, 2.0), body: help, frames: 1 },
        Shape { names: &["context-menu"], hot: (3.0, 2.0), body: context, frames: 1 },
    ]
}

fn svg(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 32 32">{body}</svg>"#)
}

/// One image: `px` square, straight RGBA from the picture helper.
fn render(body: &str, px: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    let p = vergence_decode::decode_bytes(svg(body).as_bytes(), Some((px, px)))?;
    if (p.width, p.height) != (px, px) {
        return Err(format!("drawn {}x{}, not {px}", p.width, p.height).into());
    }
    Ok(p.frames.into_iter().next().ok_or("no frame")?.rgba)
}

/// An Xcursor file: images (nominal size, width, hot x, hot y, delay ms, RGBA).
pub fn xcursor(images: &[(u32, u32, u32, u32, u32, Vec<u8>)]) -> Vec<u8> {
    const IMAGE: u32 = 0xfffd_0002;
    let mut out = Vec::new();
    let put = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
    out.extend_from_slice(b"Xcur");
    put(&mut out, 16);
    put(&mut out, 0x0001_0000);
    put(&mut out, images.len() as u32);
    let mut pos = 16 + 12 * images.len() as u32;
    for (nominal, w, ..) in images {
        put(&mut out, IMAGE);
        put(&mut out, *nominal);
        put(&mut out, pos);
        pos += 36 + w * w * 4;
    }
    for (nominal, w, hx, hy, delay, rgba) in images {
        for v in [36, IMAGE, *nominal, 1, *w, *w, *hx, *hy, *delay] {
            put(&mut out, v);
        }
        // ARGB, premultiplied, one little-endian word each.
        for p in rgba.chunks_exact(4) {
            let a = p[3] as u32;
            let pm = |c: u8| (c as u32 * a + 127) / 255;
            put(&mut out, (a << 24) | (pm(p[0]) << 16) | (pm(p[1]) << 8) | pm(p[2]));
        }
    }
    out
}

/// Writes the theme's cursors/ into `dir` (with index.theme).
pub fn run(args: &[String]) -> Result<bool, Box<dyn Error>> {
    let dir = Path::new(args.first().ok_or("cursors <theme directory>")?);
    let cursors = dir.join("cursors");
    std::fs::create_dir_all(&cursors)?;
    std::fs::write(dir.join("index.theme"), "[Icon Theme]\nName=Vergence\nComment=Vergence's pointers: the classic arrow, color where it means something\nInherits=Adwaita\n")?;
    let mut count = 0;
    for shape in shapes() {
        let mut images = Vec::new();
        for px in SIZES {
            let k = px as f32 / 32.0;
            let (hx, hy) = ((shape.hot.0 * k).round() as u32, (shape.hot.1 * k).round() as u32);
            for f in 0..shape.frames {
                let delay = if shape.frames > 1 { BUSY_DELAY_MS } else { 0 };
                images.push((px, px, hx.min(px - 1), hy.min(px - 1), delay, render(&(shape.body)(f), px)?));
            }
        }
        let first = shape.names[0];
        std::fs::write(cursors.join(first), xcursor(&images))?;
        for alias in &shape.names[1..] {
            let link = cursors.join(alias);
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(first, &link)?;
        }
        count += 1;
    }
    println!("{count} shapes ({} names) in {}", shapes().iter().map(|s| s.names.len()).sum::<usize>(), cursors.display());
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shape_draws_and_the_file_is_xcursor() {
        for s in shapes() {
            let rgba = render(&(s.body)(0), 32).unwrap();
            assert_eq!(rgba.len(), 32 * 32 * 4);
            assert!(rgba.chunks_exact(4).any(|p| p[3] > 0), "{} draws something", s.names[0]);
        }
        let file = xcursor(&[(24, 2, 0, 0, 0, vec![255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 128])]);
        assert_eq!(&file[..4], b"Xcur");
        let words: Vec<u32> = file.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        assert_eq!(words[3], 1, "one image");
        assert_eq!(words[6], 28, "the image starts after the header and its table");
        assert_eq!(words[7..16], [36, 0xfffd_0002, 24, 1, 2, 2, 0, 0, 0]);
        assert_eq!(words[16], 0xffff_0000, "red, opaque");
        assert_eq!(words[19], 0x8080_8080, "white at half, premultiplied");
    }
}
