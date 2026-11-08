// One terminal: its screen, parser, shell and drawing (a tab, or later a pane).

use iced::keyboard::{self, Key, key::Named};
use iced::widget::canvas::{self, Frame, Geometry, Program};
use iced::{Color, Font, Pixels, Point, Rectangle, Renderer, Size, Theme, mouse};

use crate::grid::{Cell, Color as TColor, Grid};
use crate::{pty, vt};

/// Space around the grid, in points.
pub const PAD: f32 = 6.0;

pub struct Tab {
    pub id: u64,
    pub grid: Grid,
    parser: vt::Parser,
    pub pty: pty::Pty,
    pub cache: canvas::Cache,
    pub font_size: f32,
    pub cell: Size,
}

/// The desktop's monospace font size in points ("Monospace 10" -> 10), as
/// logical pixels (points at 96 per inch).
pub fn font_size() -> f32 {
    let path = std::path::Path::new(&std::env::var_os("HOME").unwrap_or_default()).join(".config/vergence/settings.toml");
    let pt = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| vergence_xsettings::from_toml(&t).ok())
        .and_then(|s| match s.get("Gtk/MonospaceFontName") {
            Some(vergence_xsettings::Value::Str(v)) => v.rsplit_once(' ').and_then(|(_, n)| n.parse::<f32>().ok()),
            _ => None,
        })
        .unwrap_or(10.0);
    pt * 96.0 / 72.0
}

/// A cell's size for a font size: monospace advance 0.602 em (DejaVu Sans
/// Mono, Noto Sans Mono), lines 1.25 em.
pub fn cell_for(size: f32) -> Size {
    Size::new((size * 0.6021).max(1.0), (size * 1.25).ceil())
}

impl Tab {
    /// A new terminal running the user's shell, cols x rows.
    pub fn new(id: u64, cols: usize, rows: usize) -> std::io::Result<(Tab, std::fs::File)> {
        let size = font_size();
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let pty = pty::Pty::spawn(&shell, cols as u16, rows as u16)?;
        let reader = pty.reader()?;
        Ok((
            Tab { id, grid: Grid::new(cols, rows), parser: vt::Parser::new(), pty, cache: canvas::Cache::new(), font_size: size, cell: cell_for(size) },
            reader,
        ))
    }

    /// The shell's output: onto the screen; replies back to the shell.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.grid, bytes);
        if !self.grid.out.is_empty() {
            let out = std::mem::take(&mut self.grid.out);
            self.pty.write(&out);
        }
        if self.grid.dirty {
            self.grid.dirty = false;
            self.cache.clear();
        }
    }

    /// Fit the grid to an area of `size` points.
    pub fn fit(&mut self, size: Size) {
        let cols = ((size.width - 2.0 * PAD) / self.cell.width).floor().max(2.0) as usize;
        let rows = ((size.height - 2.0 * PAD) / self.cell.height).floor().max(1.0) as usize;
        if (cols, rows) != (self.grid.cols, self.grid.rows) {
            self.grid.resize(cols, rows);
            self.pty.resize(cols as u16, rows as u16);
            self.cache.clear();
        }
    }

    pub fn title(&self) -> String {
        if self.grid.title.is_empty() { "Terminal".into() } else { self.grid.title.clone() }
    }
}

/// What a key sends to the program (xterm's sequences).
pub fn key_bytes(key: &Key, mods: keyboard::Modifiers, text: Option<&str>, app_cursor: bool) -> Option<Vec<u8>> {
    let csi = |s: &str| Some(format!("\x1b[{s}").into_bytes());
    let cursor = |c: char| Some(if app_cursor { format!("\x1bO{c}") } else { format!("\x1b[{c}") }.into_bytes());
    if let Key::Named(n) = key {
        return match n {
            Named::Enter => Some(b"\r".to_vec()),
            Named::Backspace => Some(if mods.control() { vec![0x08] } else { vec![0x7f] }),
            Named::Tab => Some(if mods.shift() { b"\x1b[Z".to_vec() } else { b"\t".to_vec() }),
            Named::Escape => Some(vec![0x1b]),
            Named::ArrowUp => cursor('A'),
            Named::ArrowDown => cursor('B'),
            Named::ArrowRight => cursor('C'),
            Named::ArrowLeft => cursor('D'),
            Named::Home => cursor('H'),
            Named::End => cursor('F'),
            Named::PageUp => csi("5~"),
            Named::PageDown => csi("6~"),
            Named::Insert => csi("2~"),
            Named::Delete => csi("3~"),
            Named::F1 => Some(b"\x1bOP".to_vec()),
            Named::F2 => Some(b"\x1bOQ".to_vec()),
            Named::F3 => Some(b"\x1bOR".to_vec()),
            Named::F4 => Some(b"\x1bOS".to_vec()),
            Named::F5 => csi("15~"),
            Named::F6 => csi("17~"),
            Named::F7 => csi("18~"),
            Named::F8 => csi("19~"),
            Named::F9 => csi("20~"),
            Named::F10 => csi("21~"),
            Named::F11 => csi("23~"),
            Named::F12 => csi("24~"),
            Named::Space => Some(if mods.control() { vec![0] } else { b" ".to_vec() }),
            _ => None,
        };
    }
    // Ctrl+letter: the control character.
    if mods.control()
        && let Key::Character(c) = key
        && let Some(ch) = c.chars().next()
    {
        let ch = ch.to_ascii_lowercase();
        let b = match ch {
            'a'..='z' => Some(ch as u8 - b'a' + 1),
            '[' => Some(0x1b),
            '\\' => Some(0x1c),
            ']' => Some(0x1d),
            '6' => Some(0x1e),
            '-' | '/' => Some(0x1f),
            _ => None,
        };
        return b.map(|b| if mods.alt() { vec![0x1b, b] } else { vec![b] });
    }
    let t = text.filter(|t| !t.is_empty() && !t.chars().any(|c| c.is_control()))?;
    let mut out = Vec::new();
    if mods.alt() {
        out.push(0x1b);
    }
    out.extend_from_slice(t.as_bytes());
    Some(out)
}

/// The colours (dark; schemes come in step 4): background, foreground, cursor,
/// and the 16 ANSI colours.
struct Palette {
    bg: Color,
    fg: Color,
    cursor: Color,
    ansi: [Color; 16],
}

fn rgb(v: u32) -> Color {
    Color::from_rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn palette() -> Palette {
    Palette {
        bg: rgb(0x1e1e2e),
        fg: rgb(0xcdd6f4),
        cursor: rgb(0x89b4fa),
        ansi: [
            rgb(0x45475a), rgb(0xf38ba8), rgb(0xa6e3a1), rgb(0xf9e2af), rgb(0x89b4fa), rgb(0xf5c2e7), rgb(0x94e2d5), rgb(0xbac2de),
            rgb(0x585b70), rgb(0xf38ba8), rgb(0xa6e3a1), rgb(0xf9e2af), rgb(0x89b4fa), rgb(0xf5c2e7), rgb(0x94e2d5), rgb(0xa6adc8),
        ],
    }
}

/// xterm's 256 colours: 16 ANSI, a 6x6x6 cube, 24 greys.
fn indexed(p: &Palette, i: u8) -> Color {
    match i {
        0..=15 => p.ansi[i as usize],
        16..=231 => {
            let n = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color::from_rgb8(level(n / 36), level((n / 6) % 6), level(n % 6))
        }
        _ => {
            let g = 8 + (i - 232) * 10;
            Color::from_rgb8(g, g, g)
        }
    }
}

fn resolve(p: &Palette, c: TColor, default: Color, bold: bool) -> Color {
    match c {
        TColor::Default => default,
        // Bold makes the first 8 colours bright, as xterm does.
        TColor::Indexed(i) if bold && i < 8 => indexed(p, i + 8),
        TColor::Indexed(i) => indexed(p, i),
        TColor::Rgb(r, g, b) => Color::from_rgb8(r, g, b),
    }
}

impl<M> Program<M> for Tab {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let p = palette();
        let geo = self.cache.draw(renderer, bounds.size(), |frame: &mut Frame| {
            frame.fill_rectangle(Point::ORIGIN, bounds.size(), p.bg);
            let (cw, ch) = (self.cell.width, self.cell.height);
            for y in 0..self.grid.rows {
                let line = self.grid.line(y);
                let top = PAD + y as f32 * ch;
                // Runs of cells with the same look: one background, one text each.
                let mut x = 0;
                while x < line.len() {
                    let style = line[x].style;
                    let mut end = x + 1;
                    while end < line.len() && line[end].style == style {
                        end += 1;
                    }
                    let (mut fg, mut bg) = (resolve(&p, style.fg, p.fg, style.bold), resolve(&p, style.bg, p.bg, false));
                    if style.inverse {
                        std::mem::swap(&mut fg, &mut bg);
                    }
                    if style.dim {
                        fg.a = 0.6;
                    }
                    if bg != p.bg {
                        frame.fill_rectangle(Point::new(PAD + x as f32 * cw, top), Size::new((end - x) as f32 * cw, ch), bg);
                    }
                    let text: String = line[x..end].iter().filter(|c: &&Cell| c.width != 0).map(|c| c.c).collect();
                    if !text.trim().is_empty() {
                        frame.fill_text(canvas::Text {
                            content: text,
                            position: Point::new(PAD + x as f32 * cw, top + (ch - self.font_size) / 2.0),
                            color: fg,
                            size: Pixels(self.font_size),
                            font: if style.bold { Font { weight: iced::font::Weight::Bold, ..Font::MONOSPACE } } else { Font::MONOSPACE },
                            ..canvas::Text::default()
                        });
                    }
                    if style.underline {
                        frame.fill_rectangle(Point::new(PAD + x as f32 * cw, top + ch - 2.0), Size::new((end - x) as f32 * cw, 1.0), fg);
                    }
                    x = end;
                }
            }
            if self.grid.cursor_visible {
                let at = Point::new(PAD + self.grid.cx as f32 * cw, PAD + self.grid.cy as f32 * ch);
                frame.fill_rectangle(at, Size::new(cw, ch), Color { a: 0.7, ..p.cursor });
            }
        });
        vec![geo]
    }
}

