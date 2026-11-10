//! The toolkit's first window: a scene on the display server, drawn at the
//! monitor's exact scale by our own renderer, through our own Wayland code.
//!
//!     cargo run --release -p vergence-ui --example window
//!     cargo run --release -p vergence-ui --example window -- --animate --frames 300
//!
//! `--animate` keeps a bar moving, to see frames arrive in step with the
//! monitor; `--frames N` closes after N frames and prints the rate, for
//! running over SSH. `--own-frame` asks for no title bar from the server.
//! `--log` prints every input as it arrives. `--copy TEXT` puts TEXT on the
//! clipboard at every left click; a right click pastes. (For testing the
//! clipboard between two windows without a keyboard.)
//!
//! Input: the shapes light up under the pointer and take a click; typing
//! goes into the line at the bottom (Backspace deletes, Ctrl+C copies the
//! line, Ctrl+V pastes into it, Ctrl+Q closes); the
//! wheel changes the grid; dragging the background moves the window, the
//! bottom right corner resizes it.

use vergence_render::text;
use vergence_scene::{Color, Point, Rect, Scene};
use vergence_text::Fonts;
use vergence_ui::{App, Button, Cursor, Edges, Frame, Input, Window, sym};

struct Demo {
    animate: bool,
    own_frame: bool,
    stop_after: Option<u64>,
    /// How long drawing took, frame by frame, and when the last few were shown.
    draw_ms: Vec<f64>,
    last: Option<(f32, f32, f32)>,
    drawn_by: [u64; 2],
    log: bool,
    /// The window's size at the last frame, to know where things are.
    size: (f32, f32),
    pointer: Option<(f32, f32)>,
    /// Which shape was last pressed, and where the last click landed.
    picked: Option<usize>,
    click: Option<(f32, f32)>,
    typed: String,
    focused: bool,
    grid: f32,
    inputs: u64,
    copy_on_click: Option<String>,
}

impl Demo {
    /// How many shapes fit across, and which one is at a point.
    fn shapes(&self) -> usize {
        ((self.size.0 - 48.0) / 96.0).floor().max(1.0) as usize
    }

    fn shape_at(&self, x: f32, y: f32) -> Option<usize> {
        let i = ((x - 24.0) / 96.0).floor();
        ((150.0..198.0).contains(&y) && i >= 0.0 && (i as usize) < self.shapes() && x - 24.0 - i * 96.0 <= 80.0).then_some(i as usize)
    }

    fn in_corner(&self, x: f32, y: f32) -> bool {
        x >= self.size.0 - 18.0 && y >= self.size.1 - 18.0
    }
}

const GROUND: Color = Color::rgb(24, 33, 41);
const INK: Color = Color::rgb(227, 233, 238);
const SOFT: Color = Color::rgb(154, 169, 181);
const TEAL: Color = Color::rgb(74, 155, 184);
const AMBER: Color = Color::rgb(224, 164, 58);

impl App for Demo {
    fn title(&self) -> String {
        "Vergence toolkit".into()
    }

    fn app_id(&self) -> String {
        "vergence-demo".into()
    }

    fn own_frame(&self) -> bool {
        self.own_frame
    }

    fn animating(&self) -> bool {
        self.animate
    }

    fn scene(&mut self, f: &Frame, fonts: &mut Fonts) -> Scene {
        let (w, h) = (f.width, f.height);
        self.size = (w, h);
        let mut s = Scene::new(w, h);
        s.fill(Rect::new(0.0, 0.0, w, h), GROUND);
        s.stroke(Rect::new(0.5, 0.5, w - 1.0, h - 1.0), 0.0, 1.0, Color::rgb(45, 59, 70));
        let sans = fonts.face("sans-serif", 400, false);
        let bold = fonts.face("sans-serif", 700, false);
        let mono = fonts.face("monospace", 400, false);
        if let Some(face) = bold {
            text(&mut s, fonts, face, 26.0, INK, Point::new(24.0, 48.0), "A window of our own");
        }
        if let Some(face) = sans {
            text(&mut s, fonts, face, 14.0, SOFT, Point::new(24.0, 74.0), "Scene, text, renderer and Wayland code written by the project.");
        }
        if let Some(face) = mono {
            let pixels = ((w * f.scale).ceil(), (h * f.scale).ceil());
            let line = format!("{w} x {h} units  scale {:.4}  {} x {} pixels  frame {}", f.scale, pixels.0, pixels.1, f.number);
            text(&mut s, fonts, face, 13.33, AMBER, Point::new(24.0, 104.0), &line);
            if let Some(avg) = (!self.draw_ms.is_empty()).then(|| self.draw_ms.iter().rev().take(30).sum::<f64>() / self.draw_ms.len().min(30) as f64) {
                text(&mut s, fonts, face, 13.33, SOFT, Point::new(24.0, 124.0), &format!("drawn in {avg:.1} ms a frame, on the {}", if f.drawn_by.is_empty() { "?" } else { f.drawn_by }));
            }
        }
        // A row of shapes that fills the width, so resizing shows.
        let n = ((w - 48.0) / 96.0).floor().max(1.0) as usize;
        for i in 0..n {
            let x = 24.0 + i as f32 * 96.0;
            let c = if i % 2 == 0 { TEAL } else { AMBER };
            let under = self.pointer.and_then(|(px, py)| self.shape_at(px, py)) == Some(i);
            let c = if self.picked == Some(i) { INK } else if under { Color::rgb(c.r.saturating_add(40), c.g.saturating_add(40), c.b.saturating_add(40)) } else { c };
            s.fill_rounded(Rect::new(x, 150.0, 80.0, 48.0), (i * 6) as f32, c);
            s.stroke(Rect::new(x, 212.0, 80.0, 48.0), (i * 6) as f32, 1.5, c);
        }
        // A hairline grid: one unit wide, which is one, two or three sharp pixels by the scale.
        let mut y = 280.0;
        while y < h - 100.0 {
            s.fill(Rect::new(24.0, y, w - 48.0, 1.0), Color::rgba(255, 255, 255, 28));
            y += self.grid;
        }
        // Where the last click landed, and what was typed.
        if let Some((cx, cy)) = self.click {
            s.line(Point::new(cx - 6.0, cy), Point::new(cx + 6.0, cy), 1.5, INK);
            s.line(Point::new(cx, cy - 6.0), Point::new(cx, cy + 6.0), 1.5, INK);
        }
        s.fill_rounded(Rect::new(24.0, h - 86.0, w - 48.0, 26.0), 5.0, Color::rgba(255, 255, 255, if self.focused { 28 } else { 12 }));
        if let Some(face) = mono {
            let shown = if self.typed.is_empty() && !self.focused { "click here and type".to_string() } else { format!("{}{}", self.typed, if self.focused { "\u{2581}" } else { "" }) };
            s.push_clip(Rect::new(24.0, h - 86.0, w - 48.0, 26.0), 0.0);
            text(&mut s, fonts, face, 13.33, if self.typed.is_empty() && !self.focused { SOFT } else { INK }, Point::new(32.0, h - 68.0), &shown);
            s.pop_clip();
            let at = self.pointer.map_or("pointer outside".to_string(), |(x, y)| format!("pointer {x:.1}, {y:.1}"));
            text(&mut s, fonts, face, 11.0, SOFT, Point::new(24.0, h - 40.0), &format!("{at}   inputs {}", self.inputs));
        }
        // The corner that resizes.
        for i in 0..3 {
            let d = 5.0 + i as f32 * 4.0;
            s.line(Point::new(w - d, h - 3.0), Point::new(w - 3.0, h - d), 1.0, SOFT);
        }
        if self.animate {
            // Back and forth across the window every four seconds.
            let phase = (f.time / 4.0).fract();
            let t = if phase < 0.5 { phase * 2.0 } else { 2.0 - phase * 2.0 } as f32;
            s.fill_rounded(Rect::new(24.0 + t * (w - 48.0 - 60.0), h - 30.0, 60.0, 12.0), 6.0, AMBER);
        }
        self.last = Some((w, h, f.scale));
        match f.drawn_by {
            "GPU" => self.drawn_by[0] += 1,
            "CPU" => self.drawn_by[1] += 1,
            _ => {}
        }
        s
    }

    fn input(&mut self, input: &Input, window: &mut Window) {
        self.inputs += 1;
        if self.log {
            println!("{input:?}");
        }
        match input {
            Input::PointerMoved { x, y } => {
                self.pointer = Some((*x, *y));
                window.set_cursor(if self.in_corner(*x, *y) {
                    Cursor::Resize(Edges { bottom: true, right: true, ..Edges::default() })
                } else if self.shape_at(*x, *y).is_some() {
                    Cursor::Pointer
                } else if (self.size.1 - 86.0..self.size.1 - 60.0).contains(y) {
                    Cursor::Text
                } else {
                    Cursor::Default
                });
            }
            Input::PointerLeft => self.pointer = None,
            Input::Button { button: Button::Right, pressed: true, .. } => window.paste(),
            Input::Paste(text) => self.typed.extend(text.chars().map(|c| if c.is_control() { ' ' } else { c })),
            Input::Button { button: Button::Left, pressed: true, x, y, .. } => {
                if let Some(text) = &self.copy_on_click {
                    window.copy(text);
                }
                if self.in_corner(*x, *y) {
                    window.start_resize(Edges { bottom: true, right: true, ..Edges::default() });
                } else if let Some(i) = self.shape_at(*x, *y) {
                    self.picked = Some(i);
                    self.click = Some((*x, *y));
                } else if *y < 140.0 {
                    window.start_move();
                } else {
                    self.click = Some((*x, *y));
                }
            }
            Input::Scroll { dy, .. } => self.grid = (self.grid + dy.signum() * 2.0).clamp(6.0, 60.0),
            Input::Key { sym: s, text, pressed: true, composing: false, modifiers, .. } => {
                let ctrl = |c: char| modifiers.ctrl && text.is_none() && (*s == c as u32 || *s == c.to_ascii_uppercase() as u32);
                if ctrl('q') {
                    window.close();
                } else if ctrl('c') {
                    window.copy(&self.typed);
                } else if ctrl('v') {
                    window.paste();
                } else if *s == sym::BACKSPACE {
                    self.typed.pop();
                } else if *s == sym::ESCAPE {
                    self.typed.clear();
                } else if let Some(t) = text {
                    self.typed.push_str(t);
                }
            }
            Input::Focus(on) => self.focused = *on,
            _ => {}
        }
        window.redraw();
    }

    fn shown(&mut self, f: &Frame, draw_ms: f64) -> bool {
        self.draw_ms.push(draw_ms);
        match self.stop_after {
            Some(n) if f.number + 1 >= n => {
                let (w, h, scale) = self.last.unwrap_or_default();
                let mean = self.draw_ms.iter().sum::<f64>() / self.draw_ms.len() as f64;
                println!("{} frames in {:.2} s: {:.1} a second; {w} x {h} units at scale {scale:.4}; drawing {mean:.1} ms a frame (first {:.1}); {} frames on the GPU, {} on the CPU", f.number + 1, f.time, (f.number + 1) as f64 / f.time.max(0.001), self.draw_ms[0], self.drawn_by[0], self.drawn_by[1]);
                false
            }
            _ => true,
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok());
    let mut demo = Demo {
        animate: flag("--animate"),
        own_frame: flag("--own-frame"),
        stop_after: value("--frames"),
        draw_ms: Vec::new(),
        last: None,
        drawn_by: [0, 0],
        log: flag("--log"),
        size: (640.0, 420.0),
        pointer: None,
        picked: None,
        click: None,
        typed: String::new(),
        focused: false,
        grid: 20.0,
        inputs: 0,
        copy_on_click: args.iter().position(|a| a == "--copy").and_then(|i| args.get(i + 1)).cloned(),
    };
    if let Err(e) = vergence_ui::run(&mut demo, (640, 420)) {
        eprintln!("vergence-ui: {e}");
        std::process::exit(1);
    }
}
