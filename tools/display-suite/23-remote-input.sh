# Remote milestone 1, step 4: a viewer's input as the machine's own. Through
# vergence-remote (as SSH would run it), the probe drags a window by its bar
# and types into Edit; the seat decides where each goes, as for local input.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    run=$(dirname "$VCTL_SOCKET")
    probe() { env XDG_RUNTIME_DIR="$run" "$TARGET/vergence-remote-probe" "$@" -- "$TARGET/vergence-remote"; }
    pattern Remote || { check 0 "the test pattern"; return; }
    move Remote 400 300
    set -- $(win_geo Remote)
    x0=$1; y0=$2
    # The viewer is the screen's size: its pixels are the screen's points.
    # The bar is drawn above the window's own rectangle (bar-h points).
    bh=$(win_field Remote bar-h)
    bx=$((x0 + 120)); by=$((y0 - ${bh%.*} / 2))
    probe --size 1920x1080 --frames 2 --seconds 8 --name Control \
        --script "pointer $bx $by; wait 100; press left; pointer $((bx + 50)) $((by + 25)); pointer $((bx + 120)) $((by + 60)); pointer $((bx + 200)) $((by + 100)); wait 100; release left; wait 300" \
        > "$OUT/drag.txt" 2>&1
    tail -2 "$OUT/drag.txt" | sed 's/^/  .. /'
    set -- $(win_geo Remote)
    check "$(yes_if "[ $1 -ge $((x0 + 180)) ] && [ $1 -le $((x0 + 220)) ] && [ $2 -ge $((y0 + 80)) ] && [ $2 -le $((y0 + 120)) ]")" "dragged by its bar from the viewer: $x0,$y0 to $1,$2"
    # Typing: a click into Edit's page, then keys.
    vctl spawn "$TARGET/vergence-edit" >/dev/null
    edit_line() { vctl windows | grep '"vergence-edit"' | head -1; }
    i=0; while [ -z "$(edit_line)" ] && [ $i -lt 100 ]; do sleep 0.1; i=$((i + 1)); done
    sleep 1
    [ -n "$(edit_line)" ] || { check 0 "Edit opened"; return; }
    set -- $(edit_line | sed -E 's/.*\{ x: (-?[0-9]+), y: (-?[0-9]+), width: ([0-9]+), height: ([0-9]+) \}.*/\1 \2 \3 \4/')
    cx=$(($1 + $3 / 2)); cy=$(($2 + $4 / 2))
    probe --size 1920x1080 --frames 2 --seconds 8 --name Control \
        --script "pointer $cx $cy; press left; release left; wait 300; type hello remote; locks 1 0; type abc; locks 0 0; type def; wait 500" \
        > "$OUT/type.txt" 2>&1
    tail -2 "$OUT/type.txt" | sed 's/^/  .. /'
    vctl tree > "$OUT/tree.txt"
    check "$(yes_if "grep -q 'hello remote' '$OUT/tree.txt'")" "typed into Edit from the viewer"
    check "$(yes_if "grep -q 'hello remoteABCdef' '$OUT/tree.txt'")" "the viewer's Caps Lock followed: ABC with it on, def with it off"
    shot MEM-1 typed >/dev/null
}
