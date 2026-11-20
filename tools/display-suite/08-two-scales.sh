# Two screens of different scale side by side: a window takes the scale
# and text of the screen most of it is on, and draws at that many pixels.
scenario() {
    headless "1920x1080 531x299@60" "3840x2160 600x340@60" || { check 0 "headless session"; return; }
    pattern W --size 400 300 || check 0 "window opens"
    move W 200 200
    check "$(yes_if "near '$(win_field W scale)' 1 0.0001")" "on the first screen, scale 1: $(win_field W scale)"
    set -- $(screen_geo MEM-2)
    move W $(($1 + 100)) 200
    sleep 1
    s2=$(screen_scale MEM-2)
    check "$(yes_if "near '$(win_field W scale)' '$s2' 0.0001")" "on the second, its scale $s2: $(win_field W scale)"
    check "$(yes_if "near '$(win_field W text)' 1.0 0.001")" "and its text, 1.0x at 200% of a 27\" 4K: $(win_field W text)"
    set -- $(win_geo W)
    p=$(picture W window)
    px=$("$HARNESS" same "$p" "$p" | sed -E 's/^PASS ([0-9]+)x([0-9]+).*/\1 \2/')
    want="$(awk -v w="$3" -v h="$4" -v s="$s2" 'BEGIN { printf "%d %d", w * s, h * s }')"
    check "$(yes_if "[ '$px' = '$want' ]")" "drawn at its pixels: $px, $want wanted"
}
