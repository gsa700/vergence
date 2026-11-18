# A screen unplugged: a window on it is rescued onto the screen left,
# wholly inside it, and told that screen's scale.
scenario() {
    headless "1920x1080 531x299@60" "3840x2160 600x340@60" || { check 0 "headless session"; return; }
    set -- $(screen_geo MEM-2)
    pattern W --size 400 300 || check 0 "window opens"
    move W $(($1 + 300)) 300
    check "$(yes_if "[ '$(win_on W)' = MEM-2 ]")" "the window on the second screen first: $(win_on W)"
    vctl screen remove MEM-2 >/dev/null
    sleep 1.5
    check "$(yes_if "[ '$(win_on W)' = MEM-1 ]")" "rescued onto the first: $(win_on W)"
    check "$(yes_if "inside W MEM-1")" "wholly inside it: $(win_geo W) in $(screen_geo MEM-1)"
    check "$(yes_if "near '$(win_field W scale)' '$(screen_scale MEM-1)' 0.0001")" "told the first screen's scale: $(win_field W scale)"
}
