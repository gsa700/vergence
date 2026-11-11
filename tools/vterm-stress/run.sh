#!/bin/sh
# The six-tab stress test of the terminal (toolkit milestone 1, step 5;
# acceptance 5), on the machine it is run on, in the running Vergence session
# (through tools/vctl, so the session needs its control socket).
#
#   tools/vterm-stress/run.sh [MINUTES] [vterm options]
#
# Opens one vterm with six tabs: btop, htop, `seq 1 1000000` again and again,
# the color flood, the cursor test, the Unicode flood (examples/flood.rs).
# Every minute it writes a line to the report: each job's rate over that
# minute, the terminal's memory, and how much of a core each of its threads
# took. The report and the jobs' own logs stay in the directory it names.
# Per-thread figures are read from /proc and so are Linux's only; elsewhere
# the line has the process's total from ps.
#
# ("vterm" was the terminal's name while the first one, on iced, still stood.)
#
# With VTERM_FRAMES=1 in the environment the terminal counts its frames and
# times tab changes on its error output (vterm.log), which clicks.sh reads.
#
# Build first: cargo build --release -p vergence-term --example flood
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$here/.." && pwd)
minutes=${1:-3}
[ $# -gt 0 ] && shift
dir=$(mktemp -d "${TMPDIR:-/tmp}/vterm-stress.XXXXXX")
cat > "$dir/start.sh" <<START
#!/bin/sh
export VSTRESS_DIR="$dir"
export VSTRESS_FLOOD="$root/target/release/examples/flood"
export SHELL="$here/vterm-stress/shell.sh"
${VTERM_FRAMES:+export VTERM_FRAMES=1}
exec "$root/target/release/vergence-term" --tabs 6 $* 2>"$dir/vterm.log"
START
chmod +x "$dir/start.sh"
"$here/vctl" spawn "$dir/start.sh" >/dev/null
sleep 3
pid=$(pgrep -n -x vergence-term)
echo "vterm pid $pid, report in $dir/report.txt"
echo "$pid" > "$dir/pid"

# Jiffies each thread has used so far: "tid name jiffies" a line.
threads() {
    for t in /proc/"$pid"/task/*; do
        [ -r "$t/stat" ] || continue
        # The name is in brackets and may hold spaces; the times are the 14th
        # and 15th fields counted after it.
        sed 's/^\([0-9]*\) (\(.*\)) \(.*\)$/\1 \2|\3/' "$t/stat" | awk -F'|' '{ split($2, f, " "); split($1, n, " "); name = substr($1, length(n[1]) + 2); gsub(/ /, "_", name); print n[1], name, f[12] + f[13] }'
    done
}
tick=$(getconf CLK_TCK 2>/dev/null || echo 100)
# (BSD's wc pads its count with spaces.)
last() { if [ -f "$1" ]; then wc -l < "$1" | tr -d ' '; else echo 0; fi; }

minute=0
[ -d /proc/"$pid"/task ] && threads > "$dir/before"
for f in seq color cursor unicode; do eval "seen_$f=0"; done
while [ "$minute" -lt "$minutes" ]; do
    sleep 60
    minute=$((minute + 1))
    kill -0 "$pid" 2>/dev/null || { echo "minute $minute: vterm is GONE" | tee -a "$dir/report.txt"; exit 1; }
    line="minute $minute:"
    for f in seq color cursor unicode; do
        eval "from=\$seen_$f"
        now=$(last "$dir/$f.log")
        eval "seen_$f=$now"
        if [ "$now" -gt "$from" ]; then
            line="$line $f $(sed -n "$((from + 1)),${now}p" "$dir/$f.log" | awk '{ s += $2; if (min == "" || $2 < min) min = $2; if ($2 > max) max = $2 } END { printf "%.3g (%.3g-%.3g, n=%d)", s / NR, min, max, NR }')"
        else
            line="$line $f NONE"
        fi
    done
    if [ -d /proc/"$pid"/task ]; then
        rss=$(awk '/^VmRSS/ { printf "%.1f", $2 / 1024 }' /proc/"$pid"/status)
        threads > "$dir/after"
        cpu=$(awk -v tick="$tick" 'NR == FNR { was[$1] = $3; next } { used = ($3 - was[$1]) / tick / 60 * 100; sum[$2] += used; if (used > top[$2]) top[$2] = used; n[$2]++ } END { for (k in sum) printf " %s x%d: %.0f%% (most %.0f%%)", k, n[k], sum[k], top[k] }' "$dir/before" "$dir/after")
        mv "$dir/after" "$dir/before"
        line="$line | RSS ${rss} MB | threads, % of a core:$cpu"
    else
        line="$line | $(ps -o rss= -o pcpu= -p "$pid" | awk '{ printf "RSS %.1f MB, cpu %s%%", $1 / 1024, $2 }')"
    fi
    echo "$line" | tee -a "$dir/report.txt"
done
echo "done; vterm $pid left running (kill it to end the jobs)"
