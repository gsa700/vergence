//! CSS: selectors (tags, `.classes`, `#ids`, `:pseudo`s, `[attributes]`), properties and custom
//! properties, values (numbers with units, colors, functions, `!important`), at-rules, strings
//! and comments. Nested rules work (a block inside a block is selectors again).
//!
//! The state between lines: `[depth, flags]`, how many blocks are open and whether a comment is
//! open or a value is being written (a declaration may run over lines).

use crate::{Highlight, Kind, Span, State, char_len, push};

pub struct Css;

const IN_COMMENT: u32 = 1;
const IN_VALUE: u32 = 2;

/// Property names, for completion.
pub const PROPERTIES: &[&str] = &[
    "align-content",
    "align-items",
    "align-self",
    "animation",
    "aspect-ratio",
    "backdrop-filter",
    "background",
    "background-color",
    "background-image",
    "background-position",
    "background-repeat",
    "background-size",
    "border",
    "border-bottom",
    "border-collapse",
    "border-color",
    "border-left",
    "border-radius",
    "border-right",
    "border-top",
    "border-width",
    "bottom",
    "box-shadow",
    "box-sizing",
    "clear",
    "color",
    "column-gap",
    "content",
    "cursor",
    "display",
    "filter",
    "flex",
    "flex-basis",
    "flex-direction",
    "flex-grow",
    "flex-shrink",
    "flex-wrap",
    "float",
    "font",
    "font-family",
    "font-size",
    "font-style",
    "font-weight",
    "gap",
    "grid",
    "grid-area",
    "grid-column",
    "grid-row",
    "grid-template-areas",
    "grid-template-columns",
    "grid-template-rows",
    "height",
    "inset",
    "justify-content",
    "justify-items",
    "left",
    "letter-spacing",
    "line-height",
    "list-style",
    "margin",
    "margin-bottom",
    "margin-left",
    "margin-right",
    "margin-top",
    "max-height",
    "max-width",
    "min-height",
    "min-width",
    "object-fit",
    "opacity",
    "outline",
    "overflow",
    "overflow-x",
    "overflow-y",
    "padding",
    "padding-bottom",
    "padding-left",
    "padding-right",
    "padding-top",
    "pointer-events",
    "position",
    "right",
    "row-gap",
    "text-align",
    "text-decoration",
    "text-overflow",
    "text-transform",
    "top",
    "transform",
    "transition",
    "user-select",
    "vertical-align",
    "visibility",
    "white-space",
    "width",
    "word-break",
    "z-index",
];

fn is_name(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c >= 0x80
}

impl Highlight for Css {
    fn line(&self, state: &mut State, line: &str, out: &mut Vec<Span>) {
        let mut st = state.0.clone();
        run(&mut st, line, 0, line.len(), out);
        state.0 = st;
    }
}

/// CSS in `line[from..to]` from and to `state` (also used for HTML's `<style>`).
pub(crate) fn run(state: &mut Vec<u32>, line: &str, from: usize, to: usize, out: &mut Vec<Span>) {
    let b = line.as_bytes();
    let mut depth = state.first().copied().unwrap_or(0);
    let mut flags = state.get(1).copied().unwrap_or(0);
    let mut i = from;
    while i < to {
        if flags & IN_COMMENT != 0 {
            match line[i..to].find("*/") {
                Some(k) => {
                    push(out, i, i + k + 2, Kind::Comment);
                    i += k + 2;
                    flags &= !IN_COMMENT;
                    continue;
                }
                None => {
                    push(out, i, to, Kind::Comment);
                    break;
                }
            }
        }
        let c = b[i];
        if line[i..to].starts_with("/*") {
            flags |= IN_COMMENT;
            push(out, i, i + 2, Kind::Comment);
            i += 2;
            continue;
        }
        match c {
            b'"' | b'\'' => {
                let mut j = i + 1;
                while j < to && b[j] != c {
                    j += if b[j] == b'\\' { 2 } else { char_len(b[j]) };
                }
                let end = (j + 1).min(to);
                push(out, i, end, Kind::String);
                i = end;
            }
            b'{' => {
                depth = depth.saturating_add(1).min(64);
                flags &= !IN_VALUE;
                i += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                flags &= !IN_VALUE;
                i += 1;
            }
            b';' => {
                flags &= !IN_VALUE;
                i += 1;
            }
            b'@' => {
                let mut j = i + 1;
                while j < to && is_name(b[j]) {
                    j += 1;
                }
                push(out, i, j, Kind::Keyword);
                i = j.max(i + 1);
            }
            b'!' if line[i..to].to_ascii_lowercase().starts_with("!important") => {
                push(out, i, i + 10, Kind::Keyword);
                i += 10;
            }
            _ if flags & IN_VALUE != 0 => i = value(line, i, to, out),
            _ => i = selector_or_property(line, i, to, depth, &mut flags, out),
        }
    }
    *state = vec![depth, flags];
}

/// Inside a value: numbers (with units), colors, functions.
fn value(line: &str, i: usize, to: usize, out: &mut Vec<Span>) -> usize {
    let b = line.as_bytes();
    let c = b[i];
    if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
        let mut j = i;
        while j < to && (b[j].is_ascii_alphanumeric() || b[j] == b'.' || b[j] == b'%') {
            j += 1;
        }
        push(out, i, j, Kind::Number);
        return j;
    }
    if c == b'#' {
        let mut j = i + 1;
        while j < to && b[j].is_ascii_hexdigit() {
            j += 1;
        }
        if j > i + 1 {
            push(out, i, j, Kind::Number);
            return j;
        }
    }
    if c == b'-' && b.get(i + 1) == Some(&b'-') {
        let mut j = i;
        while j < to && is_name(b[j]) {
            j += 1;
        }
        push(out, i, j, Kind::Variable);
        return j;
    }
    if c.is_ascii_alphabetic() {
        let mut j = i;
        while j < to && is_name(b[j]) {
            j += 1;
        }
        if b.get(j) == Some(&b'(') {
            push(out, i, j, Kind::Builtin);
        }
        return j;
    }
    i + char_len(c)
}

/// Outside a value: a property (a name, then `:`, in a block, with no `{`
/// following on the line) or a selector.
fn selector_or_property(line: &str, i: usize, to: usize, depth: u32, flags: &mut u32, out: &mut Vec<Span>) -> usize {
    let b = line.as_bytes();
    let c = b[i];
    if depth > 0 && (c.is_ascii_alphabetic() || c == b'-') {
        let mut j = i;
        while j < to && is_name(b[j]) {
            j += 1;
        }
        let rest = &line[j..to];
        // `a:hover {` is a (nested) selector; `color: red` a property.
        if rest.trim_start().starts_with(':') && !rest.contains('{') {
            push(out, i, j, if line[i..j].starts_with("--") { Kind::Variable } else { Kind::Attribute });
            let k = j + rest.find(':').unwrap_or(0);
            push(out, k, k + 1, Kind::Operator);
            *flags |= IN_VALUE;
            return k + 1;
        }
    }
    match c {
        b'.' | b'#' => {
            let mut j = i + 1;
            while j < to && is_name(b[j]) {
                j += 1;
            }
            push(out, i, j, if c == b'.' { Kind::Type } else { Kind::Function });
            j.max(i + 1)
        }
        b':' => {
            let mut j = i + 1;
            if b.get(j) == Some(&b':') {
                j += 1;
            }
            while j < to && is_name(b[j]) {
                j += 1;
            }
            push(out, i, j, Kind::Builtin);
            j
        }
        b'[' => {
            let end = line[i..to].find(']').map_or(to, |k| i + k + 1);
            push(out, i, end, Kind::Attribute);
            end
        }
        b'&' | b'>' | b'+' | b'~' | b',' => {
            push(out, i, i + 1, Kind::Operator);
            i + 1
        }
        _ if c.is_ascii_alphabetic() => {
            let mut j = i;
            while j < to && is_name(b[j]) {
                j += 1;
            }
            push(out, i, j, Kind::Tag);
            j
        }
        _ => i + char_len(c),
    }
}

#[cfg(test)]
mod tests {
    use crate::{Kind, State, by_name, highlight};

    fn spans(state: &mut State, line: &str) -> Vec<(String, Kind)> {
        let css = by_name("css").unwrap();
        highlight(css, state, line).into_iter().map(|s| (line[s.start..s.end].to_string(), s.kind)).collect()
    }

    fn has(v: &[(String, Kind)], text: &str, kind: Kind) -> bool {
        v.iter().any(|(t, k)| t == text && *k == kind)
    }

    #[test]
    fn selectors_properties_and_values() {
        let mut st = State::default();
        let v = spans(&mut st, "nav > a.active:hover, #top::before {");
        for (t, k) in [("nav", Kind::Tag), (".active", Kind::Type), (":hover", Kind::Builtin), ("#top", Kind::Function)]
        {
            assert!(has(&v, t, k), "{t}: {v:?}");
        }
        let v = spans(&mut st, "  color: #3a3a3c; margin: 0 1.5em !important;");
        assert!(has(&v, "color", Kind::Attribute), "{v:?}");
        assert!(has(&v, "#3a3a3c", Kind::Number));
        assert!(has(&v, "1.5em", Kind::Number));
        assert!(has(&v, "!important", Kind::Keyword));
        let v = spans(&mut st, "  --gap: calc(var(--x) * 2); }");
        assert!(has(&v, "--gap", Kind::Variable), "{v:?}");
        assert!(has(&v, "calc", Kind::Builtin));
        assert_eq!(st, State(vec![0, 0]), "the block closed");
    }

    #[test]
    fn comments_and_at_rules_across_lines() {
        let mut st = State::default();
        let v = spans(&mut st, "@media (max-width: 600px) { /* small");
        assert!(has(&v, "@media", Kind::Keyword), "{v:?}");
        let v = spans(&mut st, "   screens */ a { color: red }");
        assert!(has(&v, "   screens */", Kind::Comment), "{v:?}");
        assert!(has(&v, "a", Kind::Tag));
        assert!(has(&v, "color", Kind::Attribute));
    }
}
