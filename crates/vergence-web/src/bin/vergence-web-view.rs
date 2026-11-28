//! vergence-web-view: one web page in a plain Vergence window (web milestone
//! 1, step 5), for trying vergence-web's platform and for the checks.
//!
//!     vergence-web-view [--size WxH] [URL]
//!
//! A native window (as vnative's) whose picture is WebKit's frame: each frame
//! WebKit finishes is copied into shared memory and attached with the
//! rectangles that changed; the server's frame callback tells WebKit it may
//! draw the next. Pointer, wheel (a notch is one step) and keys go to the
//! page; the page's title becomes the window's.
//!
//! Start reading: `main`, then `Window` (the Host WebKit draws into).

use std::cell::RefCell;
use std::rc::Rc;

use vergence_protocol::{
    Client,
    bus::{ClientMsg, ServerMsg},
    shm::ShmFile,
    window::{Buffer, Damage, Fd, Place, SHM_ARGB8888, WinEvent, WinRequest},
};
use vergence_web::{Frame, Host, Platform, WebView};

const WINDOW: u32 = 1;

/// The window WebKit draws into: its picture, and the connection to send it on.
struct Window {
    client: Rc<RefCell<Client>>,
    picture: Option<(ShmFile, u32, u32)>,
    /// What the pointer last looked like over the page (cursor-shape numbers).
    cursor: u16,
}

/// A CSS cursor name as the cursor-shape protocol's number (default 1).
fn cursor_shape(name: &str) -> u16 {
    match name {
        "pointer" => 4,
        "progress" => 5,
        "wait" => 6,
        "crosshair" => 8,
        "text" => 9,
        "vertical-text" => 10,
        "copy" => 12,
        "move" => 13,
        "no-drop" => 14,
        "not-allowed" => 15,
        "grab" => 16,
        "grabbing" => 17,
        "e-resize" => 18,
        "n-resize" => 19,
        "ne-resize" => 20,
        "nw-resize" => 21,
        "s-resize" => 22,
        "se-resize" => 23,
        "sw-resize" => 24,
        "w-resize" => 25,
        "ew-resize" => 26,
        "ns-resize" => 27,
        "col-resize" => 30,
        "row-resize" => 31,
        "help" => 3,
        "context-menu" => 2,
        "cell" => 7,
        _ => 1,
    }
}

impl Host for Window {
    fn frame(&mut self, f: Frame) {
        let size = f.width as usize * f.height as usize * 4;
        if self.picture.as_ref().is_none_or(|(_, w, h)| (*w, *h) != (f.width, f.height)) {
            match ShmFile::new(size) {
                Ok(shm) => self.picture = Some((shm, f.width, f.height)),
                Err(e) => {
                    eprintln!("vergence-web-view: shared memory: {e}");
                    return;
                }
            }
        }
        let Some((shm, w, h)) = self.picture.as_mut() else { return };
        let row = *w as usize * 4;
        let dst = shm.bytes();
        for y in 0..*h as usize {
            let s = y * f.stride as usize;
            dst[y * row..(y + 1) * row].copy_from_slice(&f.pixels[s..s + row]);
        }
        let damage = f.damage.iter().map(|&(x, y, w, h)| Damage { x, y, w, h }).collect();
        // The server copies shared memory when it reads the message.
        let fd = {
            use std::os::fd::BorrowedFd;
            // SAFETY: the ShmFile's descriptor, open while the picture lives.
            match unsafe { BorrowedFd::borrow_raw(shm.fd()) }.try_clone_to_owned() {
                Ok(fd) => Fd(std::sync::Arc::new(fd)),
                Err(_) => return,
            }
        };
        let buffer = Buffer::Shm { fd, width: *w, height: *h, stride: *w * 4, format: SHM_ARGB8888 };
        let _ =
            self.client.borrow_mut().send(&ClientMsg::Window(WinRequest::Attach { window: WINDOW, buffer, damage }));
    }

    fn cursor(&mut self, name: &str) {
        let shape = cursor_shape(name);
        if shape != self.cursor {
            self.cursor = shape;
            let _ = self.client.borrow_mut().send(&ClientMsg::Window(WinRequest::Cursor { window: WINDOW, shape }));
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut size = (1024.0, 768.0);
    let mut url = "about:blank".to_string();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--size" {
            if let Some((w, h)) = args.get(i + 1).and_then(|s| s.split_once('x')) {
                size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
            }
            i += 2;
            continue;
        }
        url = args[i].clone();
        i += 1;
    }
    // A local path opens as a file.
    if !url.contains(':')
        && let Ok(p) = std::fs::canonicalize(&url)
    {
        url = format!("file://{}", p.display());
    }

    let client = match Client::connect("vergence-web-view") {
        Ok(c) => Rc::new(RefCell::new(c)),
        Err(e) => {
            eprintln!("vergence-web-view: no Vergence session: {e:?}");
            std::process::exit(1);
        }
    };
    let host = Window { client: client.clone(), picture: None, cursor: 1 };
    let platform = match Platform::new(Box::new(host)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("vergence-web-view: {e}");
            std::process::exit(1);
        }
    };
    let created = client.borrow_mut().request(|serial| {
        ClientMsg::Window(WinRequest::Create {
            serial,
            window: WINDOW,
            role: "main".into(),
            token: String::new(),
            title: "Web".into(),
            width: size.0,
            height: size.1,
            parent: 0,
            place: Place::Anywhere,
        })
    });
    if let Err(e) = created {
        eprintln!("vergence-web-view: the server refused the window: {e:?}");
        std::process::exit(1);
    }
    let view = Rc::new(RefCell::new(platform.web_view()));
    view.borrow().load(&url);

    let fd = client.borrow().raw_fd();
    let title = RefCell::new(String::new());
    vergence_web::run(
        fd,
        Box::new(move |readable| {
            if readable {
                match client.borrow_mut().read_ready() {
                    Ok(true) => {}
                    Ok(false) => return false,
                    Err(e) => {
                        eprintln!("vergence-web-view: lost the server: {e:?}");
                        return false;
                    }
                }
            }
            loop {
                let ev = match client.borrow_mut().try_event() {
                    Ok(Some(ServerMsg::Window(ev))) => ev,
                    Ok(Some(_)) => continue,
                    Ok(None) => break,
                    Err(e) => {
                        eprintln!("vergence-web-view: lost the server: {e:?}");
                        return false;
                    }
                };
                if !handle(&client, &mut view.borrow_mut(), ev) {
                    return false;
                }
            }
            // The page's title as the window's, when it changes.
            if let Some(t) = view.borrow().title().filter(|t| !t.is_empty())
                && *title.borrow() != t
            {
                let _ = client
                    .borrow_mut()
                    .send(&ClientMsg::Window(WinRequest::Title { window: WINDOW, title: t.clone() }));
                *title.borrow_mut() = t;
            }
            true
        }),
    );
    let _ = std::mem::ManuallyDrop::new(platform);
}

/// One event from the server; false ends the program.
fn handle(client: &Rc<RefCell<Client>>, view: &mut WebView, ev: WinEvent) -> bool {
    match ev {
        WinEvent::Configure { serial, width, height, scale, .. } => {
            let _ = client.borrow_mut().send(&ClientMsg::Window(WinRequest::AckConfigure { window: WINDOW, serial }));
            view.configure(width, height, scale);
        }
        WinEvent::Frame { .. } => view.frame_done(),
        WinEvent::Close { .. } => return false,
        WinEvent::PointerEnter { x, y, .. } => view.pointer_enter(0, x, y),
        WinEvent::PointerLeave { .. } => view.pointer_leave(0),
        WinEvent::PointerMotion { time_ms, x, y, .. } => view.pointer_motion(time_ms, x, y),
        WinEvent::PointerButton { time_ms, button, pressed, .. } => view.pointer_button(time_ms, button, pressed),
        WinEvent::PointerAxis { time_ms, source, dx, dy, v120x, v120y, .. } => {
            view.scroll(time_ms, source, dx, dy, v120x, v120y)
        }
        WinEvent::KeyboardEnter { .. } => view.focus(true),
        WinEvent::KeyboardLeave { .. } => view.focus(false),
        WinEvent::Modifiers { depressed, locked, .. } => view.modifiers(depressed, locked),
        WinEvent::Key { time_ms, key, pressed, .. } => view.key(time_ms, key, pressed),
        _ => {}
    }
    true
}
