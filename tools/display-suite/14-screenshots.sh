# Screenshots milestone 1, step 8: a capture is what the screen shows,
# pixel for pixel; an area across a 1x and a 2x screen is taken at 2x; a
# window alone has its shadow around it (and none with --no-shadow).
P1="1920x1080 531x299@60"
P2="3840x2160 600x340@60"

size_of() { "$HARNESS" same "$1" "$1" | sed -E 's/^PASS ([0-9]+)x([0-9]+).*/\1 \2/'; }

scenario() {
    headless "$P1" || { check 0 "headless session"; return; }
    pattern S --size 600 400 --rich || check 0 "the test pattern opens"
    move S 300 200
    vctl pointer 900 500 >/dev/null; sleep 0.3
    # The pointer in it, as vctl shot has it.
    vctl capture screen --pointer --to "$OUT/capture.png" >/dev/null
    i=0; while [ ! -s "$OUT/capture.png" ] && [ $i -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
    s=$(shot MEM-1 shot)
    r=$("$HARNESS" same "$OUT/capture.png" "$s" | head -1)
    # The dock's live tiles change between the two (the network's traffic,
    # the clock's minute): a few hundred pixels of digits, no more.
    n=$(echo "$r" | sed -E 's/.*: ([0-9]+) of .*/\1/')
    check "$(yes_if "echo '$r' | grep -q '^PASS' || [ '$n' -lt 600 ]")" "the screen captured is the screen shown (the dock's live tiles aside): $(echo "$r" | cut -c1-80)"

    # A window alone: its shadow's 24 points around it.
    set -- $(win_geo S)
    bar=$(win_field S bar-h)
    vctl capture window "$(win_index S)" --to "$OUT/window.png" >/dev/null
    vctl capture window "$(win_index S)" --no-shadow --to "$OUT/window-bare.png" >/dev/null
    sleep 1
    sw=$(size_of "$OUT/window.png"); bw=$(size_of "$OUT/window-bare.png")
    # Its border, a point each side, comes with it.
    want_bare="$(($3 + 2)) $(($4 + bar + 2))"
    check "$(yes_if "[ '$bw' = '$want_bare' ]")" "the window alone, bar and border and all: $bw, $want_bare wanted"
    set -- $bw
    check "$(yes_if "[ '$sw' = '$(($1 + 48)) $(($2 + 48))' ]")" "with its shadow 24 points around: $sw"

    # Across a 1x and a 2x screen: the area at 2x.
    headless "$P1" "$P2" || { check 0 "headless session"; return; }
    pattern S --size 440 300 --rich || check 0 "the test pattern opens"
    move S 1650 200
    vctl capture area 1700 150 400 300 --to "$OUT/area.png" >/dev/null
    i=0; while [ ! -s "$OUT/area.png" ] && [ $i -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
    sleep 0.3
    a=$(size_of "$OUT/area.png")
    check "$(yes_if "[ '$a' = '800 600' ]")" "an area across a 1x and a 2x screen, taken at 2x: $a"
}
