# Edit syntax milestone 1, step 8: syntax colors (docs/edit-syntax-milestone-1.md).
# In light and in dark style: a Python, a Markdown and a PHP file open in Edit
# colored by their language, which the status line names; a Rust file (its
# language from a definition file, data/syntax/rust.toml) is colored too; with
# Settings' Syntax colors off, the same text is plain (far fewer colored
# pixels). "Colored" is counted by `vergence-harness colors` in the window's
# own picture. Each part in a fresh session (Edit keeps one process, and reads
# its settings at start or from its own Settings window).
scenario() {
    files=$OUT/files
    rm -rf "$files"
    mkdir -p "$files"
    cat > "$files/rack.py" <<'PY'
"""Rack watch: read the outlets, say what changed."""
import json

def summarize(readings, limit=1_500):
    total = sum(r["watts"] for r in readings)  # all outlets
    if total > limit and not isinstance(total, str):
        return f"Total {total:,.1f} W"
    return None
PY
    cat > "$files/notes.md" <<'MD'
# Lookout notes

Runs **Ollama** and *Open WebUI* on a [fixed address](https://example.com).

- [x] Fixed IP on `eno1`
- [ ] Reboot test

```python
def ok(state): return state == "up"
```
MD
    cat > "$files/rack.php" <<'PHP'
<?php
/** One row per outlet. */
function watts(array $o): string { return number_format($o['watts'], 1) . " W"; }
?>
<table class="outlets">
  <tr><td><?= htmlspecialchars($name) ?></td><td><?= watts($o) ?></td></tr>
</table>
PHP
    cat > "$files/shelf.rs" <<'RS'
/// The dock's shelf.
#[derive(Debug, Clone)]
pub struct Shelf<'a> { name: &'a str, items: Vec<u32> }

impl Shelf<'_> {
    pub fn count(&self) -> usize { self.items.iter().filter(|n| **n > 0x10).count() }
}
RS
    cfg=$HOME/.cache/vergence-headless/home/.config/vergence/desktop.toml
    for style in light dark; do
        headless "1920x1080 531x299@60" || { check 0 "headless session ($style)"; return; }
        kept=$(cat "$cfg")
        printf '\n[appearance]\ncolor-scheme = "%s"\n' "$style" >> "$cfg"
        for pair in rack.py:Python notes.md:Markdown rack.php:PHP shelf.rs:Rust; do
            f=${pair%%:*}
            lang=${pair#*:}
            vctl spawn "$TARGET/vergence-edit $files/$f" >/dev/null
            i=0; while ! vctl windows | grep -q '"vergence-edit"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
            sleep 3
            check "$(yes_if "vctl tree | grep -q '\"$lang\"'")" "$style: $f is $lang in the status line"
            pic=$(picture "$f" "$style-$f")
            n=$("$HARNESS" colors "$pic" | awk '{print $2}')
            echo "  .. $style $f: ${n:-0} colored pixels"
            # The window's own accents are about 450 colored pixels in either
            # style (Hubble, 2026-11-24): colored text is well over twice that.
            check "$(yes_if "[ \"${n:-0}\" -gt 900 ]")" "$style: $f is colored"
            eval "colored_$(echo "$f" | tr . _)=${n:-0}"
        done
        # Syntax colors off (Settings writes it; a fresh session reads it).
        printf '\n[editor]\nsyntax-colors = false\n' >> "$cfg"
        headless "1920x1080 531x299@60" || { check 0 "headless session ($style, plain)"; return; }
        vctl spawn "$TARGET/vergence-edit $files/shelf.rs" >/dev/null
        i=0; while ! vctl windows | grep -q '"vergence-edit"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
        sleep 3
        pic=$(picture "shelf.rs" "$style-plain")
        off=$("$HARNESS" colors "$pic" | awk '{print $2}')
        echo "  .. $style shelf.rs with Syntax colors off: ${off:-0} colored pixels (on: $colored_shelf_rs)"
        check "$(yes_if "[ $((${off:-0} * 3)) -lt $colored_shelf_rs ]")" "$style: Syntax colors off makes it plain"
        printf '%s\n' "$kept" > "$cfg"
    done
}
