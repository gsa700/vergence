# Overview milestone 1: the window pictures, Alt+Tab's tiles, the dock's
# picker and Super+Tab's overview, with three test-pattern windows piled
# on one another and one of them minimized.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    pattern A --size 900 600 || check 0 "window A opens"
    move A 200 150
    pattern B --size 900 600 || check 0 "window B opens"
    move B 260 200
    pattern C --size 700 500 || check 0 "window C opens"
    move C 320 260
    # A picture fits its room and keeps its shape (900x600 into 240: 240x160).
    rm -f "$SHOTS/thumb-$(win_index A).png"
    vctl thumb "$(win_index A)" 240 >/dev/null
    i=0; while [ ! -s "$SHOTS/thumb-$(win_index A).png" ] && [ $i -lt 40 ]; do sleep 0.1; i=$((i + 1)); done
    size=$(od -An -tu1 -j16 -N8 "$SHOTS/thumb-$(win_index A).png" | awk '{ printf "%d %d", $3*256+$4, $7*256+$8 }')
    check "$(yes_if "[ '$size' = '240 160' ]")" "a picture fits its room, its shape kept: $size"
    vctl minimize "$(win_index C)" >/dev/null
    sleep 0.8
    # Alt+Tab: a tile for each window, the minimized one too.
    vctl hold alt >/dev/null; vctl key Tab >/dev/null; sleep 0.6
    sw=$(vctl switching)
    check "$(yes_if "echo '$sw' | grep -q 'of 3, shown true'")" "Alt+Tab lists the three windows as tiles: $(echo "$sw" | head -1 | cut -c1-40)"
    shot MEM-1 alt-tab >/dev/null
    # Escape cancels it (letting Alt go would choose the window stepped to).
    vctl key Escape >/dev/null; vctl let-go alt >/dev/null; sleep 0.5
    check "$(yes_if "vctl shelf | grep -q '1 minimized'")" "Escape left the minimized window minimized"
    # The dock's picker: resting on the app's icon shows its three windows.
    at=$(vctl shelf | grep 'windows 3' | sed -E 's/.* at ([0-9]+),([0-9]+).*/\1 \2/')
    set -- $at
    vctl pointer $(( $1 + 24 )) $(( $2 + 24 )) >/dev/null; sleep 1.2
    pk=$(vctl picker)
    check "$(yes_if "echo '$pk' | grep -q '3 windows'")" "the dock's picker shows the app's three windows: $(echo "$pk" | head -1 | cut -c1-60)"
    shot MEM-1 picker >/dev/null
    vctl pointer 960 400 >/dev/null; sleep 1
    check "$(yes_if "[ '$(vctl picker)' = 'closed' ]")" "the picker goes when the pointer leaves"
    # The overview: none overlapping, all inside the screen, the minimized one in the bottom fifth.
    vctl key super+Tab >/dev/null; sleep 1
    ov=$(vctl overview)
    check "$(yes_if "echo '$ov' | grep -q '^open, 3 windows'")" "Super+Tab opens the overview with three windows"
    echo "$ov" | tail -n +2 > "$OUT/overview.txt"
    bad=$(awk '{ x[NR]=$2; y[NR]=$3; w[NR]=$4; h[NR]=$5 } END {
        for (i = 1; i <= NR; i++) {
            if (x[i] < 0 || y[i] < 0 || x[i] + w[i] > 1920 || y[i] + h[i] > 1080 || w[i] < 1) print "outside " i;
            for (j = i + 1; j <= NR; j++) if (x[i] < x[j] + w[j] && x[j] < x[i] + w[i] && y[i] < y[j] + h[j] && y[j] < y[i] + h[i]) print "overlap " i " " j;
        } }' "$OUT/overview.txt")
    check "$(yes_if "[ -z '$bad' ]")" "the overview's windows inside the screen, none overlapping${bad:+: $bad}"
    miny=$(grep minimized "$OUT/overview.txt" | awk '{ print $3 }')
    check "$(yes_if "[ -n '$miny' ] && [ '$miny' -gt 700 ]")" "the minimized window in the row along the bottom: y $miny"
    shot MEM-1 overview >/dev/null
    vctl key Escape >/dev/null; sleep 0.5
    check "$(yes_if "[ '$(vctl overview)' = 'closed' ]")" "Escape closes the overview"
}
