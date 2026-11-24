# Remote milestone 1: Privacy > Remote's switch (desktop.toml [remote]
# enabled, off by default). On, the session listens for Vergence Remote;
# off, there is no socket at all and a live connection ends, its viewer told.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    run=$(dirname "$VCTL_SOCKET")
    cfg=$HOME/.cache/vergence-headless/home/.config/vergence/desktop.toml
    sleep 1.5
    check "$(yes_if "ls '$run'/vergence-remote-* >/dev/null 2>&1")" "on: the session listens"
    env XDG_RUNTIME_DIR="$run" "$TARGET/vergence-remote-probe" --size 1280x720 --frames 100000 --seconds 15 --name Control \
        -- "$TARGET/vergence-remote" > "$OUT/live.txt" 2>&1 &
    p=$!
    sleep 2
    sed -i 's/^enabled = true$/enabled = false/' "$cfg"
    # (The [agents] table's switch flips with it here; this scenario uses no agent.)
    grep -q '^\[remote\]' "$cfg" || printf '\n[remote]\nenabled = false\n' >> "$cfg"
    sleep 2.5
    check "$(yes_if "! ls '$run'/vergence-remote-* >/dev/null 2>&1")" "off: no socket at all"
    i=0; while kill -0 $p 2>/dev/null && [ $i -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
    tail -1 "$OUT/live.txt" | sed 's/^/  .. /'
    check "$(yes_if "grep -q 'turned off on this computer' '$OUT/live.txt'")" "off: the live viewer told and closed"
    check "$(yes_if "[ -z \"\$(vctl remote list)\" ] && ! vctl tiles | grep -q 'viewing this computer'")" "off: no view, no tile"
    sed -i 's/^enabled = false$/enabled = true/' "$cfg"
    sleep 2.5
    env XDG_RUNTIME_DIR="$run" "$TARGET/vergence-remote-probe" --size 1280x720 --frames 1 -- "$TARGET/vergence-remote" > "$OUT/again.txt" 2>&1
    check "$(yes_if "grep -q 'probe: 1 frame' '$OUT/again.txt'")" "on again: a viewer connects"
    # The page itself: Privacy > Remote, its switch.
    vctl spawn "$TARGET/vergence-settings --page remote" >/dev/null
    i=0; while ! vctl windows | grep -q '"vergence-settings"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    sleep 2
    vctl tree > "$OUT/settings.txt"
    check "$(yes_if "grep -q 'Allow Vergence Remote' '$OUT/settings.txt'")" "Settings: Privacy > Remote, Allow Vergence Remote"
    shot MEM-1 settings >/dev/null
}
