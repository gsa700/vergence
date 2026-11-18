#!/bin/sh
# The display check, on the machine whose session is checked (testing method,
# layer 2, 2026-11-18). Run by tools/display-check.sh from Control; by hand:
#
#   sh tools/display-check-remote.sh <screen> [scale...]
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
target=$root/target-check
screen=${1:?usage: display-check-remote.sh <screen> [scale...]}
shift
[ $# -gt 0 ] || set -- 1 1.25 1.3333333333 1.5 2

echo "building the pattern and the checker"
# Two builds: --example would leave out the checker's binary.
CARGO_TARGET_DIR=$target cargo build -q --release --manifest-path "$root/Cargo.toml" -p vergence-ui --example testpattern || exit 2
CARGO_TARGET_DIR=$target cargo build -q --release --manifest-path "$root/Cargo.toml" -p vergence-harness --bin vergence-harness || exit 2
pattern=$target/release/examples/testpattern
harness=$target/release/vergence-harness

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
        rm -f "$HOME/vergence-shots/$screen.png"
        vctl shot >/dev/null
        i=0
        while [ ! -s "$HOME/vergence-shots/$screen.png" ] && [ $i -lt 60 ]; do sleep 0.1; i=$((i + 1)); done
        sleep 0.3
        geo=$(pattern_line | sed -E 's/.*\{ x: (-?[0-9]+), y: (-?[0-9]+), width: ([0-9]+), height: ([0-9]+) \}.*bar-h=([0-9]+).*/\1 \2 \3 \4 \5/')
        read -r x y w h bar <<EOF
$geo
EOF
        shot=$out/scale$scale-step$d.png
        cp "$HOME/vergence-shots/$screen.png" "$shot"
        {
            echo "== scale $scale, step $d: $w x $h at $x,$y (bar $bar)"
            "$harness" seams "$shot" --scale "$scale" --window $((x - ox)) $((y - oy)) "$w" "$h" --bar "$bar"
        } >>"$report" 2>&1 || failed=$((failed + 1))
        checked=$((checked + 1))
    done
done

vctl scale "$screen" reset >/dev/null
pkill -f "$pattern" 2>/dev/null
grep -E '^(==|FAIL)' "$report"
echo "$checked shots checked, $failed with failures; shots and report in $out"
[ $failed -eq 0 ]
