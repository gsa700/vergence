# Remote milestone 1, step 4: the viewer takes the whole keyboard while it
# has it (bus v40): the server's own shortcuts (Alt+Tab here) go to it, not
# to this machine; Ctrl+Alt pressed and let go alone gives the keyboard back,
# and Alt+Tab is this machine's again. The viewer talks to a stand-in (its
# own session would send the keys round and round), so only the routing is
# looked at.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    pattern Other || { check 0 "the test pattern"; return; }
    vctl spawn "$TARGET/vergence-remote-viewer --command sh -c 'sleep 30'" >/dev/null
    i=0; while ! vctl windows | grep -q '"vergence-remote-viewer"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    sleep 1.5
    w=$(vctl windows | grep '"vergence-remote-viewer"' | head -1 | cut -d' ' -f1)
    set -- $(vctl windows | grep '"vergence-remote-viewer"' | head -1 | sed -E 's/.*\{ x: (-?[0-9]+), y: (-?[0-9]+), width: ([0-9]+), height: ([0-9]+) \}.*/\1 \2 \3 \4/')
    vx=$(($1 + $3 / 2)); vy=$(($2 + $4 / 2))
    # The keyboard follows the pointer: over the window, and the window focused.
    vctl pointer "$vx" "$vy" >/dev/null
    vctl focus "$w" >/dev/null; sleep 1
    # The window with the keyboard, by its title ("focused: Some(...)").
    focused() { vctl used | head -1; }
    check "$(yes_if "focused | grep -q 'Vergence Remote'")" "the viewer has the keyboard"
    check "$(yes_if "vctl windows | grep 'vergence-remote-viewer' | grep -q 'Ctrl+Alt gives the keyboard back'")" "it took the whole keyboard (its title says how to give it back)"
    vctl key alt+Tab >/dev/null; sleep 0.8
    check "$(yes_if "focused | grep -q 'Vergence Remote'")" "Alt+Tab went to the viewer, not this machine"
    # The pointer out of the window: the keyboard back at once.
    vctl pointer 5 5 >/dev/null; sleep 0.8
    check "$(yes_if "! vctl windows | grep 'vergence-remote-viewer' | grep -q 'Ctrl+Alt gives'")" "the pointer left the window: the keyboard is this machine's"
    vctl pointer "$vx" "$vy" >/dev/null; sleep 0.8
    check "$(yes_if "vctl windows | grep 'vergence-remote-viewer' | grep -q 'Ctrl+Alt gives'")" "back over the window: taken again"
    vctl key ctrl+Alt >/dev/null; sleep 0.8
    check "$(yes_if "! vctl windows | grep 'vergence-remote-viewer' | grep -q 'Ctrl+Alt gives'")" "Ctrl+Alt alone gave the keyboard back"
    vctl key alt+Tab >/dev/null; sleep 0.8
    vctl used | head -3 | sed 's/^/  .. /'
    check "$(yes_if "focused | grep -q 'Other'")" "Alt+Tab is this machine's again"
    pkill -f vergence-remote-viewer; sleep 0.5
}
