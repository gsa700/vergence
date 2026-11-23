//! The window drawn: each frame's scene (the bar we draw ourselves, the
//! panes, the lines between them, our own menu) and one terminal's cells,
//! selection, cursor and scrollbar. The window itself is in main.rs.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Instant;

use vergence_scene::{CELL_GLYPH, CELL_UNDERLINE, Cell as SceneCell, Color, Glyph, Point, Rect, Scene};
use vergence_text::{FaceId, Fonts};
use vergence_ui::Frame;
use vergence_ui::bar::{self, BAR_H, Layout};

use crate::palette::{Palette, readable_cached};
use crate::session::{NEWS_DRAW, Screen, Tab, lock};
use crate::settings::CursorShape;
use crate::{EDGE_TICK, Held, PAD, RIGHT_GUTTER, Terminal, scrollbar};

/// Text metrics, known once the fonts have been asked.
pub(crate) struct Metrics {
    regular: FaceId,
    bold: FaceId,
    /// The bar's font, if the machine has one.
    ui: Option<FaceId>,
    pub(crate) cell_w: f32,
    pub(crate) cell_h: f32,
    baseline: f32,
}

impl Terminal {
    pub(crate) fn on_scene(&mut self, frame: &Frame, fonts: &mut Fonts) -> Scene {
        (self.width, self.height) = (frame.width, frame.height);
        // For measuring (VTERM_FRAMES=1): the count of frames every thirtieth, and
        // for a tab that was asked for, how many frames and how long it took to be drawn.
        if self.measuring {
            if frame.number % 30 == 0 {
                eprintln!("frame {}", frame.number);
            }
            if let Some((tab, after, when)) = self.asked.take() {
                eprintln!(
                    "tab {tab} asked after frame {after}, drawn in frame {}, {:.1} ms later",
                    frame.number,
                    when.elapsed().as_secs_f64() * 1000.0
                );
            }
            self.frame_now = frame.number;
        }
        let mut scene = Scene::new(frame.width, frame.height);
        // Room for as much as the last frame held, so the list is not grown step by step.
        scene.commands.reserve(self.last_commands);
        // The ground is the window's background; drawn again here only when
        // nothing is to show through it.
        if self.opacity >= 1.0 {
            scene.fill(Rect::new(0.0, 0.0, frame.width, frame.height), self.palette.bg);
        }
        // As large on this screen as on any (Frame::text): new cells when it changes.
        if (frame.text - self.text).abs() > 1e-4 {
            self.text = frame.text;
            self.metrics = None;
        }
        let size = vergence_look::text_size(self.settings.mono_size * self.zoom, self.text);
        if self.metrics.is_none() {
            // The settings' fonts, or the machine's own if it does not have them.
            let mono = &self.settings.mono_family;
            self.metrics =
                fonts.face(mono, 400, false).or_else(|| fonts.face("monospace", 400, false)).map(|regular| {
                    let bold =
                        fonts.face(mono, 700, false).or_else(|| fonts.face("monospace", 700, false)).unwrap_or(regular);
                    let ui = fonts
                        .face(&self.settings.ui_family, 400, false)
                        .or_else(|| fonts.face("sans-serif", 400, false));
                    let m = fonts.metrics(regular, size);
                    let (face, glyph) = fonts.glyph(regular, 'M');
                    Metrics {
                        regular,
                        bold,
                        ui,
                        cell_w: fonts.advance(face, glyph, size),
                        cell_h: (m.ascent + m.descent + m.line_gap).ceil(),
                        baseline: m.ascent.round(),
                    }
                });
        }
        let (panes, dividers) = self.areas();
        // A selection held past the top or bottom edge: the view moves a line at a time and
        // the selection's end goes with the edge row.
        let scroll_edge = self.edge.filter(|_| self.edge_tick.elapsed() >= EDGE_TICK);
        if scroll_edge.is_some() {
            self.edge_tick = Instant::now();
        }
        let p = &self.palette;
        let Some(mt) = &self.metrics else { return scene };
        let (cw, ch) = (mt.cell_w, mt.cell_h);
        // The grid that fits a room: the whole of it for a new tab, a pane's for its terminal.
        // Columns stop short of the scrollbar at the right edge, so text never
        // runs under it, always, so a terminal's width
        // does not change when its first line scrolls away.
        let fit = |w: f32, h: f32| {
            (
                (((w - PAD - RIGHT_GUTTER) / cw).floor().max(2.0) as usize).min(2000),
                (((h - 2.0 * PAD) / ch).floor().max(1.0) as usize).min(1000),
            )
        };
        // The server draws the bar on the native road: the room starts at the top
        // then, or below the strip we draw under its bar.
        self.own_bar = !frame.bar_by_server;
        self.bar_h = if frame.bar_by_server { frame.top } else { BAR_H };
        self.fits = fit(frame.width, frame.height - self.bar_h);
        // The tabs behind are fitted as well, whenever the window's or the cells'
        // size changes: a program in a tab nobody is looking at is told its real
        // size all the same (it was left at the size it started with until shown).
        let fitted = (frame.width, frame.height, cw, ch);
        if self.fitted != fitted {
            self.fitted = fitted;
            let room = self.room();
            for (_, tab) in self.tabs.iter().enumerate().filter(|(index, _)| *index != self.shown) {
                for (id, area) in tab.tree.layout(room).0 {
                    if let Some(session) = tab.session(id) {
                        let (cols, rows) = fit(area.w, area.h);
                        session.fit(&mut lock(&session.screen), cols, rows);
                    }
                }
            }
        }

        if !frame.bar_by_server {
            let titles: Vec<String> = self.tabs.iter().map(Tab::label).collect();
            let alerts: Vec<bool> = self.tabs.iter().map(Tab::as_root).collect();
            let layout =
                Layout::new(frame.width, self.tabs.len(), self.shown, &self.settings.left, &self.settings.right);
            bar::draw(
                &mut scene,
                fonts,
                &layout,
                &bar::Look {
                    bar: vergence_look::bar_look(self.settings.light),
                    frosted: false,
                    focused: self.focused,
                    pane: p.bg,
                    pane_ink: p.fg,
                    font: mt.ui,
                    font_size: self.settings.ui_size,
                    titles: &titles,
                    alerts: &alerts,
                    edited: &[],
                    grants: &[],
                    title: "",
                    shown: self.shown,
                    hover: self.hover,
                    dragged: None,
                    tools: &[],
                    tool_names: &[],
                    app_icon: None,
                    menu_open: false,
                    root: false,
                },
            );
        }

        let Some(tab) = self.tabs.get(self.shown) else { return scene };
        // Scene font numbers for the faces used in this frame.
        let mut numbers: HashMap<FaceId, u32> = HashMap::new();
        for (id, area) in &panes {
            let Some(session) = tab.session(*id) else { continue };
            let active = *id == tab.active;
            // What is drawn now is drawn: output after this asks for a frame again.
            session.news.fetch_and(!NEWS_DRAW, Ordering::SeqCst);
            let mut guard = lock(&session.screen);
            let s = &mut *guard;
            // The terminal's grid follows its room, and its program is told.
            let (cols, rows) = fit(area.w, area.h);
            session.fit(s, cols, rows);
            if active && let Some((up, col)) = scroll_edge {
                s.scroll_by(if up { 1 } else { -1 });
                let row = s.top_line() + if up { 0 } else { s.grid.rows - 1 };
                s.drag_selection((col, row));
            }
            // A pane along the top of a window that draws under its bar fills the strip above it.
            let above = if !self.own_bar && area.y <= self.bar_h + 0.5 { self.bar_h } else { 0.0 };
            let look = PaneLook {
                palette: p,
                metrics: mt,
                size,
                keyboard: active && self.focused,
                thumb_held: active && matches!(self.held, Some(Held::Thumb(_))),
                above,
                cursor: self.cursor_shape,
                cursor_on: !self.blink || self.blink_on,
            };
            draw_pane(&mut scene, fonts, &mut numbers, s, *area, &look);
        }
        // The lines between panes; the one held, in the accent.
        for d in &dividers {
            let held = matches!(&self.held, Some(Held::Divider(path)) if *path == d.path);
            scene.fill(d.line, if held { p.accent } else { Color::rgba(p.fg.r, p.fg.g, p.fg.b, 56) });
        }
        if let Some(menu) = &self.menu {
            menu.draw(
                &mut scene,
                fonts,
                mt.ui,
                vergence_look::text_size(self.settings.ui_size, self.text),
                p.accent,
                self.menu_hover,
            );
        }
        // An agent typing in the tab in front: its mark at the top right,
        // a click on which stops it (agent.rs).
        self.agents.mark = None;
        if let Some((tab, agent)) = self.agent_typing_in_front()
            && let Some((_, area)) = panes.first()
        {
            let size = vergence_look::text_size(self.settings.ui_size, self.text);
            let words = format!("{agent} is typing \u{b7} stop");
            let w = fonts.layout(mt.ui.unwrap_or(mt.regular), size, &words).1 + 20.0;
            let h = (size * 1.8).round();
            let r = vergence_scene::Rect::new(area.x + area.w - w - 12.0, area.y + 8.0, w, h);
            scene.fill_rounded(r, h / 2.0, Color::rgba(p.accent.r, p.accent.g, p.accent.b, 235));
            vergence_render::text(
                &mut scene,
                fonts,
                mt.ui.unwrap_or(mt.regular),
                size,
                Color::rgb(255, 255, 255),
                vergence_scene::Point::new(r.x + 10.0, (r.y + h / 2.0 + size * 0.35).round()),
                &words,
            );
            self.agents.mark = Some((r, tab));
        }
        self.last_commands = scene.commands.len();
        scene
    }
}

/// The scene's number for a face, given once in a frame.
fn font_number(scene: &mut Scene, fonts: &Fonts, numbers: &mut HashMap<FaceId, u32>, face: FaceId) -> u32 {
    *numbers.entry(face).or_insert_with(|| {
        let n = fonts.name(face);
        scene.font(&n.family, n.weight, n.italic)
    })
}

/// How a pane is drawn.
struct PaneLook<'a> {
    palette: &'a Palette,
    metrics: &'a Metrics,
    size: f32,
    /// The keys go to this pane, and the window has the keyboard.
    keyboard: bool,
    /// The scrollbar's thumb is held.
    thumb_held: bool,
    /// How much room above the pane lies under the window's bar: filled with
    /// the lines before the top row (none on the alternate screen).
    above: f32,
    /// The cursor's shape, and whether it shows now (it blinks).
    cursor: CursorShape,
    cursor_on: bool,
}

/// Draws one terminal in its room.
fn draw_pane(
    scene: &mut Scene,
    fonts: &mut Fonts,
    numbers: &mut HashMap<FaceId, u32>,
    s: &Screen,
    area: Rect,
    look: &PaneLook,
) {
    let (p, mt, size) = (look.palette, look.metrics, look.size);
    let (cw, ch) = (mt.cell_w, mt.cell_h);
    let (left, first) = (area.x + PAD, area.y + PAD);
    let (selection, top_line) = (s.selection, s.top_line());
    // The font and glyph of each plain character, regular and bold, found once
    // a frame: a screen of text asks for the same hundred some thousands of times.
    let mut plain: [[Option<(u32, u32)>; 128]; 2] = [[None; 128]; 2];
    // Under the window's bar, above the top row: the lines before it, as many
    // as reach into the strip (look milestone 1). The alternate screen has none.
    let extra =
        if s.grid.alt || look.above <= 0.0 { 0 } else { (((look.above + PAD) / ch).ceil() as usize).min(top_line) };
    scene.push_clip(area, 0.0);
    // The screen is one grid of cells ([`vergence_scene::Command::Cells`]): each
    // cell its ground (none where it is the terminal's own), its ink, its glyph.
    let columns = s.grid.cols;
    let mut cells: Vec<SceneCell> = Vec::with_capacity(columns * (s.grid.rows + extra));
    // Combining marks are few, and drawn over the grid as glyph runs of their own.
    let mut marks: Vec<(usize, u32, Color, Point, Glyph)> = Vec::new();
    let mut floor_memo = ((p.fg, p.fg), p.fg);
    let floor_bits = p.floor.to_bits();
    for r in 0..s.grid.rows + extra {
        let line = if r < extra { s.grid.abs_line(top_line - (extra - r)).unwrap_or(&[]) } else { s.shown(r - extra) };
        let top = first + (r as f32 - extra as f32) * ch;
        for (col, cell) in line.iter().take(columns).enumerate() {
            let st = cell.style;
            let (mut ink, mut ground) = (p.resolve(st.fg, p.fg, st.bold), p.resolve(st.bg, p.bg, false));
            if st.inverse {
                // The text takes the ground's color as it is, not its bright form.
                (ink, ground) = (ground, ink);
            }
            // The palette's floor (light ones): runs of one color pair are the
            // rule, so the last pair's answer is kept.
            if p.floor > 0.0 {
                if floor_memo.0 != (ink, ground) {
                    floor_memo = ((ink, ground), readable_cached(ink, ground, p.floor, floor_bits));
                }
                ink = floor_memo.1;
            }
            if st.dim {
                ink.a = 153;
            }
            let mut out = SceneCell {
                ground: if ground == p.bg { Color::rgba(0, 0, 0, 0) } else { ground },
                ink,
                glyph: 0,
                font: 0,
                flags: if st.underline { CELL_UNDERLINE } else { 0 },
            };
            if cell.width != 0 && cell.c != ' ' {
                let own = if st.bold { mt.bold } else { mt.regular };
                let (font, glyph) = match plain[st.bold as usize].get_mut(cell.c as usize) {
                    Some(Some(known)) => *known,
                    slot => {
                        let (face, glyph) = fonts.glyph(own, cell.c);
                        let found = (font_number(scene, fonts, numbers, face), glyph);
                        if let Some(slot) = slot {
                            *slot = Some(found);
                        }
                        found
                    }
                };
                (out.glyph, out.font, out.flags) = (glyph, font as u16, out.flags | CELL_GLYPH);
                // Fonts make marks one of two ways: with no advance, drawn back over
                // what came before the pen (so the pen goes after the character), or as
                // wide as a cell, drawn in place (so it goes where the character's did).
                for mark in cell.marks.into_iter().filter(|m| *m != '\0') {
                    let (face, glyph) = fonts.glyph(own, mark);
                    let font = font_number(scene, fonts, numbers, face);
                    let after = fonts.advance(face, glyph, size) < cw * 0.5;
                    marks.push((
                        r,
                        font,
                        ink,
                        Point::new(left, top + mt.baseline),
                        Glyph {
                            id: glyph,
                            x: (col + if after { cell.width.max(1) as usize } else { 0 }) as f32 * cw,
                            y: 0.0,
                        },
                    ));
                }
            }
            cells.push(out);
        }
        // A line shorter than the screen is wide (kept from a narrower one) ends in nothing.
        cells.resize((r + 1) * columns, SceneCell::default());
    }
    // The lines before the top row only in the strip under the bar, cut at
    // its edge: none of them shows in the pane's own room.
    let below = cells.split_off(extra * columns);
    if extra > 0 {
        scene.pop_clip();
        scene.push_clip(Rect::new(area.x, area.y - look.above, area.w, look.above), 0.0);
        scene.cells(Point::new(left, first - extra as f32 * ch), cw, ch, mt.baseline, size, columns as u32, cells);
        for (_, font, ink, origin, glyph) in marks.iter().filter(|m| m.0 < extra) {
            scene.glyphs(*font, size, *ink, *origin, vec![*glyph]);
        }
        scene.pop_clip();
        scene.push_clip(area, 0.0);
    }
    scene.cells(Point::new(left, first), cw, ch, mt.baseline, size, columns as u32, below);
    for (_, font, ink, origin, glyph) in marks.into_iter().filter(|m| m.0 >= extra) {
        scene.glyphs(font, size, ink, origin, vec![glyph]);
    }
    // What is selected, tinted over its text.
    for r in 0..s.grid.rows {
        if let Some((a, b)) = selection.and_then(|sel| sel.span(top_line + r, s.grid.cols)) {
            scene.fill(
                Rect::new(left + a as f32 * cw, first + r as f32 * ch, (b + 1 - a) as f32 * cw, ch),
                Color::rgba(p.accent.r, p.accent.g, p.accent.b, 89),
            );
        }
    }
    // The cursor: a block where the keys go, an outline elsewhere. It moves down with the
    // screen while scrolled back, out of sight past the bottom.
    let cursor_row = s.grid.cy + s.scroll;
    if s.grid.cursor_visible && cursor_row < s.grid.rows {
        let at = Rect::new(left + s.grid.cx as f32 * cw, first + cursor_row as f32 * ch, cw, ch);
        // Its shape as the settings say: a block, a bar before the cell, a line under it.
        let shape = match look.cursor {
            CursorShape::Block => at,
            CursorShape::Bar => Rect::new(at.x, at.y, 2.0, at.h),
            CursorShape::Underline => Rect::new(at.x, at.y + at.h - 2.0, at.w, 2.0),
        };
        if look.keyboard {
            if look.cursor_on {
                scene.fill(
                    shape,
                    Color::rgba(
                        p.accent.r,
                        p.accent.g,
                        p.accent.b,
                        if look.cursor == CursorShape::Block { 178 } else { 230 },
                    ),
                );
            }
        } else if look.cursor == CursorShape::Block {
            scene.stroke(at, 0.0, 1.0, p.accent);
        } else {
            scene.fill(shape, Color::rgba(p.accent.r, p.accent.g, p.accent.b, 140));
        }
    }
    // The scrollbar: faint at the live screen, clearer while scrolled back or held.
    if let Some(bar) = s.scrollbar(area) {
        scene.fill_rounded(bar.track, scrollbar::RADIUS, Color::rgba(p.fg.r, p.fg.g, p.fg.b, 15));
        let alpha = if look.thumb_held {
            166
        } else if s.scroll > 0 {
            128
        } else {
            64
        };
        scene.fill_rounded(bar.thumb, scrollbar::RADIUS, Color::rgba(p.fg.r, p.fg.g, p.fg.b, alpha));
    }
    // Scrolled back: how far, in the top right corner (left of the scrollbar).
    if s.scroll > 0 {
        let label = if s.scroll == 1 { "1 line back".to_string() } else { format!("{} lines back", s.scroll) };
        let width = label.chars().count() as f32 * cw + 16.0;
        let at = Rect::new(area.x + area.w - width - PAD - scrollbar::GRAB, first, width, ch + 4.0);
        scene.fill_rounded(at, bar::RADIUS, Color::rgba(p.accent.r, p.accent.g, p.accent.b, 235));
        let font = font_number(scene, fonts, numbers, mt.regular);
        let glyphs = label
            .chars()
            .enumerate()
            .map(|(i, c)| Glyph { id: fonts.glyph(mt.regular, c).1, x: i as f32 * cw, y: 0.0 })
            .collect();
        scene.glyphs(font, size, p.bg, Point::new(at.x + 8.0, at.y + 2.0 + mt.baseline), glyphs);
    }
    scene.pop_clip();
}
