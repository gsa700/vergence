# Helpers for the display suite's scenarios (headless milestone 1, step 5).
# Sourced by tools/display-suite/run.sh before each scenario; POSIX sh.
# Everything talks to the scenario's own headless session (VCTL_SOCKET),
# through the checkout's vctl.

vctl() { sh "$ROOT/tools/vctl" "$@"; }

# Start this scenario's session with its screens ("WxH MMxMM@HZ" each).
headless() {
    "$ROOT/tools/vergence-headless" stop >/dev/null 2>&1
    n=$#
    for s in "$@"; do set -- "$@" --screen "$s"; done
    shift "$n"
    VERGENCE_SERVER="$TARGET/vergence-server" "$ROOT/tools/vergence-headless" start "$@" >/dev/null || return 1
    eval "$("$ROOT/tools/vergence-headless" env)"
    SHOTS=$HOME/.cache/vergence-headless/home/vergence-shots
    mkdir -p "$SHOTS"
}

# Checks: each prints a line; any failure makes the scenario fail.
FAILS=0
check() { # check <0|1 (1 = ok)> <what>
    if [ "$1" = 1 ]; then echo "  ok   $2"; else echo "  FAIL $2"; FAILS=$((FAILS + 1)); fi
}
yes_if() { if eval "$1"; then echo 1; else echo 0; fi; }
# near <a> <b> <tolerance>: floats within tolerance.
near() { awk -v a="$1" -v b="$2" -v t="$3" 'BEGIN { d = a - b; if (d < 0) d = -d; exit !(d <= t) }'; }

# A screen's line in `vctl screen`; its x, y, width, height in points; its scale.
screen_line() { vctl screen | grep "^$1 "; }
screen_geo() { screen_line "$1" | sed -E 's/.* ([0-9]+)x([0-9]+) pt at (-?[0-9]+),(-?[0-9]+) scale.*/\3 \4 \1 \2/'; }
screen_scale() { screen_line "$1" | sed -E 's/.* scale ([0-9.]+).*/\1/'; }

# A window by its title: its line in `vctl windows`, index, place and size
# in points, and a field such as scale= or text= or bar-h=.
win_line() { vctl windows | grep "\"$1\" \"" | head -1; }
win_index() { win_line "$1" | cut -d' ' -f1; }
win_geo() { win_line "$1" | sed -E 's/.*\{ x: (-?[0-9]+), y: (-?[0-9]+), width: ([0-9]+), height: ([0-9]+) \}.*/\1 \2 \3 \4/'; }
win_field() { win_line "$1" | sed -E "s/.* $2=([^ ]+).*/\\1/"; }
win_on() { win_line "$1" | sed -E 's/.* on \["([^"]+)".*/\1/'; }

# Open the test pattern (vergence-ui's example) titled $1, extra arguments
# after; wait for its window.
pattern() {
    title=$1
    shift
    vctl spawn "$PATTERN --title $title $*" >/dev/null
    i=0
    while [ -z "$(win_line "$title")" ] && [ $i -lt 80 ]; do sleep 0.1; i=$((i + 1)); done
    sleep 0.5
    [ -n "$(win_line "$title")" ]
}

move() { # move <title> <x> <y>
    vctl move "$(win_index "$1")" "$2" "$3" >/dev/null
    sleep 0.6
}

# A shot of a screen, copied to $OUT/<name>.png; prints the path.
shot() { # shot <screen> <name>
    rm -f "$SHOTS/$1.png"
    vctl shot >/dev/null
    i=0
    while [ ! -s "$SHOTS/$1.png" ] && [ $i -lt 80 ]; do sleep 0.1; i=$((i + 1)); done
    sleep 0.3
    cp "$SHOTS/$1.png" "$OUT/$2.png"
    echo "$OUT/$2.png"
}

# A window's own picture (vctl picture), copied to $OUT/<name>.png.
picture() { # picture <title> <name>
    idx=$(win_index "$1")
    rm -f "$SHOTS/window-$idx.png"
    vctl picture "$idx" >/dev/null
    i=0
    while [ ! -s "$SHOTS/window-$idx.png" ] && [ $i -lt 80 ]; do sleep 0.1; i=$((i + 1)); done
    sleep 0.3
    cp "$SHOTS/window-$idx.png" "$OUT/$2.png"
    echo "$OUT/$2.png"
}

# Is a window wholly inside a screen? inside <title> <screen>
inside() {
    set -- $(win_geo "$1") $(screen_geo "$2")
    [ "$1" -ge "$5" ] && [ "$2" -ge "$6" ] && [ $(($1 + $3)) -le $(($5 + $7)) ] && [ $(($2 + $4)) -le $(($6 + $8)) ]
}
