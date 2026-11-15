//! The text area (editor milestone 1, step 2): many lines of text to read
//! and edit, with line numbers, wrapping, indenting, undo, the clipboard,
//! the keyboard and the pointer.
//!
//! The text lives in a [`TextDoc`], a handle the program keeps and gives to
//! [`text_area`] each time it builds its tree, so a large file is never
//! copied into the tree. The widget edits it in place and sends the
//! program's message ([`Element::on_change`]) when the text or the caret
//! changed; the program reads what it needs from the handle.
//!
//! Only the lines in view are laid out (each kept until it changes), so the
//! size of the file does not slow typing or scrolling. Lines not laid out
//! yet are taken to be as tall as their length suggests; the view is held
//! by its top line, so those guesses never make what is shown jump.

use std::{
    cell::{Ref, RefCell, RefMut},
    rc::Rc,
    time::Instant,
};

use vergence_render::text;
use vergence_scene::{Color, Point, Rect, Scene};
use vergence_text::{FaceId, Fonts};
use vergence_ui::{Modifiers, sym};

use crate::edit::{Buffer, Editable, Ending, History, Kind, Pos};
use crate::element::{Element, Kind as EKind, Length};
use crate::theme::Theme;

/// The keypad's Enter.
const KP_ENTER: u32 = 0xff8d;
/// Room around the text.
const PAD_X: f32 = 6.0;
const PAD_Y: f32 = 4.0;
/// Room each side of the line numbers.
const GUTTER_PAD: f32 = 8.0;
const BAR_W: f32 = 6.0;
const MIN_THUMB: f32 = 24.0;
/// A text this long or shorter is told whole to a screen reader.
const TELL_WHOLE: usize = 64 * 1024;

/// How a text area shows and edits its text.
#[derive(Clone, Debug, PartialEq)]
pub struct AreaOptions {
    /// The font's family; "monospace" by default.
    pub font: String,
    /// The text's size in points; None: the desktop's text size.
    pub size: Option<f32>,
    pub line_numbers: bool,
    /// Long lines wrap at the area's edge; otherwise the area scrolls sideways.
    pub wrap: bool,
    /// How many spaces wide a tab is.
    pub tab_width: usize,
    /// Tab inserts spaces rather than a tab.
    pub spaces: bool,
    /// The caret's line is lightly shaded.
    pub current_line: bool,
    /// Can be edited (false: read-only, still selected and copied from).
    pub editable: bool,
}

impl Default for AreaOptions {
    fn default() -> Self {
        AreaOptions { font: "monospace".into(), size: None, line_numbers: true, wrap: true, tab_width: 4, spaces: true, current_line: true, editable: true }
    }
}

/// A text being edited, shared by the program and its text area.
#[derive(Clone)]
pub struct TextDoc(Rc<RefCell<Doc>>);

impl TextDoc {
    pub fn new(text: &str) -> TextDoc {
        TextDoc::with_options(text, AreaOptions::default())
    }

    pub fn with_options(text: &str, options: AreaOptions) -> TextDoc {
        TextDoc(Rc::new(RefCell::new(Doc::new(Buffer::from_text(text), options))))
    }

    pub fn borrow(&self) -> Ref<'_, Doc> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<'_, Doc> {
        self.0.borrow_mut()
    }

    /// The same document (not just the same text).
    pub fn same(&self, other: &TextDoc) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// One line laid out: where each character boundary is, and where its rows
/// begin when it wraps.
#[derive(Debug)]
struct Shaped {
    /// (byte, x) for every character boundary, the line's end last.
    stops: Vec<(usize, f32)>,
    /// (byte, x) where each row starts; the first is (0, 0).
    rows: Vec<(usize, f32)>,
    width: f32,
}

impl Shaped {
    fn x_of(&self, b: usize) -> f32 {
        match self.stops.binary_search_by_key(&b, |s| s.0) {
            Ok(k) => self.stops[k].1,
            Err(k) => self.stops[k.saturating_sub(1)].1,
        }
    }

    /// The row byte `b` is on (a row's start belongs to it).
    fn row_of(&self, b: usize) -> usize {
        self.rows.partition_point(|r| r.0 <= b).saturating_sub(1)
    }

    /// The bytes row `r` covers: from its start to the next row's start
    /// (the end of the line for the last row, which the caret may take).
    fn row_range(&self, r: usize, len: usize) -> (usize, usize, bool) {
        let start = self.rows[r].0;
        match self.rows.get(r + 1) {
            Some(next) => (start, next.0, false),
            None => (start, len, true),
        }
    }

    /// The character boundary on row `r` nearest `x` (from the row's start).
    fn byte_at(&self, r: usize, x: f32, len: usize) -> usize {
        let (start, end, last) = self.row_range(r, len);
        let want = x + self.rows[r].1;
        let mut best = (start, f32::MAX);
        for &(b, sx) in &self.stops {
            // A wrapped row's end is the next row's start: the caret there is on the next row.
            if b < start || b > end || (b == end && !last) {
                continue;
            }
            let d = (sx - want).abs();
            if d < best.1 {
                best = (b, d);
            }
        }
        best.0
    }
}

/// Where each character falls, by advances measured in `fonts` (tabs to the
/// next stop), and where the line wraps at `wrap_w` (0: it does not).
fn shape_line(fonts: &mut Fonts, face: FaceId, size: f32, tab_w: f32, wrap_w: f32, s: &str) -> Shaped {
    let mut stops = Vec::with_capacity(s.len() + 1);
    let mut pen = 0.0f32;
    let mut start = 0;
    loop {
        let end = s[start..].find('\t').map_or(s.len(), |k| start + k);
        let seg = &s[start..end];
        if !seg.is_empty() {
            let (runs, w) = fonts.layout(face, size, seg);
            let mut xs = runs.iter().flat_map(|r| r.glyphs.iter().map(|g| g.x)).peekable();
            for (b, c) in seg.char_indices() {
                // Control characters have no glyph: they sit where the next one does.
                let x = if c.is_control() { xs.peek().copied().unwrap_or(w) } else { xs.next().unwrap_or(w) };
                stops.push((start + b, pen + x));
            }
            pen += w;
        }
        if end == s.len() {
            break;
        }
        stops.push((end, pen));
        pen = ((pen / tab_w).floor() + 1.0) * tab_w;
        start = end + 1;
    }
    stops.push((s.len(), pen));
    let rows = wrap_rows(s, &stops, wrap_w);
    Shaped { stops, rows, width: pen }
}

/// A line's guessed shape where it has not been laid out: every character
/// `char_w` wide (exact for a monospaced font and narrow characters).
fn guess_line(char_w: f32, tab_w: f32, wrap_w: f32, s: &str) -> Shaped {
    let mut stops = Vec::with_capacity(s.len() + 1);
    let mut pen = 0.0f32;
    for (b, c) in s.char_indices() {
        stops.push((b, pen));
        pen = if c == '\t' { ((pen / tab_w).floor() + 1.0) * tab_w } else { pen + char_w };
    }
    stops.push((s.len(), pen));
    let rows = wrap_rows(s, &stops, wrap_w);
    Shaped { stops, rows, width: pen }
}

/// Where a line's rows start: after the last space that fits, or at the
/// character that does not fit if there is no space. Spaces hang past the
/// edge rather than start a row.
fn wrap_rows(s: &str, stops: &[(usize, f32)], wrap_w: f32) -> Vec<(usize, f32)> {
    let mut rows = vec![(0usize, 0.0f32)];
    if wrap_w <= 0.0 {
        return rows;
    }
    let mut brk: Option<(usize, f32)> = None;
    for k in 0..stops.len().saturating_sub(1) {
        let (b, x) = stops[k];
        let x_end = stops[k + 1].1;
        let c = s[b..].chars().next().unwrap_or(' ');
        let (rs, rx) = *rows.last().expect("rows start with one");
        if c.is_whitespace() {
            brk = Some(stops[k + 1]);
            continue;
        }
        if x_end - rx > wrap_w && b > rs {
            let at = match brk {
                Some(p) if p.0 > rs && p.0 <= b => p,
                _ => (b, x),
            };
            rows.push(at);
            brk = None;
        }
    }
    rows
}

/// What the area measured when it was last drawn.
#[derive(Clone, Copy, Debug, Default)]
struct Geom {
    rect: Rect,
    /// Where x 0 of a row is drawn (before scrolling sideways).
    text_x: f32,
    text_w: f32,
    line_h: f32,
    char_w: f32,
    /// 0: lines do not wrap.
    wrap_w: f32,
    tab_w: f32,
    thumb: Option<Rect>,
}

/// What a key or a click did, for the [`crate::Ui`] to tell the program.
#[derive(Debug, Default)]
pub(crate) struct Effect {
    pub used: bool,
    pub text: bool,
    pub moved: bool,
    pub copy: Option<String>,
    pub paste: bool,
}

/// A document: its text, its history, its caret, its view.
pub struct Doc {
    buffer: Buffer,
    history: History<Pos>,
    caret: Pos,
    anchor: Pos,
    /// Where Up and Down aim, across a row (kept through a run of them).
    goal: Option<f32>,
    /// The view: its top line, how far into that line, and the sideways scroll.
    top: usize,
    top_off: f32,
    xscroll: f32,
    options: AreaOptions,
    revision: u64,
    /// Each line's layout, once it has been in view; None until then.
    shaped: Vec<Option<Rc<Shaped>>>,
    /// What the layouts were made for: size, wrap width, tab width, font.
    shaped_for: (u32, u32, u32, String),
    geom: Geom,
    /// A double or triple click's word or line, kept while dragging.
    unit: (u8, Pos, Pos),
    /// The line endings when last saved (changing them is a change to save).
    saved_ending: Ending,
    /// Each line's words, kept as lines change (the word count).
    words: Vec<u32>,
}

impl Doc {
    fn new(buffer: Buffer, options: AreaOptions) -> Doc {
        let n = buffer.line_count();
        let mut options = options;
        options.tab_width = options.tab_width.clamp(1, 16);
        Doc {
            buffer,
            history: History::default(),
            caret: Pos::default(),
            anchor: Pos::default(),
            goal: None,
            top: 0,
            top_off: 0.0,
            xscroll: 0.0,
            options,
            revision: 0,
            shaped: vec![None; n],
            shaped_for: (0, 0, 0, String::new()),
            geom: Geom::default(),
            unit: (1, Pos::default(), Pos::default()),
            saved_ending: Ending::Lf,
            words: Vec::new(),
        }
        .with_saved_ending()
    }

    fn with_saved_ending(mut self) -> Doc {
        self.saved_ending = self.buffer.ending();
        self.recount();
        self
    }

    fn recount(&mut self) {
        self.words = self.buffer.lines().map(count_words).collect();
    }

    /// How many words the text has (runs of letters and digits, with ' and
    /// - inside a word: "don't", "well-known" are one each).
    pub fn word_count(&self) -> usize {
        self.words.iter().map(|w| *w as usize).sum()
    }

    /// How many words the selection has.
    pub fn selected_words(&self) -> usize {
        let (a, b) = self.selection();
        if a == b {
            return 0;
        }
        count_words(&self.buffer.slice(a, b)) as usize
    }

    // --- What a program reads and does ---

    /// Writes the lines back with these endings (a change to save).
    pub fn set_ending(&mut self, e: Ending) {
        self.buffer.set_ending(e);
        self.revision += 1;
    }

    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    /// The whole text, with the file's line endings.
    pub fn text(&self) -> String {
        self.buffer.to_text()
    }

    /// Puts in a new text: the history is forgotten, the caret goes to the start.
    pub fn set_text(&mut self, text: &str) {
        self.buffer = Buffer::from_text(text);
        self.saved_ending = self.buffer.ending();
        self.history.clear();
        self.caret = Pos::default();
        self.anchor = Pos::default();
        self.top = 0;
        self.top_off = 0.0;
        self.xscroll = 0.0;
        self.reshape_all();
        self.revision += 1;
    }

    /// Goes up by one each time the text changes.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn options(&self) -> &AreaOptions {
        &self.options
    }

    pub fn set_options(&mut self, options: AreaOptions) {
        let mut options = options;
        options.tab_width = options.tab_width.clamp(1, 16);
        if options != self.options {
            self.options = options;
            self.reshape_all();
        }
    }

    /// Changed since it was last saved (or loaded).
    pub fn modified(&self) -> bool {
        self.history.modified() || self.buffer.ending() != self.saved_ending
    }

    /// The text is saved as it is now.
    pub fn mark_saved(&mut self) {
        self.history.mark_saved();
        self.saved_ending = self.buffer.ending();
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn caret(&self) -> Pos {
        self.caret
    }

    /// The selection, start first; empty when start == end.
    pub fn selection(&self) -> (Pos, Pos) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    pub fn selected_text(&self) -> String {
        let (a, b) = self.selection();
        self.buffer.slice(a, b)
    }

    /// Selects from `anchor` to `caret` (the caret is where typing goes) and
    /// brings the caret into view.
    pub fn set_selection(&mut self, anchor: Pos, caret: Pos) {
        self.anchor = self.buffer.clamp(anchor);
        self.caret = self.buffer.clamp(caret);
        self.goal = None;
        self.history.break_group();
        self.reveal();
    }

    pub fn select_all(&mut self) {
        self.anchor = Pos::default();
        self.caret = self.buffer.end();
        self.history.break_group();
    }

    /// Types `text` over the selection, as one step of its own (a paste, a
    /// menu's command). Nothing if the area is read-only.
    pub fn insert(&mut self, text: &str) -> bool {
        if !self.options.editable {
            return false;
        }
        let clean: String = str::replace(&str::replace(text, "\r\n", "\n"), '\r', "\n").chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect();
        let (a, b) = self.selection();
        if clean.is_empty() && a == b {
            return false;
        }
        let end = self.edit(a, b, &clean, Kind::Other);
        self.set_caret(end, false);
        self.history.break_group();
        self.reveal();
        true
    }

    pub fn undo(&mut self) -> bool {
        let Some((caret, anchor)) = self.history.undo(&mut self.buffer) else { return false };
        self.after_history(caret, anchor);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some((caret, anchor)) = self.history.redo(&mut self.buffer) else { return false };
        self.after_history(caret, anchor);
        true
    }

    /// The line and column the caret is on, from 1, the column in
    /// characters with tabs counted to their stops.
    pub fn line_column(&self) -> (usize, usize) {
        (self.caret.line + 1, self.visual_col(self.caret) + 1)
    }

    // --- Editing ---

    fn after_history(&mut self, caret: Pos, anchor: Pos) {
        self.caret = self.buffer.clamp(caret);
        self.anchor = self.buffer.clamp(anchor);
        self.goal = None;
        self.reshape_all();
        self.revision += 1;
        self.reveal();
    }

    fn reshape_all(&mut self) {
        self.shaped = vec![None; self.buffer.line_count()];
        self.recount();
    }

    /// Replaces `from..to` with `with` through the history; the layouts of
    /// the lines it touched are dropped. Returns where the new text ends.
    fn edit(&mut self, from: Pos, to: Pos, with: &str, kind: Kind) -> Pos {
        let (from, to) = (self.buffer.clamp(from.min(to)), self.buffer.clamp(from.max(to)));
        let sel = (self.caret, self.anchor);
        let end = self.history.edit(&mut self.buffer, from, to, with, sel, kind, Instant::now());
        let old = to.line - from.line + 1;
        let new = end.line - from.line + 1;
        self.shaped.splice(from.line..from.line + old, std::iter::repeat_n(None, new));
        let counts: Vec<u32> = (from.line..from.line + new).map(|l| count_words(self.buffer.line(l))).collect();
        self.words.splice(from.line..from.line + old, counts);
        debug_assert_eq!(self.shaped.len(), self.buffer.line_count());
        self.revision += 1;
        end
    }

    fn set_caret(&mut self, p: Pos, extend: bool) {
        self.caret = p;
        if !extend {
            self.anchor = p;
        }
    }

    /// The column of `p` in characters, tabs counted to their stops.
    fn visual_col(&self, p: Pos) -> usize {
        let tw = self.options.tab_width;
        let mut col = 0;
        for c in self.buffer.line(p.line)[..p.col].chars() {
            col = if c == '\t' { (col / tw + 1) * tw } else { col + 1 };
        }
        col
    }

    /// What one press of Tab inserts at `p`.
    fn tab_text(&self, p: Pos) -> String {
        if self.options.spaces {
            let tw = self.options.tab_width;
            " ".repeat(tw - self.visual_col(p) % tw)
        } else {
            "\t".into()
        }
    }

    /// The lines a selection covers for indenting: a selection that ends at
    /// a line's start leaves that line out.
    fn selected_lines(&self) -> (usize, usize) {
        let (a, b) = self.selection();
        let last = if b.col == 0 && b.line > a.line { b.line - 1 } else { b.line };
        (a.line, last)
    }

    /// Indents (or outdents) every selected line by one step, as one undo.
    fn indent_lines(&mut self, out: bool) -> bool {
        let (first, last) = self.selected_lines();
        let unit = if self.options.spaces { " ".repeat(self.options.tab_width) } else { "\t".into() };
        let tw = self.options.tab_width;
        let sel = (self.caret, self.anchor);
        let now = Instant::now();
        let buffer = &mut self.buffer;
        let changed = self.history.group(|h| {
            let mut changed = false;
            for l in first..=last {
                if out {
                    let s = buffer.line(l);
                    let n = if s.starts_with('\t') { 1 } else { s.bytes().take(tw).take_while(|b| *b == b' ').count() };
                    if n > 0 {
                        h.edit(buffer, Pos::new(l, 0), Pos::new(l, n), "", sel, Kind::Other, now);
                        changed = true;
                    }
                } else if !buffer.line(l).is_empty() || first == last {
                    h.edit(buffer, Pos::new(l, 0), Pos::new(l, 0), &unit, sel, Kind::Other, now);
                    changed = true;
                }
            }
            changed
        });
        if changed {
            self.reshape_all();
            self.revision += 1;
            self.anchor = Pos::new(first, 0);
            self.caret = if first == last && self.anchor == self.caret { self.caret } else { Pos::new(last, self.buffer.line(last).len()) };
            self.caret = self.buffer.clamp(self.caret);
        }
        changed
    }

    /// A key, while the area has the keyboard.
    pub(crate) fn key(&mut self, key: u32, typed: Option<&str>, m: Modifiers) -> Effect {
        let mut e = Effect { used: true, ..Effect::default() };
        let command = m.ctrl || m.logo;
        let editable = self.options.editable;
        let (a, b) = self.selection();
        let selected = a != b;
        let letter = char::from_u32(key).map(|c| c.to_ascii_lowercase());
        let vertical = matches!(key, sym::UP | sym::DOWN | sym::PAGE_UP | sym::PAGE_DOWN) && !command;
        if !vertical {
            self.goal = None;
        }
        match key {
            // Ctrl+Page Up/Down are the program's (the tab before, the tab after).
            sym::PAGE_UP | sym::PAGE_DOWN if m.ctrl => e.used = false,
            sym::LEFT | sym::RIGHT | sym::UP | sym::DOWN | sym::HOME | sym::END | sym::PAGE_UP | sym::PAGE_DOWN => {
                // Ctrl+Up/Down scroll by a line, the caret staying.
                if m.ctrl && !m.shift && matches!(key, sym::UP | sym::DOWN) {
                    let lh = self.geom.line_h.max(1.0);
                    self.scroll_by(if key == sym::UP { -lh } else { lh });
                    return e;
                }
                let c = self.caret;
                let to = match key {
                    sym::LEFT if selected && !m.shift => a,
                    sym::RIGHT if selected && !m.shift => b,
                    sym::LEFT if command || m.alt => self.buffer.prev_word(c),
                    sym::RIGHT if command || m.alt => self.buffer.next_word(c),
                    sym::LEFT => self.buffer.prev_char(c),
                    sym::RIGHT => self.buffer.next_char(c),
                    // The Super key's Up and Down go to the ends, as on a Mac.
                    sym::UP if m.logo => Pos::default(),
                    sym::DOWN if m.logo => self.buffer.end(),
                    sym::UP => self.move_rows(c, -1),
                    sym::DOWN => self.move_rows(c, 1),
                    sym::PAGE_UP | sym::PAGE_DOWN => {
                        let rows = self.page_rows();
                        let dy = rows as f32 * self.geom.line_h;
                        if key == sym::PAGE_UP {
                            self.scroll_by(-dy);
                            self.move_rows(c, -(rows as isize))
                        } else {
                            self.scroll_by(dy);
                            self.move_rows(c, rows as isize)
                        }
                    }
                    sym::HOME if command => Pos::default(),
                    sym::END if command => self.buffer.end(),
                    sym::HOME => {
                        // To the first character that is not a space, then to the very start.
                        let ind = self.buffer.indent(c.line).len();
                        Pos::new(c.line, if c.col != ind { ind } else { 0 })
                    }
                    _ => Pos::new(c.line, self.buffer.line(c.line).len()),
                };
                self.set_caret(to, m.shift);
                self.history.break_group();
                self.reveal();
                e.moved = true;
            }
            sym::BACKSPACE | sym::DELETE if editable => {
                let c = self.caret;
                let (from, to, kind) = if selected {
                    (a, b, Kind::Other)
                } else if key == sym::BACKSPACE {
                    let line = self.buffer.line(c.line);
                    let in_indent = c.col > 0 && line[..c.col].bytes().all(|x| x == b' ');
                    let from = if command || m.alt {
                        self.buffer.prev_word(c)
                    } else if self.options.spaces && in_indent {
                        // Back to the tab stop before, in an indent of spaces.
                        let tw = self.options.tab_width;
                        let k = if c.col.is_multiple_of(tw) { tw } else { c.col % tw };
                        Pos::new(c.line, c.col - k)
                    } else {
                        self.buffer.prev_char(c)
                    };
                    (from, c, Kind::Deleting)
                } else {
                    (c, if command || m.alt { self.buffer.next_word(c) } else { self.buffer.next_char(c) }, Kind::Deleting)
                };
                if from != to {
                    self.edit(from, to, "", kind);
                    self.set_caret(from, false);
                    e.text = true;
                }
                self.reveal();
                e.moved = true;
            }
            sym::RETURN | KP_ENTER if editable && !command => {
                // The new line starts with the indent of this one.
                let c = a;
                let ind = self.buffer.indent(c.line);
                let ind = ind[..ind.len().min(c.col)].to_string();
                let end = self.edit(a, b, &format!("\n{ind}"), Kind::Typing);
                self.set_caret(end, false);
                self.reveal();
                e.text = true;
                e.moved = true;
            }
            sym::TAB if editable && !command && !m.alt => {
                if m.shift || a.line != b.line {
                    e.text = self.indent_lines(m.shift);
                } else {
                    let t = self.tab_text(a);
                    let end = self.edit(a, b, &t, Kind::Typing);
                    self.set_caret(end, false);
                    e.text = true;
                }
                self.reveal();
                e.moved = true;
            }
            _ if command && !m.alt => match letter {
                Some('a') => {
                    self.select_all();
                    e.moved = true;
                }
                Some(c @ ('c' | 'x')) => {
                    if selected {
                        e.copy = Some(self.buffer.slice(a, b));
                        if c == 'x' && editable {
                            self.edit(a, b, "", Kind::Other);
                            self.set_caret(a, false);
                            self.reveal();
                            e.text = true;
                            e.moved = true;
                        }
                    }
                }
                Some('v') if editable => e.paste = true,
                Some(c @ ('z' | 'y')) if editable => {
                    let done = if c == 'z' && !m.shift { self.undo() } else { self.redo() };
                    e.text = done;
                    e.moved = done;
                }
                _ => e.used = false,
            },
            _ => match typed.filter(|t| !t.is_empty() && !t.chars().any(char::is_control) && !m.alt && !command) {
                Some(t) if editable => {
                    let end = self.edit(a, b, t, Kind::Typing);
                    self.set_caret(end, false);
                    self.reveal();
                    e.text = true;
                    e.moved = true;
                }
                // Read-only: typing is taken (not passed on as a command) but does nothing.
                Some(_) => {}
                None => e.used = false,
            },
        }
        e
    }

    // --- Geometry ---

    /// Line `i`'s layout: the real one if it has been in view, else a guess.
    fn shape_of(&self, i: usize) -> Rc<Shaped> {
        if let Some(Some(s)) = self.shaped.get(i) {
            return s.clone();
        }
        let g = &self.geom;
        let char_w = if g.char_w > 0.0 { g.char_w } else { 8.0 };
        let tab_w = if g.tab_w > 0.0 { g.tab_w } else { char_w * self.options.tab_width as f32 };
        Rc::new(guess_line(char_w, tab_w, g.wrap_w, self.buffer.line(i)))
    }

    /// How many rows line `i` takes (guessed cheaply if not laid out).
    fn rows(&self, i: usize) -> usize {
        if let Some(Some(s)) = self.shaped.get(i) {
            return s.rows.len();
        }
        if self.geom.wrap_w <= 0.0 {
            return 1;
        }
        let char_w = self.geom.char_w.max(1.0);
        let w = self.buffer.line(i).len() as f32 * char_w;
        ((w / self.geom.wrap_w).ceil() as usize).max(1)
    }

    fn line_height(&self, i: usize) -> f32 {
        self.rows(i) as f32 * self.geom.line_h
    }

    /// Rows in a page: the view's height less one row.
    fn page_rows(&self) -> usize {
        let lh = self.geom.line_h.max(1.0);
        (((self.geom.rect.h - 2.0 * PAD_Y) / lh).floor() as usize).saturating_sub(1).max(1)
    }

    /// Scrolls by `dy` points (down is positive), held by the top line. The
    /// last line can come up to the top, no further.
    fn scroll_by(&mut self, dy: f32) {
        let n = self.buffer.line_count();
        let mut off = self.top_off + dy;
        while off < 0.0 {
            if self.top == 0 {
                off = 0.0;
                break;
            }
            self.top -= 1;
            off += self.line_height(self.top);
        }
        loop {
            let h = self.line_height(self.top);
            if self.top + 1 >= n {
                off = off.min((h - self.geom.line_h).max(0.0));
                break;
            }
            if off < h {
                break;
            }
            off -= h;
            self.top += 1;
        }
        self.top_off = off.max(0.0);
    }

    /// Where line `i`'s top is, from the view's top (negative above it).
    fn line_y(&self, i: usize) -> f32 {
        if i >= self.top {
            (self.top..i).map(|k| self.line_height(k)).sum::<f32>() - self.top_off
        } else {
            -(i..self.top).map(|k| self.line_height(k)).sum::<f32>() - self.top_off
        }
    }

    /// The row `p` is on in its line, and its x from that row's start.
    fn visual(&self, p: Pos) -> (usize, f32) {
        let s = self.shape_of(p.line);
        let r = s.row_of(p.col);
        (r, s.x_of(p.col) - s.rows[r].1)
    }

    /// `p` moved `n` rows down (up if negative), aiming at the goal column.
    fn move_rows(&mut self, p: Pos, n: isize) -> Pos {
        let (mut row, x) = self.visual(p);
        let goal = *self.goal.get_or_insert(x);
        let mut line = p.line;
        let last = self.buffer.line_count() - 1;
        if n < 0 {
            for _ in 0..n.unsigned_abs() {
                if row > 0 {
                    row -= 1;
                } else if line > 0 {
                    line -= 1;
                    row = self.shape_of(line).rows.len() - 1;
                } else {
                    // Up from the first row: the very start.
                    return Pos::default();
                }
            }
        } else {
            for _ in 0..n {
                if row + 1 < self.shape_of(line).rows.len() {
                    row += 1;
                } else if line < last {
                    line += 1;
                    row = 0;
                } else {
                    return self.buffer.end();
                }
            }
        }
        let s = self.shape_of(line);
        Pos::new(line, s.byte_at(row, goal, self.buffer.line(line).len()))
    }

    /// Scrolls so the caret is in view.
    fn reveal(&mut self) {
        let g = self.geom;
        if g.line_h <= 0.0 {
            return;
        }
        let view_h = g.rect.h - 2.0 * PAD_Y;
        let c = self.caret;
        let (row, x) = self.visual(c);
        let row_y = row as f32 * g.line_h;
        // Far away: jump to it rather than add up every line between.
        if c.line + 2000 < self.top || c.line > self.top + 2000 {
            let down = c.line > self.top;
            self.top = c.line;
            self.top_off = row_y;
            if down {
                self.scroll_by(-(view_h - g.line_h).max(0.0));
            }
        } else {
            let y = self.line_y(c.line) + row_y;
            if y < 0.0 {
                self.scroll_by(y);
            } else if y + g.line_h > view_h {
                self.scroll_by(y + g.line_h - view_h);
            }
        }
        if g.wrap_w <= 0.0 {
            let room = (g.text_w - g.char_w).max(g.char_w);
            if x - self.xscroll > room {
                self.xscroll = x - room;
            }
            if x < self.xscroll {
                self.xscroll = (x - g.text_w / 3.0).max(0.0);
            }
        }
    }

    /// The place under a point in the window.
    fn pos_at(&self, x: f32, y: f32) -> Pos {
        let g = &self.geom;
        let n = self.buffer.line_count();
        let mut local = y - (g.rect.y + PAD_Y) + self.top_off;
        let mut i = self.top.min(n - 1);
        while local < 0.0 {
            if i == 0 {
                return Pos::default();
            }
            i -= 1;
            local += self.line_height(i);
        }
        while local >= self.line_height(i) {
            if i + 1 >= n {
                return self.buffer.end();
            }
            local -= self.line_height(i);
            i += 1;
        }
        let s = self.shape_of(i);
        let row = ((local / g.line_h.max(1.0)) as usize).min(s.rows.len() - 1);
        Pos::new(i, s.byte_at(row, x - g.text_x + self.xscroll, self.buffer.line(i).len()))
    }

    // --- The pointer ---

    /// Whether a point is on the scroll bar's thumb or track; with the grab
    /// (where on the thumb it was taken).
    pub(crate) fn bar_hit(&self, x: f32, y: f32) -> Option<f32> {
        let t = self.geom.thumb?;
        let r = self.geom.rect;
        if x < t.x - 3.0 || x > r.x + r.w || y < r.y || y > r.y + r.h {
            return None;
        }
        Some(if y >= t.y && y < t.y + t.h { y - t.y } else { t.h / 2.0 })
    }

    /// The scroll bar dragged: the thumb's top to `y - grab`.
    pub(crate) fn bar_to(&mut self, y: f32, grab: f32) {
        let Some(t) = self.geom.thumb else { return };
        let r = self.geom.rect;
        let (_, total, view) = self.extent();
        let f = ((y - grab - r.y) / (r.h - t.h).max(1.0)).clamp(0.0, 1.0);
        let want = f * (total - view).max(0.0);
        self.top = 0;
        self.top_off = 0.0;
        self.scroll_by(want);
    }

    /// The text above the view, all of it (with room to bring the last line
    /// to the top), and the view's height, in points.
    fn extent(&self) -> (f32, f32, f32) {
        let n = self.buffer.line_count();
        let above: f32 = (0..self.top.min(n)).map(|k| self.line_height(k)).sum::<f32>() + self.top_off;
        let below: f32 = (self.top.min(n)..n).map(|k| self.line_height(k)).sum();
        let view = self.geom.rect.h - 2.0 * PAD_Y;
        (above, above + below - self.top_off + (view - self.geom.line_h).max(0.0), view)
    }

    /// A press: one click puts the caret there (Shift: the selection runs
    /// there), two take a word, three the line.
    pub(crate) fn press(&mut self, x: f32, y: f32, count: u8, shift: bool) {
        let p = self.pos_at(x, y);
        self.goal = None;
        self.history.break_group();
        match count {
            2 => {
                let (a, b) = self.buffer.word_at(p);
                self.anchor = a;
                self.caret = b;
                self.unit = (2, a, b);
            }
            3 => {
                let a = Pos::new(p.line, 0);
                let b = if p.line + 1 < self.buffer.line_count() { Pos::new(p.line + 1, 0) } else { self.buffer.end() };
                self.anchor = a;
                self.caret = b;
                self.unit = (3, a, b);
            }
            _ => {
                self.set_caret(p, shift);
                self.unit = (1, p, p);
            }
        }
    }

    /// The pointer dragged while pressed: the selection follows it, by words
    /// or lines after a double or triple click. Past the top or bottom, the
    /// view scrolls.
    pub(crate) fn drag_to(&mut self, x: f32, y: f32) {
        let r = self.geom.rect;
        let lh = self.geom.line_h.max(1.0);
        if y < r.y {
            self.scroll_by(-lh);
        } else if y > r.y + r.h {
            self.scroll_by(lh);
        }
        let p = self.pos_at(x, y.clamp(r.y, r.y + r.h - 1.0));
        let (unit, a, b) = self.unit;
        match unit {
            2 | 3 => {
                let (pa, pb) = if unit == 2 {
                    self.buffer.word_at(p)
                } else {
                    (Pos::new(p.line, 0), if p.line + 1 < self.buffer.line_count() { Pos::new(p.line + 1, 0) } else { self.buffer.end() })
                };
                if p < a {
                    self.anchor = b;
                    self.caret = pa;
                } else {
                    self.anchor = a;
                    self.caret = pb.max(b);
                }
            }
            _ => self.caret = p,
        }
        if self.geom.wrap_w <= 0.0 {
            self.reveal();
        }
    }

    /// The wheel or a touchpad, in points.
    pub(crate) fn wheel(&mut self, dx: f32, dy: f32) {
        if self.geom.wrap_w <= 0.0 && dx != 0.0 {
            self.xscroll = (self.xscroll + dx).max(0.0);
        }
        if dy != 0.0 {
            self.scroll_by(dy);
        }
    }

    /// What a screen reader is told: the whole text if not too long, the caret's place.
    pub(crate) fn meaning_value(&self) -> String {
        let (l, c) = self.line_column();
        if self.buffer.bytes() <= TELL_WHOLE { format!("{} (line {l}, column {c})", self.buffer.to_text()) } else { format!("line {l} of {}, column {c}", self.buffer.line_count()) }
    }

    // --- Drawing ---

    pub(crate) fn draw(&mut self, scene: &mut Scene, fonts: &mut Fonts, r: Rect, theme: &Theme, focused: bool, caret_on: bool) {
        let size = self.options.size.unwrap_or(theme.text_size).clamp(4.0, 200.0);
        let Some(face) = fonts.face(&self.options.font, 400, false).or_else(|| fonts.face("monospace", 400, false)).or_else(|| fonts.face(&theme.font, 400, false)) else { return };
        let m = fonts.metrics(face, size);
        let line_h = (m.ascent + m.descent + m.line_gap).max(size * 1.3).ceil();
        let adv = |fonts: &mut Fonts, c: char| {
            let (f, g) = fonts.glyph(face, c);
            fonts.advance(f, g, size)
        };
        let char_w = adv(fonts, ' ').max(1.0);
        let digit_w = adv(fonts, '0').max(1.0);
        let n = self.buffer.line_count();
        let digits = n.to_string().len().max(3);
        let gutter = if self.options.line_numbers { (digits as f32 * digit_w + 2.0 * GUTTER_PAD).ceil() } else { 0.0 };
        let text_x = r.x + gutter + PAD_X;
        let text_w = (r.w - gutter - PAD_X - BAR_W - 6.0).max(char_w * 4.0);
        let wrap_w = if self.options.wrap { text_w } else { 0.0 };
        let tab_w = char_w * self.options.tab_width as f32;
        let key = (size.to_bits(), wrap_w.to_bits(), tab_w.to_bits(), self.options.font.clone());
        if key != self.shaped_for {
            self.shaped_for = key;
            self.reshape_all();
        }
        let resized = self.geom.rect.h != r.h || self.geom.rect.w != r.w;
        self.geom = Geom { rect: r, text_x, text_w, line_h, char_w, wrap_w, tab_w, thumb: self.geom.thumb };
        self.top = self.top.min(n - 1);
        if resized {
            // The top line's rows may have changed: stay on the same row.
            self.scroll_by(0.0);
        }

        scene.fill(r, theme.page);
        // The line numbers on a gray of their own (Dave, 2026-11-15: "to set
        // it apart from the rest").
        if gutter > 0.0 {
            scene.fill(Rect::new(r.x, r.y, gutter, r.h), theme.card);
        }
        scene.push_clip(r, 0.0);
        let (sa, sb) = self.selection();
        let selected = sa != sb;
        let sel_c = if focused { alpha(theme.accent, 90) } else { alpha(theme.text, 40) };
        let shade = alpha(theme.text, if theme.dark { 16 } else { 10 });
        let text_c = theme.text;
        let num_c = theme.dim;
        let text_clip = Rect::new(r.x + gutter, r.y, (r.w - gutter).max(0.0), r.h);
        let mut y = r.y + PAD_Y - self.top_off;
        let mut i = self.top;
        let mut widest = 0.0f32;
        while i < n && y < r.y + r.h {
            if self.shaped[i].is_none() {
                self.shaped[i] = Some(Rc::new(shape_line(fonts, face, size, tab_w, wrap_w, self.buffer.line(i))));
            }
            let s = self.shaped[i].clone().expect("laid out above");
            let h = s.rows.len() as f32 * line_h;
            widest = widest.max(s.width);
            if y + h > r.y {
                let line = self.buffer.line(i);
                if self.options.current_line && i == self.caret.line && !selected {
                    scene.fill(Rect::new(r.x, y, r.w, h), shade);
                }
                scene.push_clip(text_clip, 0.0);
                for (k, &(rs, rx)) in s.rows.iter().enumerate() {
                    let ry = y + k as f32 * line_h;
                    if ry > r.y + r.h || ry + line_h < r.y {
                        continue;
                    }
                    let (_, re, last) = s.row_range(k, line.len());
                    let x0 = text_x - self.xscroll - rx;
                    // The selection on this row.
                    if selected && sa.line <= i && sb.line >= i {
                        let from = if sa.line == i { sa.col.max(rs) } else { rs };
                        let to = if sb.line == i { sb.col.min(re) } else { re };
                        if from <= to && !(sb.line == i && sb.col < rs) && !(sa.line == i && sa.col > re) {
                            let mut w = s.x_of(to) - s.x_of(from);
                            // The line's end is chosen too: a little more.
                            if last && sb.line > i {
                                w += char_w * 0.6;
                            }
                            if w > 0.0 {
                                scene.fill(Rect::new(x0 + s.x_of(from), ry, w, line_h), sel_c);
                            }
                        }
                    }
                    // The text, a piece between tabs at a time.
                    let base = ry + ((line_h - (m.ascent + m.descent)) / 2.0 + m.ascent).round();
                    let mut p = rs;
                    while p < re {
                        let q = line[p..re].find('\t').map_or(re, |k| p + k);
                        if q > p {
                            text(scene, fonts, face, size, text_c, Point::new(x0 + s.x_of(p), base), &line[p..q]);
                        }
                        p = q + 1;
                    }
                }
                scene.pop_clip();
                if self.options.line_numbers {
                    let label = (i + 1).to_string();
                    let w = label.len() as f32 * digit_w;
                    let base = y + ((line_h - (m.ascent + m.descent)) / 2.0 + m.ascent).round();
                    let c = if i == self.caret.line { text_c } else { num_c };
                    text(scene, fonts, face, size, c, Point::new(r.x + gutter - GUTTER_PAD - w, base), &label);
                }
            }
            y += h;
            i += 1;
        }
        // The lines just past each end too, so Up and Down land exactly.
        for k in [self.top.checked_sub(1), (i < n).then_some(i)].into_iter().flatten() {
            if self.shaped[k].is_none() {
                self.shaped[k] = Some(Rc::new(shape_line(fonts, face, size, tab_w, wrap_w, self.buffer.line(k))));
            }
        }
        if wrap_w <= 0.0 {
            let room = (widest - text_w + char_w).max(0.0);
            self.xscroll = self.xscroll.clamp(0.0, room);
        } else {
            self.xscroll = 0.0;
        }
        // The caret.
        if focused && caret_on && !selected {
            let (row, x) = self.visual(self.caret);
            let cy = r.y + PAD_Y + self.line_y(self.caret.line) + row as f32 * line_h;
            let cx = text_x - self.xscroll + x;
            if cy + line_h > r.y && cy < r.y + r.h && cx >= text_clip.x {
                scene.fill(Rect::new(cx.round() - 0.5, cy + 1.0, 1.5, line_h - 2.0), text_c);
            }
        }
        // The edge of the line numbers.
        if gutter > 0.0 {
            scene.fill(Rect::new(r.x + gutter - 1.0, r.y, 1.0, r.h), alpha(theme.line, 160));
        }
        scene.pop_clip();
        // The scroll bar, when there is more than fits.
        let (above, total, view) = self.extent();
        self.geom.thumb = (total > view + 1.0 && view > 0.0).then(|| {
            let th = (r.h * view / total).max(MIN_THUMB).min(r.h);
            let f = (above / (total - view).max(1.0)).clamp(0.0, 1.0);
            Rect::new(r.x + r.w - BAR_W - 2.0, r.y + (r.h - th) * f, BAR_W, th)
        });
        if let Some(t) = self.geom.thumb {
            scene.fill_rounded(t, BAR_W / 2.0, alpha(theme.text, 80));
        }
    }
}

/// Words in `s`: runs of letters and digits; an apostrophe or hyphen between
/// two letters keeps the word going.
fn count_words(s: &str) -> u32 {
    let mut n = 0;
    let mut in_word = false;
    let mut prev_join = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_alphanumeric() {
            if !in_word {
                n += 1;
                in_word = true;
            }
            prev_join = false;
        } else if in_word && !prev_join && matches!(c, '\'' | '’' | '-') && chars.peek().is_some_and(|d| d.is_alphanumeric()) {
            prev_join = true;
        } else {
            in_word = false;
        }
    }
    n
}

fn alpha(c: Color, a: u8) -> Color {
    Color { a, ..c }
}

/// A text area showing and editing `doc` (editor milestone 1). It fills
/// the room it is given.
pub fn text_area<M>(doc: &TextDoc) -> Element<M> {
    Element::new(EKind::Area { doc: doc.clone(), on_change: None, autofocus: false }, Length::FILL, Length::FILL)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Doc {
        let mut d = Doc::new(Buffer::from_text(text), AreaOptions::default());
        // Laid out as if drawn: 10-point rows, characters 8 wide, 80 wide.
        d.geom = Geom { rect: Rect::new(0.0, 0.0, 200.0, 2.0 * PAD_Y + 50.0), text_x: 0.0, text_w: 80.0, line_h: 10.0, char_w: 8.0, wrap_w: 80.0, tab_w: 32.0, thumb: None };
        d
    }

    fn key(d: &mut Doc, k: u32) -> Effect {
        d.key(k, None, Modifiers::default())
    }

    fn typed(d: &mut Doc, s: &str) {
        for c in s.chars() {
            let k = if c == '\n' { sym::RETURN } else { c as u32 };
            d.key(k, Some(&c.to_string()), Modifiers::default());
        }
    }

    #[test]
    fn wrapping_breaks_after_spaces_and_inside_long_words() {
        let s = "aaaa bbbb cccc";
        let sh = guess_line(8.0, 32.0, 80.0, s);
        // 10 characters fit: "aaaa bbbb " then "cccc".
        assert_eq!(sh.rows.iter().map(|r| r.0).collect::<Vec<_>>(), vec![0, 10]);
        let sh = guess_line(8.0, 32.0, 40.0, "abcdefghijkl");
        assert_eq!(sh.rows.iter().map(|r| r.0).collect::<Vec<_>>(), vec![0, 5, 10]);
        assert_eq!(sh.row_of(5), 1, "a row's start is on that row");
        assert_eq!(sh.byte_at(1, 0.0, 12), 5);
        assert_eq!(sh.byte_at(2, 100.0, 12), 12, "the last row takes the line's end");
        assert_eq!(sh.byte_at(0, 100.0, 12), 4, "a wrapped row's end is before the next row");
        let t = guess_line(8.0, 32.0, 0.0, "a\tb");
        assert_eq!(t.x_of(2), 32.0, "a tab goes to the next stop");
    }

    #[test]
    fn typing_enter_keeps_the_indent_and_tab_inserts_to_the_stop() {
        let mut d = doc("    x");
        d.set_selection(Pos::new(0, 5), Pos::new(0, 5));
        typed(&mut d, "\ny");
        assert_eq!(d.text(), "    x\n    y");
        d.set_selection(Pos::new(1, 4), Pos::new(1, 4));
        typed(&mut d, "ab");
        key(&mut d, sym::TAB);
        assert_eq!(d.buffer().line(1), "    ab  y", "to the next stop of 4");
        // Backspace in an indent of spaces goes back a stop.
        d.set_selection(Pos::new(1, 4), Pos::new(1, 4));
        key(&mut d, sym::BACKSPACE);
        assert_eq!(d.buffer().line(1), "ab  y");
        // Home goes to the first character, then to the start.
        let mut d = doc("   word");
        key(&mut d, sym::END);
        key(&mut d, sym::HOME);
        assert_eq!(d.caret(), Pos::new(0, 3));
        key(&mut d, sym::HOME);
        assert_eq!(d.caret(), Pos::new(0, 0));
    }

    #[test]
    fn indenting_selected_lines_is_one_undo() {
        let mut d = doc("a\nb\n\nc");
        d.set_selection(Pos::new(0, 0), Pos::new(3, 1));
        key(&mut d, sym::TAB);
        assert_eq!(d.text(), "    a\n    b\n\n    c", "empty lines stay empty");
        d.key(sym::TAB, None, Modifiers { shift: true, ..Modifiers::default() });
        assert_eq!(d.text(), "a\nb\n\nc");
        d.undo();
        assert_eq!(d.text(), "    a\n    b\n\n    c");
        d.undo();
        assert_eq!(d.text(), "a\nb\n\nc");
    }

    #[test]
    fn up_and_down_keep_their_column_and_follow_wrapped_rows() {
        let mut d = doc("abcdefgh\nab\nabcdefghijklmnop");
        d.set_selection(Pos::new(0, 6), Pos::new(0, 6));
        key(&mut d, sym::DOWN);
        assert_eq!(d.caret(), Pos::new(1, 2), "a short line: its end");
        key(&mut d, sym::DOWN);
        assert_eq!(d.caret(), Pos::new(2, 6), "back to the column aimed at");
        // Line 2 wraps after 10 characters: down goes to its second row.
        key(&mut d, sym::DOWN);
        assert_eq!(d.caret(), Pos::new(2, 16), "the second row is 6 long");
        key(&mut d, sym::DOWN);
        assert_eq!(d.caret(), Pos::new(2, 16), "the end");
        key(&mut d, sym::UP);
        assert_eq!(d.caret(), Pos::new(2, 6));
        d.set_selection(Pos::new(0, 3), Pos::new(0, 3));
        key(&mut d, sym::UP);
        assert_eq!(d.caret(), Pos::new(0, 0), "up from the first row: the start");
    }

    #[test]
    fn the_view_follows_the_caret_and_clicks_land_on_characters() {
        let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let mut d = doc(&text);
        // Five rows show; the caret 20 lines down brings the view along.
        for _ in 0..20 {
            key(&mut d, sym::DOWN);
        }
        assert_eq!(d.caret().line, 20);
        assert_eq!(d.top, 16, "the caret on the last row in view");
        assert_eq!(d.pos_at(17.0, PAD_Y + 5.0), Pos::new(16, 2), "row 0 in view, between the 2nd and 3rd characters");
        // Ctrl+End: far away, jumped to.
        d.key(sym::END, None, Modifiers { ctrl: true, ..Modifiers::default() });
        assert_eq!(d.caret(), Pos::new(100, 0));
        assert_eq!(d.top, 96);
        d.key(sym::HOME, None, Modifiers { ctrl: true, ..Modifiers::default() });
        assert_eq!((d.top, d.top_off), (0, 0.0));
        // Scrolling stops with the last line at the top.
        d.wheel(0.0, 1e6);
        assert_eq!(d.top, 100);
        d.wheel(0.0, -1e6);
        assert_eq!(d.top, 0);
    }

    #[test]
    fn clicks_choose_words_and_lines_and_dragging_extends_them() {
        let mut d = doc("one two three\nfour");
        d.geom.wrap_w = 0.0;
        d.geom.text_w = 400.0;
        d.press(8.0 * 5.0 + 1.0, PAD_Y + 2.0, 2, false);
        assert_eq!(d.selected_text(), "two");
        d.drag_to(8.0 * 9.0, PAD_Y + 2.0);
        assert_eq!(d.selected_text(), "two three");
        d.press(1.0, PAD_Y + 12.0, 3, false);
        assert_eq!(d.selected_text(), "four");
        d.press(8.0 * 2.0, PAD_Y + 2.0, 1, false);
        d.press(8.0 * 2.0, PAD_Y + 12.0, 1, true);
        assert_eq!(d.selected_text(), "e two three\nfo");
    }

    #[test]
    fn words_are_counted_and_kept_as_lines_change() {
        assert_eq!(count_words("don't stop, well-known 42 -- x"), 5);
        assert_eq!(count_words("  "), 0);
        let mut d = doc("one two\nthree");
        assert_eq!(d.word_count(), 3);
        d.set_selection(Pos::new(1, 5), Pos::new(1, 5));
        typed(&mut d, " four\nfive six");
        assert_eq!(d.word_count(), 6);
        d.undo();
        assert_eq!(d.word_count(), 3);
        d.set_selection(Pos::new(0, 4), Pos::new(1, 2));
        assert_eq!(d.selected_words(), 2, "two, and th");
    }

    #[test]
    fn copy_cut_paste_undo_and_read_only() {
        let mut d = doc("hello world");
        d.set_selection(Pos::new(0, 0), Pos::new(0, 5));
        let ctrl = Modifiers { ctrl: true, ..Modifiers::default() };
        assert_eq!(d.key('c' as u32, None, ctrl).copy.as_deref(), Some("hello"));
        let e = d.key('x' as u32, None, ctrl);
        assert!(e.text);
        assert_eq!(d.text(), " world");
        assert!(d.key('v' as u32, None, ctrl).paste);
        d.insert("bye\r\n\u{7}");
        assert_eq!(d.text(), "bye\n world");
        d.key('z' as u32, None, ctrl);
        assert_eq!(d.text(), " world");
        d.key('z' as u32, None, ctrl);
        assert_eq!(d.text(), "hello world");
        assert!(!d.modified());
        // Keys the area does not use go on to the program.
        assert!(!d.key('s' as u32, None, ctrl).used);
        let mut o = d.options().clone();
        o.editable = false;
        d.set_options(o);
        typed(&mut d, "zz");
        key(&mut d, sym::BACKSPACE);
        assert_eq!(d.text(), "hello world");
        assert!(!d.key('v' as u32, None, ctrl).paste);
    }
}
