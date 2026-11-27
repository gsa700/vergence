# AlbumWall's bar, its Avalonia app's look with Vergence's controls (Dave,
# 2026-11-27: "match the appearance color height of the old version ... but
# integrate our window controls and menu"): Bar::Brand's look (bus v47), the
# warm near-black ground #1C1915 with its rule, 44 points tall at the Standard
# size, and the window's buttons still drawn by the server at its right end.
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
    set -- $(python3 "$ROOT/tools/display-suite/bar-rows.py" "$png" $((wx + ww * 2 / 3)) "$wy" 1c1915 2b2620) x 0
    echo "  .. the bar's ground $1, $2 rows of it above the window" >&2
    check "$(yes_if "[ '$1' = 1c1915 ]")" "AlbumWall's bar on its own warm near-black ($1)"
    check "$(yes_if "near '$2' 44 2")" "AlbumWall's bar 44 points tall ($2 rows at 1x)"
    # The window's buttons at the bar's right end: their marks lighter than
    # the bare ground beside them.
    region() { python3 "$ROOT/tools/display-suite/region.py" "$@"; }
    bare=$(region "$png" - $((wx + ww / 2)) $((wy - 34)) 100 24)
    ends=$(region "$png" - $((wx + ww - 110)) $((wy - 34)) 100 24)
    echo "  .. brightness: bare ground $bare, the buttons' end $ends" >&2
    check "$(yes_if "awk -v a='$ends' -v b='$bare' 'BEGIN { exit !(a > b + 2) }'")" "the window's buttons drawn on it"
}
