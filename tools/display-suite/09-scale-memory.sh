# Window memory across a scale change (Dave, 2026-11-18: "keep point size
# on scale changes"): a window closed at 100% opens at 200% of the same
# screen at the same size in points, not the same pixels; at another
# resolution (a 4K screen's 1440p mode stands in) places still scale in
# proportion.
scenario() {
    headless "2560x1440 597x336@60" || { check 0 "headless session"; return; }
    pattern W --size 420 260 || check 0 "window opens"
    move W 300 200
    before=$(win_geo W)
    pkill -x testpattern
    sleep 1
    vctl scale MEM-1 2 >/dev/null
    sleep 1.5
    pattern W --size 420 260 || check 0 "window opens again"
    set -- $(win_geo W)
    check "$(yes_if "[ '$3 $4' = '420 260' ]")" "same size in points at 200%: was $before, now $*"
    check "$(yes_if "[ '$1 $2' = '300 200' ]")" "same place in points: $1,$2"
}
