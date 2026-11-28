#!/bin/bash
# The lab's: rebuild the wpewebkit package for a new WPE WebKit release
# (web milestone 1: WebKit ships security releases about monthly; the
# package is ours, packaging/void/srcpkgs/wpewebkit).
#
#   tools/lab/wpewebkit-update.sh            check, and if a newer release of
#                                            our series is out, update the
#                                            template and build it
#   tools/lab/wpewebkit-update.sh --check    only say what is out
#   tools/lab/wpewebkit-update.sh --series   allow a new series (2.54 -> 2.56)
#
# Stable releases have an even minor number (2.54.x); odd ones are
# development snapshots and never taken. A new series is only reported
# unless --series: it may change WPE's class layouts (vergence-web refuses a
# WebKit whose classes are not the size it was written for) or no longer
# take patches/obscured-content-insets.patch, so it wants a look first.
#
# The build runs on techbench (xbps-src in ~/.cache/vergence-lab/void-packages,
# about 18 minutes); the package goes into techbench's and Hubble's local
# repositories. Installing is left to the commands printed at the end
# (techbench's needs Dave's sudo). Then: tools/lab/display-suite.sh <host>
# 42-web-view on each, and commit the template.
set -euo pipefail
here=$(cd "$(dirname "$0")/../.." && pwd)
template=$here/packaging/void/srcpkgs/wpewebkit/template
check_only=0 new_series=0
for a in "$@"; do
    case "$a" in
    --check) check_only=1 ;;
    --series) new_series=1 ;;
    *) echo "usage: $0 [--check] [--series]" >&2; exit 2 ;;
    esac
done

current=$(sed -n 's/^version=//p' "$template")
series=${current%.*}
# The releases wpewebkit.org lists, stable ones only (even minor), newest last.
releases=$(curl -sSfL https://wpewebkit.org/releases/ |
    grep -oE 'wpewebkit-2\.[0-9]+\.[0-9]+\.tar\.xz' | sed -E 's/wpewebkit-(.*)\.tar\.xz/\1/' |
    awk -F. '$2 % 2 == 0' | sort -u -t. -k1,1n -k2,2n -k3,3n)
newest=$(printf '%s\n' "$releases" | tail -1)
in_series=$(printf '%s\n' "$releases" | awk -F. -v s="$series" '$1 "." $2 == s' | tail -1)
echo "packaged: $current; newest stable: $newest; newest of $series: ${in_series:-none}"

target=$in_series
if [ "${newest%.*}" != "$series" ]; then
    if [ "$new_series" = 1 ]; then
        target=$newest
    else
        echo "a new series is out ($newest): read its release notes, then run with --series"
    fi
fi
if [ -z "$target" ] || [ "$target" = "$current" ]; then
    echo "nothing newer to build"
    exit 0
fi
[ "$check_only" = 1 ] && { echo "would build $target"; exit 0; }

# Upstream's checksum (the .sums file beside each tarball, over HTTPS);
# xbps-src checks the tarball it fetches against it.
sum=$(curl -sSfL "https://wpewebkit.org/releases/wpewebkit-$target.tar.xz.sums" |
    sed -n 's/.*sha256sum: *\([0-9a-f]\{64\}\).*/\1/p' | head -1)
[ -n "$sum" ] || { echo "no sha256 for $target on wpewebkit.org" >&2; exit 1; }
echo "building $target (sha256 $sum)"
sed -i '' -e "s/^version=.*/version=$target/" -e "s/^revision=.*/revision=1/" -e "s/^checksum=.*/checksum=$sum/" "$template"

vp=.cache/vergence-lab/void-packages
rsync -r "$here/packaging/void/srcpkgs/wpewebkit/" "techbench:$vp/srcpkgs/wpewebkit/"
log=.cache/vergence-lab/logs/wpewebkit-$target.log
# Long and heavy: nice'd, -j16 at most (27 GB at the peak on techbench's 31).
ssh techbench "cd ~/$vp && ./xbps-src binary-bootstrap >/dev/null 2>&1; nice -n 19 ./xbps-src -j16 pkg wpewebkit > ~/$log 2>&1" || {
    echo "the build failed: techbench:~/$log (a patch that no longer applies shows near its top)" >&2
    exit 1
}
pkg=wpewebkit-${target}_1.x86_64.xbps
ssh techbench "cp ~/$vp/hostdir/binpkgs/$pkg ~/.local/share/vergence-repo/ && cd ~/.local/share/vergence-repo && xbps-rindex -a $pkg"
tmp=$(mktemp -d)
scp -q "techbench:$vp/hostdir/binpkgs/$pkg" "$tmp/"
ssh hubble 'mkdir -p ~/.cache/vergence-lab/web-repo'
scp -q "$tmp/$pkg" hubble:.cache/vergence-lab/web-repo/
rm -rf "$tmp"
ssh hubble "cd ~/.cache/vergence-lab/web-repo && xbps-rindex -a $pkg"
cat <<EOF
built wpewebkit $target; in both local repositories. Install:
  techbench: sudo xbps-install -u --repository=\$HOME/.local/share/vergence-repo wpewebkit
  hubble:    ssh hubble 'sudo xbps-install -yu --repository=\$HOME/.cache/vergence-lab/web-repo wpewebkit'
then: tools/lab/display-suite.sh techbench 42-web-view; tools/lab/display-suite.sh hubble 42-web-view
and commit packaging/void/srcpkgs/wpewebkit/template.
EOF
