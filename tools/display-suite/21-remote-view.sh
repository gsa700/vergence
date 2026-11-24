# Remote milestone 1, step 1: a remote view draws a screen again at a
# viewer's size and scale (here 1.5 times a 1920x1080 screen at scale 1), and
# a pull hands back what changed: the whole view first, nothing while the
# screen is still, then only the rectangles a change touched, each matching
# the view's picture.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    sleep 2
    out=$(vctl remote start MEM-1 2880x1620 Control)
    id=$(echo "$out" | sed -E 's/remote view ([0-9]+)/\1/')
    check "$(yes_if "[ -n '$id' ] && [ '$id' -gt 0 ] 2>/dev/null")" "a view started ($out)"
    sleep 1
    line=$(vctl remote list | grep "^$id ")
    check "$(yes_if "echo '$line' | grep -q 'MEM-1 2880x1620 scale 1.500'")" "drawn at the viewer's size and scale ($line)"
    vctl tiles > "$OUT/tiles-one.txt"
    check "$(yes_if "grep -q 'Control is viewing this computer' '$OUT/tiles-one.txt'")" "the Dockworker's tile: Control is viewing this computer"
    first=$(vctl remote check "$id")
    echo "  .. $first"
    check "$(yes_if "echo '$first' | grep -q '1 rectangle(s), 18662400 bytes, match yes'")" "the first pull is the whole view"
    sleep 1
    cp "$SHOTS/remote-$id.png" "$OUT/remote-desktop.png" 2>/dev/null
    idle=$(vctl remote check "$id")
    echo "  .. still: $idle"
    check "$(yes_if "echo '$idle' | grep -q ', 0 rectangle(s), 0 bytes'")" "a still screen: nothing sent"
    # A window opened and moved: only what it touched comes back.
    pattern Remote || { check 0 "the test pattern"; return; }
    vctl remote check "$id" >/dev/null
    move Remote 300 200
    moved=$(vctl remote check "$id")
    echo "  .. moved: $moved"
    bytes=$(echo "$moved" | sed -E 's/.* ([0-9]+) bytes.*/\1/')
    check "$(yes_if "echo '$moved' | grep -q 'match yes' && [ '$bytes' -gt 0 ] && [ '$bytes' -lt 9331200 ]")" "a change: only its rectangles (under half the view), matching the picture"
    sleep 1
    cp "$SHOTS/remote-$id.png" "$OUT/remote-moved.png" 2>/dev/null
    # The desk's monitors off (energy): the view goes on being drawn.
    vctl energy screens-off >/dev/null
    vctl remote check "$id" >/dev/null
    move Remote 600 300
    dark=$(vctl remote check "$id")
    echo "  .. monitors off: $dark"
    check "$(yes_if "echo '$dark' | grep -q 'match yes' && ! echo '$dark' | grep -q ', 0 rectangle'")" "the monitors off: the view still follows a change"
    vctl energy wake >/dev/null
    vctl remote resize "$id" 1280x720 >/dev/null; sleep 0.8
    check "$(yes_if "vctl remote list | grep -q '^$id MEM-1 1280x720 scale 0.667'")" "resized for a smaller viewer"
    check "$(yes_if "vctl remote stop '$id' | grep -q stopped && [ -z \"\$(vctl remote list)\" ]")" "stopped"
    check "$(yes_if "! vctl tiles | grep -q '\"remote\"\\|remote '")" "the tile gone with the last view"
    # The Dockworker's End, from the desk: every view ends.
    vctl remote start MEM-1 1600x900 Control >/dev/null
    vctl remote start MEM-1 800x450 Laptop >/dev/null
    sleep 0.5
    vctl tiles > "$OUT/tiles.txt"
    check "$(yes_if "grep -q 'Control and 1 more are viewing this computer' '$OUT/tiles.txt'")" "two viewers named on the tile"
    vctl tile-action remote end >/dev/null; sleep 0.3
    check "$(yes_if "[ -z \"\$(vctl remote list)\" ] && ! vctl tiles | grep -q 'viewing this computer'")" "End on the tile ends them all"
}
