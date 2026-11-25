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
    collections::HashSet,
    rc::Rc,
    time::Instant,
};

use vergence_render::text;
use vergence_scene::{Color, Point, Rect, Scene};
use vergence_syntax as syntax;
use vergence_text::{FaceId, Fonts};
use vergence_ui::{Modifiers, sym};

use crate::edit::{Buffer, Editable, Ending, History, Kind, Pos};
use crate::element::{Element, Kind as EKind, Length};
use crate::theme::Theme;

/// The keypad's Enter.
const KP_ENTER: u32 = 0xff8d;
const F5: u32 = 0xffc2;
/// The most words the completion list shows.
const LIST_ROWS: usize = 12;
/// How many lines each side of the caret are searched for words.
const NEAR_LINES: usize = 3000;

/// More words to complete with, from outside the document (the other open
/// documents, a dictionary): those starting with the prefix (without case),
/// best first, at most the limit.
pub type Words = Rc<dyn Fn(&str, usize) -> Vec<String>>;

/// What choosing a line of the popup does.
#[derive(Clone, Debug, PartialEq)]
enum Action {
    /// A completion: the word in place of the one being typed.
    Word,
    /// A spelling suggestion: this text in place of `from..to`.
    Replace(Pos, Pos, String),
    /// The word goes in the user's dictionary.
    AddWord(String),
    /// The word is let be in this document.
    Ignore(String),
    Cut,
    Copy,
    Paste,
    Undo,
    Redo,
    SelectAll,
    /// One of the program's own lines ([`AreaOptions::commands`]).
    Command(u32),
    /// A dividing line in the menu (nothing to choose).
    Line,
}

/// The popup under the caret or the pointer: completions, spelling
/// suggestions, or the clipboard's commands. `words` are its lines' labels.
struct List {
    words: Vec<String>,
    actions: Vec<Action>,
    /// The right-click menu (drawn as the desktop's menus are: the keys at
    /// the right, lines that cannot be used now faint): which can be, and
    /// the keys that do the same. Empty for the completion popup.
    enabled: Vec<bool>,
    keys: Vec<&'static str>,
    /// Where it opens (the right click's point); None: under the caret.
    at: Option<(f32, f32)>,
    /// The server draws it (menu_for_server): kept, not drawn, until its
    /// chosen line comes back or anything else is done here.
    server: bool,
    chosen: usize,
    /// Where the word being completed starts.
    start: Pos,
    /// The first word shown (the list scrolls to keep the chosen one in view).
    first: usize,
}
/// Room around the text.
const PAD_X: f32 = 6.0;
const PAD_Y: f32 = 4.0;
/// Room each side of the line numbers.
const GUTTER_PAD: f32 = 8.0;
const BAR_W: f32 = 6.0;
const MIN_THUMB: f32 = 24.0;
/// A text this long or shorter is told whole to a screen reader and to an
/// agent, which reads it in pages (bus v37; 64 KB before: an editor's longer
/// document gave an agent nothing to read). The tree is built again at each
/// change: copying this much costs about a millisecond.
const TELL_WHOLE: usize = 4 << 20;

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
    /// The likely end of a word is shown as it is typed, and Tab takes it
    /// (step 6, the TextEdit way). The list (Alt+Escape, F5, Ctrl+Space)
    /// works either way.
    pub complete: bool,
    /// Words spelled wrong are underlined (step 7); a right click on one
    /// gives suggestions.
    pub spell: bool,
    /// The program's own lines at the end of the right-click menu (the
    /// Text Editor's New Tab, New Window, Find…, so its menu is as full as
    /// the terminal's; see docs/apps-roadmap.md): the label, its id, its keys. A chosen one is read
    /// with [`Doc::take_command`] when the area's change message comes.
    pub commands: Vec<(String, u32, &'static str)>,
    /// Line height as a multiple of the text size (at least the font's own
    /// spacing); None: [`CODE_SPACING`].
    pub line_spacing: Option<f32>,
    /// The text's language, colored by it (Edit syntax milestone 1); None:
    /// plain text, one color.
    pub syntax: Option<syntax::LangId>,
}

/// Line spacing for code and for prose (text milestone 1, step 6): code
/// editors set about 1.3 (VS Code 1.35, Sublime 1.3); running text reads
/// best near 1.5, which WCAG 2 (1.4.12) asks to be possible.
pub const CODE_SPACING: f32 = 1.3;
pub const PROSE_SPACING: f32 = 1.5;

impl Default for AreaOptions {
    fn default() -> Self {
        AreaOptions {
            font: "monospace".into(),
            size: None,
            line_numbers: true,
            wrap: true,
            tab_width: 4,
            spaces: true,
            current_line: true,
            editable: true,
            complete: true,
            spell: true,
            commands: Vec::new(),
            line_spacing: None,
            syntax: None,
        }
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

    /// A handle that does not keep the document alive (for one document's
    /// completions to look into another's).
    pub fn downgrade(&self) -> WeakDoc {
        WeakDoc(Rc::downgrade(&self.0))
    }
}

/// A [`TextDoc`] that may have gone.
#[derive(Clone)]
pub struct WeakDoc(std::rc::Weak<RefCell<Doc>>);

impl WeakDoc {
    pub fn upgrade(&self) -> Option<TextDoc> {
        self.0.upgrade().map(TextDoc)
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

/// The faces a text area sets text in: regular, and bold and italic for
/// the pieces a language styles (Markdown's headings and strong, its
/// emphasis). A family without them gives its nearest, the regular.
#[derive(Clone, Copy)]
struct Faces {
    regular: FaceId,
    bold: FaceId,
    italic: FaceId,
}

impl Faces {
    fn of(&self, kind: Option<syntax::Kind>) -> FaceId {
        match kind {
            Some(syntax::Kind::Heading | syntax::Kind::Strong) => self.bold,
            Some(syntax::Kind::Emphasis) => self.italic,
            _ => self.regular,
        }
    }
}

/// The pieces of a line in a face other than the regular one: where the
/// line's layout is cut, so a bold word is measured in bold.
fn styled(spans: &[syntax::Span]) -> impl Iterator<Item = (usize, usize, syntax::Kind)> + '_ {
    spans
        .iter()
        .filter(|s| matches!(s.kind, syntax::Kind::Heading | syntax::Kind::Strong | syntax::Kind::Emphasis))
        .map(|s| (s.start, s.end, s.kind))
}

/// From byte `a` to `end` of a line: how far one face goes, and which face.
fn face_run(spans: &[syntax::Span], faces: &Faces, a: usize, end: usize) -> (usize, FaceId) {
    for (s, e, k) in styled(spans) {
        if s <= a && a < e {
            return (e.min(end), faces.of(Some(k)));
        }
        if s > a {
            return (s.min(end), faces.regular);
        }
    }
    (end, faces.regular)
}

/// Where each character falls, by advances measured in `fonts` (tabs to the
/// next stop), each piece in its face (`spans`: the line's syntax), and
/// where the line wraps at `wrap_w` (0: it does not).
fn shape_line(
    fonts: &mut Fonts,
    faces: &Faces,
    size: f32,
    tab_w: f32,
    wrap_w: f32,
    s: &str,
    spans: &[syntax::Span],
) -> Shaped {
    let mut stops = Vec::with_capacity(s.len() + 1);
    let mut pen = 0.0f32;
    let mut start = 0;
    loop {
        let end = s[start..].find('\t').map_or(s.len(), |k| start + k);
        let mut a = start;
        while a < end {
            let (b, face) = face_run(spans, faces, a, end);
            let seg = &s[a..b];
            let (runs, w) = fonts.layout(face, size, seg);
            let mut xs = runs.iter().flat_map(|r| r.glyphs.iter().map(|g| g.x)).peekable();
            for (k, c) in seg.char_indices() {
                // Control characters have no glyph: they sit where the next one does.
                let x = if c.is_control() { xs.peek().copied().unwrap_or(w) } else { xs.next().unwrap_or(w) };
                stops.push((a + k, pen + x));
            }
            pen += w;
            a = b;
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
    /// The view: the area less the strip under the window's bar, where the
    /// text starts and the caret is kept (the strip shows what scrolled
    /// under the bar, frosted, as the terminal's does).
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
    /// The completion list where it was drawn, and its rows' height.
    list: Option<(Rect, f32)>,
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

/// One line's syntax: the state it was worked out from, its colored
/// pieces, the state it ended in.
#[derive(Debug)]
struct Hl {
    start: syntax::State,
    spans: Box<[syntax::Span]>,
    end: syntax::State,
}

/// A document: its text, its history, its caret, its view.
pub struct Doc {
    /// The program's own menu line chosen, until it asks (take_command).
    asked: Option<u32>,
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
    /// Places to show lit (Find's matches), in order; let go at any edit.
    marks: Vec<(Pos, Pos)>,
    /// The likely end of the word just typed, shown after the caret.
    ghost: Option<String>,
    /// Where the caret was drawn last frame (window points), for the
    /// window's meaning (the magnifier follows it); None when not shown.
    pub(crate) caret_drawn: Option<Rect>,
    /// Space may take the ghost: what was typed is no word of its own
    /// (Space takes it, as on macOS, but a real word such as "in" stays as
    /// typed; see docs/apps-roadmap.md).
    ghost_space: bool,
    list: Option<List>,
    /// Words from outside the document ([`Doc::set_words`]).
    words_from: Option<Words>,
    /// Each line's words spelled wrong, once worked out; None until then.
    spelled: Vec<Option<Rc<[(usize, usize)]>>>,
    /// Words let be in this document ("Ignore it here").
    ignored: HashSet<String>,
    /// A word is being typed at the caret: not marked until it is left.
    typing_word: bool,
    /// Each line's syntax once worked out (None until then), and how many
    /// lines from the top agree with the lines above them: an edit lowers
    /// it to the edited line, and lines after are checked again as they
    /// are wanted, keeping their pieces when the state they start from is
    /// the same as before (usually at once).
    hl: Vec<Option<Rc<Hl>>>,
    hl_valid: usize,
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
            marks: Vec::new(),
            ghost: None,
            caret_drawn: None,
            ghost_space: false,
            list: None,
            words_from: None,
            spelled: Vec::new(),
            ignored: HashSet::new(),
            typing_word: false,
            asked: None,
            hl: Vec::new(),
            hl_valid: 0,
        }
        .with_saved_ending()
    }

    fn with_saved_ending(mut self) -> Doc {
        self.saved_ending = self.buffer.ending();
        self.spelled = vec![None; self.buffer.line_count()];
        self.hl = vec![None; self.buffer.line_count()];
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

    /// Where more words to complete with come from (other documents, a dictionary).
    pub fn set_words(&mut self, words: Option<Words>) {
        self.words_from = words;
    }

    /// The word before the caret, if the caret is at its end: where it starts, and it.
    fn word_before(&self) -> Option<(Pos, String)> {
        let c = self.caret;
        let line = self.buffer.line(c.line);
        let w = |ch: char| ch.is_alphanumeric() || ch == '_' || ch == '\'';
        if line[c.col..].chars().next().is_some_and(|ch| ch.is_alphanumeric() || ch == '_') {
            return None;
        }
        let mut a = c.col;
        while let Some(ch) = line[..a].chars().next_back().filter(|ch| w(*ch)) {
            a -= ch.len_utf8();
        }
        let word = line[a..c.col].trim_start_matches('\'').to_string();
        (!word.is_empty()).then(|| (Pos::new(c.line, c.col - word.len()), word))
    }

    /// Words that complete `prefix`, best first: those of the document near
    /// the caret (nearer and more often first), then the ones from outside.
    pub fn completions(&self, prefix: &str, limit: usize) -> Vec<String> {
        let want = prefix.to_lowercase();
        // In code: the language's own words first (keywords, builtins), then
        // names from the documents; no dictionary words (Edit syntax
        // milestone 1, step 6). In prose, as before.
        let code = self.code_words();
        let mut lang_first: Vec<String> = code
            .map(|ws| {
                ws.iter().filter(|w| w.len() > prefix.len() && w.to_lowercase().starts_with(&want)).cloned().collect()
            })
            .unwrap_or_default();
        lang_first.sort_by_key(|w| w.len());
        let n = self.buffer.line_count();
        let here = self.caret.line;
        let (from, to) = (here.saturating_sub(NEAR_LINES), (here + NEAR_LINES + 1).min(n));
        let mut score: std::collections::HashMap<&str, f32> = std::collections::HashMap::new();
        for i in from..to {
            let line = self.buffer.line(i);
            let near = 40.0 / (1.0 + (i as f32 - here as f32).abs());
            for word in line.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '\'')) {
                let word = word.trim_matches('\'');
                if word.chars().count() < 3 || word.len() <= prefix.len() || !word.to_lowercase().starts_with(&want) {
                    continue;
                }
                // The word being typed itself does not count.
                if i == here && self.word_before().is_some_and(|(_, w)| w == word) {
                    continue;
                }
                *score.entry(word).or_insert(0.0) += 1.0 + near;
            }
        }
        let mut found: Vec<(&str, f32)> = score.into_iter().collect();
        found.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        let mut out: Vec<String> = Vec::new();
        for w in lang_first.into_iter().chain(found.into_iter().map(|(w, _)| w.to_string())) {
            if out.len() < limit && !out.contains(&w) {
                out.push(w);
            }
        }
        let mut sources: Vec<Vec<String>> = Vec::new();
        if let Some(more) = &self.words_from {
            sources.push(more(prefix, limit));
        }
        // The dictionary's words last (step 7), as TextEdit offers them; not in code.
        if code.is_none()
            && let Some(sp) = vergence_spell::speller()
        {
            sources.push(sp.complete(prefix, limit));
        }
        for more in sources {
            for w in more {
                if out.len() >= limit {
                    break;
                }
                if w.len() > prefix.len() && !out.iter().any(|o| o.eq_ignore_ascii_case(&w)) {
                    out.push(w);
                }
            }
        }
        out
    }

    /// After a letter typed: the likely end of the word, if two or more of
    /// it are typed and something completes it.
    fn suggest(&mut self) {
        self.ghost = None;
        if !self.options.complete || self.caret != self.anchor {
            return;
        }
        let Some((_, word)) = self.word_before() else { return };
        if word.chars().count() < 2 {
            return;
        }
        if let Some(best) = self.completions(&word, 1).into_iter().next()
            && best.to_lowercase().starts_with(&word.to_lowercase())
        {
            // The rest of it, in the case it was written.
            let rest: String = best.chars().skip(word.chars().count()).collect();
            if !rest.is_empty() {
                self.ghost = Some(rest);
                self.ghost_space = !self.known_word(&word);
            }
        }
    }

    /// Is `word` a word as it is: in the dictionary, or written whole in the
    /// lines near the caret (other than where it is being typed)? Then Space
    /// after it is a space, not the suggestion.
    fn known_word(&self, word: &str) -> bool {
        if vergence_spell::speller().is_some_and(|sp| sp.check(word)) {
            return true;
        }
        // A word of the language (`in`, `elif`, `isset`) is one of its own.
        if let Some(lang) = self.options.syntax
            && syntax::language(lang).words.iter().any(|w| w.eq_ignore_ascii_case(word))
        {
            return true;
        }
        let n = self.buffer.line_count();
        let here = self.caret.line;
        let (from, to) = (here.saturating_sub(NEAR_LINES), (here + NEAR_LINES + 1).min(n));
        let mut seen = 0;
        for i in from..to {
            for w in self.buffer.line(i).split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '\'')) {
                if w.trim_matches('\'').eq_ignore_ascii_case(word) {
                    seen += 1;
                }
            }
        }
        // The one being typed counts once.
        seen > 1
    }

    /// Takes the ghost, and a space after it (Tab, or Space where the word
    /// typed is no word of its own).
    fn take_ghost(&mut self, rest: &str) -> Effect {
        let c = self.caret;
        let end = self.edit(c, c, &format!("{rest} "), Kind::Other);
        self.set_caret(end, false);
        self.history.break_group();
        self.reveal();
        Effect { used: true, text: true, moved: true, ..Effect::default() }
    }

    /// Opens (or narrows) the list for the word before the caret.
    fn open_list(&mut self) -> bool {
        let Some((start, word)) = self.word_before() else {
            self.list = None;
            return false;
        };
        let words = self.completions(&word, 200);
        if words.is_empty() {
            self.list = None;
            return false;
        }
        let keep = self.list.as_ref().and_then(|l| l.words.get(l.chosen).cloned());
        let chosen = keep.and_then(|k| words.iter().position(|w| *w == k)).unwrap_or(0);
        let actions = vec![Action::Word; words.len()];
        self.list = Some(List {
            words,
            actions,
            enabled: Vec::new(),
            keys: Vec::new(),
            at: None,
            chosen,
            start,
            first: 0,
            server: false,
        });
        true
    }

    /// Puts `word` in place of the word being typed (from `start` to the caret).
    fn take_word(&mut self, start: Pos, word: &str) {
        let c = self.caret;
        let end = self.edit(start, c, word, Kind::Other);
        self.set_caret(end, false);
        self.history.break_group();
        self.reveal();
    }

    /// Whether the completion list is open (the program leaves Escape to it).
    pub fn completing(&self) -> bool {
        self.list.is_some()
    }

    /// Lights these places (Find's matches: start and end on one line), in order.
    pub fn set_marks(&mut self, marks: Vec<(Pos, Pos)>) {
        self.marks = marks;
    }

    pub fn marks(&self) -> &[(Pos, Pos)] {
        &self.marks
    }

    /// Every place `needle` is, at most `limit`, in order (editor milestone
    /// 1, step 5). `case`: letters must match in case; `whole`: only where
    /// it is a word of its own. Matches lie within a line.
    pub fn find_all(&self, needle: &str, case: bool, whole: bool, limit: usize) -> Vec<(Pos, Pos)> {
        let mut out = Vec::new();
        if needle.is_empty() || needle.contains('\n') {
            return out;
        }
        for (i, line) in self.buffer.lines().enumerate() {
            for (a, b) in find_in(line, needle, case) {
                if whole && !(word_edge(line, a) && word_edge(line, b)) {
                    continue;
                }
                out.push((Pos::new(i, a), Pos::new(i, b)));
                if out.len() >= limit {
                    return out;
                }
            }
        }
        out
    }

    /// Replaces each of `places` (in order, not overlapping) with `with`, as
    /// one step to undo. How many were replaced.
    pub fn replace_all(&mut self, places: &[(Pos, Pos)], with: &str) -> usize {
        if !self.options.editable || places.is_empty() {
            return 0;
        }
        let sel = (self.caret, self.anchor);
        let now = Instant::now();
        let buffer = &mut self.buffer;
        // From the end back, so the places before stay where they are.
        self.history.group(|h| {
            for &(a, b) in places.iter().rev() {
                h.edit(buffer, a, b, with, sel, Kind::Other, now);
            }
        });
        self.marks.clear();
        self.reshape_all();
        self.revision += 1;
        self.caret = self.buffer.clamp(self.caret);
        self.anchor = self.buffer.clamp(self.anchor);
        self.history.break_group();
        places.len()
    }

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

    /// The program's own menu line chosen since last asked, if one was.
    pub fn take_command(&mut self) -> Option<u32> {
        self.asked.take()
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

    /// The text is not the saved one (unsaved work put back after a restart).
    pub fn mark_unsaved(&mut self) {
        self.history.forget_saved();
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
        let clean: String = str::replace(&str::replace(text, "\r\n", "\n"), '\r', "\n")
            .chars()
            .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
            .collect();
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
        self.marks.clear();
        self.caret = self.buffer.clamp(caret);
        self.anchor = self.buffer.clamp(anchor);
        self.goal = None;
        self.reshape_all();
        self.revision += 1;
        self.reveal();
    }

    fn reshape_all(&mut self) {
        self.shaped = vec![None; self.buffer.line_count()];
        self.spelled = vec![None; self.buffer.line_count()];
        self.hl = vec![None; self.buffer.line_count()];
        self.hl_valid = 0;
        self.recount();
    }

    /// Line `i`'s colored pieces in the text's language (None: plain text),
    /// working out the lines above it that need it first.
    fn line_syntax(&mut self, i: usize) -> Option<Rc<Hl>> {
        let lang = self.options.syntax?;
        let n = self.buffer.line_count();
        if i >= n {
            return None;
        }
        if self.hl.len() != n {
            self.hl = vec![None; n];
            self.hl_valid = 0;
        }
        if i < self.hl_valid
            && let Some(h) = &self.hl[i]
        {
            return Some(h.clone());
        }
        for j in self.hl_valid.min(i)..=i {
            let start = match j {
                0 => syntax::State::default(),
                _ => self.hl[j - 1].as_ref().map(|h| h.end.clone()).unwrap_or_default(),
            };
            // A line whose start is as before is as before: the edit above
            // stopped mattering here.
            if matches!(&self.hl[j], Some(h) if h.start == start) {
                continue;
            }
            let mut end = start.clone();
            let spans = syntax::highlight(lang, &mut end, self.buffer.line(j));
            // A line whose bold or italic pieces moved is laid out again.
            let old: Vec<_> = self.hl[j].as_ref().map(|h| styled(&h.spans).collect()).unwrap_or_default();
            if !old.iter().copied().eq(styled(&spans))
                && let Some(slot) = self.shaped.get_mut(j)
            {
                *slot = None;
            }
            // Its words may have moved in or out of prose (a string opened above).
            if let Some(slot) = self.spelled.get_mut(j) {
                *slot = None;
            }
            self.hl[j] = Some(Rc::new(Hl { start, spans: spans.into(), end }));
        }
        self.hl_valid = i + 1;
        self.hl[i].clone()
    }

    /// Line `i` laid out (if it is not yet), each piece in its face.
    fn shape(&mut self, i: usize, fonts: &mut Fonts, faces: &Faces, size: f32, tab_w: f32, wrap_w: f32) {
        let hl = self.line_syntax(i);
        if self.shaped[i].is_none() {
            let spans: &[syntax::Span] = hl.as_ref().map_or(&[], |h| &h.spans);
            self.shaped[i] = Some(Rc::new(shape_line(fonts, faces, size, tab_w, wrap_w, self.buffer.line(i), spans)));
        }
    }

    /// Byte `col` of line `line` is prose (as its pieces were last worked
    /// out; a line not yet worked out counts as prose). Always, with no
    /// language.
    fn prose_at(&self, line: usize, col: usize) -> bool {
        let Some(lang) = self.options.syntax else { return true };
        match self.hl.get(line).and_then(|h| h.as_ref()) {
            Some(h) => syntax::is_prose_in(lang, syntax::kind_at(&h.spans, col)),
            None => true,
        }
    }

    /// The language's words when the caret is in its code (not in a
    /// comment, a string or prose): what completion offers there.
    fn code_words(&self) -> Option<&'static [String]> {
        let lang = self.options.syntax?;
        let c = self.caret;
        // The piece of the word just typed (its last character).
        (!self.prose_at(c.line, c.col.saturating_sub(1))).then(|| syntax::language(lang).words.as_slice())
    }

    /// The kind of piece at `p` (None: plain text, or no language): where
    /// prose is (comments, strings), for spelling and completion.
    pub fn syntax_at(&mut self, p: Pos) -> Option<syntax::Kind> {
        let h = self.line_syntax(p.line)?;
        syntax::kind_at(&h.spans, p.col)
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
        if self.hl.len() == self.shaped.len() + old - new {
            self.hl.splice(from.line..from.line + old, std::iter::repeat_n(None, new));
        } else {
            self.hl = vec![None; self.shaped.len()];
        }
        self.hl_valid = self.hl_valid.min(from.line);
        let spelled_lines = self.spelled.len();
        if spelled_lines == self.shaped.len() + old - new {
            self.spelled.splice(from.line..from.line + old, std::iter::repeat_n(None, new));
        } else {
            self.spelled = vec![None; self.shaped.len()];
        }
        self.marks.clear();
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
            self.caret = if first == last && self.anchor == self.caret {
                self.caret
            } else {
                Pos::new(last, self.buffer.line(last).len())
            };
            self.caret = self.buffer.clamp(self.caret);
        }
        changed
    }

    /// A key, while the area has the keyboard.
    pub(crate) fn key(&mut self, key: u32, typed: Option<&str>, m: Modifiers) -> Effect {
        self.drop_server_menu();
        let ghost = self.ghost.take();
        // A word being typed is marked only once it is left.
        let was_typing = std::mem::take(&mut self.typing_word);
        // The completion list has the keys it uses.
        if self.list.is_some() {
            if let Some(e) = self.list_key(key, typed, m) {
                return e;
            }
            self.list = None;
        }
        // The suggestion shown: Tab takes it, Space takes
        // it where the word typed is no word of its own, each with a space
        // after; Escape lets it go, so Space is a plain space.
        let plain_key = !(m.ctrl || m.logo || m.alt);
        if let Some(rest) = ghost.as_deref()
            && self.options.editable
            && plain_key
        {
            if key == sym::TAB && !m.shift {
                return self.take_ghost(rest);
            }
            if key == ' ' as u32 && !m.shift && self.ghost_space {
                return self.take_ghost(rest);
            }
            if key == sym::ESCAPE {
                return Effect { used: true, ..Effect::default() };
            }
        }
        // The list on request: Alt+Escape, F5 (as on a Mac), Ctrl+Space.
        let asks = (key == sym::ESCAPE && m.alt)
            || (key == F5 && !m.ctrl && !m.alt)
            || (key == ' ' as u32 && m.ctrl && !m.alt);
        if asks {
            self.open_list();
            return Effect { used: true, ..Effect::default() };
        }
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
                    (
                        c,
                        if command || m.alt { self.buffer.next_word(c) } else { self.buffer.next_char(c) },
                        Kind::Deleting,
                    )
                };
                if from != to {
                    self.edit(from, to, "", kind);
                    self.set_caret(from, false);
                    e.text = true;
                    self.typing_word = was_typing && key == sym::BACKSPACE;
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
                    // A letter typed: the word's likely end.
                    if t.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '\'') {
                        self.typing_word = true;
                        self.suggest();
                    }
                }
                // Read-only: typing is taken (not passed on as a command) but does nothing.
                Some(_) => {}
                None => e.used = false,
            },
        }
        e
    }

    /// A key while the list is open: Some if the list used it.
    fn list_key(&mut self, key: u32, typed: Option<&str>, m: Modifiers) -> Option<Effect> {
        let plain = !(m.ctrl || m.logo || m.alt);
        let n = self.list.as_ref().map_or(0, |l| l.words.len());
        let used = Effect { used: true, ..Effect::default() };
        match key {
            sym::UP | sym::DOWN | sym::PAGE_UP | sym::PAGE_DOWN if plain && n > 0 => {
                let l = self.list.as_mut()?;
                let step: isize = match key {
                    sym::UP => -1,
                    sym::DOWN => 1,
                    sym::PAGE_UP => -(LIST_ROWS as isize),
                    _ => LIST_ROWS as isize,
                };
                let mut k = (l.chosen as isize + step).clamp(0, n as isize - 1) as usize;
                // The menu's faint and dividing lines are stepped over.
                while !l.enabled.is_empty() && l.enabled.get(k) == Some(&false) {
                    let next = k as isize + step.signum();
                    if next < 0 || next >= n as isize {
                        k = l.chosen;
                        break;
                    }
                    k = next as usize;
                }
                l.chosen = k;
                Some(used)
            }
            sym::RETURN | KP_ENTER | sym::TAB if plain => {
                let l = self.list.take()?;
                let k = l.chosen;
                Some(self.take_item(l, k))
            }
            sym::ESCAPE => {
                self.list = None;
                Some(used)
            }
            // The other popups take no typing: it closes them.
            _ if self.list.as_ref().is_some_and(|l| l.actions.first() != Some(&Action::Word)) => None,
            // Typing goes on, and narrows the list.
            sym::BACKSPACE => {
                let e = self.key_plain(key, typed, m);
                if !self.open_list() {
                    self.list = None;
                }
                Some(e)
            }
            _ if plain
                && typed.is_some_and(|t| !t.is_empty() && t.chars().all(|c| c.is_alphanumeric() || c == '_')) =>
            {
                let e = self.key_plain(key, typed, m);
                self.ghost = None;
                if !self.open_list() {
                    self.list = None;
                }
                Some(e)
            }
            _ => None,
        }
    }

    /// Does what line `k` of the popup says.
    fn take_item(&mut self, l: List, k: usize) -> Effect {
        let mut e = Effect { used: true, ..Effect::default() };
        // A faint line, or a dividing one, does nothing (the menu stays open).
        if l.enabled.get(k) == Some(&false) {
            self.list = Some(l);
            return e;
        }
        let (Some(word), Some(action)) = (l.words.get(k).cloned(), l.actions.get(k).cloned()) else { return e };
        let editable = self.options.editable;
        match action {
            Action::Word if editable => {
                self.take_word(l.start, &word);
                e.text = true;
                e.moved = true;
            }
            Action::Replace(a, b, with) if editable => {
                let end = self.edit(a, b, &with, Kind::Other);
                self.set_caret(end, false);
                self.history.break_group();
                self.reveal();
                e.text = true;
                e.moved = true;
            }
            Action::AddWord(w) => {
                if let Some(sp) = vergence_spell::speller() {
                    let _ = sp.add(&w);
                }
                self.spelled.iter_mut().for_each(|s| *s = None);
            }
            Action::Ignore(w) => {
                self.ignored.insert(w);
            }
            Action::Cut | Action::Copy => {
                let (a, b) = self.selection();
                if a != b {
                    e.copy = Some(self.buffer.slice(a, b));
                    if action == Action::Cut && editable {
                        self.edit(a, b, "", Kind::Other);
                        self.set_caret(a, false);
                        e.text = true;
                        e.moved = true;
                    }
                }
            }
            Action::Paste if editable => e.paste = true,
            Action::Undo if editable => {
                if self.undo() {
                    e.text = true;
                    e.moved = true;
                }
            }
            Action::Redo if editable => {
                if self.redo() {
                    e.text = true;
                    e.moved = true;
                }
            }
            Action::SelectAll => {
                self.select_all();
                e.moved = true;
            }
            Action::Command(id) => {
                self.asked = Some(id);
                e.moved = true;
            }
            _ => {}
        }
        e
    }

    /// A right click at `x`, `y`: on a word spelled wrong, its suggestions
    /// (and Add to dictionary, Ignore); elsewhere Cut, Copy and Paste. The
    /// caret goes there unless the click is inside the selection.
    pub(crate) fn context(&mut self, x: f32, y: f32) {
        self.ghost = None;
        let p = self.pos_at(x, y);
        let (a, b) = self.selection();
        let inside = a != b && p >= a && p <= b;
        if !inside {
            self.set_caret(p, false);
        }
        let mut words = Vec::new();
        let mut actions = Vec::new();
        if let Some((wa, wb, w)) = self.misspelled_at(p) {
            let sp = vergence_spell::speller();
            for s in sp.map(|sp| sp.suggest(&w, 5)).unwrap_or_default() {
                words.push(s.clone());
                actions.push(Action::Replace(wa, wb, s));
            }
            if words.is_empty() {
                words.push("No suggestions".into());
                actions.push(Action::Ignore(String::new()));
            }
            words.push(format!("Add “{w}” to the dictionary"));
            actions.push(Action::AddWord(w.clone()));
            words.push("Ignore it here".into());
            actions.push(Action::Ignore(w));
        }
        // The standard menu, as the terminal's (see docs/files-milestone-1.md):
        // every line always there, faint when it cannot be used, the keys at
        // the right; the spelling lines above it.
        let mut enabled = vec![true; words.len()];
        let mut keys = vec![""; words.len()];
        let selected = self.caret != self.anchor;
        let editable = self.options.editable;
        let spelling = !words.is_empty();
        let mut add = |label: &str, act: Action, on: bool, key: &'static str| {
            words.push(label.into());
            actions.push(act);
            enabled.push(on);
            keys.push(key);
        };
        if spelling {
            add("", Action::Line, false, "");
        }
        add("Undo", Action::Undo, editable && self.history.can_undo(), "Ctrl+Z");
        add("Redo", Action::Redo, editable && self.history.can_redo(), "Ctrl+Shift+Z");
        add("", Action::Line, false, "");
        add("Cut", Action::Cut, selected && editable, "Ctrl+X");
        add("Copy", Action::Copy, selected, "Ctrl+C");
        add("Paste", Action::Paste, editable, "Ctrl+V");
        add("", Action::Line, false, "");
        add("Select all", Action::SelectAll, true, "Ctrl+A");
        let own = self.options.commands.clone();
        if !own.is_empty() {
            add("", Action::Line, false, "");
            for (label, id, key) in own {
                add(&label, Action::Command(id), true, key);
            }
        }
        let chosen = enabled.iter().position(|e| *e).unwrap_or(0);
        self.list =
            Some(List { words, actions, enabled, keys, at: Some((x, y)), chosen, start: p, first: 0, server: false });
    }

    /// The right-click menu just opened (context), for the server to draw as
    /// every window's right-click menu is (bus v20: one kind of menu
    /// everywhere; see docs/menus-milestone-1.md): where, and its lines, each line's id its
    /// index from `base`. The menu is kept here, not drawn, until
    /// menu_chosen or anything else done in the area.
    pub(crate) fn menu_for_server(&mut self, base: u32) -> Option<((f32, f32), Vec<vergence_ui::MenuLine>)> {
        let l = self.list.as_mut().filter(|l| !l.enabled.is_empty())?;
        let at = l.at?;
        l.server = true;
        let lines = l
            .actions
            .iter()
            .enumerate()
            .map(|(i, a)| match a {
                Action::Line => vergence_ui::MenuLine::Separator,
                _ => vergence_ui::MenuLine::Item {
                    id: base + i as u32,
                    label: l.words[i].clone(),
                    keys: l.keys.get(i).copied().unwrap_or("").to_string(),
                    enabled: l.enabled.get(i).copied().unwrap_or(true),
                    checked: None,
                },
            })
            .collect();
        Some((at, lines))
    }

    /// Line `k` of the menu the server drew was chosen: done as a press on it.
    pub(crate) fn menu_chosen(&mut self, k: usize) -> Effect {
        match self.list.take() {
            Some(l) if l.server && l.enabled.get(k).copied().unwrap_or(false) => self.take_item(l, k),
            _ => Effect::default(),
        }
    }

    /// A menu the server drew and that was closed without a choice is
    /// forgotten at the next key or press.
    fn drop_server_menu(&mut self) {
        if self.list.as_ref().is_some_and(|l| l.server) {
            self.list = None;
        }
    }

    /// The word spelled wrong at `p`, if any: where it starts and ends, and it.
    fn misspelled_at(&self, p: Pos) -> Option<(Pos, Pos, String)> {
        if !self.options.spell {
            return None;
        }
        let sp = vergence_spell::speller()?;
        let line = self.buffer.line(p.line);
        let (a, b) = vergence_spell::words_to_check(line).into_iter().find(|(a, b)| *a <= p.col && p.col <= *b)?;
        if !self.prose_at(p.line, a) {
            return None;
        }
        let w = &line[a..b];
        (!sp.check(w) && !self.ignored.contains(w)).then(|| (Pos::new(p.line, a), Pos::new(p.line, b), w.to_string()))
    }

    /// Line `i`'s words spelled wrong (worked out once until the line changes).
    fn spelled_wrong(&mut self, i: usize) -> Rc<[(usize, usize)]> {
        if let Some(Some(s)) = self.spelled.get(i) {
            return s.clone();
        }
        // In code only its prose is checked (comments, strings; Markdown's
        // text): a name is not a word spelled wrong.
        let hl = self.line_syntax(i);
        let lang = self.options.syntax;
        let line = self.buffer.line(i);
        let prose = |a: usize| match (lang, &hl) {
            (Some(l), Some(h)) => syntax::is_prose_in(l, syntax::kind_at(&h.spans, a)),
            _ => true,
        };
        let bad: Rc<[(usize, usize)]> = match vergence_spell::speller() {
            Some(sp) => vergence_spell::words_to_check(line)
                .into_iter()
                .filter(|(a, b)| prose(*a) && !sp.check(&line[*a..*b]))
                .collect(),
            None => Rc::from(Vec::new()),
        };
        if let Some(slot) = self.spelled.get_mut(i) {
            *slot = Some(bad.clone());
        }
        bad
    }

    /// A key as if no list or suggestion were showing.
    fn key_plain(&mut self, key: u32, typed: Option<&str>, m: Modifiers) -> Effect {
        let list = self.list.take();
        let e = self.key(key, typed, m);
        self.list = list;
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

    /// Scrolls by `dy` points (down is positive), held by the top line. It
    /// stops when the last line is at the view's bottom, so text that fits
    /// doesn't scroll at all (otherwise a few lines that don't fill the window
    /// scroll up under the bar, and the last line can come up to the top).
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
        let (above, total, view) = self.extent();
        let most = (total - view).max(0.0);
        if above > most + 0.01 {
            self.top = 0;
            self.top_off = 0.0;
            self.walk_down(most);
        }
    }

    /// From the very top, `dy` points down (within the text).
    fn walk_down(&mut self, mut dy: f32) {
        let n = self.buffer.line_count();
        while self.top + 1 < n {
            let h = self.line_height(self.top);
            if dy < h {
                break;
            }
            dy -= h;
            self.top += 1;
        }
        self.top_off = dy.max(0.0);
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

    /// The text above the view, all of it, and the view's height, in points.
    fn extent(&self) -> (f32, f32, f32) {
        let n = self.buffer.line_count();
        let above: f32 = (0..self.top.min(n)).map(|k| self.line_height(k)).sum::<f32>() + self.top_off;
        let below: f32 = (self.top.min(n)..n).map(|k| self.line_height(k)).sum();
        let view = self.geom.rect.h - 2.0 * PAD_Y;
        (above, above + below - self.top_off, view)
    }

    /// A press: one click puts the caret there (Shift: the selection runs
    /// there), two take a word, three the line.
    /// A press on the popup does what its line says (`used` set); elsewhere
    /// it closes it and the press is the text's.
    pub(crate) fn press(&mut self, x: f32, y: f32, count: u8, shift: bool) -> Effect {
        self.drop_server_menu();
        self.ghost = None;
        self.typing_word = false;
        if let Some((r, row_h)) = self.geom.list
            && let Some(l) = self.list.take()
            && x >= r.x
            && x < r.x + r.w
            && y >= r.y
            && y < r.y + r.h
        {
            let k = ((y - r.y - 4.0) / row_h).floor().max(0.0) as usize + l.first;
            return self.take_item(l, k);
        }
        self.list = None;
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
        Effect::default()
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
                    (
                        Pos::new(p.line, 0),
                        if p.line + 1 < self.buffer.line_count() { Pos::new(p.line + 1, 0) } else { self.buffer.end() },
                    )
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

    /// What a screen reader or an agent is told: the whole text if not too
    /// long (the text alone, so reading it gives the document back; the
    /// caret's place is the tree's caret, `focused` for an agent), else where
    /// the caret is.
    pub(crate) fn meaning_value(&self) -> String {
        let (l, c) = self.line_column();
        if self.buffer.bytes() <= TELL_WHOLE {
            self.buffer.to_text()
        } else {
            format!("line {l} of {}, column {c}", self.buffer.line_count())
        }
    }

    // --- Drawing ---

    /// Draws the area in `r`; its top `inset` points are under the window's
    /// bar: the text starts below them and scrolls up beneath them.
    pub(crate) fn draw(
        &mut self,
        scene: &mut Scene,
        fonts: &mut Fonts,
        r: Rect,
        inset: f32,
        theme: &Theme,
        focused: bool,
        caret_on: bool,
    ) {
        let inset = inset.clamp(0.0, (r.h - 40.0).max(0.0));
        let v = Rect::new(r.x, r.y + inset, r.w, r.h - inset);
        // "monospace" is the desktop's monospace font, at its size unless the
        // area names one (text milestone 1, step 5).
        let mono = self.options.font == "monospace";
        let family = if mono { theme.mono.as_str() } else { self.options.font.as_str() };
        let size = match self.options.size {
            Some(s) => vergence_look::text_size(s, theme.text_factor),
            None if mono => vergence_look::text_size(theme.mono_size, theme.text_factor),
            None => theme.text_size,
        }
        .clamp(4.0, 200.0);
        let Some(face) = fonts
            .face(family, 400, false)
            .or_else(|| fonts.face("monospace", 400, false))
            .or_else(|| fonts.face(&theme.font, 400, false))
        else {
            return;
        };
        let faces = Faces {
            regular: face,
            bold: fonts.face(family, 700, false).or_else(|| fonts.face("monospace", 700, false)).unwrap_or(face),
            italic: fonts.face(family, 400, true).or_else(|| fonts.face("monospace", 400, true)).unwrap_or(face),
        };
        let m = fonts.metrics(face, size);
        let spacing = self.options.line_spacing.filter(|s| s.is_finite()).unwrap_or(CODE_SPACING).clamp(1.0, 3.0);
        let line_h = (m.ascent + m.descent + m.line_gap).max(size * spacing).ceil();
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
        let resized = self.geom.rect.h != v.h || self.geom.rect.w != v.w;
        self.geom = Geom { rect: v, text_x, text_w, line_h, char_w, wrap_w, tab_w, thumb: self.geom.thumb, list: None };
        self.top = self.top.min(n - 1);
        if resized {
            // The top line's rows may have changed: stay on the same row.
            self.scroll_by(0.0);
        }

        // No ground of its own: the window's shows, as opaque as its settings
        // say (a second fill would make it nearly solid). The line numbers on a
        // shade of their own over it, to set them apart from the text.
        if gutter > 0.0 {
            scene.fill(Rect::new(r.x, r.y, gutter, r.h), alpha(theme.text, if theme.dark { 14 } else { 11 }));
        }
        scene.push_clip(r, 0.0);
        let (sa, sb) = self.selection();
        let selected = sa != sb;
        let sel_c = if focused { alpha(theme.accent, 90) } else { alpha(theme.accent, 60) };
        let shade = alpha(theme.text, if theme.dark { 16 } else { 10 });
        let wave_c = if theme.dark { Color::rgb(255, 99, 92) } else { Color::rgb(214, 40, 40) };
        let mark_c = if theme.dark { Color::rgba(255, 200, 60, 70) } else { Color::rgba(255, 196, 0, 110) };
        let text_c = theme.text;
        let syn = vergence_look::syntax_palette(theme.dark);
        let num_c = theme.dim;
        let text_clip = Rect::new(r.x + gutter, r.y, (r.w - gutter).max(0.0), r.h);
        let mut y = v.y + PAD_Y - self.top_off;
        let mut i = self.top;
        // The lines above the view that show under the bar.
        while i > 0 && y > r.y {
            i -= 1;
            self.shape(i, fonts, &faces, size, tab_w, wrap_w);
            y -= self.line_height(i);
        }
        let mut widest = 0.0f32;
        while i < n && y < r.y + r.h {
            self.shape(i, fonts, &faces, size, tab_w, wrap_w);
            let s = self.shaped[i].clone().expect("laid out above");
            let h = s.rows.len() as f32 * line_h;
            widest = widest.max(s.width);
            if y + h > r.y {
                // The words spelled wrong, less those let be and the one being typed.
                let mut bad: Vec<(usize, usize)> =
                    if self.options.spell { self.spelled_wrong(i).to_vec() } else { Vec::new() };
                if !self.ignored.is_empty() {
                    let line = self.buffer.line(i);
                    bad.retain(|(a, b)| !self.ignored.contains(&line[*a..*b]));
                }
                if self.typing_word && i == self.caret.line {
                    let c = self.caret.col;
                    bad.retain(|(a, b)| !(*a <= c && c <= *b));
                }
                let hl = self.line_syntax(i);
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
                    // Find's matches on this row.
                    let first = self.marks.partition_point(|m| m.0.line < i);
                    for &(ma, mb) in self.marks[first..].iter().take_while(|m| m.0.line == i) {
                        // The one chosen shows as the selection alone.
                        if (ma, mb) == (sa, sb) {
                            continue;
                        }
                        let (from, to) = (ma.col.max(rs), mb.col.min(re));
                        if from < to {
                            scene.fill(
                                Rect::new(x0 + s.x_of(from), ry + 1.0, s.x_of(to) - s.x_of(from), line_h - 2.0),
                                mark_c,
                            );
                        }
                    }
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
                    // A red wave under each word spelled wrong on this row.
                    for &(wa, wb) in &bad {
                        let (from, to) = (wa.max(rs), wb.min(re));
                        if from >= to {
                            continue;
                        }
                        let (x1, x2) = (x0 + s.x_of(from), x0 + s.x_of(to));
                        let wy = (base + (m.descent * 0.5).max(2.0)).min(ry + line_h - 2.0);
                        let mut x = x1;
                        let mut up = true;
                        while x < x2 {
                            let nx = (x + 2.0).min(x2);
                            let (a, b) = if up { (wy + 1.0, wy - 1.0) } else { (wy - 1.0, wy + 1.0) };
                            scene.line(Point::new(x, a), Point::new(nx, b), 1.0, wave_c);
                            x = nx;
                            up = !up;
                        }
                    }
                    let mut p = rs;
                    let spans: &[syntax::Span] = hl.as_ref().map_or(&[], |h| &h.spans);
                    let mut k = spans.partition_point(|sp| sp.end <= rs);
                    while p < re {
                        let q = line[p..re].find('\t').map_or(re, |k| p + k);
                        // This piece between tabs, cut where its colors change.
                        let mut a = p;
                        while a < q {
                            while k < spans.len() && spans[k].end <= a {
                                k += 1;
                            }
                            let (b, kind) = match spans.get(k) {
                                Some(sp) if sp.start <= a => (sp.end.min(q), Some(sp.kind)),
                                Some(sp) => (sp.start.min(q), None),
                                None => (q, None),
                            };
                            // High contrast keeps one ink (colors would lower it), not the weights.
                            let c = match kind {
                                Some(k) if !theme.high_contrast => syntax_color(k, syn, text_c),
                                _ => text_c,
                            };
                            if b > a {
                                let f = faces.of(kind);
                                text(scene, fonts, f, size, c, Point::new(x0 + s.x_of(a), base), &line[a..b]);
                            }
                            a = b;
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
            self.shape(k, fonts, &faces, size, tab_w, wrap_w);
        }
        if wrap_w <= 0.0 {
            let room = (widest - text_w + char_w).max(0.0);
            self.xscroll = self.xscroll.clamp(0.0, room);
        } else {
            self.xscroll = 0.0;
        }
        // The caret, the word's likely end after it, the list under it.
        let (row, x) = self.visual(self.caret);
        let cy = v.y + PAD_Y + self.line_y(self.caret.line) + row as f32 * line_h;
        let cx = text_x - self.xscroll + x;
        let caret_seen = cy + line_h > r.y && cy < r.y + r.h && cx >= text_clip.x;
        self.caret_drawn = (focused && caret_seen).then(|| Rect::new(cx.round() - 0.5, cy + 1.0, 1.5, line_h - 2.0));
        let base = |y: f32| y + ((line_h - (m.ascent + m.descent)) / 2.0 + m.ascent).round();
        if focused && !selected && caret_seen {
            if let Some(g) = &self.ghost {
                scene.push_clip(text_clip, 0.0);
                text(scene, fonts, face, size, num_c, Point::new(cx, base(cy)), g);
                scene.pop_clip();
            }
            if caret_on {
                scene.fill(Rect::new(cx.round() - 0.5, cy + 1.0, 1.5, line_h - 2.0), text_c);
            }
        }
        if focused
            && (caret_seen || self.list.as_ref().is_some_and(|l| l.at.is_some()))
            && let Some(l) = self.list.as_mut().filter(|l| !l.server)
        {
            let menu = !l.enabled.is_empty();
            let rows = if menu { l.words.len() } else { l.words.len().min(LIST_ROWS) };
            if l.chosen < l.first {
                l.first = l.chosen;
            } else if l.chosen >= l.first + rows {
                l.first = l.chosen + 1 - rows;
            }
            let widest = l
                .words
                .iter()
                .zip(l.keys.iter().chain(std::iter::repeat(&"")))
                .map(|(w, k)| {
                    fonts.layout(face, size, w).1
                        + if k.is_empty() { 0.0 } else { fonts.layout(face, size, k).1 + 28.0 }
                })
                .fold(0.0f32, f32::max);
            let w = (widest + 24.0).clamp(if menu { 220.0 } else { 140.0 }, (r.w - 8.0).max(140.0));
            let h = rows as f32 * line_h + 8.0;
            // Under the caret's row (above it where there is no room), or at the right click.
            let (ax, ay, below) = match l.at {
                Some((px, py)) => (px, py, py),
                None => (cx - 12.0, cy, cy + line_h),
            };
            let lx = ax.clamp(r.x + 4.0, (r.x + r.w - w - 4.0).max(r.x + 4.0));
            let ly = if below + h <= r.y + r.h { below } else { (ay - h).max(r.y) };
            let lr = Rect::new(lx, ly, w, h);
            let menu_c = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
            let ground = if menu {
                menu_c(vergence_look::MENU)
            } else if theme.dark {
                theme.card
            } else {
                Color::rgb(255, 255, 255)
            };
            scene.fill_rounded(
                Rect::new(lx - 1.0, ly + 1.0, w + 2.0, h + 2.0),
                vergence_look::RADIUS + 1.0,
                alpha(Color::rgb(0, 0, 0), 50),
            );
            scene.fill_rounded(lr, vergence_look::RADIUS, ground);
            scene.stroke(lr, vergence_look::RADIUS, 1.0, theme.line);
            for (k, word) in l.words.iter().enumerate().skip(l.first).take(rows) {
                let ry = ly + 4.0 + (k - l.first) as f32 * line_h;
                if menu && l.actions.get(k) == Some(&Action::Line) {
                    scene.fill(
                        Rect::new(lx + 10.0, ry + line_h / 2.0, w - 20.0, 1.0),
                        alpha(menu_c(vergence_look::MENU_INK_OFF), 120),
                    );
                    continue;
                }
                let on = l.enabled.get(k) != Some(&false);
                let chosen = k == l.chosen && on;
                if chosen {
                    scene.fill_rounded(Rect::new(lx + 4.0, ry, w - 8.0, line_h), vergence_look::RADIUS, theme.accent);
                }
                let ink = if chosen {
                    Color::rgb(255, 255, 255)
                } else if menu {
                    menu_c(if on { vergence_look::MENU_INK } else { vergence_look::MENU_INK_OFF })
                } else {
                    text_c
                };
                text(scene, fonts, face, size, ink, Point::new(lx + 12.0, base(ry)), word);
                if let Some(key) = l.keys.get(k).filter(|k| !k.is_empty()) {
                    let kw = fonts.layout(face, size, key).1;
                    let kc = if chosen { Color::rgb(255, 255, 255) } else { menu_c(vergence_look::MENU_INK_OFF) };
                    text(scene, fonts, face, size, kc, Point::new(lx + w - 12.0 - kw, base(ry)), key);
                }
            }
            self.geom.list = Some((lr, line_h));
        }
        // The edge of the line numbers.
        if gutter > 0.0 {
            scene.fill(Rect::new(r.x + gutter - 1.0, r.y, 1.0, r.h), alpha(theme.line, 160));
        }
        scene.pop_clip();
        // The scroll bar, when there is more than fits.
        let (above, total, view) = self.extent();
        // Its track starts below the bar.
        self.geom.thumb = (total > view + 1.0 && view > 0.0).then(|| {
            let th = (v.h * view / total).max(MIN_THUMB).min(v.h);
            let f = (above / (total - view).max(1.0)).clamp(0.0, 1.0);
            Rect::new(v.x + v.w - BAR_W - 2.0, v.y + (v.h - th) * f, BAR_W, th)
        });
        if let Some(t) = self.geom.thumb {
            scene.fill_rounded(t, BAR_W / 2.0, alpha(theme.text, 80));
        }
    }
}

/// Where `needle` is in `line`, as byte ranges, not overlapping; without
/// case unless `case`.
fn find_in(line: &str, needle: &str, case: bool) -> Vec<(usize, usize)> {
    if case {
        return line.match_indices(needle).map(|(a, m)| (a, a + m.len())).collect();
    }
    let mut out = Vec::new();
    if needle.is_ascii() {
        // Bytes, at character starts.
        let (l, n) = (line.as_bytes(), needle.as_bytes());
        let mut i = 0;
        while i + n.len() <= l.len() {
            if line.is_char_boundary(i) && l[i..i + n.len()].eq_ignore_ascii_case(n) {
                out.push((i, i + n.len()));
                i += n.len();
            } else {
                i += 1;
            }
        }
        return out;
    }
    let want: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    let mut start = 0;
    while start < line.len() {
        // Compare character by character, folded.
        let mut got = line[start..].char_indices().flat_map(|(k, c)| c.to_lowercase().map(move |l| (k, c, l)));
        let mut end = None;
        let mut ok = true;
        for w in &want {
            match got.next() {
                Some((k, c, l)) if l == *w => end = Some(start + k + c.len_utf8()),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        match (ok, end) {
            (true, Some(e)) => {
                out.push((start, e));
                start = e;
            }
            _ => start += line[start..].chars().next().map_or(1, char::len_utf8),
        }
    }
    out
}

/// Whether byte `i` of `line` is at a word's edge (no letter, digit or _ on both sides).
fn word_edge(line: &str, i: usize) -> bool {
    let w = |c: char| c.is_alphanumeric() || c == '_';
    !(line[..i].chars().next_back().is_some_and(w) && line[i..].chars().next().is_some_and(w))
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
        } else if in_word
            && !prev_join
            && matches!(c, '\'' | '’' | '-')
            && chars.peek().is_some_and(|d| d.is_alphanumeric())
        {
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
/// A kind of piece's color in the style's syntax palette; the text's own
/// for kinds that differ only in weight or slant (emphasis, strong).
fn syntax_color(kind: syntax::Kind, p: &vergence_look::SyntaxPalette, text: Color) -> Color {
    use syntax::Kind as K;
    let rgb = match kind {
        K::Keyword => p.keyword,
        K::Type => p.type_name,
        K::Builtin => p.builtin,
        K::Function => p.function,
        K::Constant => p.constant,
        K::Variable => p.variable,
        K::String | K::AttrValue | K::Code => p.string,
        K::Escape => p.escape,
        K::Number => p.number,
        K::Comment | K::Quote => p.comment,
        K::DocComment => p.doc_comment,
        K::Operator => p.operator,
        K::Meta => p.meta,
        K::Tag => p.tag,
        K::Attribute => p.attribute,
        K::Heading => p.heading,
        K::Link => p.link,
        K::ListMarker => p.list_marker,
        K::Emphasis | K::Strong | K::Source => return text,
    };
    Color::rgb(rgb.0, rgb.1, rgb.2)
}

pub fn text_area<M>(doc: &TextDoc) -> Element<M> {
    Element::new(EKind::Area { doc: doc.clone(), on_change: None, autofocus: false }, Length::FILL, Length::FILL)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In code, spelling checks only the prose (a comment here), and
    /// completion offers the language's words, not the dictionary's.
    #[test]
    fn code_spells_its_prose_and_completes_its_words() {
        let py = syntax::by_name("python").unwrap();
        let options = AreaOptions { syntax: Some(py), ..AreaOptions::default() };
        let mut d = Doc::new(Buffer::from_text("import jsonx  # a helo here\nx = isi"), options);
        if vergence_spell::speller().is_some() {
            let bad: Vec<String> =
                d.spelled_wrong(0).iter().map(|(a, b)| d.buffer.line(0)[*a..*b].to_string()).collect();
            assert_eq!(bad, ["helo"], "names are not spelled; the comment is");
        }
        d.line_syntax(1);
        d.set_selection(Pos { line: 1, col: 7 }, Pos { line: 1, col: 7 });
        let got = d.completions("isi", 5);
        assert_eq!(got.first().map(String::as_str), Some("isinstance"), "{got:?}");
        assert!(got.iter().all(|w| w != "isinglass"), "no dictionary words in code: {got:?}");
        assert!(d.known_word("elif"), "a keyword is a word of its own");
    }

    /// Colors follow edits across lines: opening a triple-quoted string
    /// turns the lines after it into string, and undoing puts them back,
    /// without the document being opened again.
    #[test]
    fn syntax_follows_edits_across_lines() {
        let py = syntax::by_name("python").unwrap();
        let options = AreaOptions { syntax: Some(py), ..AreaOptions::default() };
        let mut d = Doc::new(Buffer::from_text("x = 1\ny = 'a'\nz = len(y)\n"), options);
        let at = Pos { line: 2, col: 4 };
        assert_eq!(d.syntax_at(at), Some(syntax::Kind::Builtin));
        d.set_selection(Pos::default(), Pos::default());
        d.insert("s = '''");
        assert_eq!(d.syntax_at(at), Some(syntax::Kind::String), "inside the open string now");
        d.undo();
        assert_eq!(d.syntax_at(at), Some(syntax::Kind::Builtin), "closed again by the undo");
        // An edit inside a line keeps the lines after it as they were.
        d.set_selection(Pos { line: 0, col: 5 }, Pos { line: 0, col: 5 });
        d.insert("0");
        assert_eq!(d.syntax_at(at), Some(syntax::Kind::Builtin));
        // No language: no pieces.
        let mut plain = Doc::new(Buffer::from_text("z = len(y)"), AreaOptions::default());
        assert_eq!(plain.syntax_at(Pos { line: 0, col: 4 }), None);
    }

    fn doc(text: &str) -> Doc {
        let mut d = Doc::new(Buffer::from_text(text), AreaOptions::default());
        // Laid out as if drawn: 10-point rows, characters 8 wide, 80 wide.
        d.geom = Geom {
            rect: Rect::new(0.0, 0.0, 200.0, 2.0 * PAD_Y + 50.0),
            text_x: 0.0,
            text_w: 80.0,
            line_h: 10.0,
            char_w: 8.0,
            wrap_w: 80.0,
            tab_w: 32.0,
            thumb: None,
            list: None,
        };
        d
    }

    fn key(d: &mut Doc, k: u32) -> Effect {
        d.key(k, None, Modifiers::default())
    }

    fn space(d: &mut Doc) -> Effect {
        d.key(' ' as u32, Some(" "), Modifiers::default())
    }

    fn typed(d: &mut Doc, s: &str) {
        for c in s.chars() {
            let k = if c == '\n' { sym::RETURN } else { c as u32 };
            d.key(k, Some(&c.to_string()), Modifiers::default());
        }
    }

    #[test]
    fn text_that_fits_does_not_scroll() {
        // The view holds 5 rows; 3 lines fit and never move.
        let mut d = doc("a\nb\nc");
        d.scroll_by(100.0);
        assert_eq!((d.top, d.top_off), (0, 0.0));
        let (above, total, view) = d.extent();
        assert!(above == 0.0 && total <= view, "no scroll bar: {total} in {view}");
        // 12 lines: down to the last line at the view's bottom, no further.
        let mut d = doc(&(0..12).map(|i| i.to_string()).collect::<Vec<_>>().join("\n"));
        d.scroll_by(1000.0);
        assert_eq!((d.top, d.top_off), (7, 0.0));
        let (above, total, view) = d.extent();
        assert_eq!(above, total - view);
        d.scroll_by(-25.0);
        assert_eq!((d.top, d.top_off), (4, 5.0));
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
        // Scrolling stops with the last line at the bottom (101 lines, 5 in view).
        d.wheel(0.0, 1e6);
        assert_eq!((d.top, d.top_off), (96, 0.0));
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
    fn find_and_replace_all() {
        let mut d = doc("Cat cat CAT\ncatalog concat\nÉcole école");
        let p = |l, a, b| (Pos::new(l, a), Pos::new(l, b));
        assert_eq!(d.find_all("cat", true, false, 100), vec![p(0, 4, 7), p(1, 0, 3), p(1, 11, 14)]);
        assert_eq!(d.find_all("cat", false, false, 100).len(), 5);
        assert_eq!(d.find_all("cat", false, true, 100), vec![p(0, 0, 3), p(0, 4, 7), p(0, 8, 11)]);
        assert_eq!(d.find_all("ÉCOLE", false, false, 100).len(), 2, "folded beyond ASCII");
        assert_eq!(d.find_all("a", false, false, 2).len(), 2, "the limit holds");
        let found = d.find_all("cat", false, true, 100);
        d.set_marks(found.clone());
        assert_eq!(d.replace_all(&found, "dog"), 3);
        assert_eq!(d.buffer().line(0), "dog dog dog");
        assert!(d.marks().is_empty(), "let go once changed");
        d.undo();
        assert_eq!(d.buffer().line(0), "Cat cat CAT", "one undo for all of them");
    }

    #[test]
    fn a_word_is_suggested_as_it_is_typed_and_tab_takes_it() {
        let mut d = doc("vergence verge vertical\n");
        d.set_selection(Pos::new(1, 0), Pos::new(1, 0));
        typed(&mut d, "ve");
        assert_eq!(d.ghost.as_deref(), Some("rge"), "verge: as near, and first by name");
        typed(&mut d, "rt");
        assert_eq!(d.ghost.as_deref(), Some("ical"));
        key(&mut d, sym::TAB);
        assert_eq!(d.buffer().line(1), "vertical ", "a space after it");
        assert!(d.ghost.is_none());
        // Any other key goes on as if nothing were shown.
        typed(&mut d, "ve");
        key(&mut d, sym::LEFT);
        assert!(d.ghost.is_none());
        assert_eq!(d.buffer().line(1), "vertical ve");
        // Off: nothing is shown.
        let mut o = d.options().clone();
        o.complete = false;
        d.set_options(o);
        d.set_selection(Pos::new(1, 11), Pos::new(1, 11));
        typed(&mut d, "r");
        assert!(d.ghost.is_none());
    }

    #[test]
    fn space_takes_the_suggestion_unless_the_word_typed_is_one() {
        // "verg" is no word: Space takes "vergence", a space after it.
        let mut d = doc("vergence vergence\n");
        d.set_selection(Pos::new(1, 0), Pos::new(1, 0));
        typed(&mut d, "verg");
        assert_eq!(d.ghost.as_deref(), Some("ence"));
        space(&mut d);
        assert_eq!(d.buffer().line(1), "vergence ");
        // "in" is a word (written whole above): Space is a space.
        let mut d = doc("in inside inside\n");
        d.set_selection(Pos::new(1, 0), Pos::new(1, 0));
        typed(&mut d, "in");
        assert_eq!(d.ghost.as_deref(), Some("side"));
        space(&mut d);
        assert_eq!(d.buffer().line(1), "in ");
        // Escape lets a suggestion go: Space is a plain space.
        let mut d = doc("vergence vergence\n");
        d.set_selection(Pos::new(1, 0), Pos::new(1, 0));
        typed(&mut d, "verg");
        key(&mut d, sym::ESCAPE);
        space(&mut d);
        assert_eq!(d.buffer().line(1), "verg ");
    }

    #[test]
    fn the_list_chooses_and_narrows_and_takes_words_from_outside() {
        let mut d = doc("apple apricot\n");
        d.set_words(Some(Rc::new(|p: &str, _| {
            if "april".starts_with(&p.to_lowercase()) { vec!["April".to_string()] } else { vec![] }
        })));
        d.set_selection(Pos::new(1, 0), Pos::new(1, 0));
        typed(&mut d, "a");
        d.key(F5, None, Modifiers::default());
        // The document's words, then the program's, then the dictionary's.
        let first3 = |d: &Doc| d.list.as_ref().map(|l| l.words.iter().take(3).cloned().collect::<Vec<_>>());
        assert_eq!(first3(&d), Some(vec!["apple".to_string(), "apricot".into(), "April".into()]));
        typed(&mut d, "p");
        assert_eq!(first3(&d), Some(vec!["apple".to_string(), "apricot".into(), "April".into()]), "all start with ap");
        typed(&mut d, "r");
        assert_eq!(first3(&d).map(|v| v[..2].to_vec()), Some(vec!["apricot".to_string(), "April".into()]), "narrowed");
        assert!(d.list.as_ref().is_some_and(|l| l.words.iter().all(|w| w.to_lowercase().starts_with("apr"))));
        key(&mut d, sym::DOWN);
        key(&mut d, sym::RETURN);
        assert_eq!(d.buffer().line(1), "April");
        assert!(!d.completing());
        typed(&mut d, " ap");
        d.key(' ' as u32, None, Modifiers { ctrl: true, ..Modifiers::default() });
        assert!(d.completing());
        key(&mut d, sym::ESCAPE);
        assert!(!d.completing());
        assert_eq!(d.buffer().line(1), "April ap");
    }

    #[test]
    fn words_spelled_wrong_are_found_and_fixed_from_a_right_click() {
        let mut d = doc("Teh colour of the hosue\n");
        d.geom.wrap_w = 0.0;
        d.geom.text_w = 400.0;
        let bad = d.spelled_wrong(0).to_vec();
        let line = d.buffer().line(0).to_string();
        let words: Vec<&str> = bad.iter().map(|(a, b)| &line[*a..*b]).collect();
        assert_eq!(words, vec!["Teh", "colour", "hosue"]);
        // A right click on "hosue": suggestions first, house among them.
        d.context(8.0 * 20.0 + 1.0, PAD_Y + 2.0);
        let l = d.list.as_ref().expect("a menu");
        assert_eq!(l.words[0], "house");
        assert!(l.words.iter().any(|w| w.starts_with("Add “hosue”")));
        let e = d.key(sym::RETURN, None, Modifiers::default());
        assert!(e.text);
        assert_eq!(d.buffer().line(0), "Teh colour of the house");
        // Ignored here, it is no longer marked.
        d.context(1.0, PAD_Y + 2.0);
        let k = d.list.as_ref().unwrap().words.iter().position(|w| w == "Ignore it here").unwrap();
        for _ in 0..k {
            key(&mut d, sym::DOWN);
        }
        key(&mut d, sym::RETURN);
        assert!(d.misspelled_at(Pos::new(0, 1)).is_none());
        // A word still being typed is not marked yet.
        d.set_selection(Pos::new(1, 0), Pos::new(1, 0));
        typed(&mut d, "wrng");
        assert!(d.typing_word);
        key(&mut d, sym::LEFT);
        assert!(!d.typing_word);
        // Elsewhere, the clipboard's commands.
        d.set_selection(Pos::new(0, 0), Pos::new(0, 3));
        d.context(8.0 + 1.0, PAD_Y + 2.0);
        assert!(d.list.as_ref().unwrap().words.iter().any(|w| w == "Copy"));
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
