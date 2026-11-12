//! vnative: a window drawn over the native protocol (native milestone 1,
//! step 3), to try the server's side before the toolkit has a native road.
//!
//!     vnative [--title T] [--size WxH] [--own-frame] [--frames N] [--short-file]
//!
//! It asks for a window, draws a gradient with a border at the scale it is
//! told, and then moves a bar one step for every frame the server offers,
//! sending only the two strips that changed as damage. It says what it
//! hears; it quits when the window is closed or after N frames, printing how
//! many frames came and how far apart. `--short-file` claims one row more
//! than its file holds, to check that the server refuses it and stays up.
//!
//! Input (step 4): a square follows the pointer, which takes the crosshair
//! shape over the window; keys are printed as evdev codes (Esc quits);
//! with `--own-frame`, a press on the top 24 points moves the window and a
//! press within 16 points of the bottom-right corner resizes it. Everything
//! else it hears (keymap, repeat, focus, modifiers, the wheel) is printed.

use std::time::Instant;

use vergence_protocol::{
    Client,
    bus::{ClientMsg, ServerMsg},
    shm::ShmFile,
    window::{Buffer, Damage, Fd, Place, SHM_ARGB8888, WinEvent, WinRequest},
};

const WINDOW: u32 = 1;
const BAR: i32 = 24;
/// The pointer's square, and the own frame's strip and corner, in reference units.
const DOT: f64 = 8.0;
const STRIP: f64 = 24.0;
const CORNER: f64 = 16.0;
/// wp_cursor_shape's crosshair.
const CROSSHAIR: u16 = 8;
const KEY_ESC: u32 = 1;

struct Picture {
    shm: ShmFile,
    w: i32,
    h: i32,
    scale: f64,
}

impl Picture {
    fn new(width: f64, height: f64, scale: f64) -> Result<Picture, Box<dyn std::error::Error>> {
        let (w, h) = ((width * scale).round().max(1.0) as i32, (height * scale).round().max(1.0) as i32);
        let shm = ShmFile::new(w as usize * h as usize * 4)?;
        let mut p = Picture { shm, w, h, scale };
        p.background(0, 0, w, h);
        Ok(p)
    }

    fn put(&mut self, x: i32, y: i32, (r, g, b): (u8, u8, u8)) {
        let i = (y as usize * self.w as usize + x as usize) * 4;
        self.shm.bytes()[i..i + 4].copy_from_slice(&[b, g, r, 0xff]);
    }

    /// The gradient and the border, in a rectangle of the picture.
    fn background(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let border = self.scale.round().max(1.0) as i32 * 2;
        for y in y0.max(0)..y1.min(self.h) {
            for x in x0.max(0)..x1.min(self.w) {
                let edge = x < border || y < border || x >= self.w - border || y >= self.h - border;
                let c = if edge {
                    (40, 90, 160)
                } else {
                    (225 - (x * 60 / self.w) as u8, 230 - (y * 50 / self.h) as u8, 240)
                };
                self.put(x, y, c);
            }
        }
    }

    /// The pointer's square at (x, y) reference units; returns it in pixels.
    fn dot(&mut self, x: f64, y: f64) -> Damage {
        let side = (DOT * self.scale).round() as i32;
        let (px, py) = ((x * self.scale) as i32 - side / 2, (y * self.scale) as i32 - side / 2);
        let r = Damage { x: px.max(0), y: py.max(0), w: (px + side).min(self.w) - px.max(0), h: (py + side).min(self.h) - py.max(0) };
        for yy in r.y..r.y + r.h {
            for xx in r.x..r.x + r.w {
                self.put(xx, yy, (20, 20, 20));
            }
        }
        r
    }

    /// The bar at step `n`, in pixels from the left; returns its strip.
    fn bar(&mut self, n: u32) -> (i32, i32) {
        let width = (BAR as f64 * self.scale).round() as i32;
        let travel = (self.w - width).max(1);
        let x = (n as i32 * (self.scale.round() as i32).max(1) * 2) % travel;
        let (top, bottom) = (self.h / 3, self.h * 2 / 3);
        for y in top..bottom {
            for i in 0..width {
                self.put(x + i, y, (230, 120, 30));
            }
        }
        (x, width)
    }
}

fn attach(c: &mut Client, p: &Picture, damage: Vec<Damage>, lie: bool) -> Result<(), Box<dyn std::error::Error>> {
    // The descriptor travels with the message (SCM_RIGHTS); the server reads it at once.
    let fd = Fd(std::sync::Arc::new(unsafe_dup(p.shm.fd())?));
    let height = p.h as u32 + lie as u32;
    let buffer = Buffer::Shm { fd, width: p.w as u32, height, stride: p.w as u32 * 4, format: SHM_ARGB8888 };
    c.send(&ClientMsg::Window(WinRequest::Attach { window: WINDOW, buffer, damage }))?;
    Ok(())
}

/// A second descriptor for the same file, owned by the message.
fn unsafe_dup(fd: std::os::fd::RawFd) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::BorrowedFd;
    // SAFETY: `fd` is the ShmFile's descriptor, open for the lifetime of the Picture.
    unsafe { BorrowedFd::borrow_raw(fd) }.try_clone_to_owned()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let title = arg("--title").unwrap_or_else(|| "vnative".into());
    let (width, height) = arg("--size")
        .and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse::<f64>().ok()?, h.parse::<f64>().ok()?))))
        .unwrap_or((480.0, 320.0));
    let own_frame = args.iter().any(|a| a == "--own-frame");
    let frames_max: Option<u32> = arg("--frames").and_then(|n| n.parse().ok());
    let lie = args.iter().any(|a| a == "--short-file");

    let mut c = Client::connect("vnative")?;
    let reply = c.request(|serial| {
        ClientMsg::Window(WinRequest::Create {
            serial,
            window: WINDOW,
            role: "main".into(),
            token: String::new(),
            title: title.clone(),
            width,
            height,
            own_frame,
            parent: 0,
            place: Place::Anywhere,
        })
    })?;
    if let ServerMsg::Window(WinEvent::Created { token, .. }) = &reply {
        println!("created, token {token:?}");
    }

    let mut picture: Option<Picture> = None;
    // Where the pointer is (None: not on the window), and the square last drawn.
    let mut pointer: Option<(f64, f64)> = None;
    let mut last_dot: Option<Damage> = None;
    let mut size = (width, height);
    let mut step = 0u32;
    let mut last_bar: Option<(i32, i32)> = None;
    let (mut frames, started) = (0u32, Instant::now());
    let mut gaps: Vec<u32> = Vec::new();
    let mut last_time: Option<u32> = None;
    loop {
        let ServerMsg::Window(ev) = c.next_event()? else { continue };
        match ev {
            WinEvent::Configure { serial, width, height, scale, maximized, activated, .. } => {
                size = (width, height);
                last_dot = None;
                println!("configure {serial}: {width}x{height} at scale {scale:.4}{}{}", if maximized { ", maximized" } else { "" }, if activated { ", active" } else { "" });
                let mut p = Picture::new(width, height, scale)?;
                last_bar = Some(p.bar(step));
                c.send(&ClientMsg::Window(WinRequest::AckConfigure { window: WINDOW, serial }))?;
                attach(&mut c, &p, Vec::new(), lie)?;
                picture = Some(p);
            }
            WinEvent::Frame { time_ms, .. } => {
                frames += 1;
                if let Some(t) = last_time {
                    gaps.push(time_ms.wrapping_sub(t));
                }
                last_time = Some(time_ms);
                if frames_max.is_some_and(|n| frames >= n) {
                    break;
                }
                let Some(p) = picture.as_mut() else { continue };
                step += 1;
                // Only the strips the bar left and now covers are sent as damage.
                let mut damage = Vec::new();
                if let Some((x, w)) = last_bar {
                    p.background(x, p.h / 3, x + w, p.h * 2 / 3);
                    damage.push(Damage { x, y: p.h / 3, w, h: p.h * 2 / 3 - p.h / 3 });
                }
                let (x, w) = p.bar(step);
                damage.push(Damage { x, y: p.h / 3, w, h: p.h * 2 / 3 - p.h / 3 });
                last_bar = Some((x, w));
                // The pointer's square: the old one painted over, the new one drawn.
                if let Some(d) = last_dot.take() {
                    p.background(d.x, d.y, d.x + d.w, d.y + d.h);
                    damage.push(d);
                }
                if let Some((x, y)) = pointer {
                    let d = p.dot(x, y);
                    if d.w > 0 && d.h > 0 {
                        damage.push(d);
                        last_dot = Some(d);
                    }
                }
                attach(&mut c, p, damage, false)?;
            }
            WinEvent::Close { .. } => {
                println!("asked to close");
                break;
            }
            WinEvent::PointerEnter { x, y, .. } => {
                println!("pointer in at {x:.1},{y:.1}");
                pointer = Some((x, y));
                c.send(&ClientMsg::Window(WinRequest::Cursor { window: WINDOW, shape: CROSSHAIR }))?;
            }
            WinEvent::PointerLeave { .. } => {
                println!("pointer out");
                pointer = None;
            }
            WinEvent::PointerMotion { x, y, .. } => pointer = Some((x, y)),
            WinEvent::PointerButton { serial, button, pressed, .. } => {
                println!("button {button:#x} {}", if pressed { "pressed" } else { "released" });
                if pressed && own_frame && let Some((x, y)) = pointer {
                    if y < STRIP {
                        c.send(&ClientMsg::Window(WinRequest::Move { window: WINDOW, input_serial: serial }))?;
                    } else if x > size.0 - CORNER && y > size.1 - CORNER {
                        c.send(&ClientMsg::Window(WinRequest::Resize { window: WINDOW, input_serial: serial, edges: 2 | 8 }))?;
                    }
                }
            }
            WinEvent::PointerAxis { source, dx, dy, v120x, v120y, .. } => println!("scroll {dx:.1},{dy:.1} (v120 {v120x},{v120y}, source {source})"),
            WinEvent::Keymap { size, .. } => println!("keymap: {size} bytes"),
            WinEvent::Repeat { rate, delay } => println!("key repeat: {rate} a second after {delay} ms"),
            WinEvent::KeyboardEnter { .. } => println!("keyboard in"),
            WinEvent::KeyboardLeave { .. } => println!("keyboard out"),
            WinEvent::Modifiers { depressed, locked, .. } => println!("modifiers {depressed:#x} (locked {locked:#x})"),
            WinEvent::Key { key, pressed, .. } => {
                println!("key {key} {}", if pressed { "down" } else { "up" });
                if key == KEY_ESC && pressed {
                    break;
                }
            }
            other => println!("{other:?}"),
        }
    }
    c.send(&ClientMsg::Window(WinRequest::Destroy { window: WINDOW }))?;
    let secs = started.elapsed().as_secs_f64();
    gaps.sort_unstable();
    let median = gaps.get(gaps.len() / 2).copied().unwrap_or(0);
    println!("{frames} frames in {secs:.1} s ({:.1} a second), median gap {median} ms", frames as f64 / secs.max(1e-9));
    Ok(())
}
