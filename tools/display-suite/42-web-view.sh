# Web milestone 1: a page in vergence-web-view (WebKit behind our own WPE
# platform), drawn on the GPU (dmabuf) and on the processor (shared memory,
# VERGENCE_WEB_CPU=1): the window at its size, the page's blocks where the
# page puts them, the page's title as the window's, a click on a link,
# typing into a field, Page Down. Skipped where WebKit is not installed.
scenario() {
    if [ -z "${VERGENCE_WEBKIT_LIB:-}" ] && [ ! -e /usr/lib/libWPEWebKit-2.0.so.1 ]; then
        echo "  skip WebKit is not installed (the wpewebkit package)"
        return
    fi
    page=$ROOT/tools/display-suite/web/page.html
    # web_line: the web view's line in `vctl windows` (its title changes, its app id does not).
    web_line() { vctl windows | grep '"vergence-web-view"' | head -1; }
    web_title() { web_line | sed -E 's/^[0-9]+ native "([^"]*)".*/\1/'; }
    # wait_title <pattern>: up to 8 s for the window's title to match.
    wait_title() { i=0; while ! web_title | grep -q "$1" && [ $i -lt 80 ]; do sleep 0.1; i=$((i + 1)); done; }
    # mean <png> x y w h: the region's mean color, "r g b".
    mean() {
        python3 -c "
import sys; sys.path.insert(0, '$ROOT/tools/display-suite'); import region
px = region.region(region.read('$1'), $2, $3, $4, $5)
print(*(round(sum(p[i] for p in px) / max(1, len(px))) for i in range(3)))"
    }
    # near_color \"r g b\" r g b: each channel within 12.
    near_color() { set -- $1 $2 $3 $4; [ $((${1} - $4)) -le 12 ] && [ $(($4 - ${1})) -le 12 ] && [ $((${2} - $5)) -le 12 ] && [ $(($5 - ${2})) -le 12 ] && [ $((${3} - $6)) -le 12 ] && [ $(($6 - ${3})) -le 12 ]; }
    for road in gpu cpu; do
        cpu=0
        [ "$road" = cpu ] && cpu=1
        headless "1920x1080 531x299@60" || { check 0 "headless session ($road)"; return; }
        vctl spawn "env VERGENCE_WEB_CPU=$cpu $TARGET/vergence-web-view --size 800x600 $page" >/dev/null
        wait_title "web suite"
        check "$(yes_if "web_title | grep -q 'web suite'")" "$road: the window opens, titled from the page ($(web_title))"
        sleep 1
        shot_png=$(picture "$(web_title)" "web-$road")
        set -- $(python3 -c "import struct; d = open('$shot_png', 'rb').read(24); print(*struct.unpack('>II', d[16:24]))" 2>/dev/null)
        check "$(yes_if "[ '${1:-0}' = 800 ] && [ '${2:-0}' = 600 ]")" "$road: the picture is the window's size (${1:-?}x${2:-?})"
        red=$(mean "$shot_png" 20 20 160 60)
        blue=$(mean "$shot_png" 620 20 160 60)
        white=$(mean "$shot_png" 400 450 200 100)
        check "$(yes_if "near_color '$red' 224 32 32")" "$road: the red block top left ($red)"
        check "$(yes_if "near_color '$blue' 32 96 255")" "$road: the blue block top right ($blue)"
        check "$(yes_if "near_color '$white' 255 255 255")" "$road: the page's ground ($white)"
        set -- $(web_line | sed -E 's/^([0-9]+) .*x: (-?[0-9]+), y: (-?[0-9]+).*/\1 \2 \3/')
        idx=$1 x=$2 y=$3
        vctl focus "$idx" >/dev/null
        sleep 0.3
        vctl pointer $((x + 120)) $((y + 190)) >/dev/null; sleep 0.2
        vctl press left >/dev/null; vctl release left >/dev/null
        wait_title "^clicked"
        check "$(yes_if "[ '$(web_title)' = clicked ]")" "$road: a click on the link ($(web_title))"
        vctl pointer $((x + 150)) $((y + 280)) >/dev/null; sleep 0.2
        vctl press left >/dev/null; vctl release left >/dev/null; sleep 0.3
        vctl type "abc" >/dev/null
        wait_title "typed:abc"
        check "$(yes_if "[ '$(web_title)' = typed:abc ]")" "$road: typing into the field ($(web_title))"
        vctl pointer $((x + 600)) $((y + 450)) >/dev/null
        vctl press left >/dev/null; vctl release left >/dev/null; sleep 0.3
        vctl key Page_Down >/dev/null
        wait_title "^scrolled:"
        check "$(yes_if "web_title | grep -qE '^scrolled:[1-9]'")" "$road: Page Down scrolls the page ($(web_title))"
    done
}
