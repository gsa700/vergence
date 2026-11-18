# A screen plugged in: it goes on the right of the others, and an open
# window stays where it was.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    pattern W --size 400 300 || check 0 "window opens"
    move W 300 200
    before=$(win_geo W)
    vctl screen add --fake 2560x1440 597x336 --name HL2560x1440 >/dev/null
    sleep 1.5
    set -- $(screen_geo MEM-2)
    check "$(yes_if "[ '$1 $2' = '1920 0' ]")" "the new screen right of the first, tops aligned: at $1,$2"
    check "$(yes_if "[ '$(win_geo W)' = '$before' ]")" "the window stays put: $before, now $(win_geo W)"
}
