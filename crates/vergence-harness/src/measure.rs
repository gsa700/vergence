//! `vergence-harness measure <title>...`: how big a size box is on the
//! panel, in millimeters (milestone 4, acceptance 3; whitepaper §4.8.3).
//!
//! The window with that title is read back from the X server (GetImage on
//! the window itself, so a compositor does not matter, and windows on a
//! fake monitor that shows nothing can still be read). The magenta box's
//! bounding box is in drawn pixels; dividing by the RandR factor of the
//! monitor it is on gives panel pixels, and the monitor's physical size
//! gives millimeters.

use std::error::Error;

use x11rb::connection::Connection as _;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, ImageFormat, ImageOrder};
use x11rb::rust_connection::RustConnection;

type Res<T> = Result<T, Box<dyn Error>>;

/// What one measurement found.
pub struct Found {
    pub output: String,
    pub drawn: (u32, u32),
    pub panel: (f64, f64),
    pub mm: (f64, f64),
}

fn atom(conn: &RustConnection, n: &str) -> Res<u32> {
    Ok(conn.intern_atom(false, n.as_bytes())?.reply()?.atom)
}

fn find_window(conn: &RustConnection, root: u32, title: &str) -> Res<Option<u32>> {
    let list = conn.get_property(false, root, atom(conn, "_NET_CLIENT_LIST")?, AtomEnum::WINDOW, 0, 4096)?.reply()?;
    let net_name = atom(conn, "_NET_WM_NAME")?;
    let utf8 = atom(conn, "UTF8_STRING")?;
    for w in list.value32().into_iter().flatten() {
        let n = conn.get_property(false, w, net_name, utf8, 0, 256)?.reply()?;
        let name = if n.value.is_empty() {
            conn.get_property(false, w, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 256)?.reply()?.value
        } else {
            n.value
        };
        if name == title.as_bytes() {
            return Ok(Some(w));
        }
    }
    Ok(None)
}

/// The magenta box in `win`: (x, y, width, height) in the window's pixels.
fn find_box(conn: &RustConnection, win: u32) -> Res<Option<(u32, u32, u32, u32)>> {
    let g = conn.get_geometry(win)?.reply()?;
    let img = conn.get_image(ImageFormat::Z_PIXMAP, win, 0, 0, g.width, g.height, !0)?.reply()?;
    let bpp = conn
        .setup()
        .pixmap_formats
        .iter()
        .find(|f| f.depth == img.depth)
        .map(|f| f.bits_per_pixel)
        .unwrap_or(32);
    if bpp != 32 {
        return Err(format!("depth {} uses {bpp} bits per pixel; only 32 is supported", img.depth).into());
    }
    let msb = conn.setup().image_byte_order == ImageOrder::MSB_FIRST;
    let (w, h) = (g.width as usize, g.height as usize);
    let stride = img.data.len() / h.max(1);
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            let p = &img.data[y * stride + x * 4..y * stride + x * 4 + 4];
            let (r, gg, b) = if msb { (p[1], p[2], p[3]) } else { (p[2], p[1], p[0]) };
            if r >= 240 && gg <= 16 && b >= 240 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 == usize::MAX {
        return Ok(None);
    }
    Ok(Some((x0 as u32, y0 as u32, (x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32)))
}

pub fn measure(conn: &RustConnection, root: u32, title: &str) -> Res<Option<Found>> {
    let Some(win) = find_window(conn, root, title)? else { return Ok(None) };
    let Some((bx, by, bw, bh)) = find_box(conn, win)? else {
        return Err(format!("{title}: no magenta box in the window").into());
    };
    let pos = conn.translate_coordinates(win, root, 0, 0)?.reply()?;
    let cx = pos.dst_x as i32 + bx as i32 + bw as i32 / 2;
    let cy = pos.dst_y as i32 + by as i32 + bh as i32 / 2;

    let res = conn.randr_get_screen_resources_current(root)?.reply()?;
    for &c in &res.crtcs {
        let ci = conn.randr_get_crtc_info(c, res.config_timestamp)?.reply()?;
        if ci.mode == 0 || ci.outputs.is_empty() {
            continue;
        }
        let inside = cx >= ci.x as i32
            && cx < ci.x as i32 + ci.width as i32
            && cy >= ci.y as i32
            && cy < ci.y as i32 + ci.height as i32;
        if !inside {
            continue;
        }
        let t = conn.randr_get_crtc_transform(c)?.reply()?.current_transform;
        let factor = if t.matrix11 > 0 { t.matrix11 as f64 / 65536.0 } else { 1.0 };
        let mode = res.modes.iter().find(|m| m.id == ci.mode).ok_or("CRTC mode not in the screen resources")?;
        let oi = conn.randr_get_output_info(ci.outputs[0], res.config_timestamp)?.reply()?;
        if oi.mm_width == 0 || oi.mm_height == 0 {
            return Err(format!("{} has no physical size", String::from_utf8_lossy(&oi.name)).into());
        }
        let panel = (bw as f64 / factor, bh as f64 / factor);
        let mm = (
            panel.0 * oi.mm_width as f64 / mode.width as f64,
            panel.1 * oi.mm_height as f64 / mode.height as f64,
        );
        return Ok(Some(Found { output: String::from_utf8_lossy(&oi.name).into_owned(), drawn: (bw, bh), panel, mm }));
    }
    Err(format!("{title}: the box is on no lit monitor").into())
}

/// `measure <title>...`: one line per window.
pub fn run(titles: &[String]) -> Res<bool> {
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let mut ok = true;
    for t in titles {
        match measure(&conn, root, t) {
            Ok(Some(f)) => println!(
                "{t:<24} {:<7} drawn {}x{} px, panel {:.1}x{:.1} px, {:.1} x {:.1} mm",
                f.output, f.drawn.0, f.drawn.1, f.panel.0, f.panel.1, f.mm.0, f.mm.1
            ),
            Ok(None) => {
                println!("{t:<24} no such window");
                ok = false;
            }
            Err(e) => {
                println!("{t:<24} {e}");
                ok = false;
            }
        }
    }
    Ok(ok)
}
