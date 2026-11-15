//! Text being edited (editor milestone 1, step 1): a buffer of lines for
//! the text area, and the undo history both it and the one-line field use.
//!
//! The buffer keeps its text as lines without their endings, so a place
//! in it is a line and a byte offset in that line ([`Pos`]) and an edit is
//! "replace from here to there with this". The ending the file came with
//! (LF or CRLF) and whether it ended in one are kept for writing it back.
//! Lines in a `Vec` are enough for a 200,000-line file (inserting a line
//! moves pointers, not text); a piece table comes only if measurements ask.
//!
//! The history records every edit with what it removed, so undoing puts
//! the removed text back, and the caret and selection with it. Typing
//! groups into one step until a pause ([`GROUP_PAUSE`]), a caret jump, or a
//! change between typing and deleting.

use std::time::{Duration, Instant};

/// A place in a [`Buffer`]: a line, and a byte offset in that line (always
/// on a character boundary).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub const fn new(line: usize, col: usize) -> Pos {
        Pos { line, col }
    }
}

/// How the file's lines ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ending {
    #[default]
    Lf,
    CrLf,
}

impl Ending {
    pub fn as_str(self) -> &'static str {
        match self {
            Ending::Lf => "\n",
            Ending::CrLf => "\r\n",
        }
    }
}

/// Something the history can edit: a buffer of lines, or a field's string.
pub trait Editable {
    type Pos: Copy + Eq + Ord + std::fmt::Debug;
    /// Replaces `from..to` with `with`; returns where the inserted text ends.
    fn replace(&mut self, from: Self::Pos, to: Self::Pos, with: &str) -> Self::Pos;
    /// The text from `from` to `to`.
    fn slice(&self, from: Self::Pos, to: Self::Pos) -> String;
}

impl Editable for String {
    type Pos = usize;
    fn replace(&mut self, from: usize, to: usize, with: &str) -> usize {
        self.replace_range(from..to, with);
        from + with.len()
    }
    fn slice(&self, from: usize, to: usize) -> String {
        self[from..to].to_string()
    }
}

/// Text as lines (editor milestone 1).
#[derive(Clone, Debug)]
pub struct Buffer {
    lines: Vec<String>,
    ending: Ending,
    /// Whether the text ended with a line ending (written back as found).
    final_newline: bool,
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer { lines: vec![String::new()], ending: Ending::Lf, final_newline: true }
    }
}

impl Buffer {
    /// The buffer for `text`: its lines, the ending most of them use, and
    /// whether it ends with one. A lone CR (old Mac files) ends a line too.
    pub fn from_text(text: &str) -> Buffer {
        let mut lines = Vec::with_capacity(text.len() / 40 + 1);
        let (mut crlf, mut lf) = (0usize, 0usize);
        let bytes = text.as_bytes();
        let mut start = 0;
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => {
                    lf += 1;
                    lines.push(text[start..i].to_string());
                    start = i + 1;
                }
                b'\r' => {
                    lines.push(text[start..i].to_string());
                    if bytes.get(i + 1) == Some(&b'\n') {
                        crlf += 1;
                        i += 1;
                    }
                    start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        let final_newline = start == text.len() && !text.is_empty();
        if !final_newline || text.is_empty() {
            lines.push(text[start..].to_string());
        } else {
            // "a\n" is one line, "a", that ends; the caret may still go
            // below it, onto an empty last line.
            lines.push(String::new());
        }
        let ending = if crlf > lf { Ending::CrLf } else { Ending::Lf };
        Buffer { lines, ending, final_newline }
    }

    /// The whole text, with the buffer's line ending. The empty last line
    /// a final ending makes is not written twice.
    pub fn to_text(&self) -> String {
        let end = self.ending.as_str();
        let mut out = String::with_capacity(self.bytes() + self.lines.len() * end.len());
        let n = self.lines.len();
        for (i, l) in self.lines.iter().enumerate() {
            out.push_str(l);
            if i + 1 < n {
                out.push_str(end);
            }
        }
        out
    }

    pub fn ending(&self) -> Ending {
        self.ending
    }

    pub fn set_ending(&mut self, e: Ending) {
        self.ending = e;
    }

    /// Whether the file ended with a line ending when it was read.
    pub fn final_newline(&self) -> bool {
        self.final_newline
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line(&self, i: usize) -> &str {
        self.lines.get(i).map_or("", String::as_str)
    }

    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(String::as_str)
    }

    /// The text's length in bytes, without line endings.
    pub fn bytes(&self) -> usize {
        self.lines.iter().map(String::len).sum()
    }

    /// The end of the text.
    pub fn end(&self) -> Pos {
        let last = self.lines.len() - 1;
        Pos::new(last, self.lines[last].len())
    }

    /// `p` moved onto the text: a line that exists, a character boundary.
    pub fn clamp(&self, p: Pos) -> Pos {
        let line = p.line.min(self.lines.len() - 1);
        let s = &self.lines[line];
        let mut col = p.col.min(s.len());
        while !s.is_char_boundary(col) {
            col -= 1;
        }
        Pos::new(line, col)
    }

    /// The character after `p`, if any (a line's end gives '\n').
    pub fn char_at(&self, p: Pos) -> Option<char> {
        let s = self.line(p.line);
        match s[p.col..].chars().next() {
            Some(c) => Some(c),
            None if p.line + 1 < self.lines.len() => Some('\n'),
            None => None,
        }
    }

    /// The place one character before `p` (a line's start goes to the end
    /// of the line before).
    pub fn prev_char(&self, p: Pos) -> Pos {
        if p.col == 0 {
            return if p.line == 0 { p } else { Pos::new(p.line - 1, self.lines[p.line - 1].len()) };
        }
        let s = &self.lines[p.line];
        Pos::new(p.line, s[..p.col].char_indices().next_back().map_or(0, |(j, _)| j))
    }

    /// The place one character after `p`.
    pub fn next_char(&self, p: Pos) -> Pos {
        let s = &self.lines[p.line];
        match s[p.col..].chars().next() {
            Some(c) => Pos::new(p.line, p.col + c.len_utf8()),
            None if p.line + 1 < self.lines.len() => Pos::new(p.line + 1, 0),
            None => p,
        }
    }

    /// The start of the word before `p` (Ctrl+Left): spaces, then a word.
    pub fn prev_word(&self, p: Pos) -> Pos {
        let mut q = p;
        while q != Pos::default() && self.char_at(self.prev_char(q)).is_some_and(char::is_whitespace) {
            q = self.prev_char(q);
        }
        let kind = self.char_at(self.prev_char(q)).map(word_kind);
        while q != Pos::default() && self.char_at(self.prev_char(q)).is_some_and(|c| !c.is_whitespace() && Some(word_kind(c)) == kind) {
            q = self.prev_char(q);
        }
        q
    }

    /// The end of the word after `p` (Ctrl+Right): spaces, then a word.
    pub fn next_word(&self, p: Pos) -> Pos {
        let end = self.end();
        let mut q = p;
        while q != end && self.char_at(q).is_some_and(char::is_whitespace) {
            q = self.next_char(q);
        }
        let kind = self.char_at(q).map(word_kind);
        while q != end && self.char_at(q).is_some_and(|c| !c.is_whitespace() && Some(word_kind(c)) == kind) {
            q = self.next_char(q);
        }
        q
    }

    /// The word around `p` (a double-click): letters, digits and `_`; or the
    /// run of punctuation or spaces there.
    pub fn word_at(&self, p: Pos) -> (Pos, Pos) {
        let s = &self.lines[p.line];
        let here = s[p.col..].chars().next().or_else(|| s[..p.col].chars().next_back());
        let Some(c) = here else { return (p, p) };
        let kind = |x: char| x.is_whitespace() == c.is_whitespace() && word_kind(x) == word_kind(c);
        let mut a = p.col;
        while let Some(x) = s[..a].chars().next_back().filter(|x| kind(*x)) {
            a -= x.len_utf8();
        }
        let mut b = p.col;
        while let Some(x) = s[b..].chars().next().filter(|x| kind(*x)) {
            b += x.len_utf8();
        }
        (Pos::new(p.line, a), Pos::new(p.line, b))
    }

    /// The indent (leading spaces and tabs) of line `i`.
    pub fn indent(&self, i: usize) -> &str {
        let s = self.line(i);
        &s[..s.len() - s.trim_start_matches([' ', '\t']).len()]
    }

    /// Whether the text indents with tabs: more lines start with a tab than
    /// with spaces. None if no line is indented.
    pub fn uses_tabs(&self) -> Option<bool> {
        let (mut tabs, mut spaces) = (0usize, 0usize);
        for l in self.lines.iter().take(10_000) {
            match l.as_bytes().first() {
                Some(b'\t') => tabs += 1,
                Some(b' ') if l.starts_with("  ") => spaces += 1,
                _ => {}
            }
        }
        (tabs + spaces > 0).then_some(tabs > spaces)
    }
}

/// Words, punctuation: what a word move or a double-click takes together.
fn word_kind(c: char) -> u8 {
    if c.is_alphanumeric() || c == '_' { 0 } else { 1 }
}

impl Editable for Buffer {
    type Pos = Pos;

    fn replace(&mut self, from: Pos, to: Pos, with: &str) -> Pos {
        let (from, to) = (self.clamp(from.min(to)), self.clamp(from.max(to)));
        // What stays after `to` on its line.
        let tail = self.lines[to.line][to.col..].to_string();
        let mut new = with.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l));
        let first = new.next().unwrap_or("");
        let head = &mut self.lines[from.line];
        head.truncate(from.col);
        head.push_str(first);
        let rest: Vec<String> = new.map(str::to_string).collect();
        let end = if rest.is_empty() {
            Pos::new(from.line, from.col + first.len())
        } else {
            Pos::new(from.line + rest.len(), rest.last().map_or(0, String::len))
        };
        self.lines.splice(from.line + 1..to.line + 1, rest);
        self.lines[end.line].push_str(&tail);
        end
    }

    fn slice(&self, from: Pos, to: Pos) -> String {
        let (from, to) = (self.clamp(from.min(to)), self.clamp(from.max(to)));
        if from.line == to.line {
            return self.lines[from.line][from.col..to.col].to_string();
        }
        let mut out = self.lines[from.line][from.col..].to_string();
        for l in &self.lines[from.line + 1..to.line] {
            out.push('\n');
            out.push_str(l);
        }
        out.push('\n');
        out.push_str(&self.lines[to.line][..to.col]);
        out
    }
}

/// How long a pause ends a typing step.
pub const GROUP_PAUSE: Duration = Duration::from_secs(1);
/// The most steps kept; the oldest go first.
const MAX_STEPS: usize = 10_000;

/// What kind of edit a step is, for grouping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Characters typed: a run of them is one step.
    Typing,
    /// Backspace or Delete: a run of them is one step.
    Deleting,
    /// Anything else (paste, cut, replace, indent): a step of its own.
    Other,
}

#[derive(Clone, Debug)]
struct Change<P> {
    from: P,
    /// What was there.
    removed: String,
    /// Where the removed text ended, before the change.
    removed_end: P,
    inserted: String,
    /// Where the inserted text ends, after the change.
    inserted_end: P,
}

/// A selection: the caret and the other end.
pub type Sel<P> = (P, P);

#[derive(Clone, Debug)]
struct Step<P> {
    changes: Vec<Change<P>>,
    before: Sel<P>,
    after: Sel<P>,
    kind: Kind,
    last: Instant,
}

/// Undo and redo for one text.
#[derive(Clone, Debug)]
pub struct History<P> {
    done: Vec<Step<P>>,
    undone: Vec<Step<P>>,
    /// How many steps were done when the text was last saved; None if the
    /// saved state has been lost (undone past and then edited).
    saved: Option<usize>,
    /// The next edit starts a new step.
    broken: bool,
}

impl<P> Default for History<P> {
    fn default() -> Self {
        History { done: Vec::new(), undone: Vec::new(), saved: Some(0), broken: true }
    }
}

impl<P: Copy + Eq + Ord + std::fmt::Debug> History<P> {
    /// Makes an edit on `text` and records it: replaces `from..to` with
    /// `with`. `sel` is the selection before the edit; returns where the
    /// inserted text ends (the caret after it, usually).
    pub fn edit<T: Editable<Pos = P>>(&mut self, text: &mut T, from: P, to: P, with: &str, sel: Sel<P>, kind: Kind, now: Instant) -> P {
        let (from, to) = (from.min(to), from.max(to));
        let removed = text.slice(from, to);
        if removed.is_empty() && with.is_empty() {
            return from;
        }
        let end = text.replace(from, to, with);
        let change = Change { from, removed, removed_end: to, inserted: with.to_string(), inserted_end: end };
        let after = (end, end);
        self.undone.clear();
        if self.saved.is_some_and(|s| s > self.done.len()) {
            self.saved = None;
        }
        let joins = !self.broken
            && kind != Kind::Other
            && self.done.last().is_some_and(|s| {
                s.kind == kind
                    && now.saturating_duration_since(s.last) < GROUP_PAUSE
                    && self.saved != Some(self.done.len())
                    && match kind {
                        // Typed right where the last typing ended.
                        Kind::Typing => s.after.0 == from && change.removed.is_empty(),
                        // Backspace runs back to `from`; Delete stays at it.
                        _ => s.after.0 == to || s.after.0 == from,
                    }
            });
        if joins {
            let s = self.done.last_mut().expect("joins checked a last step");
            s.changes.push(change);
            s.after = after;
            s.last = now;
        } else {
            self.done.push(Step { changes: vec![change], before: sel, after, kind, last: now });
            if self.done.len() > MAX_STEPS {
                self.done.remove(0);
                self.saved = self.saved.and_then(|s| s.checked_sub(1));
            }
        }
        self.broken = kind == Kind::Other;
        end
    }

    /// Ends the current step: the next edit starts a new one (the caret
    /// moved, the window lost the keys).
    pub fn break_group(&mut self) {
        self.broken = true;
    }

    /// Several edits as one step (Replace all, indenting lines): `f` makes
    /// them through [`History::edit`] with `Kind::Other`; they are joined.
    pub fn group<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let before = self.done.len();
        let r = f(self);
        if self.done.len() > before + 1 {
            let mut steps = self.done.split_off(before);
            let mut first = steps.remove(0);
            for s in steps {
                first.changes.extend(s.changes);
                first.after = s.after;
                first.last = s.last;
            }
            self.done.push(first);
        }
        self.broken = true;
        r
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Undoes the last step; returns the selection to put back.
    pub fn undo<T: Editable<Pos = P>>(&mut self, text: &mut T) -> Option<Sel<P>> {
        let step = self.done.pop()?;
        for c in step.changes.iter().rev() {
            text.replace(c.from, c.inserted_end, &c.removed);
        }
        let sel = step.before;
        self.undone.push(step);
        self.broken = true;
        Some(sel)
    }

    /// Does again the last step undone; returns the selection after it.
    pub fn redo<T: Editable<Pos = P>>(&mut self, text: &mut T) -> Option<Sel<P>> {
        let step = self.undone.pop()?;
        for c in &step.changes {
            text.replace(c.from, c.removed_end, &c.inserted);
        }
        let sel = step.after;
        self.done.push(step);
        self.broken = true;
        Some(sel)
    }

    /// The text is saved as it is now.
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.done.len());
        self.broken = true;
    }

    /// Whether the text differs from when it was last saved (or loaded).
    pub fn modified(&self) -> bool {
        self.saved != Some(self.done.len())
    }

    /// The text differs from the one saved, whatever was done (work kept
    /// from an earlier run and put back).
    pub fn forget_saved(&mut self) {
        self.saved = None;
    }

    /// Forgets everything (the text was replaced from outside).
    pub fn clear(&mut self) {
        *self = History::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(line: usize, col: usize) -> Pos {
        Pos::new(line, col)
    }

    #[test]
    fn lines_and_endings_are_read_and_written_back_as_found() {
        for t in ["", "a", "a\n", "a\nb", "a\nb\n", "a\r\nb\r\n", "\n\n", "x\r\ny"] {
            let b = Buffer::from_text(t);
            assert_eq!(b.to_text(), t, "{t:?}");
        }
        let b = Buffer::from_text("one\r\ntwo\r\n");
        assert_eq!(b.ending(), Ending::CrLf);
        assert_eq!(b.line_count(), 3);
        assert_eq!(b.line(1), "two");
        assert!(b.final_newline());
        assert!(!Buffer::from_text("a").final_newline());
        // A lone CR ends a line; written back with the main ending.
        assert_eq!(Buffer::from_text("a\rb\n").to_text(), "a\nb\n");
    }

    #[test]
    fn replacing_across_lines() {
        let mut b = Buffer::from_text("hello\nworld\n");
        let end = b.replace(p(0, 2), p(1, 3), "y\nthere\nwo");
        assert_eq!(b.to_text(), "hey\nthere\nwold\n");
        assert_eq!(end, p(2, 2));
        assert_eq!(b.slice(p(0, 1), p(2, 2)), "ey\nthere\nwo");
        let end = b.replace(p(0, 3), p(2, 0), " ");
        assert_eq!(b.to_text(), "hey wold\n");
        assert_eq!(end, p(0, 4));
        // CRLF pasted in comes in as lines.
        b.replace(p(0, 0), p(0, 0), "a\r\nb\r\n");
        assert_eq!(b.to_text(), "a\nb\nhey wold\n");
    }

    #[test]
    fn moving_by_character_and_word_crosses_lines() {
        let b = Buffer::from_text("ab cd\n  é_f(g)");
        assert_eq!(b.next_char(p(0, 5)), p(1, 0));
        assert_eq!(b.prev_char(p(1, 0)), p(0, 5));
        assert_eq!(b.next_char(p(1, 2)), p(1, 4), "é is two bytes");
        assert_eq!(b.next_word(p(0, 0)), p(0, 2));
        assert_eq!(b.next_word(p(0, 2)), p(0, 5));
        assert_eq!(b.next_word(p(0, 5)), p(1, 6), "over the line end and the indent");
        assert_eq!(b.next_word(p(1, 6)), p(1, 7), "punctuation is a word of its own");
        assert_eq!(b.prev_word(p(1, 2)), p(0, 3));
        assert_eq!(b.word_at(p(1, 4)), (p(1, 2), p(1, 6)));
        assert_eq!(b.indent(1), "  ");
        assert_eq!(b.clamp(p(9, 99)), p(1, 9));
        assert_eq!(b.clamp(p(1, 3)), p(1, 2), "inside é goes back to its start");
    }

    #[test]
    fn typing_groups_until_a_pause_or_a_jump() {
        let t0 = Instant::now();
        let mut b = Buffer::default();
        let mut h = History::default();
        let mut at = p(0, 0);
        for (i, c) in "hello".chars().enumerate() {
            at = h.edit(&mut b, at, at, &c.to_string(), (at, at), Kind::Typing, t0 + Duration::from_millis(i as u64 * 100));
        }
        // A pause: a new step.
        let later = t0 + Duration::from_secs(5);
        at = h.edit(&mut b, at, at, " you", (at, at), Kind::Typing, later);
        assert_eq!(b.to_text(), "hello you");
        assert_eq!(h.undo(&mut b), Some((p(0, 5), p(0, 5))));
        assert_eq!(b.to_text(), "hello");
        assert_eq!(h.undo(&mut b), Some((p(0, 0), p(0, 0))));
        assert_eq!(b.to_text(), "");
        assert_eq!(h.undo(&mut b), None);
        assert_eq!(h.redo(&mut b), Some((p(0, 5), p(0, 5))));
        assert_eq!(h.redo(&mut b), Some((p(0, 9), p(0, 9))));
        assert_eq!(b.to_text(), "hello you");
        assert_eq!(at, p(0, 9));
        // A jump breaks the group.
        let at2 = h.edit(&mut b, p(0, 9), p(0, 9), "!", (p(0, 9), p(0, 9)), Kind::Typing, later);
        h.break_group();
        h.edit(&mut b, at2, at2, "?", (at2, at2), Kind::Typing, later);
        h.undo(&mut b);
        assert_eq!(b.to_text(), "hello you!");
    }

    #[test]
    fn backspaces_group_and_undo_puts_the_text_and_caret_back() {
        let t = Instant::now();
        let mut b = Buffer::from_text("abc\ndef");
        let mut h = History::default();
        let mut at = p(1, 3);
        for _ in 0..5 {
            let from = b.prev_char(at);
            at = h.edit(&mut b, from, at, "", (at, at), Kind::Deleting, t);
        }
        assert_eq!(b.to_text(), "ab");
        // Typing after deleting is a step of its own.
        h.edit(&mut b, at, at, "X", (at, at), Kind::Typing, t);
        h.undo(&mut b);
        assert_eq!(b.to_text(), "ab");
        assert_eq!(h.undo(&mut b), Some((p(1, 3), p(1, 3))));
        assert_eq!(b.to_text(), "abc\ndef");
    }

    #[test]
    fn saved_point_and_grouped_edits() {
        let t = Instant::now();
        let mut b = Buffer::from_text("a a a");
        let mut h: History<Pos> = History::default();
        assert!(!h.modified());
        // Replace all, as one step.
        h.group(|h| {
            for col in [4, 2, 0] {
                h.edit(&mut b, p(0, col), p(0, col + 1), "bb", (p(0, 0), p(0, 0)), Kind::Other, t);
            }
        });
        assert_eq!(b.to_text(), "bb bb bb");
        assert!(h.modified());
        h.mark_saved();
        assert!(!h.modified());
        h.undo(&mut b);
        assert_eq!(b.to_text(), "a a a");
        assert!(h.modified());
        h.redo(&mut b);
        assert!(!h.modified());
        // Typing right after a save starts a new step, so undo returns to
        // the saved text.
        h.edit(&mut b, p(0, 8), p(0, 8), "x", (p(0, 8), p(0, 8)), Kind::Typing, t);
        h.undo(&mut b);
        assert!(!h.modified());
        // Undone past the save, then edited: the saved state is gone.
        h.undo(&mut b);
        h.edit(&mut b, p(0, 0), p(0, 0), "z", (p(0, 0), p(0, 0)), Kind::Other, t);
        assert!(h.modified());
        h.undo(&mut b);
        assert!(h.modified());
    }

    #[test]
    fn the_history_edits_a_plain_string_too() {
        let t = Instant::now();
        let mut s = String::from("field");
        let mut h = History::default();
        let at = h.edit(&mut s, 5, 5, "s", (5, 5), Kind::Typing, t);
        assert_eq!((s.as_str(), at), ("fields", 6));
        h.edit(&mut s, 0, 6, "x", (0, 6), Kind::Other, t);
        assert_eq!(h.undo(&mut s), Some((0, 6)));
        assert_eq!(s, "fields");
    }

    #[test]
    fn a_large_text_edits_quickly() {
        let mut text = String::new();
        for i in 0..200_000 {
            text.push_str(&format!("line {i} of a reasonably long file, with words in it\n"));
        }
        let start = Instant::now();
        let mut b = Buffer::from_text(&text);
        let read = start.elapsed();
        assert_eq!(b.line_count(), 200_001);
        let mut h = History::default();
        let t = Instant::now();
        let start = Instant::now();
        // A hundred line breaks typed at the top: each moves every line.
        for i in 0..100 {
            let at = p(i, 0);
            h.edit(&mut b, at, at, "\n", (at, at), Kind::Typing, t);
        }
        let typed = start.elapsed();
        assert_eq!(b.to_text().len(), text.len() + 100);
        // Generous bounds for debug builds on the slowest box.
        assert!(read < Duration::from_secs(3), "reading took {read:?}");
        assert!(typed < Duration::from_secs(3), "typing took {typed:?}");
    }
}
