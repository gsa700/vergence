#!/bin/sh
# Run a command (cargo test) with a time limit; if it runs over, keep what
# the box knows about why (tools/gpu-report.sh, with every process under it)
# before stopping it. 2026-11-14: kepler's vergence-render GPU test hung once
# at 0% CPU and was killed with nothing kept.
#
# Usage: tools/test-guard.sh SECONDS -- cargo test -q --workspace
# Exit: the command's own, or 124 when it ran over.
set -u
limit=$1
shift
[ "${1:-}" = "--" ] && shift
here=$(cd "$(dirname "$0")" && pwd)
"$@" &
cmd=$!
waited=0
while kill -0 "$cmd" 2>/dev/null; do
    if [ "$waited" -ge "$limit" ]; then
        # Every process under the command, children first.
        all=""
        todo=$cmd
        while [ -n "$todo" ]; do
            next=""
            for p in $todo; do
                all="$all $p"
                next="$next $(pgrep -P "$p" 2>/dev/null | tr '\n' ' ')"
            done
            todo=$(echo $next)
        done
        echo "test-guard: still running after ${limit}s; saving a report on:$all" >&2
        "$here/gpu-report.sh" $all >&2
        for p in $all; do kill "$p" 2>/dev/null; done
        sleep 2
        for p in $all; do kill -9 "$p" 2>/dev/null; done
        exit 124
    fi
    sleep 1
    waited=$((waited + 1))
done
wait "$cmd"
