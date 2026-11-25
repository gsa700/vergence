//! Languages described in files (docs/syntax-files.md), not code: one TOML file a language,
//! read by one rule-driven highlighter. Shipped in `$prefix/share/vergence/syntax/` (the
//! checkout's `data/syntax/`); a person's own in `~/.config/vergence/syntax/`, which replace a
//! language of the same name. Dave, 2026-11-24: "build so they can be added".
//!
//! A file is untrusted: read with limits (its size, the lists' lengths), and a file that is
//! not right is skipped with a line on standard error saying why.
//!
//! The state between lines: `[]`, or `[COMMENT, which, depth]` (a block comment open, nested
//! ones counted where the language nests them), or `[STRING, which]`.

use std::path::{Path, PathBuf};

use crate::{Highlight, Kind, Language, Span, State, char_len, push};

/// The largest definition file read, the most files, the most words a list.
const MAX_FILE: u64 = 256 << 10;
const MAX_FILES: usize = 200;
const MAX_WORDS: usize = 5_000;
const MAX_MARK: usize = 16;

const COMMENT: u32 = 1;
const STRING: u32 = 2;

#[derive(Debug, Clone)]
struct Quote {
    open: String,
    close: String,
    /// The escape character ("" for none).
    escape: Option<char>,
    multiline: bool,
    /// `$name` and `${...}` inside are variables (shell's double quotes).
    variables: bool,
    /// Not a string unless it closes within this many characters (Rust's
    /// `'x'` against its lifetimes `'a`); 0 for no limit.
    max_chars: usize,
}

/// A language read from a file.
#[derive(Debug, Default)]
pub struct Rules {
    keywords: Vec<String>,
    types: Vec<String>,
    builtins: Vec<String>,
    constants: Vec<String>,
    any_case: bool,
    line_comments: Vec<String>,
    block_comments: Vec<(String, String)>,
    nested_comments: bool,
    doc_comments: Vec<String>,
    quotes: Vec<Quote>,
    variables: bool,
    numbers: bool,
    operators: String,
    function_after: Vec<String>,
    type_after: Vec<String>,
    attributes: Vec<(String, String)>,
    /// A line starting with this is a preprocessor line, Meta (C's `#`).
    meta_lines: Vec<String>,
    /// At a line's start, a name before this is a key (TOML's and INI's
    /// `=`, YAML's `:`).
    keys: String,
    /// At a line's start, a header between these (`[section]`).
    sections: Option<(String, String)>,
    /// A string followed by this is a key (JSON's `"name":`).
    string_keys: String,
}

fn is_word_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

/// Where definition files are read from, shipped ones first, the person's last.
pub fn dirs() -> Vec<PathBuf> {
    let mut d: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin) = exe.parent()
    {
        d.push(bin.join("../share/vergence/syntax"));
        d.push(bin.join("../../data/syntax"));
    }
    #[cfg(test)]
    d.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/syntax"));
    d.push("/usr/local/share/vergence/syntax".into());
    d.push("/usr/share/vergence/syntax".into());
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    if let Some(c) = config {
        d.push(c.join("vergence/syntax"));
    }
    d
}

/// Every language in the files under `dirs`, later ones replacing earlier
/// ones of the same name (so a person's own replace the shipped).
pub fn load(dirs: &[PathBuf]) -> Vec<Language> {
    let mut out: Vec<Language> = Vec::new();
    let mut files = 0;
    let mut seen: Vec<PathBuf> = Vec::new();
    for d in dirs {
        let Ok(canon) = d.canonicalize() else { continue };
        if seen.contains(&canon) {
            continue;
        }
        seen.push(canon);
        let Ok(read) = std::fs::read_dir(d) else { continue };
        let mut paths: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for p in paths.into_iter().filter(|p| p.extension().is_some_and(|e| e == "toml")) {
            files += 1;
            if files > MAX_FILES {
                return out;
            }
            match read_file(&p) {
                Ok(l) => match out.iter().position(|o| o.name.eq_ignore_ascii_case(&l.name)) {
                    Some(i) => out[i] = l,
                    None => out.push(l),
                },
                Err(e) => eprintln!("vergence-syntax: {}: {e} (skipped)", p.display()),
            }
        }
    }
    out
}

fn read_file(p: &Path) -> Result<Language, String> {
    let len = std::fs::metadata(p).map_err(|e| e.to_string())?.len();
    if len > MAX_FILE {
        return Err(format!("larger than {} KB", MAX_FILE >> 10));
    }
    let text = std::fs::read_to_string(p).map_err(|e| e.to_string())?;
    parse(&text)
}

fn list(t: &toml::Table, key: &str) -> Result<Vec<String>, String> {
    match t.get(key) {
        None => Ok(Vec::new()),
        Some(toml::Value::Array(a)) if a.len() <= MAX_WORDS => a
            .iter()
            .map(|v| v.as_str().map(str::to_string).ok_or_else(|| format!("{key}: a list of strings")))
            .collect(),
        Some(_) => Err(format!("{key}: a list of at most {MAX_WORDS} strings")),
    }
}

fn text(t: &toml::Table, key: &str) -> Result<String, String> {
    match t.get(key) {
        None => Ok(String::new()),
        Some(toml::Value::String(s)) if s.len() <= 256 => Ok(s.clone()),
        Some(_) => Err(format!("{key}: a short string")),
    }
}

fn flag(t: &toml::Table, key: &str, default: bool) -> Result<bool, String> {
    match t.get(key) {
        None => Ok(default),
        Some(toml::Value::Boolean(b)) => Ok(*b),
        Some(_) => Err(format!("{key}: true or false")),
    }
}

fn pairs(t: &toml::Table, key: &str) -> Result<Vec<(String, String)>, String> {
    match t.get(key) {
        None => Ok(Vec::new()),
        Some(toml::Value::Array(a)) => a
            .iter()
            .map(|v| match v.as_array().map(|p| p.as_slice()) {
                Some([toml::Value::String(o), toml::Value::String(c)])
                    if !o.is_empty() && !c.is_empty() && o.len() <= MAX_MARK && c.len() <= MAX_MARK =>
                {
                    Ok((o.clone(), c.clone()))
                }
                _ => Err(format!("{key}: pairs like [\"/*\", \"*/\"]")),
            })
            .collect(),
        Some(_) => Err(format!("{key}: a list of pairs")),
    }
}

/// A definition file's language.
pub fn parse(text_: &str) -> Result<Language, String> {
    let t: toml::Table = text_.parse().map_err(|e: toml::de::Error| e.message().to_string())?;
    let name = text(&t, "name")?;
    if name.trim().is_empty() {
        return Err("no name".into());
    }
    let mut quotes = Vec::new();
    if let Some(v) = t.get("strings") {
        let a = v.as_array().ok_or("strings: a list of [[strings]] tables")?;
        for q in a.iter().take(32) {
            let q = q.as_table().ok_or("strings: tables")?;
            let open = text(q, "open")?;
            if open.is_empty() || open.len() > MAX_MARK {
                return Err("strings: each needs an open (up to 16 bytes)".into());
            }
            let close = match text(q, "close")? {
                c if c.is_empty() => open.clone(),
                c => c,
            };
            let escape = match q.get("escape") {
                None => Some('\\'),
                Some(toml::Value::String(s)) => s.chars().next(),
                Some(_) => return Err("strings.escape: a character, or \"\" for none".into()),
            };
            let max_chars = match q.get("max-chars") {
                None => 0,
                Some(toml::Value::Integer(n)) if (0..=1000).contains(n) => *n as usize,
                Some(_) => return Err("strings.max-chars: 0 to 1000".into()),
            };
            quotes.push(Quote {
                open,
                close,
                escape,
                multiline: flag(q, "multiline", false)?,
                variables: flag(q, "variables", false)?,
                max_chars,
            });
        }
    }
    // The longest opening first: r#" before r", """ before ".
    quotes.sort_by_key(|q| std::cmp::Reverse(q.open.len()));
    let sections = match pairs(&t, "sections")?.as_slice() {
        [] => None,
        [p] => Some(p.clone()),
        _ => return Err("sections: one pair".into()),
    };
    let rules = Rules {
        keywords: list(&t, "keywords")?,
        types: list(&t, "types")?,
        builtins: list(&t, "builtins")?,
        constants: list(&t, "constants")?,
        any_case: flag(&t, "any-case", false)?,
        line_comments: list(&t, "line-comments")?,
        block_comments: pairs(&t, "block-comments")?,
        nested_comments: flag(&t, "nested-comments", false)?,
        doc_comments: list(&t, "doc-comments")?,
        quotes,
        variables: flag(&t, "variables", false)?,
        numbers: flag(&t, "numbers", true)?,
        operators: text(&t, "operators")?,
        function_after: list(&t, "function-after")?,
        type_after: list(&t, "type-after")?,
        attributes: pairs(&t, "attributes")?,
        meta_lines: list(&t, "meta-lines")?,
        keys: text(&t, "keys")?,
        sections,
        string_keys: text(&t, "string-keys")?,
    };
    let words =
        rules.keywords.iter().chain(&rules.types).chain(&rules.builtins).chain(&rules.constants).cloned().collect();
    Ok(Language {
        name: name.trim().to_string(),
        extensions: list(&t, "extensions")?
            .into_iter()
            .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
            .collect(),
        filenames: list(&t, "filenames")?,
        first_lines: list(&t, "first-lines")?,
        interpreters: list(&t, "interpreters")?,
        words,
        prose: flag(&t, "prose", false)?,
        indent_after: text(&t, "indent-after")?,
        highlight: Box::new(rules),
    })
}

impl Rules {
    fn has(&self, list: &[String], w: &str) -> bool {
        if self.any_case { list.iter().any(|k| k.eq_ignore_ascii_case(w)) } else { list.iter().any(|k| k == w) }
    }

    fn comment_kind(&self, line: &str, at: usize) -> Kind {
        if self.doc_comments.iter().any(|d| line[at..].starts_with(d.as_str())) {
            Kind::DocComment
        } else {
            Kind::Comment
        }
    }

    /// A block comment from `i` (its span from `seg`): where it ends, or
    /// None if it runs past the line (the state then says how deep).
    #[allow(clippy::too_many_arguments)]
    fn block(
        &self,
        line: &str,
        seg: usize,
        mut i: usize,
        which: usize,
        mut depth: u32,
        kind: Kind,
        out: &mut Vec<Span>,
    ) -> Result<usize, u32> {
        let (open, close) = &self.block_comments[which];
        while i < line.len() {
            if line[i..].starts_with(close.as_str()) {
                depth -= 1;
                i += close.len();
                if depth == 0 {
                    push(out, seg, i, kind);
                    return Ok(i);
                }
            } else if self.nested_comments && line[i..].starts_with(open.as_str()) {
                depth = (depth + 1).min(64);
                i += open.len();
            } else {
                i += char_len(line.as_bytes()[i]);
            }
        }
        push(out, seg, line.len(), kind);
        Err(depth)
    }

    /// A string's content from `i` (its span from `seg`) to its close.
    /// Returns where it ended and whether it runs on past the line.
    fn string(&self, line: &str, seg: usize, mut i: usize, q: &Quote, out: &mut Vec<Span>) -> (usize, bool) {
        let b = line.as_bytes();
        let mut seg = seg;
        while i < b.len() {
            if q.escape.is_some_and(|e| line[i..].starts_with(e)) {
                push(out, seg, i, Kind::String);
                let n = 1 + b.get(i + 1).map_or(0, |c| char_len(*c));
                let end = (i + n).min(b.len());
                push(out, i, end, Kind::Escape);
                i = end;
                seg = i;
                continue;
            }
            if line[i..].starts_with(q.close.as_str()) {
                push(out, seg, i + q.close.len(), Kind::String);
                return (i + q.close.len(), false);
            }
            if q.variables
                && b[i] == b'$'
                && b.get(i + 1).is_some_and(|c| is_word_start(*c) || c.is_ascii_digit() || b"{?@#$!*-".contains(c))
            {
                push(out, seg, i, Kind::String);
                let end = variable_end(b, i);
                push(out, i, end, Kind::Variable);
                i = end;
                seg = i;
                continue;
            }
            i += char_len(b[i]);
        }
        push(out, seg, b.len(), Kind::String);
        (b.len(), q.multiline)
    }
}

/// `$name`, `${...}`, `$1`, `$?`: where it ends.
fn variable_end(b: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    match b.get(j) {
        Some(b'{') => {
            while j < b.len() && b[j] != b'}' {
                j += 1;
            }
            (j + 1).min(b.len())
        }
        Some(c) if c.is_ascii_digit() || b"?@#$!*-".contains(c) => j + 1,
        _ => {
            while j < b.len() && is_word(b[j]) {
                j += char_len(b[j]);
            }
            j.min(b.len())
        }
    }
}

impl Highlight for Rules {
    fn line(&self, state: &mut State, line: &str, out: &mut Vec<Span>) {
        let b = line.as_bytes();
        let mut i = 0;
        match state.0.first().copied() {
            Some(COMMENT) => {
                let which = state.0.get(1).copied().unwrap_or(0) as usize;
                let depth = state.0.get(2).copied().unwrap_or(1).max(1);
                if which >= self.block_comments.len() {
                    state.0.clear();
                } else {
                    match self.block(line, 0, 0, which, depth, Kind::Comment, out) {
                        Ok(end) => {
                            i = end;
                            state.0.clear();
                        }
                        Err(d) => {
                            state.0 = vec![COMMENT, which as u32, d];
                            return;
                        }
                    }
                }
            }
            Some(STRING) => {
                let which = state.0.get(1).copied().unwrap_or(0) as usize;
                state.0.clear();
                if let Some(q) = self.quotes.get(which) {
                    let (end, open) = self.string(line, 0, 0, q, out);
                    if open {
                        state.0 = vec![STRING, which as u32];
                        return;
                    }
                    i = end;
                }
            }
            _ => state.0.clear(),
        }
        let indent = b.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(b.len());
        if i == 0 && indent < b.len() {
            let rest = &line[indent..];
            if self.meta_lines.iter().any(|m| rest.starts_with(m.as_str())) {
                push(out, indent, line.len(), Kind::Meta);
                return;
            }
            if let Some((o, c)) = &self.sections
                && rest.starts_with(o.as_str())
                && let Some(k) = rest[o.len()..].find(c.as_str())
            {
                push(out, indent, indent + o.len() + k + c.len(), Kind::Type);
                i = indent + o.len() + k + c.len();
            } else if !self.keys.is_empty()
                && let Some(k) = key_end(rest, &self.keys)
            {
                push(out, indent, indent + k, Kind::Attribute);
                i = indent + k;
            }
        }
        let mut after: Option<String> = None;
        while i < b.len() {
            let c = b[i];
            // Comments.
            if let Some(m) = self.line_comments.iter().find(|m| line[i..].starts_with(m.as_str())) {
                // A marker that is also a word's end (shell's `#` in `$#`) must start one.
                if !(m == "#" && i > 0 && b[i - 1] == b'$') {
                    push(out, i, b.len(), self.comment_kind(line, i));
                    return;
                }
            }
            if let Some(w) = self.block_comments.iter().position(|(o, _)| line[i..].starts_with(o.as_str())) {
                let kind = self.comment_kind(line, i);
                let open = self.block_comments[w].0.len();
                match self.block(line, i, i + open, w, 1, kind, out) {
                    Ok(end) => {
                        i = end;
                        continue;
                    }
                    Err(d) => {
                        state.0 = vec![COMMENT, w as u32, d];
                        return;
                    }
                }
            }
            if let Some((o, cl)) = self.attributes.iter().find(|(o, _)| line[i..].starts_with(o.as_str())) {
                let end = line[i + o.len()..].find(cl.as_str()).map_or(b.len(), |k| i + o.len() + k + cl.len());
                push(out, i, end, Kind::Meta);
                i = end;
                continue;
            }
            // Strings (an opening that starts with a letter, r"..", not inside a word).
            let quote = self.quotes.iter().enumerate().find(|(_, q)| {
                line[i..].starts_with(q.open.as_str())
                    && !(q.open.as_bytes()[0].is_ascii_alphabetic() && i > 0 && is_word(b[i - 1]))
            });
            if let Some((k, q)) = quote {
                let fits = q.max_chars == 0 || {
                    let body = &line[i + q.open.len()..];
                    let body = if q.escape.is_some_and(|e| body.starts_with(e)) { &body[1..] } else { body };
                    body.char_indices()
                        .take(q.max_chars + 1)
                        .any(|(n, _)| body[n..].starts_with(q.close.as_str()) && n > 0)
                };
                if fits {
                    let (end, open) = self.string(line, i, i + q.open.len(), q, out);
                    if open {
                        state.0 = vec![STRING, k as u32];
                        return;
                    }
                    if !self.string_keys.is_empty() && line[end..].trim_start().starts_with(self.string_keys.as_str()) {
                        // JSON's "name": the string is a key.
                        retag(out, i, end, Kind::Attribute);
                    }
                    i = end;
                    after = None;
                    continue;
                }
            }
            if self.variables
                && c == b'$'
                && b.get(i + 1).is_some_and(|x| is_word_start(*x) || x.is_ascii_digit() || b"{?@#$!*-".contains(x))
            {
                let end = variable_end(b, i);
                push(out, i, end, Kind::Variable);
                i = end;
                continue;
            }
            if is_word_start(c) {
                let start = i;
                while i < b.len() && is_word(b[i]) {
                    i += char_len(b[i]);
                }
                let word = &line[start..i];
                let kind = match &after {
                    Some(a) if self.function_after.contains(a) => Some(Kind::Function),
                    Some(a) if self.type_after.contains(a) => Some(Kind::Type),
                    _ if self.has(&self.keywords, word) => Some(Kind::Keyword),
                    _ if self.has(&self.constants, word) => Some(Kind::Constant),
                    _ if self.has(&self.types, word) => Some(Kind::Type),
                    _ if self.has(&self.builtins, word) => Some(Kind::Builtin),
                    _ => None,
                };
                if let Some(k) = kind {
                    push(out, start, i, k);
                }
                after =
                    (self.function_after.iter().chain(&self.type_after).any(|a| a == word)).then(|| word.to_string());
                continue;
            }
            if self.numbers && (c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit))) {
                let start = i;
                let hex = c == b'0' && b.get(i + 1).is_some_and(|x| (x | 0x20) == b'x');
                while i < b.len() {
                    let d = b[i];
                    let sign = (d == b'+' || d == b'-') && !hex && i > start && (b[i - 1] | 0x20) == b'e';
                    if d.is_ascii_alphanumeric() || d == b'_' || (d == b'.' && b.get(i + 1) != Some(&b'.')) || sign {
                        i += 1;
                    } else {
                        break;
                    }
                }
                push(out, start, i, Kind::Number);
                after = None;
                continue;
            }
            if self.operators.as_bytes().contains(&c) {
                push(out, i, i + 1, Kind::Operator);
            }
            if !c.is_ascii_whitespace() {
                after = None;
            }
            i += char_len(c);
        }
    }
}

/// A key at a line's start (`name =`, `name:`, `"name" =`, `a.b-c =`): its
/// length, before the marker.
fn key_end(rest: &str, marker: &str) -> Option<usize> {
    let b = rest.as_bytes();
    let mut i = 0;
    if b.first() == Some(&b'"') || b.first() == Some(&b'\'') {
        let q = b[0];
        i = rest[1..].find(q as char)? + 2;
    } else {
        while i < b.len() && (is_word(b[i]) || b"-.".contains(&b[i])) {
            i += char_len(b[i]);
        }
        if i == 0 {
            return None;
        }
    }
    rest[i..].trim_start().starts_with(marker).then_some(i)
}

/// The spans from `start` to `end` made one of `kind`.
fn retag(out: &mut Vec<Span>, start: usize, end: usize, kind: Kind) {
    out.retain(|s| s.end <= start || s.start >= end);
    let at = out.partition_point(|s| s.end <= start);
    out.insert(at, Span { start, end, kind });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{by_name, highlight};

    fn spans(lang: &str, state: &mut State, line: &str) -> Vec<(String, Kind)> {
        let l = by_name(lang).unwrap_or_else(|| panic!("{lang} is shipped"));
        highlight(l, state, line).into_iter().map(|s| (line[s.start..s.end].to_string(), s.kind)).collect()
    }

    fn has(v: &[(String, Kind)], text: &str, kind: Kind) -> bool {
        v.iter().any(|(t, k)| t == text && *k == kind)
    }

    #[test]
    fn a_bad_file_says_why() {
        assert!(parse("name = \"\"").err().unwrap().contains("no name"));
        assert!(parse("name = \"X\"\nkeywords = \"if\"").err().unwrap().contains("keywords"));
        assert!(parse("name = \"X\"\n[[strings]]\nclose = \"'\"").err().unwrap().contains("open"));
        assert!(parse("not toml at all [").is_err());
        let l = parse("name = \"Tiny\"\nextensions = [\".TNY\"]\nkeywords = [\"go\"]").unwrap();
        assert_eq!(l.extensions, ["tny"]);
    }

    #[test]
    fn a_persons_file_replaces_a_shipped_one() {
        let dir = std::env::temp_dir().join(format!("vergence-syntax-{}", std::process::id()));
        let (a, b) = (dir.join("shipped"), dir.join("mine"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("x.toml"), "name = \"Thing\"\nkeywords = [\"old\"]").unwrap();
        std::fs::write(b.join("x.toml"), "name = \"thing\"\nkeywords = [\"new\"]").unwrap();
        std::fs::write(b.join("broken.toml"), "name = 3").unwrap();
        let langs = load(&[a, b]);
        assert_eq!(langs.len(), 1, "the broken one skipped");
        assert_eq!(langs[0].words, ["new"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shell() {
        let mut st = State::default();
        let v = spans("shell", &mut st, r#"for f in "$HOME"/*.txt; do echo "${f%.txt} $1" # loop"#);
        assert!(has(&v, "for", Kind::Keyword), "{v:?}");
        assert!(has(&v, "$HOME", Kind::Variable));
        assert!(has(&v, "${f%.txt}", Kind::Variable));
        assert!(has(&v, "$1", Kind::Variable));
        assert!(has(&v, "echo", Kind::Builtin));
        assert!(has(&v, "# loop", Kind::Comment));
        assert!(!spans("shell", &mut State::default(), "echo $# args").iter().any(|(_, k)| *k == Kind::Comment));
        assert_eq!(crate::detect(Some(Path::new("/x/build")), "#!/usr/bin/env bash"), by_name("shell"));
        assert_eq!(crate::detect(Some(Path::new("/home/d/.zshrc")), ""), by_name("shell"));
    }

    #[test]
    fn rust_comments_nest_and_lifetimes_are_not_chars() {
        let mut st = State::default();
        let v = spans(
            "rust",
            &mut st,
            "#[derive(Debug)] pub struct Shelf<'a> { c: char = 'x', s: &'a str } /* a /* nested */",
        );
        assert!(has(&v, "#[derive(Debug)]", Kind::Meta), "{v:?}");
        assert!(has(&v, "Shelf", Kind::Type));
        assert!(has(&v, "'x'", Kind::String));
        assert!(!v.iter().any(|(t, k)| t.starts_with("'a") && *k == Kind::String), "a lifetime: {v:?}");
        assert_ne!(st, State::default(), "still in the outer comment");
        let v = spans("rust", &mut st, "still */ fn main() { let r = r#\"raw \"q\"\"#; }");
        assert!(has(&v, "still */", Kind::Comment), "{v:?}");
        assert!(has(&v, "main", Kind::Function));
        assert!(has(&v, "r#\"raw \"q\"\"#", Kind::String));
        assert!(has(&spans("rust", &mut State::default(), "/// Docs."), "/// Docs.", Kind::DocComment));
    }

    #[test]
    fn toml_ini_json_yaml_and_c() {
        let mut st = State::default();
        let v = spans("toml", &mut st, "[workspace.package]");
        assert!(has(&v, "[workspace.package]", Kind::Type), "{v:?}");
        let v = spans("toml", &mut st, "edition = \"2024\"  # the newest");
        assert!(has(&v, "edition", Kind::Attribute), "{v:?}");
        assert!(has(&v, "\"2024\"", Kind::String));
        assert!(has(&v, "# the newest", Kind::Comment));
        let v = spans("json", &mut State::default(), r#"{"name": "Lookout", "watts": 170, "on": true}"#);
        assert!(has(&v, "\"name\"", Kind::Attribute), "{v:?}");
        assert!(has(&v, "\"Lookout\"", Kind::String));
        assert!(has(&v, "170", Kind::Number));
        assert!(has(&v, "true", Kind::Constant));
        let v = spans("yaml", &mut State::default(), "  image: \"nginx:1.27\"  # pinned");
        assert!(has(&v, "image", Kind::Attribute), "{v:?}");
        let v = spans("c", &mut State::default(), "#include <stdio.h>");
        assert!(has(&v, "#include <stdio.h>", Kind::Meta), "{v:?}");
        let v = spans("c", &mut State::default(), "static int count(const char *s) { return 0x1F; }");
        assert!(
            has(&v, "static", Kind::Keyword) && has(&v, "int", Kind::Type) && has(&v, "0x1F", Kind::Number),
            "{v:?}"
        );
        let v = spans("ini", &mut State::default(), "[Unit]");
        assert!(has(&v, "[Unit]", Kind::Type));
        assert!(has(
            &spans("ini", &mut State::default(), "Description=The rack watch"),
            "Description",
            Kind::Attribute
        ));
    }
}
