//! HTML, and PHP (HTML with `<?php ... ?>` in it): tags, attributes and their values,
//! entities, comments and the doctype; a `<script>`'s inside as JavaScript and a `<style>`'s as
//! CSS, each with its own state; PHP's code between `<?php` (or `<?=`, `<?`) and `?>`, anywhere,
//! even inside an attribute's value, on the C-family engine.
//!
//! The state between lines: HTML's (`[TEXT]`, `[TAG, quote, then]` a tag open across lines,
//! `[COMMENT]`, `[SCRIPT, js...]`, `[STYLE, css...]`), and for PHP while its code is open,
//! `[PHP, n, the HTML state (n words), PHP's own...]`: the HTML to go back to at `?>`.

use crate::clike::Rules;
use crate::{Highlight, Kind, Span, State, char_len, css, fill_source, js, push};

pub struct Html {
    /// `<?php` opens PHP's code (PHP's files); plain HTML leaves it as text.
    pub php: bool,
}

const TEXT: u32 = 1;
const TAG: u32 = 2;
const COMMENT: u32 = 3;
const SCRIPT: u32 = 4;
const STYLE: u32 = 5;
const PHP: u32 = 6;
/// XML's `<![CDATA[ ... ]]>`, open across lines.
const CDATA: u32 = 7;

/// What follows a tag's `>`: text, or a script's or style's inside.
const THEN_TEXT: u32 = 0;
const THEN_SCRIPT: u32 = 1;
const THEN_STYLE: u32 = 2;

/// Tag and attribute names, for completion.
pub const TAGS: &[&str] = &[
    "a",
    "abbr",
    "address",
    "article",
    "aside",
    "audio",
    "b",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "code",
    "col",
    "details",
    "dialog",
    "div",
    "dl",
    "dt",
    "dd",
    "em",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "i",
    "iframe",
    "img",
    "input",
    "kbd",
    "label",
    "legend",
    "li",
    "link",
    "main",
    "mark",
    "meta",
    "nav",
    "noscript",
    "ol",
    "optgroup",
    "option",
    "p",
    "picture",
    "pre",
    "progress",
    "script",
    "section",
    "select",
    "small",
    "source",
    "span",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "svg",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "title",
    "tr",
    "u",
    "ul",
    "video",
];
pub const ATTRIBUTES: &[&str] = &[
    "action",
    "alt",
    "aria-label",
    "async",
    "autocomplete",
    "autofocus",
    "charset",
    "checked",
    "class",
    "cols",
    "content",
    "crossorigin",
    "data-",
    "defer",
    "disabled",
    "download",
    "enctype",
    "for",
    "form",
    "height",
    "hidden",
    "href",
    "id",
    "lang",
    "loading",
    "max",
    "maxlength",
    "media",
    "method",
    "min",
    "multiple",
    "name",
    "onclick",
    "onchange",
    "oninput",
    "onload",
    "onsubmit",
    "pattern",
    "placeholder",
    "readonly",
    "rel",
    "required",
    "role",
    "rows",
    "selected",
    "sizes",
    "src",
    "srcset",
    "style",
    "tabindex",
    "target",
    "title",
    "type",
    "value",
    "width",
];

pub const PHP_RULES: Rules = Rules {
    keywords: &[
        "abstract",
        "and",
        "as",
        "break",
        "callable",
        "case",
        "catch",
        "class",
        "clone",
        "const",
        "continue",
        "declare",
        "default",
        "do",
        "echo",
        "else",
        "elseif",
        "empty",
        "enddeclare",
        "endfor",
        "endforeach",
        "endif",
        "endswitch",
        "endwhile",
        "enum",
        "extends",
        "final",
        "finally",
        "fn",
        "for",
        "foreach",
        "function",
        "global",
        "goto",
        "if",
        "implements",
        "include",
        "include_once",
        "instanceof",
        "insteadof",
        "interface",
        "isset",
        "list",
        "match",
        "namespace",
        "new",
        "or",
        "print",
        "private",
        "protected",
        "public",
        "readonly",
        "require",
        "require_once",
        "return",
        "static",
        "switch",
        "throw",
        "trait",
        "try",
        "unset",
        "use",
        "var",
        "while",
        "xor",
        "yield",
        "die",
        "exit",
        "array",
    ],
    constants: &[
        "true",
        "false",
        "null",
        "__CLASS__",
        "__DIR__",
        "__FILE__",
        "__FUNCTION__",
        "__LINE__",
        "__METHOD__",
        "__NAMESPACE__",
        "PHP_EOL",
        "PHP_VERSION",
        "PHP_INT_MAX",
        "E_ALL",
        "E_ERROR",
        "E_WARNING",
    ],
    types: &["int", "float", "string", "bool", "object", "mixed", "void", "never", "iterable", "self", "parent"],
    builtins: &[
        "strlen",
        "str_replace",
        "str_contains",
        "str_starts_with",
        "str_ends_with",
        "substr",
        "strpos",
        "strrpos",
        "explode",
        "implode",
        "sprintf",
        "printf",
        "trim",
        "rtrim",
        "ltrim",
        "strtolower",
        "strtoupper",
        "ucfirst",
        "nl2br",
        "htmlspecialchars",
        "htmlentities",
        "strip_tags",
        "number_format",
        "count",
        "in_array",
        "array_key_exists",
        "array_keys",
        "array_values",
        "array_merge",
        "array_map",
        "array_filter",
        "array_slice",
        "array_search",
        "array_push",
        "array_pop",
        "sort",
        "usort",
        "ksort",
        "json_encode",
        "json_decode",
        "is_array",
        "is_string",
        "is_int",
        "is_numeric",
        "is_null",
        "intval",
        "floatval",
        "date",
        "time",
        "strtotime",
        "mktime",
        "preg_match",
        "preg_replace",
        "preg_split",
        "file_get_contents",
        "file_put_contents",
        "file_exists",
        "fopen",
        "fclose",
        "fwrite",
        "var_dump",
        "print_r",
        "header",
        "setcookie",
        "session_start",
        "define",
        "defined",
        "function_exists",
        "class_exists",
        "method_exists",
        "password_hash",
        "password_verify",
        "filter_var",
        "urlencode",
        "rawurlencode",
        "base64_encode",
        "md5",
        "hash",
        "random_int",
        "mail",
        "error_log",
        "ini_set",
        "mysqli_connect",
        "PDO",
    ],
    any_case: true,
    hash_comments: true,
    dollar_vars: true,
    multiline_quotes: true,
    templates: false,
    regex: false,
    interpolate: true,
    heredoc: true,
    php_close: true,
};

impl Highlight for Html {
    fn line(&self, state: &mut State, line: &str, out: &mut Vec<Span>) {
        let mut st = state.0.clone();
        if st.is_empty() {
            st = vec![TEXT];
        }
        let mut pos = 0;
        while pos <= line.len() {
            if st.first() == Some(&PHP) {
                let n = st.get(1).copied().unwrap_or(0) as usize;
                let (html, mut code) = (st[2..2 + n.min(st.len() - 2)].to_vec(), st[(2 + n).min(st.len())..].to_vec());
                let first = out.len();
                let closed = PHP_RULES.run(&mut code, line, pos, line.len(), out);
                fill_source(out, first, pos, closed.unwrap_or(line.len()));
                match closed {
                    Some(k) => {
                        push(out, k, k + 2, Kind::Meta);
                        st = if html.is_empty() { vec![TEXT] } else { html };
                        pos = k + 2;
                        continue;
                    }
                    None => {
                        let mut s = vec![PHP, n as u32];
                        s.extend(html);
                        s.extend(code);
                        st = s;
                        break;
                    }
                }
            }
            // The HTML up to PHP's next opening, if PHP is on.
            let open = if self.php { line[pos..].find("<?").map(|k| pos + k) } else { None };
            let end = open.unwrap_or(line.len());
            html_run(&mut st, line, pos, end, out);
            let Some(k) = open else { break };
            let tag = if line[k..].starts_with("<?php") {
                5
            } else if line[k..].starts_with("<?=") {
                3
            } else {
                2
            };
            push(out, k, k + tag, Kind::Meta);
            let mut s = vec![PHP, st.len() as u32];
            s.extend(st);
            st = s;
            pos = k + tag;
        }
        state.0 = st;
    }
}

/// HTML in `line[from..to]` from and to `st`.
fn html_run(st: &mut Vec<u32>, line: &str, from: usize, to: usize, out: &mut Vec<Span>) {
    let b = line.as_bytes();
    let mut i = from;
    while i < to {
        match st.first().copied().unwrap_or(TEXT) {
            CDATA => match line[i..to].find("]]>") {
                Some(k) => {
                    push(out, i, i + k + 3, Kind::String);
                    i += k + 3;
                    *st = vec![TEXT];
                }
                None => {
                    push(out, i, to, Kind::String);
                    return;
                }
            },
            COMMENT => match line[i..to].find("-->") {
                Some(k) => {
                    push(out, i, i + k + 3, Kind::Comment);
                    i += k + 3;
                    *st = vec![TEXT];
                }
                None => {
                    push(out, i, to, Kind::Comment);
                    return;
                }
            },
            SCRIPT | STYLE => {
                let script = st[0] == SCRIPT;
                let close = if script { "</script" } else { "</style" };
                let lower = line[i..to].to_ascii_lowercase();
                let end = lower.find(close).map_or(to, |k| i + k);
                let mut inner = st[1..].to_vec();
                let first = out.len();
                if script {
                    js::RULES.run(&mut inner, line, i, end, out);
                } else {
                    css::run(&mut inner, line, i, end, out);
                }
                fill_source(out, first, i, end);
                if end < to {
                    *st = vec![TEXT];
                    i = end;
                } else {
                    let mut s = vec![st[0]];
                    s.extend(inner);
                    *st = s;
                    return;
                }
            }
            TAG => i = tag_inside(st, line, i, to, out),
            _ => {
                let c = b[i];
                if c == b'&' {
                    // An entity: &amp; &#8212; &#x2014;
                    let end = line[i..to]
                        .char_indices()
                        .skip(1)
                        .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || *ch == '#')
                        .last()
                        .map(|(k, ch)| i + k + ch.len_utf8());
                    if let Some(e) = end
                        && b.get(e) == Some(&b';')
                    {
                        push(out, i, e + 1, Kind::Escape);
                        i = e + 1;
                        continue;
                    }
                    i += 1;
                } else if c == b'<' {
                    if line[i..to].starts_with("<!--") {
                        push(out, i, i + 4, Kind::Comment);
                        i += 4;
                        *st = vec![COMMENT];
                    } else if line[i..to].starts_with("<![CDATA[") {
                        // XML's raw text: as a string, to its ]]>.
                        match line[i + 9..to].find("]]>") {
                            Some(k) => {
                                push(out, i, i + 9 + k + 3, Kind::String);
                                i += 9 + k + 3;
                            }
                            None => {
                                push(out, i, to, Kind::String);
                                *st = vec![CDATA];
                                return;
                            }
                        }
                    } else if line[i..to].starts_with("<!") {
                        let end = line[i..to].find('>').map_or(to, |k| i + k + 1);
                        push(out, i, end, Kind::Meta);
                        i = end;
                    } else if line[i..to].starts_with("<?") {
                        // A processing instruction (<?xml version="1.0"?>), where PHP is not on.
                        let end = line[i..to].find("?>").map_or(to, |k| i + k + 2);
                        push(out, i, end, Kind::Meta);
                        i = end;
                    } else {
                        let closing = b.get(i + 1) == Some(&b'/');
                        let ns = i + 1 + closing as usize;
                        let mut ne = ns;
                        while ne < to && (b[ne].is_ascii_alphanumeric() || b[ne] == b'-' || b[ne] == b':') {
                            ne += 1;
                        }
                        if ne > ns && b[ns].is_ascii_alphabetic() {
                            push(out, i, ne, Kind::Tag);
                            let name = line[ns..ne].to_ascii_lowercase();
                            let then = match name.as_str() {
                                "script" if !closing => THEN_SCRIPT,
                                "style" if !closing => THEN_STYLE,
                                _ => THEN_TEXT,
                            };
                            *st = vec![TAG, 0, then];
                            i = ne;
                        } else {
                            i += 1;
                        }
                    }
                } else {
                    i += char_len(c);
                }
            }
        }
    }
}

/// Inside a tag: attributes and values, to its `>`.
fn tag_inside(st: &mut Vec<u32>, line: &str, mut i: usize, to: usize, out: &mut Vec<Span>) -> usize {
    let b = line.as_bytes();
    let then = st.get(2).copied().unwrap_or(THEN_TEXT);
    let quote = st.get(1).copied().unwrap_or(0) as u8;
    if quote != 0 {
        // A value open from the line before (or before PHP's part).
        match line[i..to].find(quote as char) {
            Some(k) => {
                push(out, i, i + k + 1, Kind::AttrValue);
                st[1] = 0;
                return i + k + 1;
            }
            None => {
                push(out, i, to, Kind::AttrValue);
                return to;
            }
        }
    }
    let c = b[i];
    match c {
        b'>' => {
            push(out, i, i + 1, Kind::Tag);
            *st = match then {
                THEN_SCRIPT => vec![SCRIPT],
                THEN_STYLE => vec![STYLE, 0, 0],
                _ => vec![TEXT],
            };
            i + 1
        }
        b'/' if b.get(i + 1) == Some(&b'>') => {
            push(out, i, i + 2, Kind::Tag);
            *st = vec![TEXT];
            i + 2
        }
        b'=' => {
            push(out, i, i + 1, Kind::Operator);
            i + 1
        }
        b'"' | b'\'' => match line[i + 1..to].find(c as char) {
            Some(k) => {
                push(out, i, i + 1 + k + 1, Kind::AttrValue);
                i + 1 + k + 1
            }
            None => {
                push(out, i, to, Kind::AttrValue);
                st[1] = c as u32;
                to
            }
        },
        _ if c.is_ascii_whitespace() => i + 1,
        _ => {
            // A name, or an unquoted value after `=`.
            let start = i;
            while i < to && !b[i].is_ascii_whitespace() && !matches!(b[i], b'>' | b'=' | b'"' | b'\'') {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'>') {
                    break;
                }
                i += char_len(b[i]);
            }
            let i = i.min(to).max(start + 1);
            let after_eq = line[..start].trim_end().ends_with('=');
            push(out, start, i, if after_eq { Kind::AttrValue } else { Kind::Attribute });
            i
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Kind, State, by_name, highlight};

    fn spans(lang: &str, state: &mut State, line: &str) -> Vec<(String, Kind)> {
        let l = by_name(lang).unwrap();
        highlight(l, state, line).into_iter().map(|s| (line[s.start..s.end].to_string(), s.kind)).collect()
    }

    fn has(v: &[(String, Kind)], text: &str, kind: Kind) -> bool {
        v.iter().any(|(t, k)| t == text && *k == kind)
    }

    #[test]
    fn tags_attributes_entities_and_comments() {
        let mut st = State::default();
        let v = spans("html", &mut st, r#"<!DOCTYPE html><a href="/x" class=big hidden>Tom &amp; Jerry</a> <!-- note"#);
        assert!(has(&v, "<!DOCTYPE html>", Kind::Meta), "{v:?}");
        assert!(has(&v, "<a", Kind::Tag));
        assert!(has(&v, "href", Kind::Attribute));
        assert!(has(&v, "\"/x\"", Kind::AttrValue));
        assert!(has(&v, "big", Kind::AttrValue));
        assert!(has(&v, "hidden", Kind::Attribute));
        assert!(has(&v, "&amp;", Kind::Escape));
        assert!(has(&v, "</a>", Kind::Tag));
        let v = spans("html", &mut st, "still a note --> <br/>");
        assert!(has(&v, "still a note -->", Kind::Comment), "{v:?}");
        assert!(has(&v, "<br/>", Kind::Tag));
        assert!(spans("html", &mut State::default(), "x < y and 5>3").iter().all(|(_, k)| *k != Kind::Tag));
    }

    #[test]
    fn a_tag_and_a_value_across_lines() {
        let mut st = State::default();
        spans("html", &mut st, "<img src=\"a.png\"");
        let v = spans("html", &mut st, "     alt=\"two");
        assert!(has(&v, "alt", Kind::Attribute), "{v:?}");
        let v = spans("html", &mut st, "lines\">text");
        assert!(has(&v, "lines\"", Kind::AttrValue), "{v:?}");
        assert!(has(&v, ">", Kind::Tag));
    }

    #[test]
    fn script_and_style_insides() {
        let mut st = State::default();
        let v = spans("html", &mut st, "<script type=\"module\">const n = 1; // js");
        assert!(has(&v, "const", Kind::Keyword), "{v:?}");
        assert!(has(&v, "// js", Kind::Comment));
        let v = spans("html", &mut st, "if (n) {} </script> <p>");
        assert!(has(&v, "if", Kind::Keyword), "{v:?}");
        assert!(has(&v, "</script>", Kind::Tag));
        assert!(has(&v, "<p>", Kind::Tag));
        let v = spans("html", &mut st, "<style>p { color: red }</style>");
        assert!(has(&v, "color", Kind::Attribute), "{v:?}");
        assert!(has(&v, "</style>", Kind::Tag));
    }

    #[test]
    fn php_inside_html() {
        let mut st = State::default();
        let v = spans("php", &mut st, r#"<a href="<?= $url ?>"><?php if ($n > 0): echo "Hi $name"; // greet ?></a>"#);
        assert!(has(&v, "<?=", Kind::Meta), "{v:?}");
        assert!(has(&v, "$url", Kind::Variable));
        assert!(has(&v, "?>", Kind::Meta));
        assert!(has(&v, "if", Kind::Keyword));
        assert!(has(&v, "$name", Kind::Variable), "interpolated");
        assert!(has(&v, "// greet ", Kind::Comment), "a line comment ends at ?>");
        assert!(has(&v, "</a>", Kind::Tag));
        // Code open across lines, then back to the HTML it came from.
        let mut st = State::default();
        spans("php", &mut st, "<ul><?php");
        let v = spans("php", &mut st, "  function item(string $t): string { return strtoupper($t); }");
        assert!(has(&v, "function", Kind::Keyword), "{v:?}");
        assert!(has(&v, "item", Kind::Function));
        assert!(has(&v, "string", Kind::Type));
        assert!(has(&v, "strtoupper", Kind::Builtin));
        let v = spans("php", &mut st, "?><li>done</li>");
        assert!(has(&v, "?>", Kind::Meta));
        assert!(has(&v, "<li>", Kind::Tag));
    }

    #[test]
    fn php_keywords_on_their_own_lines() {
        let mut st = State::default();
        spans("php", &mut st, "<?php");
        spans("php", &mut st, "/** Docs. */");
        let v = spans("php", &mut st, "declare(strict_types=1);");
        assert!(has(&v, "declare", Kind::Keyword), "{v:?}");
        let v = spans("php", &mut State::default(), "  <?php foreach ($rows as $r): ?>");
        assert!(has(&v, "foreach", Kind::Keyword), "{v:?}");
        assert!(has(&v, "as", Kind::Keyword), "{v:?}");
    }

    /// Code inside a page is marked as code throughout, so its plain words
    /// are not taken for the page's prose; text between tags is prose.
    #[test]
    fn embedded_code_is_not_prose() {
        let php = by_name("php").unwrap();
        let line = "<p>Some words <?php echo watts($o); ?> more words</p>";
        let s = highlight(php, &mut State::default(), line);
        let at = |w: &str| crate::kind_at(&s, line.find(w).unwrap());
        assert!(!crate::is_prose_in(php, at("watts")), "{s:?}");
        assert!(crate::is_prose_in(php, at("Some")));
        assert!(crate::is_prose_in(php, at("more")));
        let md = by_name("markdown").unwrap();
        let mut st = State(vec![1]);
        highlight(md, &mut st, "```python");
        let line = "result = compute(value)";
        let s = highlight(md, &mut st, line);
        assert!(!crate::is_prose_in(md, crate::kind_at(&s, 0)), "{s:?}");
    }

    #[test]
    fn xml_declarations_and_cdata() {
        let mut st = State::default();
        let v = spans("xml", &mut st, "<?xml version='1.0' encoding='UTF-8'?>");
        assert!(has(&v, "<?xml version='1.0' encoding='UTF-8'?>", Kind::Meta), "{v:?}");
        let v = spans("xml", &mut st, r#"<Audible type="i" default="1">1</Audible><![CDATA[a < b"#);
        assert!(has(&v, "<Audible", Kind::Tag), "{v:?}");
        assert!(has(&v, "default", Kind::Attribute));
        let v = spans("xml", &mut st, "and c ]]><x/>");
        assert!(has(&v, "and c ]]>", Kind::String), "{v:?}");
        assert!(has(&v, "<x/>", Kind::Tag));
        assert_eq!(crate::detect(None, "<?xml version=\"1.0\"?>"), by_name("xml"));
    }

    #[test]
    fn php_heredocs_and_case() {
        let mut st = State::default();
        spans("php", &mut st, "<?php $page = <<<HTML");
        let v = spans("php", &mut st, "  <b>$title</b>");
        assert!(has(&v, "$title", Kind::Variable), "{v:?}");
        let v = spans("php", &mut st, "HTML; ECHO TRUE;");
        assert!(has(&v, "HTML", Kind::String), "{v:?}");
        assert!(has(&v, "ECHO", Kind::Keyword), "any case");
        assert!(has(&v, "TRUE", Kind::Constant));
    }
}
