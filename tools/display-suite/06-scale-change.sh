# A screen's scale changed under an open window: the window goes onto the
# new pixel grid (place and size in steps of 3 points at 133%) and its bar
# and body still meet, its picture shown one for one.
scenario() {
    headless "2560x1440 597x336@60" || { check 0 "headless session"; return; }
    pattern W --size 401 301 || check 0 "window opens"
    move W 101 101
    vctl scale MEM-1 1.3333333333 >/dev/null
    sleep 1.5
    set -- $(win_geo W)
    check "$(yes_if "[ $(($1 % 3)) -eq 0 ] && [ $(($2 % 3)) -eq 0 ] && [ $(($3 % 3)) -eq 0 ] && [ $(($4 % 3)) -eq 0 ]")" "on the 3-point grid: $*"
    s=$(shot MEM-1 shot)
    p=$(picture W window)
    bar=$(win_field W bar-h)
    r=$("$HARNESS" seams "$s" --scale 1.3333333333 --window "$1" "$2" "$3" "$4" --bar "$bar" | tail -1)
    check "$(yes_if "[ '$r' = '0 of the checks failed' ]")" "seams: $r"
    at=$(awk -v x="$1" -v y="$2" 'BEGIN { printf "%d %d", int(x * 4 / 3 + 0.5), int(y * 4 / 3 + 0.5) }')
    r=$("$HARNESS" same "$s" "$p" --at $at --corners 10)
    check "$(yes_if "echo '$r' | grep -q '^PASS'")" "shown as drawn: $r"
}
