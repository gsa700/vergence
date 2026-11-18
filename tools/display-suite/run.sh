#!/bin/sh
# The display suite, on the machine it tests (headless milestone 1, step 5).
# From Control: tools/display-suite.sh <host> [scenario...]. By hand:
#
#   sh tools/display-suite/run.sh [scenario...]     # e.g. 04-hotplug-add
#
# Builds the server, the checker and the test pattern into target-check
# (never the build a session runs), then runs each scenario
# (tools/display-suite/NN-*.sh) in a headless session of its own: the
# session on the monitors is never touched. One line per scenario, PASS or
# FAIL, its checks below a failure; everything in
# ~/vergence-display-suite/<time>/. Exit 1 if any failed.
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
export ROOT
TARGET=$ROOT/target-check/release
echo "building into target-check"
CARGO_TARGET_DIR=$ROOT/target-check cargo build -q --release --manifest-path "$ROOT/Cargo.toml" -p vergence-ui --example testpattern || exit 2
CARGO_TARGET_DIR=$ROOT/target-check cargo build -q --release --manifest-path "$ROOT/Cargo.toml" -p vergence-server -p vergence-harness || exit 2
PATTERN=$TARGET/examples/testpattern
HARNESS=$TARGET/vergence-harness
BASE=$HOME/vergence-display-suite/$(date +%Y%m%d-%H%M%S)
mkdir -p "$BASE"

if [ $# -gt 0 ]; then
    list=$(for n in "$@"; do ls "$ROOT"/tools/display-suite/"$n"*.sh; done)
else
    list=$(ls "$ROOT"/tools/display-suite/[0-9][0-9]-*.sh)
fi
failed=0
total=0
for f in $list; do
    name=$(basename "$f" .sh)
    OUT=$BASE/$name
    mkdir -p "$OUT"
    total=$((total + 1))
    started=$(date +%s)
    # Each in a subshell: its own variables, its own headless session.
    (
        export ROOT TARGET PATTERN HARNESS OUT
        . "$ROOT/tools/display-suite/lib.sh"
        . "$f"
        scenario
        [ "$FAILS" -eq 0 ]
    ) >"$OUT/log.txt" 2>&1
    status=$?
    "$ROOT/tools/vergence-headless" stop >/dev/null 2>&1
    cp "$HOME/.cache/vergence-headless/server.log" "$OUT/server.log" 2>/dev/null
    took=$(($(date +%s) - started))
    if [ $status -eq 0 ]; then
        echo "PASS $name (${took}s)"
    else
        echo "FAIL $name (${took}s)"
        grep -E 'FAIL|error|Error' "$OUT/log.txt" | head -8 | sed 's/^/     /'
        failed=$((failed + 1))
    fi
done
echo "$((total - failed)) of $total scenarios passed; logs and shots in $BASE"
[ $failed -eq 0 ]
