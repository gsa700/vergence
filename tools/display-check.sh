#!/bin/sh
# The display check from Control (testing method, layer 2, 2026-11-18):
#
#   tools/display-check.sh <host> <screen> [scale...]
#   tools/display-check.sh techbench DP-1 1 1.25 1.3333333333 1.5 2
#
# Syncs the tree to <host> (tools/sync-hubble.sh), runs
# tools/display-check-remote.sh there in the running session (vctl; the
# scales are set for the test only and the screen's own comes back), then
# copies the report and the failing shots to ~/vergence-display-checks/.
# What it checks: crates/vergence-harness/src/seams.rs.
set -u
here=$(cd "$(dirname "$0")" && pwd)
host=${1:?usage: display-check.sh <host> <screen> [scale...]}
screen=${2:?usage: display-check.sh <host> <screen> [scale...]}
shift 2
"$here/sync-hubble.sh" "$host" >/dev/null || exit 2
ssh "$host" "sh src/vergence/tools/display-check-remote.sh $screen $*"
status=$?
last=$(ssh "$host" 'ls -td ~/vergence-display-check/*/ | head -1')
dest=$HOME/vergence-display-checks/$host-$(basename "$last")
mkdir -p "$dest"
scp -q "$host:${last}report.txt" "$dest/"
# The shots of the steps that failed, to look at.
for f in $(ssh "$host" "awk '/^== /{s=\$3; d=\$5} /^FAIL/{print \"scale\" s \"-step\" d}' ${last}report.txt | tr -d ',:' | sort -u"); do
    scp -q "$host:${last}$f.png" "$dest/" 2>/dev/null
done
echo "report: $dest/report.txt"
exit $status
