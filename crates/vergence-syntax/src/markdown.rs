//! Markdown (CommonMark's common parts, and GitHub's tables and task lists): headings, rules,
//! quotes, list markers, tables, link definitions, front matter, HTML comments, and code fences
//! colored in their own language (```` ```python ````); inline, code spans, emphasis and
//! strong, links and images, autolinks and bare URLs, HTML tags, escapes.
//!
//! The state between lines: the document's start (the empty state, where front matter may
//! open), plain text, front matter, an HTML comment, or a fence (its character, length, and
//! language, with that language's own state after them). Emphasis and links end with their
//! line (CommonMark lets them run across a paragraph's lines; a highlighter that cannot know
//! where the paragraph ends would color too much when one is left open).

use crate::{Highlight, Kind, LangId, Span, State, by_name, language, push};

pub struct Markdown;

const NORMAL: u32 = 1;
const FRONT: u32 = 2;
const COMMENT: u32 = 3;
const FENCE: u32 = 4;

/// Up to three spaces of indentation, as block markers allow: the rest, or
/// None if indented more (or with a tab).
fn block_start(line: &str) -> Option<usize> {
    let n = line.bytes().take_while(|c| *c == b' ').count();
    (n <= 3 && !line[n..].starts_with('\t')).then_some(n)
}

/// A fence's opening: its character, length and info string.
fn fence_open(line: &str) -> Option<(u8, usize, &str)> {
    let at = block_start(line)?;
    let rest = &line[at..];
    let c = *rest.as_bytes().first()?;
    if c != b'`' && c != b'~' {
        return None;
    }
    let len = rest.bytes().take_while(|b| *b == c).count();
    if len < 3 {
        return None;
    }
    let info = rest[len..].trim();
    // A backtick fence's info string has no backticks (else it is inline code).
    if c == b'`' && info.contains('`') {
        return None;
    }
    Some((c, len, info))
}

/// A fence's closing line: the same character, at least as many, nothing after.
fn fence_close(line: &str, c: u8, len: usize) -> bool {
    let Some(at) = block_start(line) else { return false };
    let rest = &line[at..];
    let n = rest.bytes().take_while(|b| *b == c).count();
    n >= len && rest[n..].trim().is_empty()
}

/// The language an info string names (```` ```python ````, ```` ```{.rust} ````).
fn info_language(info: &str) -> Option<LangId> {
    let word = info.split_whitespace().next()?.trim_matches(|c| c == '{' || c == '}' || c == '.');
    by_name(word.split(',').next().unwrap_or(word))
}

/// A thematic break (`---`, `* * *`, `___`).
fn is_rule(line: &str) -> bool {
    let Some(at) = block_start(line) else { return false };
    let rest = line[at..].trim_end();
    let Some(c) = rest.chars().next() else { return false };
    matches!(c, '-' | '*' | '_')
        && rest.chars().all(|x| x == c || x == ' ')
        && rest.chars().filter(|x| *x == c).count() >= 3
}

impl Highlight for Markdown {
    fn line(&self, state: &mut State, line: &str, out: &mut Vec<Span>) {
        let at_start = state.0.is_empty();
        match state.0.first().copied() {
            Some(FRONT) => {
                let t = line.trim_end();
                if t == "---" || t == "..." {
                    *state = State(vec![NORMAL]);
                }
                push(out, 0, line.len(), Kind::Meta);
                return;
            }
            Some(COMMENT) => {
                match line.find("-->") {
                    Some(k) => {
                        push(out, 0, k + 3, Kind::Comment);
                        *state = State(vec![NORMAL]);
                        inline(line, k + 3, out, state);
                    }
                    None => push(out, 0, line.len(), Kind::Comment),
                }
                return;
            }
            Some(FENCE) if state.0.len() >= 4 => {
                let (c, len, lang) = (state.0[1] as u8, state.0[2] as usize, state.0[3]);
                if fence_close(line, c, len) {
                    push(out, 0, line.len(), Kind::Meta);
                    *state = State(vec![NORMAL]);
                    return;
                }
                match lang.checked_sub(1) {
                    // The fence's language, with its own state kept after ours.
                    Some(l) if (l as usize) < crate::languages().len() => {
                        let mut inner = State(state.0[4..].to_vec());
                        let first = out.len();
                        if line.len() <= crate::MAX_LINE {
                            language(LangId(l as u16)).highlight.line(&mut inner, line, out);
                        }
                        // Its plain words are code too, not the document's prose.
                        crate::fill_source(out, first, 0, line.len());
                        state.0.truncate(4);
                        state.0.extend(inner.0);
                    }
                    _ => push(out, 0, line.len(), Kind::Code),
                }
                return;
            }
            _ => {}
        }
        *state = State(vec![NORMAL]);
        if at_start && line.trim_end() == "---" {
            push(out, 0, line.len(), Kind::Meta);
            *state = State(vec![FRONT]);
            return;
        }
        if let Some((c, len, info)) = fence_open(line) {
            push(out, 0, line.len(), Kind::Meta);
            let lang = info_language(info).map_or(0, |l| l.0 as u32 + 1);
            *state = State(vec![FENCE, c as u32, len as u32, lang]);
            return;
        }
        let Some(at) = block_start(line) else {
            // Indented four or more: inside a list item, or indented code;
            // its inline parts are colored either way.
            list_or_inline(line, 0, out, state);
            return;
        };
        let rest = &line[at..];
        if rest.starts_with('#') {
            let hashes = rest.bytes().take_while(|b| *b == b'#').count();
            if hashes <= 6 && rest[hashes..].chars().next().is_none_or(|c| c == ' ' || c == '\t') {
                push(out, at, line.trim_end().len().max(at), Kind::Heading);
                return;
            }
        }
        let t = rest.trim_end();
        if !t.is_empty() && t.bytes().all(|b| b == b'=') {
            // A setext heading's underline.
            push(out, at, at + t.len(), Kind::Heading);
            return;
        }
        if is_rule(line) {
            push(out, at, line.trim_end().len(), Kind::Meta);
            return;
        }
        if rest.starts_with('>') {
            push(out, at, line.len(), Kind::Quote);
            return;
        }
        // A link's definition: [label]: url "title"
        if let Some(close) = rest.find("]:")
            && rest.starts_with('[')
            && !rest[1..close].contains(']')
            && close > 1
        {
            let label_end = at + close + 2;
            push(out, at, label_end, Kind::Link);
            let after = &line[label_end..];
            let skip = after.len() - after.trim_start().len();
            let url_start = label_end + skip;
            let url_end = line[url_start..].find(char::is_whitespace).map_or(line.len(), |k| url_start + k);
            push(out, url_start, url_end, Kind::AttrValue);
            if url_end < line.len() && !line[url_end..].trim().is_empty() {
                let title = url_end + (line[url_end..].len() - line[url_end..].trim_start().len());
                push(out, title, line.trim_end().len(), Kind::String);
            }
            return;
        }
        // A table's delimiter row: | --- | :-: |
        if t.contains('-') && t.contains('|') && t.bytes().all(|b| matches!(b, b'|' | b'-' | b':' | b' ' | b'\t')) {
            push(out, at, at + t.len(), Kind::Operator);
            return;
        }
        list_or_inline(line, at, out, state);
    }
}

/// A list item's marker (and task box), then the line's inline parts.
fn list_or_inline(line: &str, at: usize, out: &mut Vec<Span>, state: &mut State) {
    let b = line.as_bytes();
    let mut i = at + line[at..].bytes().take_while(|c| *c == b' ' || *c == b'\t').count();
    let marker_start = i;
    if i < b.len() && matches!(b[i], b'-' | b'*' | b'+') && b.get(i + 1).is_none_or(|c| *c == b' ' || *c == b'\t') {
        i += 1;
    } else {
        let digits = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        if (1..=9).contains(&digits)
            && matches!(b.get(i + digits), Some(b'.' | b')'))
            && b.get(i + digits + 1).is_none_or(|c| *c == b' ' || *c == b'\t')
        {
            i += digits + 1;
        }
    }
    if i > marker_start {
        // A task box after the marker: [ ] or [x]
        let rest = &line[i..];
        let box_len = rest.len() - rest.trim_start().len();
        let r = rest.trim_start();
        let end =
            if r.starts_with("[ ]") || r.starts_with("[x]") || r.starts_with("[X]") { i + box_len + 3 } else { i };
        push(out, marker_start, end, Kind::ListMarker);
        i = end;
    }
    let table = line[at..].trim_start().starts_with('|');
    inline_from(line, i, out, state, table);
}

fn inline(line: &str, from: usize, out: &mut Vec<Span>, state: &mut State) {
    inline_from(line, from, out, state, false);
}

/// Inline Markdown in `line[from..]`. `table`: the row's pipes are marked.
fn inline_from(line: &str, from: usize, out: &mut Vec<Span>, state: &mut State, table: bool) {
    let b = line.as_bytes();
    let n = b.len();
    let mut i = from;
    while i < n {
        let c = b[i];
        match c {
            b'\\' if i + 1 < n && b[i + 1].is_ascii_punctuation() => {
                push(out, i, i + 2, Kind::Escape);
                i += 2;
            }
            b'`' => {
                let run = b[i..].iter().take_while(|x| **x == b'`').count();
                match closing_run(b, i + run, b'`', run) {
                    Some(end) => {
                        push(out, i, end, Kind::Code);
                        i = end;
                    }
                    None => i += run,
                }
            }
            b'<' => {
                if line[i..].starts_with("<!--") {
                    match line[i + 4..].find("-->") {
                        Some(k) => {
                            push(out, i, i + 4 + k + 3, Kind::Comment);
                            i += 4 + k + 3;
                        }
                        None => {
                            push(out, i, n, Kind::Comment);
                            *state = State(vec![COMMENT]);
                            return;
                        }
                    }
                } else if let Some(end) = autolink_or_tag(line, i) {
                    push(out, i, end.0, end.1);
                    i = end.0;
                } else {
                    i += 1;
                }
            }
            b'[' | b'!' if c == b'[' || b.get(i + 1) == Some(&b'[') => match link(line, i) {
                Some(((text_end, end), kind_end)) => {
                    push(out, i, text_end, Kind::Link);
                    push(out, text_end, end, kind_end);
                    i = end;
                }
                None => i += 1,
            },
            b'*' | b'_' => {
                let run = b[i..].iter().take_while(|x| **x == c).count();
                match emphasis(line, i, c, run) {
                    Some(end) => {
                        push(out, i, end, if run >= 2 { Kind::Strong } else { Kind::Emphasis });
                        i = end;
                    }
                    None => i += run,
                }
            }
            b'|' if table => {
                push(out, i, i + 1, Kind::Operator);
                i += 1;
            }
            b'h' | b'w' if bare_url_start(line, i) => {
                let end = bare_url_end(line, i);
                push(out, i, end, Kind::Link);
                i = end;
            }
            _ => i += crate::char_len(c),
        }
    }
}

/// The end of a run of exactly `len` of `c` at or after `from` (a code
/// span's closing backticks).
fn closing_run(b: &[u8], from: usize, c: u8, len: usize) -> Option<usize> {
    let mut i = from;
    while i < b.len() {
        if b[i] == c {
            let run = b[i..].iter().take_while(|x| **x == c).count();
            if run == len {
                return Some(i + run);
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

/// Emphasis opened by `run` of `c` at `i`: where its closing run ends.
/// Opening needs a non-space after it; closing a non-space before; an
/// underscore inside a word is just a character (snake_case).
fn emphasis(line: &str, i: usize, c: u8, run: usize) -> Option<usize> {
    let b = line.as_bytes();
    let after = *b.get(i + run)?;
    if after.is_ascii_whitespace() {
        return None;
    }
    if c == b'_' && i > 0 && b[i - 1].is_ascii_alphanumeric() {
        return None;
    }
    let want = run.min(3);
    let mut j = i + run;
    while j < b.len() {
        if b[j] == b'`' {
            // A code span inside is skipped whole.
            let r = b[j..].iter().take_while(|x| **x == b'`').count();
            j = closing_run(b, j + r, b'`', r).unwrap_or(j + r);
            continue;
        }
        if b[j] == c {
            let r = b[j..].iter().take_while(|x| **x == c).count();
            let closes = !b[j - 1].is_ascii_whitespace()
                && (if want >= 3 { r >= 3 } else { r == want })
                && (c != b'_' || b.get(j + r).is_none_or(|x| !x.is_ascii_alphanumeric()));
            if closes {
                return Some(j + r);
            }
            j += r;
            continue;
        }
        j += crate::char_len(b[j]);
    }
    None
}

/// A link or image at `i`: `[text](url)`, `[text][ref]`, `[text]`,
/// `![alt](src)`. Returns where its text part ends and where it ends, and
/// the kind of the part after the text (the destination's).
fn link(line: &str, i: usize) -> Option<((usize, usize), Kind)> {
    let b = line.as_bytes();
    let open = if b[i] == b'!' { i + 1 } else { i };
    let mut depth = 0usize;
    let mut j = open;
    let close = loop {
        match b.get(j)? {
            b'\\' => j += 1,
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break j;
                }
            }
            _ => {}
        }
        j += 1;
    };
    if close == open + 1 {
        return None;
    }
    let text_end = close + 1;
    match b.get(text_end) {
        Some(b'(') => {
            let mut depth = 0usize;
            let mut k = text_end;
            loop {
                match b.get(k)? {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(((text_end, k + 1), Kind::AttrValue));
                        }
                    }
                    _ => {}
                }
                k += 1;
            }
        }
        Some(b'[') => {
            let k = line[text_end..].find(']')? + text_end;
            Some(((text_end, k + 1), Kind::Link))
        }
        // A shortcut reference or a footnote ([^1]): the text alone.
        _ if line[open..].starts_with("[^") || b[i] != b'!' => {
            if line[open..text_end].contains(' ') && !line[open..].starts_with("[^") {
                // [some words] alone is too often just brackets in prose.
                None
            } else {
                Some(((text_end, text_end), Kind::Link))
            }
        }
        _ => None,
    }
}

/// `<https://...>`, `<name@host>` (a link) or an HTML tag `<b>`, `</div>`,
/// `<img src="x">` at `i`: where it ends and its kind.
fn autolink_or_tag(line: &str, i: usize) -> Option<(usize, Kind)> {
    let end = line[i..].find('>')? + i + 1;
    let inner = &line[i + 1..end - 1];
    if inner.is_empty() || inner.contains('<') {
        return None;
    }
    if !inner.contains(' ') && (inner.contains("://") || inner.contains('@') && inner.contains('.')) {
        return Some((end, Kind::Link));
    }
    let name = inner.strip_prefix('/').unwrap_or(inner);
    if name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name.split([' ', '/', '\t']).next().is_some_and(|n| n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
    {
        return Some((end, Kind::Tag));
    }
    None
}

fn bare_url_start(line: &str, i: usize) -> bool {
    let r = &line[i..];
    (r.starts_with("https://") || r.starts_with("http://") || r.starts_with("www."))
        && (i == 0 || !line.as_bytes()[i - 1].is_ascii_alphanumeric())
}

/// A bare URL's end: before the space after it, less trailing punctuation
/// that ends the sentence rather than the address.
fn bare_url_end(line: &str, i: usize) -> usize {
    let mut end = line[i..].find(|c: char| c.is_whitespace() || c == '<').map_or(line.len(), |k| i + k);
    while end > i && line[..end].ends_with(['.', ',', ';', ':', '!', '?', ')', '\'', '"']) {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use crate::{Kind, State, by_name, highlight};

    fn spans(state: &mut State, line: &str) -> Vec<(String, Kind)> {
        let md = by_name("markdown").unwrap();
        highlight(md, state, line).into_iter().map(|s| (line[s.start..s.end].to_string(), s.kind)).collect()
    }

    /// A line in the middle of a document.
    fn mid(line: &str) -> Vec<(String, Kind)> {
        spans(&mut State(vec![super::NORMAL]), line)
    }

    fn has(v: &[(String, Kind)], text: &str, kind: Kind) -> bool {
        v.iter().any(|(t, k)| t == text && *k == kind)
    }

    #[test]
    fn headings_rules_quotes_and_lists() {
        assert!(has(&mid("## Where things stand"), "## Where things stand", Kind::Heading));
        assert!(mid("#hashtag").is_empty(), "no space: not a heading");
        assert!(has(&mid("======"), "======", Kind::Heading));
        assert!(has(&mid("* * *"), "* * *", Kind::Meta));
        assert!(has(&mid("> quoted words"), "> quoted words", Kind::Quote));
        let v = mid("- [x] done, with `code`");
        assert!(has(&v, "- [x]", Kind::ListMarker), "{v:?}");
        assert!(has(&v, "`code`", Kind::Code));
        assert!(has(&mid("  12. twelfth"), "12.", Kind::ListMarker));
        assert!(!has(&mid("2 * 3 = 6"), "*", Kind::ListMarker));
    }

    #[test]
    fn emphasis_strong_and_code() {
        let v = mid("an *italic* word, a **strong** one, and ***both***");
        assert!(has(&v, "*italic*", Kind::Emphasis), "{v:?}");
        assert!(has(&v, "**strong**", Kind::Strong));
        assert!(has(&v, "***both***", Kind::Strong));
        assert!(mid("snake_case_name and 2 * 3 * 4").is_empty(), "no emphasis inside words or around spaces");
        assert!(has(&mid("_under_ it"), "_under_", Kind::Emphasis));
        assert!(has(&mid("``a ` b``"), "``a ` b``", Kind::Code));
        assert!(has(&mid(r"\*not emphasis\*"), r"\*", Kind::Escape));
    }

    #[test]
    fn links_images_and_urls() {
        let v = mid("see [the docs](https://example.com/a_(b)) and ![logo](img.png)");
        assert!(has(&v, "[the docs]", Kind::Link), "{v:?}");
        assert!(has(&v, "(https://example.com/a_(b))", Kind::AttrValue));
        assert!(has(&v, "![logo]", Kind::Link));
        let v = mid("at https://github.com/gsa700/vergence. Or <me@example.com>");
        assert!(has(&v, "https://github.com/gsa700/vergence", Kind::Link), "{v:?}");
        assert!(has(&v, "<me@example.com>", Kind::Link));
        let v = mid("[ref]: https://example.com \"Title\"");
        assert!(has(&v, "[ref]:", Kind::Link));
        assert!(has(&v, "https://example.com", Kind::AttrValue));
        assert!(has(&mid("a <kbd>Ctrl</kbd> key"), "<kbd>", Kind::Tag));
        assert!(!mid("x < y and y > z").iter().any(|(_, k)| *k == Kind::Tag));
    }

    #[test]
    fn fences_color_their_language() {
        let mut st = State(vec![super::NORMAL]);
        assert!(has(&spans(&mut st, "```python"), "```python", Kind::Meta));
        let v = spans(&mut st, "def f(): return '''open");
        assert!(has(&v, "def", Kind::Keyword), "{v:?}");
        let v = spans(&mut st, "still the string ```");
        assert_eq!(v.len(), 1, "Python's own state is kept: {v:?}");
        let v = spans(&mut st, "''' # closed");
        assert!(has(&v, "# closed", Kind::Comment));
        assert!(has(&spans(&mut st, "```"), "```", Kind::Meta));
        assert!(has(&spans(&mut st, "*back* to prose"), "*back*", Kind::Emphasis));
        // A fence with no language known: plain code.
        let mut st = State(vec![super::NORMAL]);
        spans(&mut st, "~~~~");
        assert!(has(&spans(&mut st, "*not emphasis*"), "*not emphasis*", Kind::Code));
        assert!(has(&spans(&mut st, "~~~~~"), "~~~~~", Kind::Meta), "a longer fence closes it");
    }

    #[test]
    fn front_matter_comments_and_tables() {
        let mut st = State::default();
        assert!(has(&spans(&mut st, "---"), "---", Kind::Meta));
        assert!(has(&spans(&mut st, "title: Notes"), "title: Notes", Kind::Meta));
        spans(&mut st, "---");
        assert!(has(&spans(&mut st, "# Body"), "# Body", Kind::Heading));
        // Later, --- is a rule.
        assert!(has(&spans(&mut st, "---"), "---", Kind::Meta));
        assert_eq!(st, State(vec![super::NORMAL]));
        let mut st = State(vec![super::NORMAL]);
        spans(&mut st, "text <!-- a note");
        assert!(has(&spans(&mut st, "still noted --> *after*"), "*after*", Kind::Emphasis));
        assert!(has(&mid("| a | b |"), "|", Kind::Operator));
        assert!(has(&mid("|---|:-:|"), "|---|:-:|", Kind::Operator));
    }
}
