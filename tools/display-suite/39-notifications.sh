# Notifications that are seen (Dave, 2026-11-27: "I have had notifications go
# unseen for a while because it just gets lost in the desktop"): cards 400
# points wide with the app's name, a semibold title and an accent stripe (the
# alarm red for a critical one), sent as any app sends them, over D-Bus
# (org.freedesktop.Notifications, with gdbus); stacked at the top right of the
# work area by default, and in the middle of its top with corner = "top-center"
# (as GNOME's).
scenario() {
    command -v gdbus >/dev/null || { echo "  ok   skipped: no gdbus on this machine"; return 0; }
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    home=$HOME/.cache/vergence-headless/home
    cfg=$home/.config/vergence/desktop.toml
    kept=$(cat "$cfg" 2>/dev/null)
    send() { # send <app> <title> <body> <urgency 0-2> [actions]
        gdbus call --session --dest org.freedesktop.Notifications --object-path /org/freedesktop/Notifications \
            --method org.freedesktop.Notifications.Notify "$1" 0 "" "$2" "$3" "${5:-[]}" "{\"urgency\": <byte $4>}" 0 >/dev/null 2>&1
    }
    # rect <n>: the card's x, y, width, height (vctl notes, newest first).
    rect() { vctl notes | sed -n "${1}p" | sed -nE 's/.*\{ x: (-?[0-9]+), y: (-?[0-9]+), width: ([0-9]+), height: ([0-9]+) \}.*/\1 \2 \3 \4/p'; }
    send Firefox "Download finished" "report-2026.pdf is in your Downloads folder, ready to open." 1 '["open", "Open"]'
    send Backup "The disk is nearly full" "Only 2 GB is left on NASBOX: the next backup may not fit." 2
    sleep 0.8
    vctl notes > "$OUT/notes-top-right.txt"
    set -- $(rect 1) 0 0 0 0
    echo "  .. top right: the newest at $1,$2 ${3}x$4" >&2
    check "$(yes_if "[ '$3' = 400 ] && [ $(($1 + $3)) -ge 1890 ] && [ $2 -lt 80 ]")" "cards 400 points wide, at the top right of the work area"
    check "$(yes_if "[ \$(vctl notes | wc -l) = 2 ]")" "both cards shown"
    shot MEM-1 top-right >/dev/null

    # Top center: the settings changed, the stack moves within a second.
    printf '\n[notifications]\ncorner = "top-center"\n' >> "$cfg"
    sleep 1.5
    set -- $(rect 1) 0 0 0 0
    echo "  .. top center: the newest at $1,$2 ${3}x$4" >&2
    check "$(yes_if "near $(($1 + $3 / 2)) 960 30 && [ $2 -lt 80 ]")" "corner = top-center: the cards in the middle of the top"
    shot MEM-1 top-center >/dev/null
    printf '%s\n' "$kept" > "$cfg"
}
