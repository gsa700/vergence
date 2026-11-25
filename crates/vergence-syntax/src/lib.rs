//! Syntax highlighting: a line of text cut into spans of a kind (keyword, string, comment...),
//! one line at a time, with a state carried from each line to the next.
//!
//! Edit syntax milestone 1 (docs/edit-syntax-milestone-1.md). Used by vergence-widgets (the
//! text area draws each span in its kind's color, from vergence-look) and vergence-edit (which
//! language a file is, View > Language). No UI here and no dependencies.
//!
//! - **Line by line**: [`highlight`] takes the state the line before ended in and a line, and
//!   returns the line's spans and leaves the state this line ends in. A multi-line construct (a
//!   triple-quoted string, a block comment, a code fence) lives in that state. After an edit
//!   the text area works lines out again from the edited one until a line ends in the state it
//!   ended in before, which is usually at once, so typing in a long file stays cheap.
//! - **Kinds, not colors**: [`Kind`]. The look chooses colors, for light and dark.
//! - **Languages are pluggable** (Dave, 2026-11-24: "build so they can be added"): every
//!   language is a [`Language`] in one registry ([`languages`]), found by name, by a file's
//!   extension or by its first line ([`detect`]). Built-in ones implement [`Highlight`];
//!   definition files (step 7) will add rule-driven ones the same way. Nothing outside this
//!   crate names a language.
//!
//! Spans cover only what is not plain text, in order, never overlapping, on character
//! boundaries. Every line is untrusted (a file from anywhere): the fuzz tests feed the
//! highlighters damage.
//!
//! Start reading: [`highlight`], [`Language`], then python.rs.

use std::path::Path;
use std::sync::OnceLock;

mod clike;
mod css;
#[cfg(test)]
mod fuzz;
mod html;
mod js;
mod markdown;
mod python;

/// What a piece of a line is. The look gives each a color (and Markdown's
/// structure a weight or slant).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A word the language reserves (`def`, `if`, `function`).
    Keyword,
    /// A type's name, or a class being defined.
    Type,
    /// A name the language provides (`len`, `print`, `self`).
    Builtin,
    /// A function's name where it is defined.
    Function,
    /// `True`, `None`, `null`: values the language names.
    Constant,
    /// A variable marked as one (`$name` in PHP and shell).
    Variable,
    String,
    /// `\n`, `{{` and the like inside a string.
    Escape,
    Number,
    Comment,
    /// A docstring or a documentation comment.
    DocComment,
    Operator,
    /// Decorators, preprocessor lines, PHP's tags.
    Meta,
    /// Markup: a tag's name, an attribute, its value.
    Tag,
    Attribute,
    AttrValue,
    /// Markdown: a heading, emphasis, strong, code, a link, a quote, a list's marker.
    Heading,
    Emphasis,
    Strong,
    Code,
    Link,
    Quote,
    ListMarker,
}

/// A piece of a line: bytes `start..end`, of a kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

/// Where a line ended: what is still open (a string, a comment, a fence),
/// in each language's own words. Compared to know when a change stops
/// mattering to the lines after it; the default is "nothing open".
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct State(pub Vec<u32>);

/// A language's highlighter: the spans of one line, from and to a state.
pub trait Highlight: Send + Sync {
    fn line(&self, state: &mut State, line: &str, out: &mut Vec<Span>);
}

/// A language: its name, how files of it are known, its words (keywords
/// and builtins, which completion offers), and its highlighter.
pub struct Language {
    pub name: String,
    /// File extensions, lower case, without the dot ("py", "pyw").
    pub extensions: Vec<String>,
    /// A first line starting with one of these is this language ("#!/usr/bin/env python").
    pub first_lines: Vec<String>,
    /// Names after `#!` and `env` that mean this language ("python3").
    pub interpreters: Vec<String>,
    pub words: Vec<String>,
    /// Its plain text is prose (Markdown): spelled and completed from the
    /// dictionary. In code only comments and strings are.
    pub prose: bool,
    highlight: Box<dyn Highlight>,
}

/// A language's place in the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LangId(pub u16);

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// Every language, built-in ones first. Read once per process.
pub fn languages() -> &'static [Language] {
    static ALL: OnceLock<Vec<Language>> = OnceLock::new();
    ALL.get_or_init(|| {
        vec![
            Language {
                name: "Python".into(),
                extensions: strings(&["py", "pyw", "pyi"]),
                first_lines: Vec::new(),
                interpreters: strings(&["python", "python3", "python2", "pypy", "pypy3"]),
                words: python::words(),
                prose: false,
                highlight: Box::new(python::Python),
            },
            Language {
                name: "Markdown".into(),
                extensions: strings(&["md", "markdown", "mdown", "mkd", "mkdn"]),
                first_lines: Vec::new(),
                interpreters: Vec::new(),
                words: Vec::new(),
                prose: true,
                highlight: Box::new(markdown::Markdown),
            },
            Language {
                name: "HTML".into(),
                extensions: strings(&["html", "htm", "xhtml", "shtml"]),
                first_lines: strings(&["<!doctype html", "<html"]),
                interpreters: Vec::new(),
                words: html::TAGS.iter().chain(html::ATTRIBUTES).map(|s| s.to_string()).collect(),
                prose: true,
                highlight: Box::new(html::Html { php: false }),
            },
            Language {
                name: "PHP".into(),
                extensions: strings(&["php", "phtml", "php3", "php4", "php5", "php7", "php8", "phps"]),
                first_lines: strings(&["<?php"]),
                interpreters: strings(&["php"]),
                words: {
                    let mut w = html::PHP_RULES.words();
                    w.extend(html::TAGS.iter().chain(html::ATTRIBUTES).map(|s| s.to_string()));
                    w
                },
                prose: true,
                highlight: Box::new(html::Html { php: true }),
            },
            Language {
                name: "JavaScript".into(),
                extensions: strings(&["js", "mjs", "cjs", "jsx", "ts", "tsx", "mts", "cts"]),
                first_lines: Vec::new(),
                interpreters: strings(&["node", "nodejs", "deno", "bun"]),
                words: js::RULES.words(),
                prose: false,
                highlight: Box::new(js::JavaScript),
            },
            Language {
                name: "CSS".into(),
                extensions: strings(&["css"]),
                first_lines: Vec::new(),
                interpreters: Vec::new(),
                words: css::PROPERTIES.iter().map(|s| s.to_string()).collect(),
                prose: false,
                highlight: Box::new(css::Css),
            },
        ]
    })
}

pub fn language(id: LangId) -> &'static Language {
    &languages()[id.0 as usize]
}

/// A language by its name, any case ("python").
pub fn by_name(name: &str) -> Option<LangId> {
    let name = name.trim();
    languages()
        .iter()
        .position(|l| l.name.eq_ignore_ascii_case(name) || l.extensions.iter().any(|e| e.eq_ignore_ascii_case(name)))
        .map(|i| LangId(i as u16))
}

/// Which language a file is: by its extension, else its first line (a
/// `#!` line naming an interpreter, or a language's own opening). None:
/// plain text.
pub fn detect(path: Option<&Path>, first_line: &str) -> Option<LangId> {
    let all = languages();
    if let Some(ext) = path.and_then(|p| p.extension()).and_then(|e| e.to_str()) {
        let ext = ext.to_ascii_lowercase();
        if let Some(i) = all.iter().position(|l| l.extensions.contains(&ext)) {
            return Some(LangId(i as u16));
        }
    }
    let first = first_line.trim_start_matches('\u{feff}');
    if let Some(rest) = first.strip_prefix("#!") {
        // "#!/usr/bin/python3 -u", "#!/usr/bin/env python3", "#!/usr/bin/env -S python3 -u"
        let mut words = rest.split_whitespace();
        let mut prog = words.next().map(|p| p.rsplit('/').next().unwrap_or(p)).unwrap_or("");
        if prog == "env" {
            prog = words.find(|w| !w.starts_with('-')).unwrap_or("");
        }
        if let Some(i) = all.iter().position(|l| l.interpreters.iter().any(|n| n == prog)) {
            return Some(LangId(i as u16));
        }
    }
    all.iter()
        .position(|l| l.first_lines.iter().any(|f| first.to_ascii_lowercase().starts_with(&f.to_ascii_lowercase())))
        .map(|i| LangId(i as u16))
}

/// The longest line highlighted: past this a line is left plain (a minified
/// file's one enormous line would cost every keystroke otherwise).
pub const MAX_LINE: usize = 10_000;

/// Line `line`'s spans in `lang`, from `state` (the state the line before
/// ended in), which is left as this line ends.
pub fn highlight(lang: LangId, state: &mut State, line: &str) -> Vec<Span> {
    let mut out = Vec::new();
    if line.len() > MAX_LINE {
        return out;
    }
    language(lang).highlight.line(state, line, &mut out);
    out
}

/// The kind at byte `at` of a line, given its spans (None: plain).
pub fn kind_at(spans: &[Span], at: usize) -> Option<Kind> {
    spans.iter().find(|s| s.start <= at && at < s.end).map(|s| s.kind)
}

/// Kinds in which prose is written, where spelling is checked and the
/// dictionary completes words: comments, docstrings, strings.
pub fn is_prose(kind: Option<Kind>) -> bool {
    matches!(kind, Some(Kind::Comment | Kind::DocComment | Kind::String))
}

/// The byte length of the character starting with byte `c`.
pub(crate) fn char_len(c: u8) -> usize {
    match c {
        0..0x80 => 1,
        0xc0..0xe0 => 2,
        0xe0..0xf0 => 3,
        0xf0.. => 4,
        _ => 1, // a continuation byte: never a start in valid UTF-8
    }
}

/// Shared by the highlighters: spans pushed in order, merging a span into
/// the one before when it is the same kind and they touch.
pub(crate) fn push(out: &mut Vec<Span>, start: usize, end: usize, kind: Kind) {
    if start >= end {
        return;
    }
    if let Some(last) = out.last_mut()
        && last.kind == kind
        && last.end == start
    {
        last.end = end;
        return;
    }
    out.push(Span { start, end, kind });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_are_found_by_extension_first_line_and_name() {
        let py = by_name("python").expect("Python is built in");
        assert_eq!(by_name("PY"), Some(py), "an extension names it too");
        assert_eq!(detect(Some(Path::new("/x/tool.PY")), ""), Some(py));
        assert_eq!(detect(Some(Path::new("tool")), "#!/usr/bin/env python3"), Some(py));
        assert_eq!(detect(Some(Path::new("tool")), "#!/usr/bin/env -S python3 -u"), Some(py));
        assert_eq!(detect(Some(Path::new("tool")), "#!/usr/local/bin/python"), Some(py));
        assert_eq!(detect(None, "\u{feff}#!/usr/bin/python3"), Some(py), "a byte-order mark first");
        assert_eq!(detect(Some(Path::new("notes.txt")), "hello"), None);
        assert_eq!(detect(None, "#!/bin/sh"), None, "no shell yet");
    }

    #[test]
    fn a_line_past_the_limit_stays_plain() {
        let py = by_name("python").unwrap();
        let long = "x = 'a'".repeat(MAX_LINE);
        assert!(highlight(py, &mut State::default(), &long).is_empty());
    }
}
