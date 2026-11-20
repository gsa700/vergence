# Meaning milestone 1, step 7: the magnifier. Zoomed about a point, the
# screen must be the plain screen scaled about it (checked pixel by pixel
# by vergence-harness zoomed); the lens leaves the screen around it as it
# was; typing in Edit moves the point to the caret.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    rm -rf "$HOME/.cache/vergence-headless/home/.local/state/vergence/edit"
    pattern A --size 900 600 || check 0 "the test pattern opens"
    move A 300 200
    # The pointer out of the way: drawn, it is magnified too.
    vctl pointer 2 2 >/dev/null; sleep 0.5
    plain=$(shot MEM-1 plain)
    # A point as the keyboard focus would give it, then on at 2x.
    vctl zoom focus 700 450 >/dev/null
    vctl zoom toggle >/dev/null; sleep 1.2
    z=$(vctl zoom)
    check "$(yes_if "echo '$z' | grep -q '^on level 2.00 (drawn 2.00) screen at 700,450 by focus'")" "zoomed in at 2x about the focus point: $z"
    zoomed=$(shot MEM-1 zoomed)
    r=$("$HARNESS" zoomed "$plain" "$zoomed" 700 450 2)
    echo "$r"
    check "$(yes_if "echo '$r' | grep -q '^PASS'")" "the screen scaled about the point, pixel by pixel: $(echo "$r" | cut -c1-90)"
    # The lens: the screen as it was outside it.
    vctl zoom lens >/dev/null; sleep 0.8
    lens=$(shot MEM-1 lens)
    r=$("$HARNESS" same "$plain" "$lens" --tolerance 2 2>/dev/null | head -1)
    check "$(yes_if "echo '$r' | grep -q '^FAIL'")" "the lens shows something other than the plain screen"
    vctl zoom toggle >/dev/null; vctl zoom lens >/dev/null; sleep 1
    off=$(shot MEM-1 off)
    r=$("$HARNESS" same "$plain" "$off" --tolerance 2 | head -1)
    # The dock's live tiles change between the shots (the network's
    # traffic, the clock's minute): a few hundred pixels of digits, no more.
    n=$(echo "$r" | sed -E 's/.*: ([0-9]+) of .*/\1/')
    check "$(yes_if "echo '$r' | grep -q '^PASS' || [ '$n' -lt 600 ]")" "off again, the screen is as it was (the dock's live tiles aside): $(echo "$r" | cut -c1-70)"
    # Typing in Edit: the point follows the caret.
    vctl spawn "$TARGET/vergence-edit" >/dev/null
    i=0; while [ -z "$(win_line Untitled)" ] && [ $i -lt 80 ]; do sleep 0.1; i=$((i + 1)); done
    sleep 1
    # Its place before typing (its title gains the edited mark after).
    set -- $(win_geo Untitled)
    # The keyboard to Edit (the test pattern opened first may keep it).
    vctl focus "$(win_index Untitled)" >/dev/null; sleep 0.5
    vctl zoom toggle >/dev/null
    vctl type "the caret is followed" >/dev/null; sleep 1.2
    z=$(vctl zoom)
    at=$(echo "$z" | sed -E 's/.* at ([0-9-]+),([0-9-]+) by.*/\1 \2/')
    ax=${at% *}; ay=${at#* }
    check "$(yes_if "echo '$z' | grep -q 'by caret' && [ $ax -gt $1 ] && [ $ax -lt $(($1 + $3)) ] && [ $ay -gt $2 ] && [ $ay -lt $(($2 + 120)) ]")" "typing in Edit moves the point to its caret, in its first line: $z"
    shot MEM-1 caret >/dev/null
}
