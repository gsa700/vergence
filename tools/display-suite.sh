#!/bin/sh
# The display suite from Control (headless milestone 1, step 5):
#
#   tools/display-suite.sh <host> [scenario...]
#   tools/display-suite.sh hubble 04-hotplug-add 05-hotplug-remove
#
# Syncs the tree to <host> and runs tools/display-suite/run.sh there: each
# scenario (tools/display-suite/NN-*.sh) in a headless session of its own,
# the session on the monitors untouched. Before every push of display code,
# on Hubble, kepler and techbench (Dave, 2026-11-18), while it stays a few
# minutes. Logs and shots stay on the host in ~/vergence-display-suite/.
set -u
here=$(cd "$(dirname "$0")" && pwd)
host=${1:?usage: display-suite.sh <host> [scenario...]}
shift
"$here/sync-hubble.sh" "$host" >/dev/null || exit 2
ssh "$host" "sh src/vergence/tools/display-suite/run.sh $*" 2>&1 | sed "s/^/$host: /"
