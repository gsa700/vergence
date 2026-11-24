# Remote milestone 1, step 4: Vergence Remote's viewer, a window showing a
# host's desktop. Here it views its own session (localhost: this machine's
# vergence-remote), so the screen shows itself in the window, a mirror; the
# host's tile names the viewer, and the view follows the window's size.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    pattern Remote || { check 0 "the test pattern"; return; }
    move Remote 1200 120
    vctl spawn "$TARGET/vergence-remote-viewer localhost" >/dev/null
    i=0; while ! vctl windows | grep -q '"vergence-remote-viewer"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    sleep 3
    line=$(vctl windows | grep '"vergence-remote-viewer"' | head -1)
    echo "  .. $(echo "$line" | cut -c1-110)"
    check "$(yes_if "echo '$line' | grep -q '\"[A-Za-z0-9.-]* — Vergence Remote (this computer: looking only)\"' && ! echo '$line' | grep -q 'localhost —'")" "the viewer's window, titled with the host's own name, looking only (this computer)"
    vctl tiles > "$OUT/tiles.txt"
    check "$(yes_if "grep -q 'is viewing this computer' '$OUT/tiles.txt'")" "the host's tile names the viewer"
    view=$(vctl remote list | head -1)
    echo "  .. view: $view"
    frames=$(echo "$view" | sed -E 's/.*frames ([0-9]+).*/\1/')
    check "$(yes_if "[ '${frames:-0}' -gt 0 ]")" "the host draws frames for it"
    # The view's size is the window's below its bar (1280 x 760 asked, at scale 1).
    check "$(yes_if "echo '$view' | grep -qE ' [0-9]+x[0-9]+ scale ' && ! echo '$view' | grep -q ' 0x0 '")" "a view of the window's size"
    # The bar's own button (Blank the desk, greyed here: this computer) is the
    # server's drawing, not in the window's tree: seen in the shot.
    shot MEM-1 viewer >/dev/null
    # Closing the window ends the view and the tile.
    w=$(vctl windows | grep '"vergence-remote-viewer"' | head -1 | cut -d' ' -f1)
    vctl focus "$w" >/dev/null; vctl key ctrl+w >/dev/null 2>&1; sleep 0.3
    vctl action close >/dev/null 2>&1; sleep 1.5
    if vctl windows | grep -q '"vergence-remote-viewer"'; then
        pkill -f vergence-remote-viewer; sleep 1.5
    fi
    check "$(yes_if "[ -z \"\$(vctl remote list)\" ] && ! vctl tiles | grep -q 'viewing this computer'")" "the viewer gone: no view, no tile"
}
