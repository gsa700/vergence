#!/bin/sh
# The lab's: the display suite run on a dev box from the machine that holds
# the git copy (headless milestone 1, step 5):
#
#   tools/lab/display-suite.sh <host> [scenario...]
#   tools/lab/display-suite.sh hubble 04-hotplug-add 05-hotplug-remove
#
# Syncs the tree to <host> (tools/lab/sync.sh) and runs
# tools/display-suite/run.sh there: each scenario in a headless session of
# its own, the session on the monitors untouched. On your own machine, run
# tools/display-suite/run.sh directly. Logs and shots stay on the host in
# ~/.cache/vergence-lab/display-suite/.
set -u
here=$(cd "$(dirname "$0")" && pwd)
host=${1:?usage: display-suite.sh <host> [scenario...]}
shift
"$here/sync.sh" "$host" >/dev/null || exit 2
ssh "$host" "sh src/vergence/tools/display-suite/run.sh $*" 2>&1 | sed "s/^/$host: /"
