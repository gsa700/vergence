# Display lists milestone 1, step 7: windows drawn by the server from their
# scenes. Drawn from its scene, a window is the same picture its app draws
# itself (at 1x and 2x); across two screens of different scales, its half
# on the denser one is sharper than its own pixels stretched; magnified,
# sharper than its pixels enlarged; no window switched to pixels.
P1="1920x1080 531x299@60"
P2="3840x2160 600x340@60"

# The test pattern, drawn by the server (scene) or by itself (pixels).
open_as() { # open_as <scene|pixels> <title> <args...>
    how=$1; shift
    if [ "$how" = pixels ]; then
        keep=$PATTERN; PATTERN="env VERGENCE_UI_PIXELS=1 $PATTERN"; pattern "$@"; r=$?; PATTERN=$keep; return $r
    fi
    pattern "$@"
}

scenario() {
    # The same picture, at 1x and 2x.
    for screen in "$P1" "$P2"; do
        for how in scene pixels; do
            headless "$screen" || { check 0 "headless session"; return; }
            open_as $how S --size 600 400 --rich || check 0 "the test pattern opens ($how)"
            move S 100 100
            s=$(screen_scale MEM-1)
            eval "pic_$how=\$(picture S same-$how-$s)"
        done
        r=$("$HARNESS" same "$pic_scene" "$pic_pixels" | head -1)
        check "$(yes_if "echo '$r' | grep -q '^PASS'")" "at ${s}x, drawn from its scene as its app draws it: $(echo "$r" | cut -c1-80)"
    done

    # Across a 1x and a 2x screen: the 2x half.
    for how in scene pixels; do
        headless "$P1" "$P2" || { check 0 "headless session"; return; }
        # 270 of its 440 points on the 1x screen: its scale stays 1; its
        # text and shapes past 170 points are on the 2x screen.
        open_as $how S --size 440 300 --rich || check 0 "the test pattern opens ($how)"
        move S 1650 200
        sleep 0.5
        check "$(yes_if "near '$(win_field S scale)' 1 0.0001")" "($how) its scale the 1x screen's: $(win_field S scale)"
        eval "two_$how=\$(shot MEM-2 two-$how)"
        [ $how = scene ] && { sc=$(vctl scenes); echo "  $sc"; }
    done
    n=$(echo "$sc" | sed -E 's/.* ([0-9]+) drawn again.*/\1/')
    check "$(yes_if "[ '$n' -gt 0 ]")" "drawn again for the 2x screen: $n"
    # Its text and shapes on MEM-2 (y 224-400 pt), in that screen's pixels.
    r=$("$HARNESS" sharper "$two_scene" "$two_pixels" 0 448 540 352)
    check "$(yes_if "echo '$r' | grep -q '^PASS'")" "its 2x half sharper from its scene than stretched: $r"

    # Magnified at 3x.
    for how in scene pixels; do
        headless "$P1" || { check 0 "headless session"; return; }
        open_as $how S --size 900 600 --rich || check 0 "the test pattern opens ($how)"
        move S 300 200
        vctl pointer 2 2 >/dev/null; sleep 0.3
        vctl zoom focus 500 300 >/dev/null
        vctl zoom toggle >/dev/null; vctl zoom in >/dev/null; vctl zoom in >/dev/null
        sleep 2.5
        z=$(vctl zoom)
        eval "zoom_$how=\$(shot MEM-1 zoom-$how)"
        [ $how = scene ] && { sc=$(vctl scenes); echo "  $z; $sc"; }
    done
    # Its text and shapes (324-724, 224-400 pt) magnified about 500,300.
    r=$("$HARNESS" sharper "$zoom_scene" "$zoom_pixels" 0 60 1200 560)
    check "$(yes_if "echo '$r' | grep -q '^PASS'")" "magnified ($(echo "$z" | cut -d' ' -f1-3)), sharper from its scene than enlarged: $r"
    off=$(echo "$sc" | sed -E 's/.* ([0-9]+) switched to pixels.*/\1/')
    check "$(yes_if "[ '$off' = 0 ]")" "no window switched to pixels: $off"
}
