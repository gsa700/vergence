#!/bin/sh
# The tab-click test (toolkit milestone 1, acceptance 5: "a tab click shown
# within two frames, 50 of 50"), against the stress test's terminal while its
# six tabs run:
#
#   VTERM_FRAMES=1 tools/vterm-stress/run.sh 5 &     (wait for "vterm pid")
#   tools/vterm-stress/clicks.sh [CLICKS]
#
# It presses each tab in turn through tools/vctl (so it MOVES THE POINTER of
# the session it runs in), then reads from the terminal's log how many frames
# after the press each tab was drawn: "within two frames" is within two of
# the monitor's frame times (REFRESH=hertz, 30 if not given: Hubble's 4K), from
# the press reaching the terminal to its frame being built. It assumes the window buttons the
# terminal draws are on the left, two of them (close, minimize), as on Hubble;
# LEFT_BUTTONS=n says otherwise.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
clicks=${1:-50}
dir=$(ls -dt "${TMPDIR:-/tmp}"/vterm-stress.*/ | head -1)
log=${dir}vterm.log
# The terminal's window: the last one listed with its program's name.
set -- $("$here/vctl" windows | grep '"vergence-term"' | tail -1 | sed 's/.*x: \([0-9-]*\), y: \([0-9-]*\), width: \([0-9]*\).*/\1 \2 \3/')
wx=$1 wy=$2 ww=$3
before=$(wc -l < "$log" | tr -d ' ')
n=0
while [ "$n" -lt "$clicks" ]; do
    n=$((n + 1))
    # 5, 4, 3, 2, 1, 0, 5, ...: never the tab already in front, after the first.
    tab=$(( (n * 5) % 6 ))
    # bar.rs: six units of margin, each button 32 and 2 apart, the tabs after them
    # sharing what is left before + (34) and the margin.
    at=$(awk -v w="$ww" -v i="$tab" -v b="${LEFT_BUTTONS:-2}" 'BEGIN { start = 6 + b * 34 + (b > 0 ? 2 : 0); tw = (w - 6 - start - 36 - 12) / 6; if (tw > 200) tw = 200; printf "%.0f", start + i * (tw + 2) + tw / 2 }')
    "$here/vctl" pointer $((wx + at)) $((wy + 21)) >/dev/null
    "$here/vctl" press left >/dev/null
    "$here/vctl" release left >/dev/null
    sleep 0.2
done
sleep 0.5
# A line reads: tab 5 asked after frame 123, drawn in frame 124, 0.1 ms later
sed -n "$((before + 1)),\$p" "$log" | grep '^tab ' | awk -v asked="$clicks" -v hz="${REFRESH:-30}" '
    { frames = ($10 + 0) - ($6 + 0); ms = $11 + 0; if (frames <= 2 && ms <= 2000 / hz) quick++; if (frames > most) most = frames; if (ms > slowest) slowest = ms; sum += ms; n++ }
    END { printf "%d clicks, %d tab changes drawn; %d within two frames (%.1f ms at %d Hz); at most %d frame(s) after the press arrived; %.2f ms on average and %.2f ms at worst from the press arriving to its frame being built\n", asked, n, quick, 2000 / hz, hz, most, n ? sum / n : 0, slowest }'
