# Vergence's zsh setup with Terminal (data/shell/zshrc; Dave, 2026-11-27): the
# shell says its folder (OSC 7) and marks where each prompt starts (OSC 133).
# A new tab opens in the folder the front one is in (a folder with a space in
# its name, sent percent-encoded); Ctrl+Shift+Up scrolls back to the prompt
# before the top line, Ctrl+Shift+Down comes back to the live screen.
scenario() {
    zsh=$(command -v zsh) || { echo "  ok   skipped: no zsh on this machine"; return 0; }
    headless "1920x1080 531x299@60" || return 1
    home=$HOME/.cache/vergence-headless/home
    # A still cursor, so pictures of the terminal compare.
    printf '\n[terminal]\nblink = false\n' >> "$home/.config/vergence/desktop.toml"
    dir="$home/vg-shell/two words"
    # vctl types small letters, digits and a few signs only: two words of the
    # test's own for what needs / ~ > and quotes.
    printf 'source %s/data/shell/zshrc\nvgcd() { cd "%s" }\nvgpwd() { pwd > "%s/newtab-pwd.txt" }\n' \
        "$ROOT" "$dir" "$OUT" > "$home/.zshrc"
    mkdir -p "$dir"
    rm -f "$OUT/newtab-pwd.txt"
    vctl spawn "env SHELL=$zsh $TARGET/vergence-term" >/dev/null
    i=0; while ! vctl windows | grep -q '"vergence-term"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    sleep 2
    t=$(vctl windows | grep '"vergence-term"' | head -1 | cut -d' ' -f1)
    vctl focus "$t" >/dev/null
    sleep 0.5

    # The folder: cd in tab 1, a new tab, where is it?
    vctl type "vgcd" >/dev/null; vctl key Return >/dev/null; sleep 1
    vctl key ctrl+shift+t >/dev/null; sleep 2
    vctl type "vgpwd" >/dev/null; vctl key Return >/dev/null; sleep 1
    check "$(yes_if "[ \"\$(cat '$OUT/newtab-pwd.txt' 2>/dev/null)\" = '$dir' ]")" "a new tab opens in the front tab's folder ($(cat "$OUT/newtab-pwd.txt" 2>/dev/null))"

    # The terminal's own picture by its index (its title follows the folder).
    pic() {
        rm -f "$SHOTS/window-$t.png"
        vctl picture "$t" >/dev/null
        i=0; while [ ! -s "$SHOTS/window-$t.png" ] && [ $i -lt 80 ]; do sleep 0.1; i=$((i + 1)); done
        sleep 0.3
        cp "$SHOTS/window-$t.png" "$OUT/$1.png"
    }

    # The prompts: three commands with a screenful of output each, then jumps.
    for n in 1 2 3; do
        vctl type "seq 1 60" >/dev/null; vctl key Return >/dev/null; sleep 0.8
    done
    sleep 0.5
    pic live >/dev/null
    vctl key ctrl+shift+Up >/dev/null; sleep 0.6
    pic up1 >/dev/null
    vctl key ctrl+shift+Up >/dev/null; sleep 0.6
    pic up2 >/dev/null
    vctl key ctrl+shift+Down >/dev/null; vctl key ctrl+shift+Down >/dev/null; vctl key ctrl+shift+Down >/dev/null; sleep 0.6
    pic down >/dev/null
    check "$(yes_if "! cmp -s '$OUT/live.png' '$OUT/up1.png'")" "Ctrl+Shift+Up scrolls back to a prompt"
    check "$(yes_if "! cmp -s '$OUT/up1.png' '$OUT/up2.png'")" "again: the prompt before that one"
    check "$(yes_if "cmp -s '$OUT/live.png' '$OUT/down.png'")" "Ctrl+Shift+Down comes back to the live screen"
    # The profile is shared by the scenarios: the next ones' terminals get no
    # zsh setup (it titles windows by their folder; 19 finds "Terminal"). An
    # empty ~/.zshrc, not none: zsh would greet a home without one with its
    # new-user questions.
    : > "$home/.zshrc"
    rm -rf "$home/vg-shell"
}
