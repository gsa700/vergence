#!/bin/bash
# Copy this working tree (committed or not) to a dev box, at ~/src/vergence,
# for building and testing there (see CLAUDE.md): hubble (Void Linux, the
# default), kepler (FreeBSD on real hardware), techbench
# (Fedora).
#
# rsync sends only what changed and deletes files removed here, but never
# touches build output on Hubble (target/, the size-box binaries, .NET
# bin/obj): excluded paths are left alone by --delete.
#
# Usage: tools/sync-hubble.sh [host]     (default: hubble; hosts by their DNS names)
set -euo pipefail
host=${1:-hubble}
here=$(cd "$(dirname "$0")/.." && pwd)
ssh -o BatchMode=yes "$host" 'mkdir -p ~/src/vergence'
rsync -a --delete \
    --exclude .git/ --exclude target/ --exclude /target-check/ --exclude /dist/ --exclude .DS_Store \
    --exclude tools/sizebox/bin/ \
    --exclude tools/sizebox/avalonia/bin/ --exclude tools/sizebox/avalonia/obj/ \
    "$here"/ "$host":src/vergence/
echo "synced $here -> $host:~/src/vergence"
