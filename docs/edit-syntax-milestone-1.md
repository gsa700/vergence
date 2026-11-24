# Edit syntax milestone 1: syntax colors by file type, and each language's words in completion

**Drafted 2026-11-24**, from Dave's wish list (Vergence-wishlist-2, Edit and
Terminal): "Syntax highlighting in Edit by file type, with completion of
each language's words. Start with .md .html .php .py". After the editor
(docs/editor-milestone-1.md); his priority when Edit's turn came back
(techbench, 2026-11-23: "Edit is coming along nicely", next: syntax
highlighting and code autocomplete).

**Accepted 2026-11-24** (Dave): "Yes to all 8 on your leans", and on the
languages: "The languages were to start, we would maybe want others. Maybe
build so they can be added?" So languages are pluggable (design §1, "Adding
languages"): built-in ones behind one interface, and simple ones described
in definition files anyone can add without building Vergence.

## Where things stand

- **The text area draws each line in one color** (vergence-widgets
  textarea.rs `draw`): the line is laid out once in one face
  (`shape_line`, cached per line in `shaped`), then drawn a piece between
  tabs at a time in `theme.text`. Over it: the selection, Find's marks, the
  current line's shade, red waves under words spelled wrong.
- **Per-line caches already follow edits**: `spelled` (words spelled wrong)
  is kept per line and cleared from the edited lines on each change
  (`splice`), so a syntax cache can work the same way.
- **Completion** (step 6 of the editor milestone) offers words of the open
  documents, then dictionary words, by prefix (`Doc::completions`); the
  ghost after the caret is taken with Tab or Space.
- **Spelling** checks every word on every line, so in code identifiers get
  red waves (`def`, `elif`, `isinstance`, `<div>`): noise in a code file.
- **The font**: Edit's text is the desktop's monospace font unless its
  settings name another (settings.rs). One face per area: bold or italic
  pieces would need laying out with their own faces.
- **Nothing knows a file's language**: no detection, no menu, no status.

## Design

### 1. A highlighter crate of our own (`vergence-syntax`, no UI)

- **Line by line, with a state carried between lines**: `fn line(&mut
  State, &str) -> Vec<Span>`, a span being a byte range and a kind. The
  state is small and comparable (inside a block comment, a triple-quoted
  string, a Markdown code fence and its language, PHP inside HTML, `<script>`
  inside HTML...). A line is worked out from the state the line before it
  ended in; after an edit, lines are worked out again from the edited one
  until a line ends in the same state as before (usually at once). That
  keeps typing cheap in a long file.
- **Kinds, not colors**: keyword, type/builtin, function name, string,
  escape, number, comment, doc comment, operator/punctuation, tag,
  attribute, attribute value, heading, emphasis, strong, code, link,
  quote, list marker, preprocessor/PHP tag. The look decides the colors.
- **Hand-written tokenizers**, one module a language, each a small state
  machine with its keyword and builtin lists (the "words of the language",
  which completion uses too). The value is in keywords, strings, comments,
  numbers and markup structure; a full parser (tree-sitter) would be
  borrowing (question 1).
- **Languages, first four (Dave's)**:
  - **Python**: keywords, builtins, decorators, strings of every prefix
    (`r`, `b`, `f`, triple quotes across lines), f-string braces, numbers,
    comments, `def`/`class` names.
  - **Markdown**: headings, emphasis and strong, code spans, fenced code
    blocks highlighted in their language (```` ```python ````), links and
    images, quotes, list markers, rules, front matter.
  - **HTML**: tags, attributes and values, entities, comments, doctype;
    `<script>` as JavaScript and `<style>` as CSS inside it (so JavaScript
    and CSS come with it, as languages of their own too).
  - **PHP**: `<?php ... ?>` inside HTML, variables, keywords, the common
    functions as builtins, strings with `$var` inside, heredocs, comments.
- **Adding languages** (Dave, 2026-11-24): two ways, both through one
  registry (`vergence_syntax::languages()`), so the text area and Edit never
  name a language themselves:
  - **Built in**: a module implementing the `Highlighter` interface (its
    state type, `line()`, its words, its extensions and first-line
    patterns), registered in one list. For languages with structure:
    Markdown, HTML, PHP, and any that need more than rules.
  - **Definition files**: a small TOML file per language in
    `$prefix/share/vergence/syntax/` (shipped) or
    `~/.config/vergence/syntax/` (the person's, overriding a shipped one by
    name): its name, extensions and first-line patterns, keyword and
    builtin lists, line and block comment markers, string quotes (with
    escapes, raw and multi-line forms), number style, and which delimiters
    nest. One general rule-driven highlighter reads it. Most languages
    (shell, Rust, C, Go, Lua, TOML, INI, YAML, SQL, Makefile...) fit this,
    so adding one is writing a file, not code. A file is untrusted input:
    read with limits, a bad one skipped with a note in the log.
  - Python could be either; it is built in (f-strings, prefixes) and also
    serves as the test of the interface.
- **Then, cheaply, since the pieces exist**: JavaScript and CSS on their
  own, JSON, shell, Rust, TOML/INI, C (question 6).
- **Which language**: by the file's extension; else its first line (`#!`
  naming python, php, sh, bash, zsh; `<?php`; `<!DOCTYPE html>`); else
  plain text. **View > Language** chooses another for the open document
  (question 7). Fuzz tests feed every tokenizer damage, as vergence-text's
  are fed (no panics, spans inside the line, in order).

### 2. Colors in the look (`vergence-look`)

- A syntax palette for light and for dark, a color per kind, each at least
  4.5:1 against the page (tested, as the look's inks are). Muted to match
  the desktop: comments a quiet gray, strings green, keywords the accent's
  family, numbers and constants warm (question 2).
- Markdown's headings bold, emphasis italic, strong bold, code in the
  string color on a faint ground (question 3). Bold and italic pieces are
  laid out in their own faces (the monospace family's bold and italic),
  so `shape_line` lays out by spans.

### 3. The text area draws spans

- `AreaOptions.syntax: Option<Language>`; the area keeps each line's spans
  and end state beside `shaped` and `spelled`, worked out for the lines on
  screen (and from the last known state above them), cleared from an edited
  line as `spelled` is.
- Drawing: each row's text in pieces by span (and by tab, as now), in its
  kind's color and face. Selection, marks and waves draw as they do.
- **Spelling in code** checks only comments, strings and (in Markdown) the
  prose, not code spans or fences (question 4).

### 4. Completion: each language's words

- In code (not in a comment or string): the language's keywords and
  builtins, then names from the open documents; no English dictionary
  words there. In comments, strings and Markdown prose: as today (open
  documents, then the dictionary). HTML: tag names after `<`, attribute
  names inside a tag (question 5).
- The ghost and the list stay as they are (Tab or Space takes it, Escape
  lets it go). Space taking a completion is the risky part in code (`in`,
  `is`, `if` typed before a space): it keeps today's rule, a word that is
  a word of its own (here: a keyword) is never replaced.

### 5. Edit

- Detects the language on open and on Save As (a new name may mean a new
  language); the status bar shows it beside the line and column ("Python"),
  and a click on it opens the language list.
- View > Language: the list, Plain Text first.
- Settings: **Syntax colors** on/off (on by default), and the existing
  Spellcheck and Autocomplete keep their own switches.

## Steps

1. `vergence-syntax`: the span and state types, the `Highlighter`
   interface and the registry, Python built in, its tests and fuzz test.
2. The text area draws spans (colors only), the per-line cache and its
   invalidation; the palette in vergence-look with the contrast tests.
3. Edit: detection, the status, View > Language, the Syntax colors switch.
   **First look for Dave: Python in Edit.**
4. Markdown (with fences highlighting their languages), bold and italic
   faces in the text area.
5. HTML with JavaScript and CSS; PHP in HTML.
6. Spelling in code limited to comments, strings and prose; completion with
   the language's words.
7. Definition files: the rule-driven highlighter, the TOML format
   (documented in docs/syntax-files.md), loading from the shipped and the
   person's directories; then the next languages as files (question 6:
   JavaScript and CSS stay built in with HTML; shell, JSON, Rust, TOML, C
   as files).
8. Display-suite scenario (a Python, a Markdown and a PHP file, light and
   dark, shots compared), tests on Hubble, techbench and kepler; a package
   for techbench.

## Acceptance

- A .py, .md, .html and .php file open in Edit colored by their language,
  in light and in dark style, with the language in the status bar.
- Typing in a 10,000-line file stays as quick as with colors off.
- Opening a triple-quoted string or `/*` colors the lines after it, and
  closing it puts them back, without reopening the file.
- No red waves under code; waves still under misspelled comments and prose.
- In a Python file, `isi` offers `isinstance`; in HTML, `<bl` offers
  `blockquote`.

## Questions for Dave (answered 2026-11-24: all eight as leaned)

1. **Our own tokenizers, or tree-sitter?** Lean: **our own**. Tree-sitter
   would be borrowing (a C library and a grammar per language, each its own
   project); ours cover keywords, strings, comments, numbers and markup,
   which is what colors need, in a few hundred lines a language. A parser
   would matter later for things like folding or outlines.
2. **Colors**: (a) our own muted palette, light and dark, tested for
   contrast; (b) the terminal's color scheme (tango etc.) so Edit and
   Terminal match; (c) a choice of named themes in Settings. Lean: **(a)
   now, (c) later** if wanted.
3. **Bold and italic** (Markdown headings bold, emphasis italic; Python
   `def` names bold?): Lean: **yes for Markdown's structure, colors only
   for code.**
4. **Spellcheck in code files only in comments, strings and Markdown
   prose?** Lean: **yes**: today every identifier gets a red wave.
5. **Completion in code**: the language's words and the open documents'
   names, no dictionary words outside comments and strings? Lean: **yes**.
   HTML tag and attribute names too.
6. **Which languages after .md .html .php .py, and in what order?** Lean:
   **JavaScript and CSS (they come with HTML), shell, JSON, Rust, TOML,
   C**.
7. **Choosing the language**: by extension, then the first line, with View
   > Language to change it for the open document. Remembered per file
   after that? Lean: **not remembered** (the extension decides next time);
   a file with no extension keeps what was chosen while it is open.
8. **Anything beyond colors now?** For example: matching brackets shown at
   the caret, auto-indent after `:` in Python and inside tags, closing a
   tag or quote as it opens, a rendered Markdown preview. Lean: **matching
   brackets and Python's auto-indent in this milestone** (small, and part
   of coloring's feel); closing tags/quotes and a Markdown preview later.
