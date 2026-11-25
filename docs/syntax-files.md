# Syntax files: adding a language to Edit's colors

Edit colors a file by its language. Python, Markdown, HTML, XML, PHP,
JavaScript and CSS are built in (vergence-syntax); every other language is
described in a small TOML file, read by one rule-driven highlighter
(`crates/vergence-syntax/src/rules.rs`). Adding a language is writing a
file, not code (Dave, 2026-11-24: "build so they can be added").

## Where the files live

- **Shipped**: `$prefix/share/vergence/syntax/` (from the repository's
  `data/syntax/`): Shell, Rust, TOML, JSON, C (and C++), YAML, INI.
- **Your own**: `~/.config/vergence/syntax/` (or `$XDG_CONFIG_HOME`'s).
  A file whose `name` is a language's that already exists replaces it,
  built-in ones too; any other adds a language.

Files are read once when an app starts: open a new Edit window (or restart
Edit) after adding one. A file that is wrong is skipped, and the reason is
printed on Edit's standard error (`vergence-syntax: <file>: <why> (skipped)`).
Limits: 256 KB a file, 200 files, 5,000 words a list.

## An example

```toml
# ~/.config/vergence/syntax/lua.toml
name = "Lua"
extensions = ["lua"]
interpreters = ["lua", "luajit"]
line-comments = ["--"]
block-comments = [["--[[", "]]"]]
function-after = ["function"]
operators = "=+-*/%<>~#"
keywords = ["and", "break", "do", "else", "elseif", "end", "for", "function", "goto", "if", "in",
            "local", "not", "or", "repeat", "return", "then", "until", "while"]
constants = ["true", "false", "nil"]
builtins = ["print", "pairs", "ipairs", "require", "tostring", "tonumber", "type", "table", "string", "math"]

[[strings]]
open = "\""

[[strings]]
open = "'"

[[strings]]
open = "[["
close = "]]"
escape = ""
multiline = true
```

## Every key

Which files are this language (checked in this order: whole name,
extension, `#!` line, first line):

| Key | Meaning |
|---|---|
| `name` | The language's name, shown in the status line and View > Language. Required. |
| `filenames` | Whole file names: `[".zshrc", "PKGBUILD"]`. |
| `extensions` | Extensions without the dot, any case: `["yaml", "yml"]`. |
| `interpreters` | Programs a `#!` line names (`#!/usr/bin/env lua`): `["lua"]`. |
| `first-lines` | A first line starting with one of these, any case: `["<?xml"]`. |

Words (each a list of strings; completion offers them in code):

| Key | Colored as |
|---|---|
| `keywords` | keywords |
| `types` | type names |
| `builtins` | names the language provides |
| `constants` | `true`, `null` and the like |
| `any-case` | `true`: the words match in any case (SQL, PHP). |

Comments:

| Key | Meaning |
|---|---|
| `line-comments` | Markers that comment to the line's end: `["#", "//"]`. |
| `block-comments` | Pairs: `[["/*", "*/"]]`; may run over lines. |
| `nested-comments` | `true`: block comments nest (Rust). |
| `doc-comments` | A comment starting with one of these is documentation: `["///", "/**"]`. |

Strings, one `[[strings]]` table each (the longest opening is tried first):

| Key | Meaning |
|---|---|
| `open` | What opens it: `"\""`, `"'''"`, `"r#\""`. Required. |
| `close` | What closes it (default: the same as `open`). |
| `escape` | The escape character (default `"\\"`; `""` for none, as in raw strings). |
| `multiline` | `true`: it may run over lines. |
| `variables` | `true`: `$name`, `${...}`, `$1` inside are marked (shell's double quotes). |
| `max-chars` | Not a string unless it closes within this many characters: Rust's `'x'` and its lifetimes `'a` (default 0, no limit). |

The rest:

| Key | Meaning |
|---|---|
| `variables` | `true`: `$name`, `${...}`, `$1` in code are variables (shell). |
| `numbers` | Numbers colored (default `true`). |
| `operators` | Characters colored as operators: `"=+-*/<>!&\|"`. |
| `function-after` | The name after one of these words is a function being defined: `["fn"]`. |
| `type-after` | The name after one of these is a type being defined: `["struct", "class"]`. |
| `attributes` | Pairs marking attributes, colored as such: `[["#[", "]"]]`. |
| `meta-lines` | A line starting with one of these is a preprocessor line: `["#"]` (C). |
| `keys` | At a line's start, a name before this is a key: `"="` (TOML, INI), `":"` (YAML). |
| `sections` | At a line's start, a header between these: `[["[", "]"]]`. |
| `string-keys` | A string followed by this is a key: `":"` (JSON). |
| `indent-after` | Return after a line ending in one of these indents a level more: `"{[("`, `":"`. |
| `prose` | `true`: the language's plain text is prose, spelled and completed from the dictionary (a markup language). In code only comments and strings are. |

## Testing a file

Open a file of the language in Edit: the status line names the language,
and View > Language lists it. If it is missing, run Edit from a terminal
(`vergence-edit file.lua`) to see why the definition was skipped. The
shipped files are tested with the rest (rules.rs's tests, and the fuzz
test that feeds every language damage).
