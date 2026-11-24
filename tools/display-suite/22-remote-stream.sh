# Remote milestone 1, step 3: the whole path a viewer takes, without SSH:
# vergence-remote-probe runs vergence-remote as SSH would, says Hello, gets
# the host's Welcome and frames (compressed by vergence-remote, drawn by the
# server at the viewer's size), acknowledges them; the Dockworker's tile
# names it; End on the tile tells it and closes it.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    run=$(dirname "$VCTL_SOCKET")
    probe() { env XDG_RUNTIME_DIR="$run" "$TARGET/vergence-remote-probe" "$@" -- "$TARGET/vergence-remote"; }
    check "$(yes_if "ls '$run'/vergence-remote-* >/dev/null 2>&1")" "the session's remote socket"
    pattern Remote || { check 0 "the test pattern"; return; }
    probe --size 1280x720 --frames 1 --out "$OUT/probe.ppm" > "$OUT/probe1.txt" 2>&1
    cat "$OUT/probe1.txt" | sed 's/^/  .. /'
    check "$(yes_if "grep -q 'welcome: .*MEM-1' '$OUT/probe1.txt'")" "the host's Welcome names its screen"
    check "$(yes_if "grep -q 'probe: 1 frame(s), 1280x720 scale 0.667' '$OUT/probe1.txt'")" "a frame at the viewer's size and scale"
    pct=$(sed -nE 's/.*\(([0-9.]+)% of one raw picture\).*/\1/p' "$OUT/probe1.txt")
    check "$(yes_if "near '${pct:-100}' 0 30")" "compressed on the wire (${pct}% of a raw picture)"
    check "$(yes_if "[ \"\$(head -c 2 '$OUT/probe.ppm')\" = P6 ] && [ \$(wc -c < '$OUT/probe.ppm') -gt 2700000 ]")" "the viewer's picture written (1280x720)"
    # A long run: the tile names it; a window moved brings more frames; End closes it.
    probe --size 1600x900 --frames 100000 --seconds 20 --name Control > "$OUT/probe2.txt" 2>&1 &
    p=$!
    sleep 1.5
    vctl tiles > "$OUT/tiles.txt"
    check "$(yes_if "grep -q 'Control is viewing this computer' '$OUT/tiles.txt'")" "the tile names the viewer"
    move Remote 200 150
    move Remote 700 400
    sleep 0.5
    vctl tile-action remote end >/dev/null
    i=0; while kill -0 $p 2>/dev/null && [ $i -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
    tail -4 "$OUT/probe2.txt" | sed 's/^/  .. /'
    frames=$(grep -c '^frame ' "$OUT/probe2.txt")
    check "$(yes_if "[ '$frames' -ge 3 ]")" "changes brought more frames ($frames)"
    check "$(yes_if "grep -q 'notice: Ended from the desk' '$OUT/probe2.txt' && grep -q 'bye: ended from the desk' '$OUT/probe2.txt'")" "End on the tile: the viewer told and closed"
    check "$(yes_if "[ -z \"\$(vctl remote list)\" ] && ! vctl tiles | grep -q 'viewing this computer'")" "no view and no tile left"
    # No session where it looks: said plainly.
    # (A throwaway HOME, Remote off: never the person's own settings, where
    # Remote on would start a session with no monitors for them.)
    mkdir -p "$OUT/nohome"
    env HOME="$OUT/nohome" XDG_RUNTIME_DIR=/nonexistent VERGENCE_MONITORLESS_DIR=/nonexistent \
        "$TARGET/vergence-remote-probe" -- "$TARGET/vergence-remote" > "$OUT/probe3.txt" 2>&1
    check "$(yes_if "grep -q 'bye: Vergence Remote is off on this computer' '$OUT/probe3.txt'")" "no session and Remote off: said plainly"
}
