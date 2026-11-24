//! Python: keywords, builtins, strings of every prefix (triple-quoted ones across lines,
//! f-strings with their expressions), numbers, comments, decorators, and the names `def` and
//! `class` define.
//!
//! The state between lines is an open string: `[OPEN, flags]` (its quote, raw, f-string,
//! triple, docstring), from a triple quote not yet closed, or a one-quote string whose line
//! ends in a backslash. Everything else ends with its line.

use crate::{Highlight, Kind, Span, State, push};

pub struct Python;

const KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except",
    "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise",
    "return", "try", "while", "with", "yield",
];
/// Keywords only where a statement starts (`match x:`; `match = 1` is a name).
const SOFT: &[&str] = &["match", "case", "type"];
const CONSTANTS: &[&str] = &["True", "False", "None", "Ellipsis", "NotImplemented", "__debug__"];
const BUILTINS: &[&str] = &[
    "self",
    "cls",
    "abs",
    "aiter",
    "all",
    "anext",
    "any",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "help",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "vars",
    "zip",
    "__import__",
    "__name__",
    "__file__",
    "__doc__",
    "__init__",
    "__main__",
    "BaseException",
    "Exception",
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "BlockingIOError",
    "BrokenPipeError",
    "BufferError",
    "ConnectionError",
    "EOFError",
    "FileExistsError",
    "FileNotFoundError",
    "FloatingPointError",
    "GeneratorExit",
    "ImportError",
    "IndentationError",
    "IndexError",
    "InterruptedError",
    "IsADirectoryError",
    "KeyError",
    "KeyboardInterrupt",
    "LookupError",
    "MemoryError",
    "ModuleNotFoundError",
    "NameError",
    "NotADirectoryError",
    "NotImplementedError",
    "OSError",
    "OverflowError",
    "PermissionError",
    "ProcessLookupError",
    "RecursionError",
    "ReferenceError",
    "RuntimeError",
    "StopAsyncIteration",
    "StopIteration",
    "SyntaxError",
    "SystemError",
    "SystemExit",
    "TabError",
    "TimeoutError",
    "TypeError",
    "UnboundLocalError",
    "UnicodeDecodeError",
    "UnicodeEncodeError",
    "UnicodeError",
    "ValueError",
    "ZeroDivisionError",
    "Warning",
    "DeprecationWarning",
    "UserWarning",
    "RuntimeWarning",
];

/// The language's words, for completion.
pub fn words() -> Vec<String> {
    KEYWORDS.iter().chain(SOFT).chain(CONSTANTS).chain(BUILTINS).map(|s| s.to_string()).collect()
}

const OPEN: u32 = 1;
const DOUBLE: u32 = 1;
const RAW: u32 = 2;
const FSTR: u32 = 4;
const TRIPLE: u32 = 8;
const DOC: u32 = 16;

/// A string's quote and form.
#[derive(Clone, Copy)]
struct Str {
    quote: u8,
    raw: bool,
    f: bool,
    triple: bool,
    kind: Kind,
}

impl Str {
    fn flags(self) -> u32 {
        let mut f = 0;
        if self.quote == b'"' {
            f |= DOUBLE;
        }
        if self.raw {
            f |= RAW;
        }
        if self.f {
            f |= FSTR;
        }
        if self.triple {
            f |= TRIPLE;
        }
        if self.kind == Kind::DocComment {
            f |= DOC;
        }
        f
    }

    fn from_flags(f: u32) -> Str {
        Str {
            quote: if f & DOUBLE != 0 { b'"' } else { b'\'' },
            raw: f & RAW != 0,
            f: f & FSTR != 0,
            triple: f & TRIPLE != 0,
            kind: if f & DOC != 0 { Kind::DocComment } else { Kind::String },
        }
    }
}

/// The byte length of the character starting with byte `c`.
fn char_len(c: u8) -> usize {
    match c {
        0..0x80 => 1,
        0xc0..0xe0 => 2,
        0xe0..0xf0 => 3,
        0xf0.. => 4,
        _ => 1, // a continuation byte: never a start in valid UTF-8
    }
}

fn is_word_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

/// How a string ended on this line.
enum End {
    Closed,
    /// Still open at the line's end (a triple quote, or a backslash).
    Open,
    /// A one-quote string the line ended in the middle of: an error, closed.
    Broken,
}

impl Highlight for Python {
    fn line(&self, state: &mut State, line: &str, out: &mut Vec<Span>) {
        let mut i = 0;
        if state.0.first() == Some(&OPEN) {
            let s = Str::from_flags(state.0.get(1).copied().unwrap_or(0));
            let (at, end) = string(line, 0, 0, line.len(), s, out);
            match end {
                End::Open => return,
                End::Closed | End::Broken => state.0.clear(),
            }
            i = at;
        }
        state.0.clear();
        if let Some(open) = tokens(line, i, line.len(), out) {
            state.0 = vec![OPEN, open.flags()];
        }
    }
}

/// The spans of code in `line[from..to]`. Returns the string left open at
/// the end, if any.
fn tokens(line: &str, from: usize, to: usize, out: &mut Vec<Span>) -> Option<Str> {
    let b = line.as_bytes();
    let indent = b.iter().position(|c| *c != b' ' && *c != b'\t').unwrap_or(b.len());
    let mut i = from;
    // The keyword before the current word, for the names `def` and `class` define.
    let mut after: Option<&str> = None;
    while i < to {
        let c = b[i];
        if c == b'#' {
            push(out, i, to, Kind::Comment);
            break;
        }
        if c == b'\'' || c == b'"' {
            // A string starting a statement is a docstring (a module's, a
            // class's, a function's): where documentation goes.
            let doc = i == indent;
            if let Some(open) = start_string(line, i, i, to, false, false, doc, out, &mut i) {
                return Some(open);
            }
            after = None;
            continue;
        }
        if is_word_start(c) {
            let start = i;
            while i < to && is_word(b[i]) {
                i += char_len(b[i]);
            }
            let i_end = i.min(to);
            let word = &line[start..i_end];
            // A prefix and a quote: a string (r"..", f'..', Rb"""..""").
            if i_end < to && (b[i_end] == b'\'' || b[i_end] == b'"') && word.len() <= 2 {
                let lw = word.to_ascii_lowercase();
                if matches!(lw.as_str(), "r" | "u" | "b" | "f" | "rb" | "br" | "fr" | "rf") {
                    let raw = lw.contains('r');
                    let f = lw.contains('f');
                    let doc = start == indent;
                    if let Some(open) = start_string(line, start, i_end, to, raw, f, doc, out, &mut i) {
                        return Some(open);
                    }
                    after = None;
                    continue;
                }
            }
            let attr = start > 0 && line[..start].trim_end().ends_with('.');
            let kind = match after {
                Some("def") => Some(Kind::Function),
                Some("class") => Some(Kind::Type),
                _ if attr => None,
                _ if KEYWORDS.contains(&word) => Some(Kind::Keyword),
                _ if CONSTANTS.contains(&word) => Some(Kind::Constant),
                _ if SOFT.contains(&word) && start == indent && soft_keyword(&line[i_end..to]) => Some(Kind::Keyword),
                _ if BUILTINS.contains(&word) => Some(Kind::Builtin),
                _ => None,
            };
            if let Some(k) = kind {
                push(out, start, i_end, k);
            }
            after = if word == "def" || word == "class" { Some(word) } else { None };
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && i + 1 < to && b[i + 1].is_ascii_digit()) {
            let start = i;
            let hex = c == b'0' && i + 1 < to && (b[i + 1] | 0x20) == b'x';
            while i < to {
                let d = b[i];
                let sign = (d == b'+' || d == b'-') && !hex && i > start && (b[i - 1] | 0x20) == b'e';
                if d.is_ascii_alphanumeric() || d == b'_' || d == b'.' || sign {
                    i += 1;
                } else {
                    break;
                }
            }
            push(out, start, i, Kind::Number);
            after = None;
            continue;
        }
        if c == b'@' && i == indent {
            // A decorator: @name, @module.name
            let start = i;
            i += 1;
            while i < to && (is_word(b[i]) || b[i] == b'.') {
                i += char_len(b[i]);
            }
            push(out, start, i.min(to), Kind::Meta);
            continue;
        }
        if b"=+-*/%<>!&|^~@".contains(&c) {
            push(out, i, i + 1, Kind::Operator);
        }
        if !c.is_ascii_whitespace() {
            after = None;
        }
        i += char_len(c);
    }
    None
}

/// `match`, `case` and `type` are keywords when what follows is not an
/// assignment, a call or an attribute (`match x:`, `case [a, b]:`, not
/// `match = 1`).
fn soft_keyword(rest: &str) -> bool {
    let r = rest.trim_start();
    !rest.is_empty() && rest.starts_with([' ', '\t']) && !r.is_empty() && !r.starts_with(['=', '.', '(', ',', ')', ':'])
}

/// A string whose prefix starts at `seg` and quote at `q`. Leaves `i` after
/// it; returns the string if it stays open past the line.
#[allow(clippy::too_many_arguments)]
fn start_string(
    line: &str,
    seg: usize,
    q: usize,
    to: usize,
    raw: bool,
    f: bool,
    doc: bool,
    out: &mut Vec<Span>,
    i: &mut usize,
) -> Option<Str> {
    let b = line.as_bytes();
    let quote = b[q];
    let triple = q + 2 < to && b[q + 1] == quote && b[q + 2] == quote;
    let kind = if doc && triple { Kind::DocComment } else { Kind::String };
    let s = Str { quote, raw, f, triple, kind };
    let content = q + if triple { 3 } else { 1 };
    let (at, end) = string(line, seg, content, to, s, out);
    *i = at;
    match end {
        End::Open => Some(s),
        End::Closed | End::Broken => None,
    }
}

/// A string's content from `i` (its span starting at `seg`) to its close or
/// `to`: escapes as their own spans, an f-string's `{expression}` as code.
fn string(line: &str, mut seg: usize, mut i: usize, to: usize, s: Str, out: &mut Vec<Span>) -> (usize, End) {
    let b = line.as_bytes();
    while i < to {
        let c = b[i];
        if c == b'\\' {
            if i + 1 >= to {
                // A backslash at the end: the string goes on to the next line.
                if s.raw {
                    push(out, seg, to, s.kind);
                } else {
                    push(out, seg, i, s.kind);
                    push(out, i, to, Kind::Escape);
                }
                return (to, End::Open);
            }
            let n = 1 + char_len(b[i + 1]);
            if !s.raw {
                push(out, seg, i, s.kind);
                push(out, i, (i + n).min(to), Kind::Escape);
                seg = (i + n).min(to);
            }
            // In a raw string the backslash still keeps the quote after it in.
            i = (i + n).min(to);
            continue;
        }
        if s.f && (c == b'{' || c == b'}') {
            if i + 1 < to && b[i + 1] == c {
                push(out, seg, i, s.kind);
                push(out, i, i + 2, Kind::Escape);
                i += 2;
                seg = i;
                continue;
            }
            if c == b'{' {
                push(out, seg, i, s.kind);
                let close = expression_end(b, i + 1, to, s.quote);
                push(out, i, i + 1, Kind::Operator);
                tokens(line, i + 1, close, out);
                if close < to {
                    push(out, close, close + 1, Kind::Operator);
                }
                i = (close + 1).min(to);
                seg = i;
                continue;
            }
        }
        if c == s.quote {
            if !s.triple {
                push(out, seg, i + 1, s.kind);
                return (i + 1, End::Closed);
            }
            if i + 2 < to && b[i + 1] == s.quote && b[i + 2] == s.quote {
                push(out, seg, i + 3, s.kind);
                return (i + 3, End::Closed);
            }
        }
        i += char_len(c);
    }
    push(out, seg, to.min(b.len()), s.kind);
    (to, if s.triple { End::Open } else { End::Broken })
}

/// Where an f-string's `{expression}` that starts at `from` ends: its `}`
/// at depth 0 (nested braces counted, other strings skipped), or the
/// string's own quote, or the line's end.
fn expression_end(b: &[u8], from: usize, to: usize, quote: u8) -> usize {
    let mut depth = 0usize;
    let mut i = from;
    let mut inner: Option<u8> = None;
    while i < to {
        let c = b[i];
        match inner {
            Some(q) if c == q => inner = None,
            Some(_) => {}
            None if c == quote => return i,
            None if c == b'\'' || c == b'"' => inner = Some(c),
            None if c == b'{' || c == b'[' || c == b'(' => depth += 1,
            None if (c == b'}' || c == b']' || c == b')') && depth > 0 => depth -= 1,
            None if c == b'}' => return i,
            None => {}
        }
        i += char_len(c);
    }
    to
}

#[cfg(test)]
mod tests {
    use crate::{Kind, State, by_name, highlight};

    /// The line's spans as (text, kind) pairs.
    fn spans(state: &mut State, line: &str) -> Vec<(String, Kind)> {
        let py = by_name("python").unwrap();
        highlight(py, state, line).into_iter().map(|s| (line[s.start..s.end].to_string(), s.kind)).collect()
    }

    fn one(line: &str) -> Vec<(String, Kind)> {
        spans(&mut State::default(), line)
    }

    fn has(v: &[(String, Kind)], text: &str, kind: Kind) -> bool {
        v.iter().any(|(t, k)| t == text && *k == kind)
    }

    #[test]
    fn keywords_builtins_names_and_comments() {
        let v = one("def area(r):  # the circle's");
        assert!(has(&v, "def", Kind::Keyword));
        assert!(has(&v, "area", Kind::Function));
        assert!(has(&v, "# the circle's", Kind::Comment));
        let v = one("class Shape(object):");
        assert!(has(&v, "Shape", Kind::Type));
        assert!(has(&v, "object", Kind::Builtin));
        let v = one("    if x is None and not isinstance(y, list):");
        for k in ["if", "is", "and", "not"] {
            assert!(has(&v, k, Kind::Keyword), "{k}");
        }
        assert!(has(&v, "None", Kind::Constant));
        assert!(has(&v, "isinstance", Kind::Builtin));
        assert!(!has(&one("obj.list = 1"), "list", Kind::Builtin), "an attribute is not the builtin");
        assert!(!has(&one("definitely = 1"), "def", Kind::Keyword), "whole words only");
    }

    #[test]
    fn soft_keywords_only_as_statements() {
        assert!(has(&one("match command:"), "match", Kind::Keyword));
        assert!(has(&one("    case [x, y]:"), "case", Kind::Keyword));
        assert!(!has(&one("match = re.match(p, s)"), "match", Kind::Keyword));
        assert!(!has(&one("x = match"), "match", Kind::Keyword));
    }

    #[test]
    fn strings_with_prefixes_and_escapes() {
        let v = one(r#"s = "a\tb" + r'\d+' + b"x""#);
        assert!(has(&v, "\"a", Kind::String));
        assert!(has(&v, "\\t", Kind::Escape));
        assert!(has(&v, r"r'\d+'", Kind::String), "raw: the backslash is plain string");
        assert!(has(&v, "b\"x\"", Kind::String));
        assert!(has(&one(r#"x = 'it\'s'"#), r"\'", Kind::Escape), "an escaped quote does not end it");
        assert!(has(&one(r##"x = "# not a comment""##), "\"# not a comment\"", Kind::String));
    }

    #[test]
    fn f_strings_color_their_expressions() {
        let v = one(r#"msg = f"{name!r} has {len(items)} {{braces}}""#);
        assert!(has(&v, "len", Kind::Builtin), "{v:?}");
        assert!(has(&v, "{{", Kind::Escape));
        assert!(has(&v, "{", Kind::Operator));
        assert!(has(&v, "f\"", Kind::String));
    }

    #[test]
    fn triple_quoted_strings_span_lines() {
        let mut st = State::default();
        let v = spans(&mut st, "    \"\"\"Return the area.");
        assert!(has(&v, "\"\"\"Return the area.", Kind::DocComment), "a docstring: {v:?}");
        assert_ne!(st, State::default(), "still open");
        let v = spans(&mut st, "    if def is in here, it is text");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].1, Kind::DocComment);
        let v = spans(&mut st, "    \"\"\" + len(x)");
        assert_eq!(st, State::default(), "closed");
        assert!(has(&v, "len", Kind::Builtin));
        // Assigned, a triple-quoted string is a string, not documentation.
        let v = one("text = '''one line'''");
        assert!(has(&v, "'''one line'''", Kind::String));
    }

    #[test]
    fn a_backslash_carries_a_one_quote_string_on() {
        let mut st = State::default();
        spans(&mut st, "s = 'first \\");
        assert_ne!(st, State::default());
        let v = spans(&mut st, "second' + 1");
        assert!(has(&v, "second'", Kind::String));
        assert!(has(&v, "1", Kind::Number));
        assert_eq!(st, State::default());
        // Without the backslash a broken string ends with its line.
        let mut st = State::default();
        spans(&mut st, "s = 'never closed");
        assert_eq!(st, State::default());
    }

    #[test]
    fn numbers_and_decorators() {
        let v = one("@functools.lru_cache(maxsize=128)");
        assert!(has(&v, "@functools.lru_cache", Kind::Meta));
        assert!(has(&v, "128", Kind::Number));
        for n in ["0x1F", "1_000", "3.14", "1e-9", "2j", ".5", "0b1010"] {
            assert!(has(&one(&format!("x = {n}")), n, Kind::Number), "{n}");
        }
        assert!(!one("x2 = y3").iter().any(|(_, k)| *k == Kind::Number), "digits in names");
    }

    #[test]
    fn spans_are_in_order_and_inside_the_line() {
        let line = "for i, ch in enumerate(f'{x!s:>{w}}é'):  # ok ünïcödé";
        let py = by_name("python").unwrap();
        let s = highlight(py, &mut State::default(), line);
        for w in s.windows(2) {
            assert!(w[0].end <= w[1].start, "{s:?}");
        }
        for sp in &s {
            assert!(sp.start < sp.end && sp.end <= line.len());
            assert!(line.is_char_boundary(sp.start) && line.is_char_boundary(sp.end));
        }
    }
}
