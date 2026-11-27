# Two of Dave's wishes (2026-11-27), checked by their pictures (region.py):
# AlbumWall's playing mark breathes (the Avalonia app's slow pulse: the same
# place looks different half a breath later), and scroll bars widen as the
# pointer comes for them: the toolkit's (AlbumWall's wall), a text area's own
# (Edit) and the terminal's. A server album from the suite's stand-in plays,
# its songs 12 seconds long, so the mark has time to breathe.
scenario() {
    command -v python3 >/dev/null || { echo "  ok   skipped: no python3 on this machine"; return 0; }
    region() { python3 "$ROOT/tools/display-suite/region.py" "$@"; }
    more() { awk -v a="$1" -v b="$2" 'BEGIN { exit !(a > b) }'; }
    port=47543
    WAV_SECONDS=12 python3 "$ROOT/tools/display-suite/subsonic-standin.py" $port good > "$OUT/standin.log" 2>&1 &
    SRV=$!
    sleep 0.5
    headless "1920x1080 531x299@60" || { check 0 "headless session"; kill $SRV 2>/dev/null; return; }
    home=$HOME/.cache/vergence-headless/home
    rm -rf "$home/.config/vergence/albumwall" "$home/.cache/vergence/albumwall"
    printf '\n[terminal]\nblink = false\n' >> "$home/.config/vergence/desktop.toml"
    vctl spawn "env VERGENCE_SOUND_OFF=pace $TARGET/vergence-albumwall" >/dev/null
    i=0; while ! vctl windows | grep -q '"AlbumWall" "vergence-albumwall"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    sleep 1
    w=$(win_index AlbumWall)
    node() { vctl tree "$1" | grep -F "$2" | grep -oE '#[0-9]+' | head -1 | tr -d '#'; }
    press() { vctl act "$1" "$(node "$1" "$2")" press >/dev/null; }
    # pic <window index> <name>: its own picture.
    pic() {
        rm -f "$SHOTS/window-$1.png"
        vctl picture "$1" >/dev/null
        i=0; while [ ! -s "$SHOTS/window-$1.png" ] && [ $i -lt 80 ]; do sleep 0.1; i=$((i + 1)); done
        sleep 0.2
        cp "$SHOTS/window-$1.png" "$OUT/$2.png"
    }
    # Signed in to the stand-in, Lossless only, as scenario 37 does.
    press "$w" 'button "Sign in to a server'
    i=0; while ! vctl windows | grep -q '"Add a Music Server"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 25 ] && break; done
    s=$(vctl windows | grep '"Add a Music Server"' | head -1 | cut -d' ' -f1)
    vctl act "$s" "$(node "$s" 'field "Server"')" set "127.0.0.1:$port" >/dev/null
    vctl act "$s" "$(node "$s" 'field "User name"')" set dave >/dev/null
    vctl focus "$s" >/dev/null; vctl act "$s" "$(node "$s" 'field "Password"')" focus >/dev/null; sleep 0.3
    vctl type sesame >/dev/null; sleep 0.3
    press "$s" 'button "Sign in"'
    i=0; while ! vctl tree "$s" | grep -q 'checkbox "Lossy"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 25 ] && break; done
    press "$s" 'checkbox "Lossy"'; sleep 0.3
    press "$s" 'button "Add'
    # Album 10, mid-wall: room above and below for its panel to be centered.
    i=0; while ! vctl tree "$w" | grep -q 'button "Server Album 10, Artist 2"'; do sleep 0.3; i=$((i + 1)); [ $i -gt 40 ] && break; done
    press "$w" 'button "Server Album 10, Artist 2"'
    i=0; while ! vctl tree "$w" | grep -q 'button "Play Server Album 10"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 25 ] && break; done
    press "$w" 'button "Play Server Album 10"'
    i=0; while ! vctl tree "$w" | grep -q 'button "Pause"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 30 ] && break; done
    sleep 1.0

    # The playing mark: where the first song's number was (the playing song's
    # button is named without it), half a breath apart.
    set -- $(vctl tree "$w" | grep -E 'button "Song 1 ' | head -1 | sed -nE 's/.* at (-?[0-9]+),(-?[0-9]+) [0-9]+x([0-9]+).*/\1 \2 \3/p') 0 0 0
    mx=$1 my=$2 mh=$3
    # Three pictures a quarter of a breath (2.2 s) apart: whatever the phase,
    # one pair differs by at least half the full swing (two half a breath apart
    # can meet at the same opacity, crossing its middle).
    pic "$w" mark-a; sleep 0.55; pic "$w" mark-b; sleep 0.55; pic "$w" mark-c
    d=0
    for pair in "a b" "a c" "b c"; do
        set -- $pair
        x=$(region "$OUT/mark-$1.png" "$OUT/mark-$2.png" "$mx" "$my" 40 "$mh")
        more "$x" "$d" && d=$x
    done
    echo "  .. the mark at $mx,$my: at most $d apart over half a breath" >&2
    check "$(yes_if "[ '$mh' != 0 ] && more '$d' 0.5")" "the playing track's mark breathes ($d)"

    # The wall's scroll bar (the toolkit's): the pointer away, then at its edge.
    set -- $(win_geo AlbumWall)
    wx=$1 wy=$2
    set -- $(vctl tree "$w" | grep -F 'scroll area "wall"' | sed -nE 's/.* at (-?[0-9]+),(-?[0-9]+) ([0-9]+)x([0-9]+).*/\1 \2 \3 \4/p')
    sx=$1 sy=$2 sw=$3 sh=$4
    vctl pointer $((wx + sx + sw / 2)) $((wy + sy + sh / 2)) >/dev/null; sleep 0.5; pic "$w" wall-away
    vctl pointer $((wx + sx + sw - 4)) $((wy + sy + sh / 2)) >/dev/null; sleep 0.5; pic "$w" wall-near
    d=$(region "$OUT/wall-away.png" "$OUT/wall-near.png" $((sx + sw - 16)) "$sy" 16 "$sh")
    check "$(yes_if "more '$d' 0.5")" "the wall's scroll bar widens as the pointer comes ($d)"

    # The play bar's cover, the wall scrolled away: the playing album's panel
    # back in the middle of what can be seen (the wall less the play bar),
    # not the album and its panel averaged (Dave, 2026-11-27).
    vctl focus "$w" >/dev/null; vctl key Home >/dev/null; sleep 1.0
    press "$w" 'button "Go to Server Album 10"'
    sleep 1.2
    t=$(vctl tree "$w")
    set -- $(echo "$t" | grep -E 'button "Play Server Album 10"' | head -1 | sed -nE 's/.* at -?[0-9]+,(-?[0-9]+) [0-9]+x([0-9]+).*/\1 \2/p') 0 0
    mid=$(($1 + $2 / 2)); seen=$(( (sh - 82) / 2 ))
    echo "  .. after the cover click: the sleeve's middle at $mid, what can be seen's middle at $seen" >&2
    check "$(yes_if "[ '$2' != 0 ] && near $mid $seen 60")" "the play bar's cover brings the panel back to the middle"
    # A press on the play bar's bare part stays on the bar: nothing on the wall opens or closes.
    before=$(echo "$t" | grep -c 'button "Play Server Album 10"')
    set -- $(win_geo AlbumWall)
    # Between the track's words and the transport buttons: nothing there to press.
    vctl pointer $(($1 + $3 * 22 / 100)) $(($2 + sy + sh - 40)) >/dev/null; vctl press left >/dev/null; vctl release left >/dev/null; sleep 0.8
    after=$(vctl tree "$w" | grep -c 'button "Play Server Album 10"')
    check "$(yes_if "[ '$before' = 1 ] && [ '$after' = 1 ]")" "a click on the play bar's bare part does not reach the wall"
    press "$w" 'button "Pause"'
    kill $SRV 2>/dev/null; wait $SRV 2>/dev/null

    # The terminal's: three screens of lines, then the same.
    vctl spawn "$TARGET/vergence-term" >/dev/null
    i=0; while ! vctl windows | grep -q '"vergence-term"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    sleep 1.5
    t=$(vctl windows | grep '"vergence-term"' | head -1 | cut -d' ' -f1)
    vctl focus "$t" >/dev/null; sleep 0.3
    vctl type "seq 1 300" >/dev/null; vctl key Return >/dev/null; sleep 1
    set -- $(vctl windows | grep '"vergence-term"' | head -1 | sed -E 's/.*\{ x: (-?[0-9]+), y: (-?[0-9]+), width: ([0-9]+), height: ([0-9]+) \}.*/\1 \2 \3 \4/')
    tx=$1 ty=$2 tw=$3 th=$4
    vctl pointer $((tx + tw / 2)) $((ty + th / 2)) >/dev/null; sleep 0.5; pic "$t" term-away
    vctl pointer $((tx + tw - 4)) $((ty + th / 2)) >/dev/null; sleep 0.5; pic "$t" term-near
    pw=$(python3 -c "import struct,sys; print(struct.unpack('>I', open(sys.argv[1],'rb').read()[16:20])[0])" "$OUT/term-near.png")
    d=$(region "$OUT/term-away.png" "$OUT/term-near.png" $((pw - 20)) 30 20 $((th - 60)))
    check "$(yes_if "more '$d' 0.5")" "the terminal's scroll bar widens as the pointer comes ($d)"

    # A text area's own (Edit): a long file.
    seq 1 400 > "$OUT/long.txt"
    vctl spawn "$TARGET/vergence-edit $OUT/long.txt" >/dev/null
    i=0; while ! vctl windows | grep -q '"vergence-edit"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    sleep 1.5
    e=$(vctl windows | grep '"vergence-edit"' | head -1 | cut -d' ' -f1)
    set -- $(vctl windows | grep '"vergence-edit"' | head -1 | sed -E 's/.*\{ x: (-?[0-9]+), y: (-?[0-9]+), width: ([0-9]+), height: ([0-9]+) \}.*/\1 \2 \3 \4/')
    ex=$1 ey=$2 ew=$3 eh=$4
    vctl pointer $((ex + ew / 2)) $((ey + eh / 2)) >/dev/null; sleep 0.5; pic "$e" edit-away
    vctl pointer $((ex + ew - 4)) $((ey + eh / 2)) >/dev/null; sleep 0.5; pic "$e" edit-near
    # The picture's own size (the window's rectangle is not the picture's).
    pw=$(python3 -c "import struct,sys; print(struct.unpack('>I', open(sys.argv[1],'rb').read()[16:20])[0])" "$OUT/edit-near.png")
    d=$(region "$OUT/edit-away.png" "$OUT/edit-near.png" $((pw - 20)) 30 20 $((eh - 100)))
    check "$(yes_if "more '$d' 0.5")" "Edit's scroll bar widens as the pointer comes ($d)"
}
