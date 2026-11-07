#!/bin/sh
# Install vergd for the current user and start it with the desktop session.
#
#   tools/install-autostart.sh          install (or update) and start now
#   tools/install-autostart.sh remove   stop and remove
#
# Puts the release build at ~/.local/bin/vergd (a copy, so rebuilding in the
# source tree never swaps the binary under a running daemon), an XDG
# autostart entry at ~/.config/autostart/vergd.desktop (Xfce and other
# desktops start it at login), and logs to ~/.local/state/vergence/vergd.log.
# Needs no root.
set -eu

BIN="$HOME/.local/bin/vergd"
DESKTOP="$HOME/.config/autostart/vergd.desktop"
STATE="$HOME/.local/state/vergence"
LOG="$STATE/vergd.log"

stop() { pkill -u "$(id -u)" -f "$BIN watch" 2>/dev/null || true; }

if [ "${1:-}" = remove ]; then
    stop
    rm -f "$DESKTOP" "$BIN"
    echo "removed vergd autostart and binary (profiles and log kept in ~/.config/vergence, $STATE)"
    exit 0
fi

SRC="$(cd "$(dirname "$0")/.." && pwd)/target/release/vergd"
[ -x "$SRC" ] || { echo "no release build at $SRC; run: cargo build --release" >&2; exit 1; }

mkdir -p "$HOME/.local/bin" "$HOME/.config/autostart"
mkdir -p -m 700 "$STATE"
stop
cp "$SRC" "$BIN.new" && chmod 755 "$BIN.new" && mv -f "$BIN.new" "$BIN"

cat > "$DESKTOP" <<DESK
[Desktop Entry]
Type=Application
Name=Vergence daemon
Comment=Remembers and restores each monitor setup (vergd watch)
Exec=sh -c 'exec "$BIN" watch >> "$LOG" 2>&1'
Terminal=false
X-GNOME-Autostart-enabled=true
DESK

if [ -n "${DISPLAY:-}" ]; then
    nohup sh -c "exec \"$BIN\" watch >> \"$LOG\" 2>&1" >/dev/null 2>&1 &
    sleep 2
    if pgrep -u "$(id -u)" -f "$BIN watch" >/dev/null; then
        echo "vergd running; log: $LOG"
    else
        echo "vergd did not stay up; see $LOG" >&2; exit 1
    fi
else
    echo "installed; vergd starts at the next login (DISPLAY not set, not started now)"
fi
