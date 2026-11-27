# AlbumWall's bar, its Avalonia app's look with Vergence's controls (Dave,
# 2026-11-27: "match the appearance color height of the old version ... but
# integrate our window controls and menu"; "we just need our logo in place of
# the standard title"): Bar::Brand's look (bus v47), the library's bar color
# with its edge as the rule, 44 points tall at the Standard size, "αlbumWall"
# with the α in the library's hue in place of the title, and the window's
# buttons still drawn by the server at its right end.
scenario() {
    command -v python3 >/dev/null || { echo "  ok   skipped: no python3 on this machine"; return 0; }
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    home=$HOME/.cache/vergence-headless/home
    rm -rf "$home/.config/vergence/albumwall" "$home/.cache/vergence/albumwall"
    vctl spawn "env VERGENCE_SOUND_OFF=pace $TARGET/vergence-albumwall" >/dev/null
    i=0; while ! vctl windows | grep -q '"AlbumWall" "vergence-albumwall"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    sleep 1.5
    set -- $(win_geo AlbumWall)
    wx=$1 wy=$2 ww=$3
    png=$(shot MEM-1 albumwall)
    [ -s "$png" ] || png=$OUT/albumwall.png
    # Two thirds across: bare ground between the name and the buttons.
    set -- $(python3 "$ROOT/tools/display-suite/bar-rows.py" "$png" $((wx + ww * 2 / 3)) "$wy") x 0
    echo "  .. the bar's ground $1, $2 rows of it above the window" >&2
    # Not the desktop's dark bar (#333D52): the library's own.
    check "$(yes_if "[ '$1' != x ] && [ '$1' != 333d52 ] && [ '$1' != 383838 ]")" "AlbumWall's bar on the library's color ($1)"
    check "$(yes_if "near '$2' 44 2")" "AlbumWall's bar 44 points tall ($2 rows at 1x)"
    # The wordmark beside the window's buttons (three of them, about 100
    # points): its α tinted, the bar and the rest of the name gray-ish.
    c=$(python3 "$ROOT/tools/display-suite/bar-rows.py" "$png" chroma $((wx + 100)) $((wy - 36)) 30 28)
    echo "  .. the wordmark's first letter: chroma $c" >&2
    check "$(yes_if "[ '$c' -gt 60 ]")" "the wordmark's α in the library's hue ($c)"
    # The window's buttons at the bar's right end: their marks lighter than
    # the bare ground beside them.
    region() { python3 "$ROOT/tools/display-suite/region.py" "$@"; }
    bare=$(region "$png" - $((wx + ww / 2)) $((wy - 34)) 100 24)
    ends=$(region "$png" - $((wx + ww - 110)) $((wy - 34)) 100 24)
    echo "  .. brightness: bare ground $bare, the buttons' end $ends" >&2
    check "$(yes_if "awk -v a='$ends' -v b='$bare' 'BEGIN { exit !(a > b + 2) }'")" "the window's buttons drawn on it"
}
