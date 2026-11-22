#!/bin/bash
# The lab's: copy the dev boxes' GPU reports (tools/gpu-report.sh,
# ~/gpu-reports) to the machine that holds the git copy, to
# ~/vergence-gpu-reports/<host>/, so reinstalling a box cannot lose them. Not
# in the repository: error states hold the GPU's memory contents.
#
# Usage: tools/lab/fetch-gpu-reports.sh [host ...]   (default: kepler hubble techbench)
set -uo pipefail
hosts=("$@")
[ ${#hosts[@]} -eq 0 ] && hosts=(kepler hubble techbench)
for h in "${hosts[@]}"; do
    mkdir -p "$HOME/vergence-gpu-reports/$h"
    if ssh -o BatchMode=yes -o ConnectTimeout=5 "$h" 'test -d ~/gpu-reports' 2>/dev/null; then
        rsync -a "$h":gpu-reports/ "$HOME/vergence-gpu-reports/$h/" && echo "$h: $(ls "$HOME/vergence-gpu-reports/$h" | wc -l | tr -d ' ') report(s)"
    else
        echo "$h: none"
    fi
done
