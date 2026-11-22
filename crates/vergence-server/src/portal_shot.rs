// Other programs' screenshots (docs/screenshots-milestone-1.md, step 7):
// through the desktop portal, each program asked first.
//
// org.freedesktop.portal.Screenshot on the D-Bus edge (dbus.rs), version 2:
// Screenshot(parent_window, options) and PickColor(parent_window, options)
// answer at once with a Request's path; the Response comes when it is done:
// 0 and `uri` (a PNG in $XDG_RUNTIME_DIR/vergence-portal, the user's alone)
// or `color` (red, green, blue from 0 to 1); 1 if the person cancelled or
// said no; 2 if it failed. `interactive` opens Super+Shift+4's picker (an
// area, Space for a window); without it, every screen is taken as one
// picture. PickColor opens the picker as a crosshair; a click picks the
// pixel under it.
//
// A program is asked about once, by a question the server draws (as the
// agents' question, agents.rs, and with its rules): which program (the
// name it runs as, and its executable from its process: what a grant is
// tied to, so another program can't borrow it), Don't allow on the left,
// Allow once and Always allow on the right; Enter and Escape mean Don't
// allow; nothing counts for its first second; after Don't allow the same
// program is refused without asking for a minute. "Always" is kept in
// ~/.local/state/vergence/screenshot-programs (owner only); Settings will
// list them (step 8). Never while the screen is locked.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use smithay::{
    backend::renderer::{
        element::{
            Id, Kind,
            memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
            solid::SolidColorRenderElement,
        },
        gles::GlesRenderer,
        utils::CommitCounter,
    },
    utils::{Logical, Point, Rectangle},
};
use vergence_dbus::Value;

use crate::{Server, decor::SceneElement};

pub const SCREENSHOT: &str = "org.freedesktop.portal.Screenshot";

/// Nothing counts this soon after the question shows.
const ARMING: Duration = Duration::from_secs(1);
/// After Don't allow, the same program is refused without asking this long.
const COOL_DOWN: Duration = Duration::from_secs(60);
/// The most questions waiting (a program asking in a loop is refused).
const WAITING_MAX: usize = 8;
const W: i32 = 560;
const H: i32 = 168;
const PAD: i32 = 22;
const BUTTON_H: i32 = 32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind2 {
    Screenshot { interactive: bool },
    Color,
}

/// A request from a program, while it is asked about or carried out.
#[derive(Clone, Debug)]
pub struct Asked {
    pub conn: u32,
    pub handle: String,
    pub program: String,
    pub name: String,
    pub kind: Kind2,
}

#[derive(Default)]
pub struct PortalShots {
    asking: VecDeque<Asked>,
    armed: Option<Instant>,
    always: Option<HashMap<String, String>>,
    refused: HashMap<String, Instant>,
    /// The request the picker is open for (interactive, or a color).
    pub picking: Option<Asked>,
    panel: Rectangle<i32, Logical>,
    buttons: Vec<(u8, Rectangle<i32, Logical>)>,
    dim: Option<(Id, CommitCounter)>,
    picture: Option<(u64, MemoryRenderBuffer)>,
    count: u64,
}

fn grants_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("vergence/screenshot-programs"))
}

/// "`allow<TAB>program<TAB>name`" a line.
pub fn parse_grants(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            match (f.next(), f.next(), f.next()) {
                (Some("allow"), Some(p), name) if p.starts_with('/') => {
                    Some((p.to_string(), name.unwrap_or("").to_string()))
                }
                _ => None,
            }
        })
        .collect()
}

fn write_grants(g: &HashMap<String, String>) {
    use std::os::unix::fs::OpenOptionsExt;
    let Some(p) = grants_path() else { return };
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let mut lines: Vec<String> = g
        .iter()
        .map(|(p, n)| format!("allow\t{}\t{}\n", p.replace(['\t', '\n'], " "), n.replace(['\t', '\n'], " ")))
        .collect();
    lines.sort();
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&p) {
        use std::io::Write;
        let _ = f.write_all(lines.concat().as_bytes());
    }
}

/// Where portal pictures are written: $XDG_RUNTIME_DIR/vergence-portal (0700).
fn shots_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("vergence-portal")
}

fn option<'a>(options: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    let Some(Value::Array(_, items)) = options else { return None };
    items.iter().find_map(|e| match e {
        Value::Entry(k, v) if k.as_str() == Some(key) => Some(match &**v {
            Value::Variant(inner) => &**inner,
            other => other,
        }),
        _ => None,
    })
}

impl Server {
    /// Screenshot.Screenshot or Screenshot.PickColor: the Request's path at
    /// once; asked about first unless the program may always.
    pub fn portal_screenshot(
        &mut self,
        conn: u32,
        unique: &str,
        member: &str,
        args: &[Value],
    ) -> Result<String, String> {
        let options = args.get(1);
        let token = option(options, "handle_token").and_then(Value::as_str).unwrap_or("vergence").to_string();
        let handle = crate::portal::request_path(unique, &token);
        let interactive = matches!(option(options, "interactive"), Some(Value::Bool(true)));
        let kind = if member == "PickColor" { Kind2::Color } else { Kind2::Screenshot { interactive } };
        let pid = self.dbus_conn_pid(conn);
        let program =
            pid.and_then(vergence_dm::elevate::program_path).map(|p| p.display().to_string()).unwrap_or_default();
        let name = pid
            .and_then(crate::portal::process_name)
            .unwrap_or_else(|| program.rsplit('/').next().unwrap_or("A program").to_string());
        let asked = Asked { conn, handle: handle.clone(), program: program.clone(), name, kind };
        tracing::info!("portal: {} ({program}) asks for {kind:?}", asked.name);
        if self.lock.on || crate::greeter() || program.is_empty() {
            self.dbus_request_response(conn, &handle, 2, Value::Array("{sv}".into(), Vec::new()));
            return Ok(handle);
        }
        // Read each time: Settings > Screenshots takes grants back in the file.
        let always =
            grants_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse_grants(&t)).unwrap_or_default();
        self.portal_shots.always = Some(always.clone());
        if always.contains_key(&program) {
            self.portal_shot_start(asked);
        } else if self.portal_shots.refused.get(&program).is_some_and(|t| t.elapsed() < COOL_DOWN)
            || self.portal_shots.asking.len() >= WAITING_MAX
        {
            self.dbus_request_response(conn, &handle, 1, Value::Array("{sv}".into(), Vec::new()));
        } else {
            if self.portal_shots.asking.is_empty() {
                self.portal_shots.armed = Some(Instant::now());
            }
            self.portal_shots.asking.push_back(asked);
            self.redraw_all();
        }
        Ok(handle)
    }

    pub fn portal_asking(&self) -> bool {
        !self.portal_shots.asking.is_empty()
    }

    /// Allowed: carried out.
    fn portal_shot_start(&mut self, a: Asked) {
        use crate::capture::{Dest, Request, What};
        match a.kind {
            Kind2::Screenshot { interactive: false } => {
                let screens: Vec<Rectangle<i32, Logical>> =
                    self.space.outputs().filter_map(|o| self.space.output_geometry(o)).collect();
                let Some(all) = screens.iter().copied().reduce(|x, y| x.merge(y)) else {
                    return self.portal_shot_done(&a, None);
                };
                let req = Request { what: What::Area(all), dest: Dest::Portal(a), pointer: false, shadow: true };
                if let Err(e) = self.capture_ask(req.clone()) {
                    tracing::info!("portal: {e}");
                    if let Dest::Portal(a) = req.dest {
                        self.portal_shot_done(&a, None);
                    }
                }
            }
            Kind2::Screenshot { interactive: true } | Kind2::Color => {
                let color = a.kind == Kind2::Color;
                self.portal_shots.picking = Some(a);
                self.capture_pick_open(false, false, color);
            }
        }
    }

    /// The picker closed without a picture (Escape): cancelled.
    pub fn portal_pick_cancelled(&mut self) {
        if let Some(a) = self.portal_shots.picking.take() {
            self.dbus_request_response(a.conn, &a.handle, 1, Value::Array("{sv}".into(), Vec::new()));
        }
    }

    /// A capture for a program is taken (or failed): written and answered.
    pub fn portal_shot_done(&mut self, a: &Asked, pixels: Option<(usize, usize, Vec<u8>)>) {
        let empty = || Value::Array("{sv}".into(), Vec::new());
        let Some((w, h, rgba)) = pixels else {
            return self.dbus_request_response(a.conn, &a.handle, 2, empty());
        };
        if a.kind == Kind2::Color {
            let (r, g, b) = (
                rgba.first().copied().unwrap_or(0),
                rgba.get(1).copied().unwrap_or(0),
                rgba.get(2).copied().unwrap_or(0),
            );
            let color = Value::Struct(vec![
                Value::Double(r as f64 / 255.0),
                Value::Double(g as f64 / 255.0),
                Value::Double(b as f64 / 255.0),
            ]);
            let results = Value::Array(
                "{sv}".into(),
                vec![Value::Entry(Box::new(Value::Str("color".into())), Box::new(Value::Variant(Box::new(color))))],
            );
            tracing::info!("portal: {} picked the color {r},{g},{b}", a.name);
            return self.dbus_request_response(a.conn, &a.handle, 0, results);
        }
        self.portal_shots.count += 1;
        let path = shots_dir().join(format!("screenshot-{}-{}.png", std::process::id(), self.portal_shots.count));
        let (conn, handle, name) = (a.conn, a.handle.clone(), a.name.clone());
        let (tx, rx) = smithay::reexports::calloop::channel::channel::<Option<PathBuf>>();
        let _ = self.loop_handle.insert_source(rx, move |ev, _, st| {
            if let smithay::reexports::calloop::channel::Event::Msg(p) = ev {
                match p {
                    Some(p) => {
                        tracing::info!("portal: a screenshot for {name}: {}", p.display());
                        let uri = crate::portal::file_uri(&p);
                        let results = Value::Array(
                            "{sv}".into(),
                            vec![Value::Entry(
                                Box::new(Value::Str("uri".into())),
                                Box::new(Value::Variant(Box::new(Value::Str(uri)))),
                            )],
                        );
                        st.dbus_request_response(conn, &handle, 0, results);
                    }
                    None => st.dbus_request_response(conn, &handle, 2, Value::Array("{sv}".into(), Vec::new())),
                }
            }
        });
        crate::capture::save(crate::capture::Taken { w: w as u32, h: h as u32, rgba, path }, move |r| {
            let _ = tx.send(r.ok());
        });
    }

    /// The question answered: 0 Don't allow, 1 Allow once, 2 Always allow.
    fn portal_shot_answer(&mut self, choice: u8) {
        if self.portal_shots.armed.is_some_and(|t| t.elapsed() < ARMING) && choice != 0 {
            return;
        }
        let Some(a) = self.portal_shots.asking.pop_front() else { return };
        self.portal_shots.armed = (!self.portal_shots.asking.is_empty()).then(Instant::now);
        self.portal_shots.picture = None;
        self.redraw_all();
        match choice {
            1 | 2 => {
                if choice == 2 {
                    let mut g = grants_path()
                        .and_then(|p| std::fs::read_to_string(p).ok())
                        .map(|t| parse_grants(&t))
                        .unwrap_or_default();
                    g.insert(a.program.clone(), a.name.clone());
                    write_grants(&g);
                    self.portal_shots.always = Some(g);
                }
                tracing::info!("portal: {} allowed ({})", a.name, if choice == 2 { "always" } else { "once" });
                self.portal_shot_start(a);
            }
            _ => {
                tracing::info!("portal: {} not allowed", a.name);
                self.portal_shots.refused.insert(a.program.clone(), Instant::now());
                self.dbus_request_response(a.conn, &a.handle, 1, Value::Array("{sv}".into(), Vec::new()));
            }
        }
    }

    /// Its keys: Enter and Escape mean Don't allow.
    pub fn portal_ask_key(&mut self, sym: u32, pressed: bool) {
        use xkbcommon::xkb::keysyms as k;
        if pressed && matches!(sym, k::KEY_Escape | k::KEY_Return | k::KEY_KP_Enter) {
            self.portal_shot_answer(0);
        }
    }

    /// A press: a button answers; anywhere else nothing.
    pub fn portal_ask_press(&mut self, at: Point<f64, Logical>) {
        if let Some(&(choice, _)) = self.portal_shots.buttons.iter().find(|(_, r)| r.to_f64().contains(at)) {
            self.portal_shot_answer(choice);
        }
    }

    /// The question over the screen under the pointer, everything dimmed.
    pub fn portal_ask_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        out: Rectangle<i32, Logical>,
        scale: f64,
    ) -> Vec<SceneElement> {
        let mut els = Vec::new();
        if out.to_f64().contains(self.pointer_location())
            && let Some(e) = self.portal_ask_panel(renderer, out, scale)
        {
            els.push(e);
        }
        let (id, commit) = self.portal_shots.dim.get_or_insert_with(|| (Id::new(), CommitCounter::default())).clone();
        let phys =
            Rectangle::<i32, smithay::utils::Physical>::from_size(out.size.to_f64().to_physical(scale).to_i32_round());
        els.push(SceneElement::Solid(SolidColorRenderElement::new(
            id,
            phys,
            commit,
            [0.0, 0.0, 0.0, 0.45],
            Kind::Unspecified,
        )));
        els
    }

    fn portal_ask_panel(
        &mut self,
        renderer: &mut GlesRenderer,
        out: Rectangle<i32, Logical>,
        scale: f64,
    ) -> Option<SceneElement> {
        let a = self.portal_shots.asking.front()?.clone();
        let light = self.light_style();
        let accent = self.accent();
        let panel = Rectangle::<i32, Logical>::new(
            (out.loc.x + (out.size.w - W) / 2, out.loc.y + (out.size.h - H) / 2).into(),
            (W, H).into(),
        );
        let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
        let face = fonts.face("sans-serif", 400, false)?;
        let bold = fonts.face("sans-serif", 600, false).unwrap_or(face);
        let labels: [(u8, &str); 3] = [(0, "Don't allow"), (1, "Allow once"), (2, "Always allow")];
        let widths: Vec<i32> =
            labels.iter().map(|(_, l)| (fonts.layout(face, 13.0, l).1 + 28.0).ceil() as i32).collect();
        let y = H - PAD - BUTTON_H;
        let mut local = vec![(0u8, PAD, widths[0])];
        let mut x = W - PAD;
        for i in [2usize, 1] {
            x -= widths[i];
            local.push((labels[i].0, x, widths[i]));
            x -= 8;
        }
        self.portal_shots.panel = panel;
        self.portal_shots.buttons = local
            .iter()
            .map(|&(c, bx, bw)| (c, Rectangle::new((panel.loc.x + bx, panel.loc.y + y).into(), (bw, BUTTON_H).into())))
            .collect();
        let what = match a.kind {
            Kind2::Color => "pick a color from your screen",
            _ => "take a picture of your screen",
        };
        let title = format!("\u{201c}{}\u{201d} wants to {what}", a.name);
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (&title, &a.program, light, scale.to_bits(), (accent.r, accent.g, accent.b)).hash(&mut h);
            h.finish()
        };
        if self.portal_shots.picture.as_ref().is_none_or(|(k, _)| *k != key) {
            let mut scene = vergence_scene::Scene::new(W as f32, H as f32);
            let s = vergence_look::shelf_look(light);
            let ink = vergence_scene::Color::rgb(s.menu_ink.0, s.menu_ink.1, s.menu_ink.2);
            let dim = vergence_scene::Color::rgba(s.menu_ink.0, s.menu_ink.1, s.menu_ink.2, 170);
            scene.fill_rounded(
                vergence_scene::Rect::new(0.0, 0.0, W as f32, H as f32),
                12.0,
                vergence_scene::Color::rgb(s.menu.0, s.menu.1, s.menu.2),
            );
            scene.stroke(
                vergence_scene::Rect::new(0.0, 0.0, W as f32, H as f32),
                12.0,
                1.0,
                vergence_scene::Color::rgba(128, 128, 128, 90),
            );
            let m = fonts.metrics(bold, 15.0);
            vergence_render::text(
                &mut scene,
                fonts,
                bold,
                15.0,
                ink,
                vergence_scene::Point::new(PAD as f32, PAD as f32 + m.ascent),
                &title,
            );
            let body = if a.kind == Kind2::Color {
                "It gets the color of the pixel you click."
            } else {
                "It gets the picture you take (or of every screen)."
            };
            let m2 = fonts.metrics(face, 13.0);
            vergence_render::text(
                &mut scene,
                fonts,
                face,
                13.0,
                dim,
                vergence_scene::Point::new(PAD as f32, PAD as f32 + m.ascent + 12.0 + m2.ascent),
                body,
            );
            vergence_render::text(
                &mut scene,
                fonts,
                face,
                12.0,
                dim,
                vergence_scene::Point::new(PAD as f32, PAD as f32 + m.ascent + 12.0 + m2.ascent + 20.0),
                &format!("Program: {}", a.program),
            );
            for &(c, bx, bw) in &local {
                let r = vergence_scene::Rect::new(bx as f32, y as f32, bw as f32, BUTTON_H as f32);
                let (bg, fg) = if c == 2 {
                    (
                        vergence_scene::Color::rgb(accent.r, accent.g, accent.b),
                        vergence_scene::Color::rgb(255, 255, 255),
                    )
                } else {
                    (vergence_scene::Color::rgba(128, 128, 128, 60), ink)
                };
                scene.fill_rounded(r, 7.0, bg);
                // Don't allow is the keyboard's (Enter, Escape): ringed.
                if c == 0 {
                    scene.stroke(r, 7.0, 2.0, vergence_scene::Color::rgba(accent.r, accent.g, accent.b, 200));
                }
                let l = labels[c as usize].1;
                let (_, tw) = fonts.layout(face, 13.0, l);
                vergence_render::text(
                    &mut scene,
                    fonts,
                    face,
                    13.0,
                    fg,
                    vergence_scene::Point::new(
                        (r.x + (r.w - tw) / 2.0).round(),
                        (r.y + (r.h - m2.ascent - m2.descent) / 2.0 + m2.ascent).round(),
                    ),
                    l,
                );
            }
            let pic = vergence_render::Cpu::new()
                .render(&scene, fonts, scale as f32, vergence_scene::Color::rgba(0, 0, 0, 0))
                .ok()?;
            let buffer = MemoryRenderBuffer::from_slice(
                &pic.data,
                smithay::backend::allocator::Fourcc::Abgr8888,
                (pic.width as i32, pic.height as i32),
                1,
                smithay::utils::Transform::Normal,
                None,
            );
            self.portal_shots.picture = Some((key, buffer));
        }
        let (_, buffer) = self.portal_shots.picture.as_ref()?;
        let loc = (panel.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            loc.to_f64(),
            buffer,
            None,
            None,
            Some(panel.size),
            Kind::Unspecified,
        )
        .ok()
        .map(SceneElement::Memory)
    }

    /// A program's connection ended: its questions and picker go.
    pub fn portal_shots_forget(&mut self, conn: u32) {
        let before = self.portal_shots.asking.len();
        self.portal_shots.asking.retain(|a| a.conn != conn);
        if self.portal_shots.asking.len() != before {
            self.portal_shots.armed = (!self.portal_shots.asking.is_empty()).then(Instant::now);
            self.portal_shots.picture = None;
            self.redraw_all();
        }
        if self.portal_shots.picking.as_ref().is_some_and(|a| a.conn == conn) {
            self.portal_shots.picking = None;
            self.capture_pick = None;
            self.redraw_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_read_back_and_nonsense_is_left_out() {
        let g = parse_grants(
            "allow\t/usr/lib/firefox/firefox\tfirefox\nallow\trelative\tx\ndeny\t/bin/sh\tsh\nallow\t/usr/bin/obs\n",
        );
        assert_eq!(g.len(), 2);
        assert_eq!(g.get("/usr/lib/firefox/firefox").map(String::as_str), Some("firefox"));
        assert_eq!(g.get("/usr/bin/obs").map(String::as_str), Some(""));
    }
}
