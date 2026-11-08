#!/bin/sh
# Offer "Vergence" at the login screen: a Wayland session file for the display
# manager, running tools/vergence-session from this checkout. Needs root (sudo).
# Undo: sudo rm /usr/local/share/wayland-sessions/vergence.desktop
set -eu
here=$(cd "$(dirname "$0")" && pwd)
dir=/usr/local/share/wayland-sessions
[ -d /usr/share/wayland-sessions ] && [ ! -d "$dir" ] && dir=/usr/share/wayland-sessions
tmp=$(mktemp)
cat > "$tmp" <<DESKTOP
[Desktop Entry]
Name=Vergence
Comment=Vergence display server (development build, $here)
Exec=$here/vergence-session
Type=Application
DesktopNames=Vergence
DESKTOP
sudo install -m 644 "$tmp" "$dir/vergence.desktop"
rm -f "$tmp"
echo "installed $dir/vergence.desktop -> $here/vergence-session"
