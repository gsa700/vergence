#!/bin/sh
# What a dev box knows about a GPU hang, saved before anything is killed
# (2026-11-14: a test hung on kepler and was killed with nothing kept, and the
# 2026-11-10 hang dump was lost with kepler's reinstall).
#
# Usage: tools/gpu-report.sh [pid ...]
#
# Writes ~/gpu-reports/<host>-<time>/ with: the system and its GPU stack's
# versions; for each pid given (and every vergence program running), its
# state, what each thread waits on and its kernel stacks; the i915 error
# state if the driver kept one; the kernel's log; the server's log's end.
# FreeBSD and Linux. Root (sudo -n, never a password prompt) reads the kernel
# stacks of others' processes and the error state; without it, what can be
# read is, and the report says what could not. Prints the report's folder.
# Copy the reports to Control with tools/fetch-gpu-reports.sh.
set -u
os=$(uname -s)
host=$(hostname -s 2>/dev/null || hostname)
dir="$HOME/gpu-reports/$host-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$dir" || exit 1
root() { sudo -n "$@" 2>/dev/null; }
say() { printf '%s\n' "$*" >>"$dir/summary.txt"; }

say "gpu-report on $host at $(date '+%Y-%m-%d %H:%M:%S %Z')"
say "uname: $(uname -a)"
# The GPU stack's versions.
{
    if [ "$os" = FreeBSD ]; then
        freebsd-version -ku
        pkg info -x 'drm-.*kmod' mesa-dri mesa-libs 'gpu-firmware-intel' 2>/dev/null
        sysctl hw.dri 2>/dev/null | grep -E 'name|busid' | head
    else
        cat /etc/os-release 2>/dev/null | grep -E '^(PRETTY_NAME|VERSION)='
        for c in /sys/class/drm/card[0-9]; do
            [ -e "$c/device/driver" ] && echo "$(basename "$c"): $(basename "$(readlink "$c/device/driver")")"
        done
        ls /usr/lib*/dri/ 2>/dev/null | head -5
        command -v xbps-query >/dev/null && xbps-query -s mesa-dri 2>/dev/null | head -3
        command -v rpm >/dev/null && rpm -q mesa-dri-drivers 2>/dev/null
    fi
} >"$dir/versions.txt" 2>&1

# The processes: those given, and every vergence program and test binary.
pids="$*"
for p in $(pgrep -f 'vergence-|target/(debug|release)/(deps|build)/' 2>/dev/null); do
    case " $pids " in *" $p "*) ;; *) pids="$pids $p" ;; esac
done
for p in $pids; do
    f="$dir/process-$p.txt"
    {
        echo "== $p: $(ps -o command= -p "$p" 2>/dev/null)"
        if [ "$os" = FreeBSD ]; then
            ps -H -o pid,lwp,state,wchan,%cpu,time,command -p "$p" 2>/dev/null
            echo "-- threads (procstat -t)"
            procstat -t "$p" 2>/dev/null || root procstat -t "$p"
            echo "-- kernel stacks (procstat -kk)"
            procstat -kk "$p" 2>/dev/null || root procstat -kk "$p" || echo "(could not be read)"
        else
            grep -E '^(Name|State|Threads)' "/proc/$p/status" 2>/dev/null
            echo "-- each thread: state, what it waits on"
            for t in /proc/"$p"/task/*; do
                echo "$(basename "$t") $(awk '{print $3}' "$t/stat" 2>/dev/null) $(cat "$t/wchan" 2>/dev/null)"
            done
            echo "-- kernel stacks"
            for t in /proc/"$p"/task/*; do
                echo "thread $(basename "$t"):"
                cat "$t/stack" 2>/dev/null || root cat "$t/stack" || echo "(could not be read: needs root)"
            done
        fi
    } >"$f" 2>&1
    say "process $p: $(ps -o command= -p "$p" 2>/dev/null | cut -c1-120)"
done

# The i915 error state: what the GPU was doing when the driver last reset it.
err="$dir/i915-error-state.txt"
if [ "$os" = FreeBSD ]; then
    dbg=/tmp/gpu-report-dbg
    if [ ! -e "$dbg/dri/0" ]; then
        mkdir -p "$dbg"
        root mount -t lindebugfs lindebugfs "$dbg"
    fi
    # `cat` fails on this file (2026-11-10); `head -c` reads it.
    root head -c 8000000 "$dbg/dri/0/i915_error_state" >"$err"
else
    for c in /sys/class/drm/card[0-9]; do
        if [ "$(basename "$(readlink "$c/device/driver" 2>/dev/null)")" = i915 ] && [ -e "$c/error" ]; then
            root head -c 8000000 "$c/error" >"$err"
        fi
    done
fi
if [ "$os" = FreeBSD ]; then src="$dbg/dri/0/i915_error_state"; else src=$(ls /sys/class/drm/card*/error 2>/dev/null | head -1); fi
if [ ! -s "$err" ] && ! root test -e "$src"; then
    say "i915 error state: none could be read (no root, or not this driver)"
elif [ ! -s "$err" ] || head -1 "$err" | grep -q 'No error state collected'; then
    say "i915 error state: none (the driver has not reset the GPU since boot)"
else
    say "i915 error state: KEPT ($(wc -c <"$err" | tr -d ' ') bytes): the GPU was reset"
fi

# The kernel's log: everything recent, and every GPU line since boot.
{
    if [ "$os" = FreeBSD ]; then
        dmesg | tail -300
        echo "-- GPU lines in /var/log/messages"
        grep -i -E 'drm|i915|gpu hang|reset' /var/log/messages 2>/dev/null | tail -100
    else
        dmesg 2>/dev/null | tail -300 || root dmesg | tail -300
    fi
} >"$dir/kernel-log.txt" 2>&1
hangs=$(grep -c -i -E 'gpu hang|resetting rcs|preemption time' "$dir/kernel-log.txt")
say "kernel log: $hangs line(s) of GPU hang or reset"
say "server log: $(grep -a -c -i -E 'GPU hang suspected|new GL context|watchdog' "$HOME"/.local/state/vergence/server.log 2>/dev/null || echo 0) line(s) of the server's own hang detection"
tail -400 "$HOME/.local/state/vergence/server.log" >"$dir/server-log-end.txt" 2>/dev/null

cat "$dir/summary.txt"
echo "$dir"
