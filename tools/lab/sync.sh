#!/bin/bash
# The lab's: copy this working tree (committed or not) from the machine that
# holds the git copy to a dev box, at ~/src/vergence, for building and
# testing there (CLAUDE.md has the lab's machines).
#
# rsync sends only what changed and deletes files removed here, but never
# touches build output on the box (target/, target-check/, the size-box
# binaries, .NET bin/obj): excluded paths are left alone by --delete.
# The Void templates' -devel subpackage symlinks are skipped: macOS's
# openrsync makes them but then fails setting their permissions (exit 23),
# and a dev box builds from the checkout, never from packaging/.
#
# Usage: tools/lab/sync.sh [host]     (default: hubble; hosts by their DNS names)
set -euo pipefail
host=${1:-hubble}
here=$(cd "$(dirname "$0")/../.." && pwd)
ssh -o BatchMode=yes "$host" 'mkdir -p ~/src/vergence'
rsync -a --delete \
    --exclude .git/ --exclude target/ --exclude /target-check/ --exclude /dist/ --exclude .DS_Store \
    --exclude tools/lab/sizebox/bin/ \
    --exclude '/packaging/void/srcpkgs/*-devel' \
    --exclude tools/lab/sizebox/avalonia/bin/ --exclude tools/lab/sizebox/avalonia/obj/ \
    "$here"/ "$host":src/vergence/
echo "synced $here -> $host:~/src/vergence"
