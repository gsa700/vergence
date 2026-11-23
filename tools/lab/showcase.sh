#!/bin/sh
# Screenshots for the README and the profile page, made in fresh headless
# sessions with Vergence's shipped defaults (no one's own settings or files):
#
#   tools/lab/showcase.sh [OUTDIR]        (on a box with Vergence installed)
#
# Each shot: a new session (state cleared, the test profile plus the shot's own
# settings), sample files from this checkout under /tmp/vergence-showcase, the
# windows placed, then `vctl shot`. PNGs land in OUTDIR (default
# ~/vergence-showcase); copy them back and look at them before using them.
set -u
cd "$(dirname "$0")/../.." || exit 1
ROOT=$(pwd)
OUT=${1:-$HOME/vergence-showcase}
mkdir -p "$OUT"
H=$HOME/.cache/vergence-headless/home
DEMO=/tmp/vergence-showcase
export VERGENCE_DEFAULTS=${VERGENCE_DEFAULTS:-/usr/local/share/vergence/defaults/desktop.toml}
[ -f "$VERGENCE_DEFAULTS" ] || VERGENCE_DEFAULTS=/usr/share/vergence/defaults/desktop.toml

# The session's programs from this checkout, all of them (a headless session
# runs programs from target-check/release first; an old one there would show).
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
CARGO_TARGET_DIR="$ROOT/target-check" cargo build -q --release --workspace --bins || exit 2

# Sample files: Vergence's own documents and pictures.
rm -rf "$DEMO"
mkdir -p "$DEMO/Documents" "$DEMO/Pictures" "$DEMO/Music"
cp docs/Vergence-Whitepaper.pdf CONTRIBUTING.md README.md STATUS.md "$DEMO/Documents/"
cp data/backgrounds/*.svg "$DEMO/Pictures/"
[ -f docs/screenshots/desktop.jpg ] && cp docs/screenshots/desktop.jpg "$DEMO/Pictures/vergence-desktop.jpg"

vctl() { "$ROOT/tools/vctl" "$@"; }
# A fresh session; $1: lines added to its desktop.toml (the shot's settings).
fresh() {
    tools/vergence-headless stop >/dev/null 2>&1
    rm -rf "$H/.local/state" "$H/vergence-shots" "$H/.config/vergence/desktop.toml"
    tools/vergence-headless start --screen "2560x1440 597x336@60" >/dev/null 2>&1 || { echo "showcase: no session" >&2; exit 1; }
    eval "$(tools/vergence-headless env)"
    [ -n "${1:-}" ] && printf '\n%s\n' "$1" >> "$H/.config/vergence/desktop.toml"
    sleep 3
}
# open <program and arguments>: started, and waited for.
open() {
    n=$(vctl windows | grep -c ' native ')
    vctl spawn "$1" >/dev/null
    i=0; while [ "$(vctl windows | grep -c ' native ')" -le "$n" ] && [ $i -lt 50 ]; do sleep 0.2; i=$((i + 1)); done
    sleep 1.5
}
# at <app id> <x> <y>: that app's (first) window moved there.
at() {
    i=$(vctl windows | grep "\"$1\"" | head -1 | cut -d' ' -f1)
    [ -n "$i" ] && vctl move "$i" "$2" "$3" >/dev/null
    sleep 0.4
}
front() {
    i=$(vctl windows | grep "\"$1\"" | head -1 | cut -d' ' -f1)
    [ -n "$i" ] && vctl focus "$i" >/dev/null
    sleep 1
}
# shot <name>: the screen, saved as OUT/<name>.png.
shot() {
    sleep 2
    rm -f "$H/vergence-shots/MEM-1.png"
    vctl shot >/dev/null
    i=0; while [ ! -s "$H/vergence-shots/MEM-1.png" ] && [ $i -lt 50 ]; do sleep 0.2; i=$((i + 1)); done
    sleep 1
    cp "$H/vergence-shots/MEM-1.png" "$OUT/$1.png" && echo "showcase: $OUT/$1.png"
}
icons='[files]
view = "icons"'
# press <app id> <role> <name prefix>: that widget pressed, as a click would.
press() {
    w=$(vctl windows | grep "\"$1\"" | head -1 | cut -d' ' -f1)
    n=$(vctl tree "$w" | grep -E "$2 \"$3" | grep -oE '#[0-9]+' | head -1 | tr -d '#')
    [ -n "$n" ] && vctl act "$w" "$n" press >/dev/null
    sleep 0.6
}
# field <app id> <name> <value>: a text field set and Enter pressed in it.
field() {
    w=$(vctl windows | grep "\"$1\"" | head -1 | cut -d' ' -f1)
    n=$(vctl tree "$w" | grep -E "text field \"$2" | grep -oE '#[0-9]+' | head -1 | tr -d '#')
    [ -n "$n" ] && vctl act "$w" "$n" set "$3" >/dev/null && vctl act "$w" "$n" press >/dev/null
    sleep 1
}
maximized() {
    front "$1"
    vctl key super+Up >/dev/null
    sleep 1.5
}

# 1. Reader: the whitepaper as a book's spread (two pages, pages 2 and 3),
# on its own PDF engine.
fresh
open "vergence-reader $DEMO/Documents/Vergence-Whitepaper.pdf"
maximized vergence-reader
press vergence-reader button "Two pages"
field vergence-reader "Page number" 3
shot reader

# 2. Dark style, Tide behind: the windows take its color.
fresh "[appearance]
color-scheme = \"dark\"

[wallpaper]
image = \"$(dirname "$VERGENCE_DEFAULTS")/../../backgrounds/vergence/vergence-tide.svg\"

$icons"
open "vergence-settings --page appearance"
open "vergence-files $DEMO/Pictures"
at vergence-settings 300 130
at vergence-files 1240 520
front vergence-files
shot dark-tint

# (FileView's preview: left out until its pane grows with the window; the
# pane's page is too small to read, and a Space preview covers the list.)

# 4. Edit and Terminal: the workspace's Cargo.toml beside a terminal
# building the tree (a fresh target folder, so it is compiling when shot).
fresh
open "vergence-edit $ROOT/Cargo.toml"
open "vergence-term -e sh -c 'cd $ROOT && CARGO_TARGET_DIR=/tmp/vergence-showcase-build cargo build --release; exec sh'"
at vergence-edit 160 110
at vergence-term 1140 360
front vergence-term
sleep 12
shot edit-terminal
rm -rf /tmp/vergence-showcase-build

# 5. The overview (Super+Tab): every window at a glance.
fresh "$icons"
open "vergence-reader $DEMO/Documents/Vergence-Whitepaper.pdf"
open "vergence-files $DEMO/Pictures"
open "vergence-edit $DEMO/Documents/README.md"
open "vergence-settings --page wallpaper"
open "vergence-calc"
vctl key super+Tab >/dev/null; sleep 2
shot overview

# 6. Settings > Screens: the scale and its sharp marks.
fresh
open "vergence-settings --page screens"
at vergence-settings 860 160
shot screens

tools/vergence-headless stop >/dev/null 2>&1
rm -rf "$DEMO"
