//! JavaScript (and TypeScript's common words), on the C-family engine (clike.rs): its own file,
//! and an HTML page's `<script>`.

use crate::clike::Rules;
use crate::{Highlight, Span, State};

pub const RULES: Rules = Rules {
    keywords: &[
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "default",
        "delete",
        "do",
        "else",
        "export",
        "extends",
        "finally",
        "for",
        "from",
        "function",
        "get",
        "if",
        "import",
        "in",
        "instanceof",
        "let",
        "new",
        "of",
        "return",
        "set",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "try",
        "typeof",
        "var",
        "void",
        "while",
        "with",
        "yield",
        "as",
        "type",
        "interface",
        "implements",
        "enum",
        "declare",
        "readonly",
        "private",
        "protected",
        "public",
        "abstract",
        "namespace",
        "keyof",
    ],
    constants: &["true", "false", "null", "undefined", "NaN", "Infinity"],
    types: &["string", "number", "boolean", "any", "unknown", "never", "object", "bigint", "symbol"],
    builtins: &[
        "console",
        "window",
        "document",
        "navigator",
        "location",
        "history",
        "localStorage",
        "sessionStorage",
        "globalThis",
        "Math",
        "JSON",
        "Object",
        "Array",
        "String",
        "Number",
        "Boolean",
        "Promise",
        "Map",
        "Set",
        "WeakMap",
        "WeakSet",
        "Date",
        "RegExp",
        "Error",
        "TypeError",
        "Symbol",
        "BigInt",
        "Intl",
        "Reflect",
        "Proxy",
        "parseInt",
        "parseFloat",
        "isNaN",
        "isFinite",
        "setTimeout",
        "setInterval",
        "clearTimeout",
        "clearInterval",
        "requestAnimationFrame",
        "fetch",
        "alert",
        "confirm",
        "prompt",
        "require",
        "module",
        "exports",
        "process",
        "arguments",
        "encodeURIComponent",
        "decodeURIComponent",
        "structuredClone",
        "queueMicrotask",
    ],
    any_case: false,
    hash_comments: false,
    dollar_vars: false,
    multiline_quotes: false,
    templates: true,
    regex: true,
    interpolate: false,
    heredoc: false,
    php_close: false,
};

pub struct JavaScript;

impl Highlight for JavaScript {
    fn line(&self, state: &mut State, line: &str, out: &mut Vec<Span>) {
        let mut st = state.0.clone();
        RULES.run(&mut st, line, 0, line.len(), out);
        state.0 = st;
    }
}

#[cfg(test)]
mod tests {
    use crate::{Kind, State, by_name, highlight};

    fn spans(state: &mut State, line: &str) -> Vec<(String, Kind)> {
        let js = by_name("javascript").unwrap();
        highlight(js, state, line).into_iter().map(|s| (line[s.start..s.end].to_string(), s.kind)).collect()
    }

    fn has(v: &[(String, Kind)], text: &str, kind: Kind) -> bool {
        v.iter().any(|(t, k)| t == text && *k == kind)
    }

    #[test]
    fn words_strings_templates_and_regexes() {
        let mut st = State::default();
        let v = spans(&mut st, "export async function load(url) { // fetch it");
        assert!(has(&v, "export", Kind::Keyword));
        assert!(has(&v, "load", Kind::Function));
        assert!(has(&v, "// fetch it", Kind::Comment));
        let v = spans(&mut st, "const re = /a[/]b\\//gi, half = total / 2 / 3;");
        assert!(has(&v, "/a[/]b\\//gi", Kind::String), "{v:?}");
        assert!(has(&v, "2", Kind::Number));
        assert!(!v.iter().any(|(t, k)| t.starts_with("/ 2") && *k == Kind::String), "division is not a regex");
        let v = spans(&mut st, "msg = `Hi ${user.name}, ${count + 1} new`;");
        assert!(has(&v, "${", Kind::Operator), "{v:?}");
        assert!(has(&v, "1", Kind::Number));
        let v = spans(&mut st, "obj.Math.max(null, new Shape())");
        assert!(!has(&v, "Math", Kind::Builtin), "after a dot: {v:?}");
        assert!(has(&v, "null", Kind::Constant));
        assert!(has(&v, "Shape", Kind::Type));
    }

    #[test]
    fn templates_and_block_comments_run_across_lines() {
        let mut st = State::default();
        spans(&mut st, "/** Docs");
        assert!(has(&spans(&mut st, " * still docs */ let x;"), " * still docs */", Kind::DocComment));
        spans(&mut st, "const html = `<p>");
        let v = spans(&mut st, "  inside</p>`; let y = 2;");
        assert!(has(&v, "  inside</p>`", Kind::String), "{v:?}");
        assert!(has(&v, "let", Kind::Keyword));
    }
}
