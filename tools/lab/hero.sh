#!/bin/sh
# The README's top picture, "this is Vergence", in a few variations, each in a
# fresh headless 4K session with the shipped defaults (no one's own files):
#
#   tools/lab/hero.sh [OUTDIR] [VARIATION...]   (on a box with Vergence installed)
#
# Variations: tide-dark ember-dark graphite-dark dusk-light mist-light (all by
# default). Each shot: the wallpaper and its tint, the frosted dock with its
# tiles, and four of Vergence's own apps overlapping (FileView, Reader on the
# 17-days story, Terminal, Settings > Appearance); no third-party programs.
# PNGs land in OUTDIR (default ~/.cache/vergence-lab/hero); look at them before use.
# The README's picture (docs/screenshots/desktop.jpg) is dusk-default, the
# desktop as shipped (Dave, 2026-11-22), made 2560 wide at JPEG quality 85.
# Variations are <wallpaper>-<style>; style default is App default.
set -u
cd "$(dirname "$0")/../.." || exit 1
ROOT=$(pwd)
OUT=${1:-${XDG_CACHE_HOME:-$HOME/.cache}/vergence-lab/hero}
[ $# -gt 0 ] && shift
VARIATIONS=${*:-tide-dark ember-dark graphite-dark dusk-light mist-light}
mkdir -p "$OUT"
H=$HOME/.cache/vergence-headless/home
DEMO=/tmp/vergence-hero
export VERGENCE_DEFAULTS=${VERGENCE_DEFAULTS:-/usr/local/share/vergence/defaults/desktop.toml}
[ -f "$VERGENCE_DEFAULTS" ] || VERGENCE_DEFAULTS=/usr/share/vergence/defaults/desktop.toml
BG=$(dirname "$VERGENCE_DEFAULTS")/../../backgrounds/vergence

# The session's programs from this checkout (a headless session runs programs
# from target-check/release first).
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
CARGO_TARGET_DIR="$ROOT/target-check" cargo build -q --release --workspace --bins || exit 2

rm -rf "$DEMO"
mkdir -p "$DEMO/Documents" "$DEMO/Pictures"
cp docs/Vergence-17-Days.pdf docs/Vergence-Whitepaper.pdf README.md STATUS.md "$DEMO/Documents/"
cp data/backgrounds/*.svg "$DEMO/Pictures/"
for f in docs/screenshots/*.jpg docs/screenshots/*.png; do [ -f "$f" ] && cp "$f" "$DEMO/Pictures/"; done

vctl() { "$ROOT/tools/vctl" "$@"; }
fresh() {
    tools/vergence-headless stop >/dev/null 2>&1
    rm -rf "$H/.local/state" "$H/.cache/vergence-lab/shots" "$H/.config/vergence/desktop.toml"
    # A 27-inch 4K screen, as on techbench.
    tools/vergence-headless start --screen "3840x2160 597x336@60" >/dev/null 2>&1 || { echo "hero: no session" >&2; exit 1; }
    eval "$(tools/vergence-headless env)"
    printf '\n%s\n' "$1" >> "$H/.config/vergence/desktop.toml"
    sleep 3
}
open() {
    n=$(vctl windows | grep -c ' native ')
    vctl spawn "$1" >/dev/null
    i=0; while [ "$(vctl windows | grep -c ' native ')" -le "$n" ] && [ $i -lt 50 ]; do sleep 0.2; i=$((i + 1)); done
    sleep 1.5
}
# at <app id> <x%> <y%>: that app's window moved to a share of the screen.
at() {
    i=$(vctl windows | grep "\"$1\"" | head -1 | cut -d' ' -f1)
    [ -n "$i" ] && vctl move "$i" $((W * $2 / 100)) $((HT * $3 / 100)) >/dev/null
    sleep 0.4
}
# size <app id> <w%> <h%>: that app's window resized to a share of the
# screen, by dragging its lower right corner as a person would.
size() {
    g=$(vctl windows | grep "\"$1\"" | head -1 | sed -n 's/.*x: \([0-9-]*\), y: \([0-9-]*\), width: \([0-9]*\), height: \([0-9]*\).*/\1 \2 \3 \4/p')
    [ -n "$g" ] || return
    set -- $g "$2" "$3"
    cx=$(($1 + $3 - 2)); cy=$(($2 + $4 - 2))
    tx=$(($1 + W * $5 / 100)); ty=$(($2 + HT * $6 / 100))
    vctl pointer $cx $cy >/dev/null; sleep 0.3
    vctl press left >/dev/null; sleep 0.2
    for k in 1 2 3 4 5 6 7 8; do
        vctl pointer $((cx + (tx - cx) * k / 8)) $((cy + (ty - cy) * k / 8)) >/dev/null; sleep 0.05
    done
    vctl release left >/dev/null; sleep 0.6
    vctl pointer 2 2 >/dev/null
}
front() {
    i=$(vctl windows | grep "\"$1\"" | head -1 | cut -d' ' -f1)
    [ -n "$i" ] && vctl focus "$i" >/dev/null
    sleep 1
}
shot() {
    sleep 3
    rm -f "$H/.cache/vergence-lab/shots/MEM-1.png"
    vctl shot >/dev/null
    i=0; while [ ! -s "$H/.cache/vergence-lab/shots/MEM-1.png" ] && [ $i -lt 50 ]; do sleep 0.2; i=$((i + 1)); done
    sleep 1
    cp "$H/.cache/vergence-lab/shots/MEM-1.png" "$OUT/$1.png" && echo "hero: $OUT/$1.png"
}

for v in $VARIATIONS; do
    wall=${v%-*}; scheme=${v##*-}
    fresh "[appearance]
color-scheme = \"$scheme\"

[wallpaper]
image = \"$BG/vergence-$wall.svg\"

[files]
view = \"icons\""
    # 150%, the scale a 27-inch 4K screen is most often used at.
    vctl scale MEM-1 1.5 >/dev/null; sleep 2
    # The screen's size in points, for placing windows by share.
    W=$(vctl windows | sed -n 's/.*width: \([0-9]*\).*/\1/p' | head -1)
    HT=$(vctl windows | sed -n 's/.*height: \([0-9]*\).*/\1/p' | head -1)
    open "vergence-settings --page appearance"
    open "vergence-files $DEMO/Pictures"
    open "vergence-reader $DEMO/Documents/Vergence-17-Days.pdf"
    open "vergence-term -e sh -c 'cd $ROOT && git log --graph --oneline --decorate --color=always -60 | cut -c1-110; exec sh'"
    at vergence-settings 3 4
    size vergence-settings 34 44
    at vergence-term 5 55
    size vergence-term 34 32
    at vergence-files 28 26
    size vergence-files 31 32
    at vergence-reader 57 4
    size vergence-reader 38 84
    front vergence-settings
    front vergence-files
    front vergence-term
    front vergence-reader
    shot "hero-$v"
done

tools/vergence-headless stop >/dev/null 2>&1
rm -rf "$DEMO"
