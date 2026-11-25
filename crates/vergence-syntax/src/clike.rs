//! The C family's code, shared by JavaScript and PHP (and later languages like them): line and
//! block comments, strings with escapes (PHP's `$variables` inside double quotes and heredocs,
//! JavaScript's template literals with `${expressions}`), regular expression literals, numbers,
//! keywords, constants, types, builtins, and the names `function` and `class` introduce.
//!
//! One engine with switches ([`Rules`]) rather than one module a language: what differs is a
//! list of words and a few habits. It scans a range of a line (so HTML can hand it a
//! `<script>`'s inside, and PHP the part between `<?php` and `?>`).
//!
//! State words (inside the caller's own): `[]` code; `[BLOCK, doc]` a block comment;
//! `[STR, quote]` a string open across lines; `[HEREDOC, interpolate, label bytes...]`.

use crate::{Kind, Span, char_len, push};

pub(crate) struct Rules {
    pub keywords: &'static [&'static str],
    pub constants: &'static [&'static str],
    pub types: &'static [&'static str],
    pub builtins: &'static [&'static str],
    /// PHP: `TRUE`, `Function`, `NULL` are the words too.
    pub any_case: bool,
    /// PHP: `#` starts a comment (`#[` an attribute).
    pub hash_comments: bool,
    /// PHP: `$name` is a variable.
    pub dollar_vars: bool,
    /// PHP: strings run on across lines; JavaScript's `'` and `"` do not.
    pub multiline_quotes: bool,
    /// JavaScript: `` `template ${x}` ``.
    pub templates: bool,
    /// JavaScript: `/regex/flags` where a value is expected.
    pub regex: bool,
    /// PHP: `"... $var ..."` and heredocs interpolate.
    pub interpolate: bool,
    /// PHP: `<<<LABEL` ... `LABEL`.
    pub heredoc: bool,
    /// PHP: `?>` ends the code (even in a line comment).
    pub php_close: bool,
}

const BLOCK: u32 = 1;
const STR: u32 = 2;
const HEREDOC: u32 = 3;

/// Words after which a `/` starts a regular expression, not a division.
const BEFORE_REGEX: &[&str] =
    &["return", "typeof", "case", "in", "of", "new", "delete", "void", "throw", "yield", "await"];

fn is_word_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

impl Rules {
    fn has(&self, list: &[&str], w: &str) -> bool {
        if self.any_case { list.iter().any(|k| k.eq_ignore_ascii_case(w)) } else { list.contains(&w) }
    }

    /// The words, for completion.
    pub fn words(&self) -> Vec<String> {
        self.keywords
            .iter()
            .chain(self.constants)
            .chain(self.types)
            .chain(self.builtins)
            .map(|s| s.to_string())
            .collect()
    }

    /// The code in `line[from..to]`, from and to `state`. Returns where `?>`
    /// starts when PHP's code ends there.
    pub fn run(&self, state: &mut Vec<u32>, line: &str, from: usize, to: usize, out: &mut Vec<Span>) -> Option<usize> {
        let b = line.as_bytes();
        let mut i = from;
        // Finish what the line before left open.
        match state.first().copied() {
            Some(BLOCK) => {
                let doc = state.get(1) == Some(&1);
                let kind = if doc { Kind::DocComment } else { Kind::Comment };
                match line[i..to].find("*/") {
                    Some(k) => {
                        push(out, i, i + k + 2, kind);
                        i += k + 2;
                        state.clear();
                    }
                    None => {
                        push(out, i, to, kind);
                        return None;
                    }
                }
            }
            Some(STR) => {
                let q = state.get(1).copied().unwrap_or(b'"' as u32) as u8;
                let (end, open) = self.string(line, i, i, to, q, out);
                if open {
                    return None;
                }
                state.clear();
                i = end;
            }
            Some(HEREDOC) => {
                let interp = state.get(1) == Some(&1);
                let label: Vec<u8> = state[2..].iter().map(|c| *c as u8).collect();
                let body = line[i..to].trim_start();
                if body.as_bytes().starts_with(&label) && !body.as_bytes().get(label.len()).is_some_and(|c| is_word(*c))
                {
                    let start = to - line[i..to].trim_start().len();
                    push(out, start, start + label.len(), Kind::String);
                    i = start + label.len();
                    state.clear();
                } else {
                    if interp {
                        self.interpolated(line, i, to, out);
                    } else {
                        push(out, i, to, Kind::String);
                    }
                    return None;
                }
            }
            _ => state.clear(),
        }
        let mut after: Option<String> = None;
        let mut value_next = true; // a `/` here would start a regular expression
        while i < to {
            let c = b[i];
            if self.php_close && line[i..to].starts_with("?>") {
                return Some(i);
            }
            let line_comment =
                line[i..to].starts_with("//") || (self.hash_comments && c == b'#' && b.get(i + 1) != Some(&b'['));
            if line_comment {
                // PHP: `?>` ends a line comment, and the code.
                let end = if self.php_close { line[i..to].find("?>").map(|k| i + k) } else { None };
                push(out, i, end.unwrap_or(to), Kind::Comment);
                return end;
            }
            if self.hash_comments && line[i..to].starts_with("#[") {
                let end = line[i..to].find(']').map_or(to, |k| i + k + 1);
                push(out, i, end, Kind::Meta);
                i = end;
                continue;
            }
            if line[i..to].starts_with("/*") {
                let doc = line[i..to].starts_with("/**") && !line[i..to].starts_with("/**/");
                let kind = if doc { Kind::DocComment } else { Kind::Comment };
                match line[i + 2..to].find("*/") {
                    Some(k) => {
                        push(out, i, i + 2 + k + 2, kind);
                        i += 2 + k + 2;
                    }
                    None => {
                        push(out, i, to, kind);
                        *state = vec![BLOCK, doc as u32];
                        return None;
                    }
                }
                continue;
            }
            if self.heredoc && line[i..to].starts_with("<<<") {
                let mut j = i + 3;
                while j < to && b[j] == b' ' {
                    j += 1;
                }
                let quoted = j < to && (b[j] == b'\'' || b[j] == b'"');
                let nowdoc = quoted && b[j] == b'\'';
                let ls = j + quoted as usize;
                let mut le = ls;
                while le < to && is_word(b[le]) {
                    le += 1;
                }
                if le > ls {
                    let end = (le + quoted as usize).min(to);
                    push(out, i, end, Kind::String);
                    let mut st = vec![HEREDOC, (!nowdoc) as u32];
                    st.extend(line[ls..le].bytes().map(u32::from));
                    *state = st;
                    i = end;
                    if !line[i..to].trim().is_empty() {
                        continue;
                    }
                    return None;
                }
            }
            if c == b'\'' || c == b'"' || (self.templates && c == b'`') {
                let (end, open) = self.string(line, i, i + 1, to, c, out);
                if open {
                    *state = vec![STR, c as u32];
                    return None;
                }
                i = end;
                after = None;
                value_next = false;
                continue;
            }
            if self.regex
                && c == b'/'
                && value_next
                && let Some(end) = regex_end(b, i + 1, to)
            {
                push(out, i, end, Kind::String);
                i = end;
                value_next = false;
                continue;
            }
            if self.dollar_vars && c == b'$' && b.get(i + 1).is_some_and(|x| is_word_start(*x)) {
                let mut j = i + 1;
                while j < to && is_word(b[j]) {
                    j += char_len(b[j]);
                }
                push(out, i, j.min(to), Kind::Variable);
                i = j.min(to);
                after = None;
                value_next = false;
                continue;
            }
            if is_word_start(c) || (c == b'$' && !self.dollar_vars) {
                let start = i;
                i += char_len(c);
                while i < to && (is_word(b[i]) || (b[i] == b'$' && !self.dollar_vars)) {
                    i += char_len(b[i]);
                }
                let i_end = i.min(to);
                let word = &line[start..i_end];
                let before = line[from..start].trim_end();
                let member = before.ends_with('.') || before.ends_with("->") || before.ends_with("?.");
                let kind = match after.as_deref() {
                    Some("function") | Some("fn") => Some(Kind::Function),
                    Some("class" | "interface" | "trait" | "enum" | "extends" | "implements" | "new") => {
                        Some(Kind::Type)
                    }
                    _ if member => None,
                    _ if self.has(self.keywords, word) => Some(Kind::Keyword),
                    _ if self.has(self.constants, word) => Some(Kind::Constant),
                    _ if self.has(self.types, word) => Some(Kind::Type),
                    _ if self.has(self.builtins, word) => Some(Kind::Builtin),
                    _ => None,
                };
                if let Some(k) = kind {
                    push(out, start, i_end, k);
                }
                let lw = if self.any_case { word.to_ascii_lowercase() } else { word.to_string() };
                value_next = BEFORE_REGEX.contains(&lw.as_str());
                after = if matches!(
                    lw.as_str(),
                    "function" | "fn" | "class" | "interface" | "trait" | "enum" | "extends" | "implements" | "new"
                ) {
                    Some(lw)
                } else {
                    None
                };
                continue;
            }
            if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
                let start = i;
                let hex = c == b'0' && b.get(i + 1).is_some_and(|x| (x | 0x20) == b'x');
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
                value_next = false;
                after = None;
                continue;
            }
            if b"=+-*/%<>!&|^~?:".contains(&c) {
                push(out, i, i + 1, Kind::Operator);
            }
            if !c.is_ascii_whitespace() {
                value_next = !matches!(c, b')' | b']' | b'}');
                if c != b'.' {
                    after = None;
                }
            }
            i += char_len(c);
        }
        None
    }

    /// A string from `i` (its span from `seg`) to its quote `q` or `to`.
    /// Returns where it ended and whether it is still open.
    fn string(&self, line: &str, mut seg: usize, mut i: usize, to: usize, q: u8, out: &mut Vec<Span>) -> (usize, bool) {
        let b = line.as_bytes();
        let interp = self.interpolate && q == b'"';
        let template = self.templates && q == b'`';
        while i < to {
            let c = b[i];
            if c == b'\\' {
                if i + 1 >= to {
                    push(out, seg, to, Kind::String);
                    return (to, true);
                }
                push(out, seg, i, Kind::String);
                let n = 1 + char_len(b[i + 1]);
                push(out, i, (i + n).min(to), Kind::Escape);
                i = (i + n).min(to);
                seg = i;
                continue;
            }
            if c == q {
                push(out, seg, i + 1, Kind::String);
                return (i + 1, false);
            }
            if interp && c == b'$' && b.get(i + 1).is_some_and(|x| is_word_start(*x) || *x == b'{') {
                push(out, seg, i, Kind::String);
                let end = interpolation_end(b, i, to);
                push(out, i, end, Kind::Variable);
                i = end;
                seg = i;
                continue;
            }
            if interp && c == b'{' && b.get(i + 1) == Some(&b'$') {
                push(out, seg, i, Kind::String);
                let end = line[i..to].find('}').map_or(to, |k| i + k + 1);
                push(out, i, end, Kind::Variable);
                i = end;
                seg = i;
                continue;
            }
            if template && line[i..to].starts_with("${") {
                push(out, seg, i, Kind::String);
                let close = brace_end(b, i + 2, to);
                push(out, i, i + 2, Kind::Operator);
                let mut inner = Vec::new();
                self.run(&mut inner, line, i + 2, close, out);
                if close < to {
                    push(out, close, close + 1, Kind::Operator);
                }
                i = (close + 1).min(to);
                seg = i;
                continue;
            }
            i += char_len(c);
        }
        push(out, seg, to, Kind::String);
        let open = template || self.multiline_quotes;
        (to, open)
    }

    /// A heredoc's line with its `$variables` marked.
    fn interpolated(&self, line: &str, from: usize, to: usize, out: &mut Vec<Span>) {
        let b = line.as_bytes();
        let mut seg = from;
        let mut i = from;
        while i < to {
            if b[i] == b'$' && b.get(i + 1).is_some_and(|x| is_word_start(*x) || *x == b'{') {
                push(out, seg, i, Kind::String);
                let end = interpolation_end(b, i, to);
                push(out, i, end, Kind::Variable);
                i = end;
                seg = i;
                continue;
            }
            i += char_len(b[i]);
        }
        push(out, seg, to, Kind::String);
    }
}

/// `$name`, `$name->prop`, `$name[key]`, `${expr}` in a PHP string: its end.
fn interpolation_end(b: &[u8], i: usize, to: usize) -> usize {
    let mut j = i + 1;
    if b.get(j) == Some(&b'{') {
        while j < to && b[j] != b'}' {
            j += 1;
        }
        return (j + 1).min(to);
    }
    while j < to && is_word(b[j]) {
        j += char_len(b[j]);
    }
    if j + 2 < to && b[j] == b'-' && b[j + 1] == b'>' && is_word_start(b[j + 2]) {
        j += 2;
        while j < to && is_word(b[j]) {
            j += char_len(b[j]);
        }
    } else if j < to
        && b[j] == b'['
        && let Some(k) = b[j..to].iter().position(|c| *c == b']')
    {
        j += k + 1;
    }
    j.min(to)
}

/// The `}` closing a `${` (nested braces counted), or `to`.
fn brace_end(b: &[u8], from: usize, to: usize) -> usize {
    let mut depth = 0usize;
    let mut i = from;
    while i < to {
        match b[i] {
            b'{' => depth += 1,
            b'}' if depth == 0 => return i,
            b'}' => depth -= 1,
            _ => {}
        }
        i += char_len(b[i]);
    }
    to
}

/// A regular expression literal's end (after its flags), from just after
/// its opening `/`; None if the line ends first (then it was a division).
fn regex_end(b: &[u8], from: usize, to: usize) -> Option<usize> {
    let mut i = from;
    let mut class = false;
    if i < to && (b[i] == b'/' || b[i] == b'*' || b[i] == b' ') {
        return None;
    }
    while i < to {
        match b[i] {
            b'\\' => i += 1,
            b'[' => class = true,
            b']' => class = false,
            b'/' if !class => {
                i += 1;
                while i < to && b[i].is_ascii_alphabetic() {
                    i += 1;
                }
                return Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}
