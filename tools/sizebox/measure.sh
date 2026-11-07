#!/usr/bin/env bash
# Every size box on every lit monitor (run on Hubble, in the X session).
# Usage: sh measure.sh [toolkit...]   (default: gtk3 gtk4 qt xlib java electron avalonia)
set -u
export DISPLAY=${DISPLAY:-:0} XAUTHORITY=${XAUTHORITY:-$HOME/.Xauthority}
cd "$(dirname "$0")"
H=${H:-$HOME/src/vergence/target/release/vergence-harness}
# As at a fresh login: the launch-time variables vergd wrote (decision 5).
[ -r "$HOME/.config/vergence/session.env" ] && . "$HOME/.config/vergence/session.env"
kits=${*:-gtk3 gtk4 qt xlib java electron avalonia}
cmd() {
    case $1 in
        gtk3|gtk4|qt|xlib) echo "bin/$1" ;;
        java) echo "java Box.java" ;;
        electron) echo "electron42 --no-sandbox electron" ;;
        avalonia) echo "bin/avalonia/SizeBox" ;;
    esac
}
# name x y of each lit monitor
mons=$(xrandr --listmonitors | awk 'NR>1 { split($3, g, /[\/x+]/); print $4, g[5], g[6] }')
echo "monitors: $(echo "$mons" | awk '{printf "%s at +%s+%s  ", $1, $2, $3}')"
echo "VERGENCE_SCALE=${VERGENCE_SCALE:-unset} Xft.dpi=$(xrdb -query | awk '/Xft.dpi/{print $2}')"
for k in $kits; do
    t=vergence-box-$k
    $(cmd $k) >/tmp/sizebox-$k.log 2>&1 &
    pid=$!
    id=""
    for i in $(seq 1 150); do
        id=$(wmctrl -l | awk -v t="$t" '{ n=$4; for (i=5; i<=NF; i++) n=n" "$i; if (n==t) print $1 }' | head -1)
        [ -n "$id" ] && break
        sleep 0.2
    done
    if [ -z "$id" ]; then echo "$t: no window after 30 s (see /tmp/sizebox-$k.log)"; kill $pid 2>/dev/null; continue; fi
    sleep 1
    echo "$mons" | while read name x y; do
        wmctrl -i -r "$id" -e "0,$((x + 150)),$((y + 150)),-1,-1"
        sleep 1.5
        $H measure "$t"
    done
    wmctrl -i -c "$id"; sleep 0.5; kill $pid 2>/dev/null; wait $pid 2>/dev/null
done
