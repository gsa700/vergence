#!/bin/sh
# The display check, on the machine whose session is checked (testing method,
# layer 2). On your own machine:
#
#   sh tools/display-check.sh <screen> [scale...]
#   sh tools/display-check.sh --headless "<spec>" [scale...]
#
# --headless: a session of its own with one screen in memory (spec as
# tools/vergence-headless takes it, "2560x1440 597x336@60"), started and
# stopped here; the session on the monitors is not touched.
#
# Builds the test pattern (vergence-ui's testpattern example) and the
# checker (vergence-harness seams) into target-check, apart from the build
# a running session may use; opens the pattern on <screen>; for each scale
# (set with `vctl scale`, never saved; the screen's own afterwards) moves it
# one point at a time through eight places, takes a shot of the screen at
# each and checks it pixel by pixel. Shots and report.txt in
# ~/vergence-display-check/<time>/. Exit 1 if any check failed.
set -u
root=$(cd "$(dirname "$0")/.." && pwd)
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
# The checkout's vctl, never one on PATH: an older installed one does not
# know VCTL_SOCKET and would find the session on the monitors instead of the
# headless one.
vctl() { sh "$root/tools/vctl" "$@"; }
target=$root/target-check
headless=
if [ "${1:-}" = "--headless" ]; then
    headless=${2:?usage: display-check.sh --headless "<spec>" [scale...]}
    shift 2
    screen=MEM-1
else
    screen=${1:?usage: display-check.sh <screen> [scale...]}
    shift
fi
[ $# -gt 0 ] || set -- 1 1.25 1.3333333333 1.5 2

echo "building the pattern and the checker"
# Two builds: --example would leave out the checker's binary.
CARGO_TARGET_DIR=$target cargo build -q --release --manifest-path "$root/Cargo.toml" -p vergence-ui --example testpattern || exit 2
CARGO_TARGET_DIR=$target cargo build -q --release --manifest-path "$root/Cargo.toml" -p vergence-harness --bin vergence-harness || exit 2
pattern=$target/release/examples/testpattern
harness=$target/release/vergence-harness
if [ -n "$headless" ]; then
    CARGO_TARGET_DIR=$target cargo build -q --release --manifest-path "$root/Cargo.toml" -p vergence-server || exit 2
    "$root/tools/vergence-headless" stop >/dev/null 2>&1
    VERGENCE_SERVER=$target/release/vergence-server "$root/tools/vergence-headless" start --screen "$headless" || exit 2
    eval "$("$root/tools/vergence-headless" env)"
    SHOTS_DIR=$HOME/.cache/vergence-headless/home/vergence-shots
    trap '"$root/tools/vergence-headless" stop >/dev/null' EXIT
fi

# Where the server writes its shots: its HOME's (a headless session has its own).
shots=${SHOTS_DIR:-$HOME/vergence-shots}
out=$HOME/vergence-display-check/$(date +%Y%m%d-%H%M%S)
mkdir -p "$out"
report=$out/report.txt
: >"$report"

# The screen's place on the desktop and its scale, from `vctl windows`.
screen_line() {
    vctl windows | grep "^output $screen "
}
[ -n "$(screen_line)" ] || { echo "no screen $screen"; vctl windows | grep '^output'; exit 2; }

pattern_line() {
    vctl windows | grep '"Test pattern" "vergence-testpattern"' | head -1
}

pkill -f "$pattern" 2>/dev/null
vctl spawn "$pattern --size 320 200" >/dev/null
i=0
while [ -z "$(pattern_line)" ] && [ $i -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
[ -n "$(pattern_line)" ] || { echo "the pattern did not open"; exit 2; }

failed=0
checked=0
for s in "$@"; do
    vctl scale "$screen" "$s" >/dev/null
    sleep 1.5
    line=$(screen_line)
    scale=$(echo "$line" | sed -E 's/.* scale ([0-9.]+) .*/\1/')
    ox=$(echo "$line" | sed -E 's/.*\{ x: (-?[0-9]+), y: (-?[0-9]+),.*/\1/')
    oy=$(echo "$line" | sed -E 's/.*\{ x: (-?[0-9]+), y: (-?[0-9]+),.*/\2/')
    # The pointer off the pattern (it would be in the shot).
    vctl pointer $((ox + 4)) $((oy + 4)) >/dev/null
    for d in 0 1 2 3 4 5 6 7; do
        idx=$(pattern_line | cut -d' ' -f1)
        vctl move "$idx" $((ox + 40 + d)) $((oy + 80 + d)) >/dev/null
        sleep 0.6
        rm -f "$shots/$screen.png"
        vctl shot >/dev/null
        i=0
        while [ ! -s "$shots/$screen.png" ] && [ $i -lt 60 ]; do sleep 0.1; i=$((i + 1)); done
        sleep 0.3
        geo=$(pattern_line | sed -E 's/.*\{ x: (-?[0-9]+), y: (-?[0-9]+), width: ([0-9]+), height: ([0-9]+) \}.*bar-h=([0-9]+).*/\1 \2 \3 \4 \5/')
        read -r x y w h bar <<EOF
$geo
EOF
        shot=$out/scale$scale-step$d.png
        cp "$shots/$screen.png" "$shot"
        # The window's own picture, as the pattern drew it (vctl picture).
        rm -f "$shots/window-$idx.png"
        vctl picture "$idx" >/dev/null
        i=0
        while [ ! -s "$shots/window-$idx.png" ] && [ $i -lt 60 ]; do sleep 0.1; i=$((i + 1)); done
        sleep 0.2
        picture=$out/scale$scale-step$d-window.png
        cp "$shots/window-$idx.png" "$picture" 2>/dev/null
        # Where its first pixel is on the screen, and its corners' radius in pixels.
        at=$(awk -v x=$((x - ox)) -v y=$((y - oy)) -v s="$scale" 'BEGIN { printf "%d %d %d", int(x * s + 0.5), int(y * s + 0.5), int(7 * s + 0.999) }')
        set -- $at
        ax=$1 ay=$2 corners=$3
        {
            echo "== scale $scale, step $d: $w x $h at $x,$y (bar $bar)"
            ok=0
            "$harness" seams "$shot" --scale "$scale" --window $((x - ox)) $((y - oy)) "$w" "$h" --bar "$bar" || ok=1
            # What the pattern drew is what the screen shows, pixel for pixel
            # (on the grid; off it the server crops a picture a pixel larger).
            if "$harness" grid "$scale" >/dev/null 2>&1; then
                "$harness" same "$shot" "$picture" --at "$ax" "$ay" --corners "$corners" || ok=1
            else
                echo "same: off the grid, not compared"
            fi
            [ $ok -eq 0 ]
        } >>"$report" 2>&1 || failed=$((failed + 1))
        checked=$((checked + 1))
    done
done

vctl scale "$screen" reset >/dev/null
pkill -f "$pattern" 2>/dev/null
grep -E '^(==|FAIL)' "$report"
echo "$checked shots checked, $failed with failures; shots and report in $out"
[ $failed -eq 0 ]
