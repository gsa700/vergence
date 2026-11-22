#!/bin/sh
# Offer "Vergence (dev)" at the login screen: a Wayland session file for the
# display manager, running tools/vergence-session from this checkout. Needs root
# (sudo). Undo: sudo rm <dir>/vergence-dev.desktop (the script says which dir).
#
# Its own file name: tools/install.sh writes vergence.desktop for the installed
# build, and a display manager lists one entry per file name, so a dev file of
# that name hid one of the two (Techbench's GDM started the installed build,
# 2026-11-12). An older dev vergence.desktop (Exec not in a bin directory) is
# removed here.
#
# Not used on Hubble since 2026-11-07: lightdm's Wayland sessions on FreeBSD
# (no logind) raced the outgoing X server, could not restart the server (seatd
# could not set the VT mode), and once looped starting greeters. Kept for a
# display manager that handles Wayland sessions well.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
dir=/usr/local/share/wayland-sessions
[ -d /usr/share/wayland-sessions ] && [ ! -d "$dir" ] && dir=/usr/share/wayland-sessions
tmp=$(mktemp)
cat > "$tmp" <<DESKTOP
[Desktop Entry]
Name=Vergence (dev)
Comment=Vergence display server (development build, $here)
Exec=$here/vergence-session
Type=Application
DesktopNames=Vergence
DESKTOP
sudo install -m 644 "$tmp" "$dir/vergence-dev.desktop"
rm -f "$tmp"
echo "installed $dir/vergence-dev.desktop -> $here/vergence-session"
for d in /usr/local/share/wayland-sessions /usr/share/wayland-sessions; do
    old=$d/vergence.desktop
    if [ -f "$old" ] && grep -q '^Exec=.*/tools/vergence-session$' "$old"; then
        sudo rm -f "$old"
        echo "removed the older dev entry $old"
    fi
done
