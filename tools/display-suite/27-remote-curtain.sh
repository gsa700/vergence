# Remote milestone 1, step 4: blanking the desk. A viewer asks for privacy:
# the desk locks behind a curtain (here a headless session's lock, which
# needs no vdm), while the viewer's view goes on drawing the session and its
# keys go on reaching the window with the keyboard. The desk's own keys meet
# the lock. Unlocking at the desk ends the connection.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    run=$(dirname "$VCTL_SOCKET")
    vctl spawn "$TARGET/vergence-edit" >/dev/null
    edit_line() { vctl windows | grep '"vergence-edit"' | head -1; }
    i=0; while [ -z "$(edit_line)" ] && [ $i -lt 100 ]; do sleep 0.1; i=$((i + 1)); done
    sleep 1
    set -- $(edit_line | sed -E 's/.*\{ x: (-?[0-9]+), y: (-?[0-9]+), width: ([0-9]+), height: ([0-9]+) \}.*/\1 \2 \3 \4/')
    cx=$(($1 + $3 / 2)); cy=$(($2 + $4 / 2))
    env XDG_RUNTIME_DIR="$run" "$TARGET/vergence-remote-probe" --size 1920x1080 --frames 1000 --seconds 12 --name Control \
        --script "pointer $cx $cy; press left; release left; wait 300; privacy on; wait 1200; type behind the curtain; wait 500; down 125; down 42; key 4; up 42; up 125; wait 3000" \
        -- "$TARGET/vergence-remote" > "$OUT/probe.txt" 2>&1 &
    p=$!
    sleep 3.5
    check "$(yes_if "vctl lock | grep -q '^locked' && vctl lock curtain | grep -q '^curtain'")" "privacy: the desk locked behind a curtain"
    shot MEM-1 desk >/dev/null
    v=$(vctl remote list | head -1 | cut -d' ' -f1)
    vctl remote check "$v" >/dev/null; sleep 0.5
    cp "$SHOTS/remote-$v.png" "$OUT/view.png" 2>/dev/null
    check "$(yes_if "[ -s '$OUT/desk.png' ] && [ -s '$OUT/view.png' ] && ! cmp -s '$OUT/desk.png' '$OUT/view.png'")" "the desk shows the lock, the view the session (pictures differ)"
    sleep 2.5
    vctl tree > "$OUT/tree.txt" 2>&1 || true
    # vctl's tree is refused while locked: the typed text is checked after the unlock.
    vctl lock test-unlock >/dev/null
    sleep 1
    i=0; while kill -0 $p 2>/dev/null && [ $i -lt 60 ]; do sleep 0.1; i=$((i + 1)); done
    vctl tree > "$OUT/tree.txt"
    check "$(yes_if "grep -q 'behind the curtain' '$OUT/tree.txt'")" "the viewer's keys reached Edit through the curtain"
    check "$(yes_if "vctl lock | grep -q '^unlocked'")" "unlocked at the desk"
    # Super+Shift+3 from the viewer behind the curtain: the session, not the lock.
    taken=$(ls -t "$HOME"/.cache/vergence-headless/home/Pictures/Screenshots/*/*.png 2>/dev/null | head -1)
    if [ -n "$taken" ]; then cp "$taken" "$OUT/screenshot.png"; fi
    check "$(yes_if "[ -s '$OUT/screenshot.png' ] && ! cmp -s '$OUT/screenshot.png' '$OUT/desk.png'")" "a screenshot behind the curtain is the session's (not the desk's lock)"
    tail -3 "$OUT/probe.txt" | sed 's/^/  .. /'
    # The probe reads the host's messages after its script: checked once it ended.
    check "$(yes_if "grep -q 'privacy: on' '$OUT/probe.txt'")" "the viewer was told the desk is blanked"
    check "$(yes_if "grep -q 'notice: Ended from the desk' '$OUT/probe.txt' || grep -q 'bye: ended from the desk' '$OUT/probe.txt'")" "unlocking at the desk ended the connection"
}
