#!/bin/sh
# Install Vergence on this machine from a release build, or stage it for a package.
#
#   sudo tools/install.sh                  under /usr/local
#   sudo tools/install.sh --prefix /usr    another prefix
#   tools/install.sh --destdir DIR         stage under DIR (packaging; no root, no system setup)
#   sudo tools/install.sh --uninstall      remove what an earlier run put there
#
# Build first, on this machine or one with the same OS and architecture:
#   cargo build --release
#
# What goes where (PREFIX=/usr/local):
#   bin/vergence-server, vergence-term, vergence-settings, vbus, vshell, vctl   the programs
#   bin/vergence-session                     the session launcher (tools/vergence-session)
#   bin/vergence -> vergence-session         start the session from a text console
#   share/wayland-sessions/vergence.desktop  "Vergence" at a display manager's login screen
#   share/applications/vergence-*.desktop    the terminal and the settings app, for the shelf and any launcher
#   share/icons/hicolor/scalable/apps/       their icons (data/icons, drawn by the project)
#   share/doc/vergence/                      LICENSE-APACHE, LICENSE-GPL, THIRD-PARTY-NOTICES.md
# User files stay in the user's home: ~/.config/vergence (settings), ~/.local/state/vergence (logs).
#
# System setup, unless --destdir: where there is no logind, seatd is enabled and
# started (FreeBSD rc.conf; runit's /var/service), and the invoking user
# (SUDO_USER / DOAS_USER, or --user NAME) joins the group that may talk to it:
# video on FreeBSD, _seatd on Void. A display manager that offers Wayland
# sessions lists Vergence after a restart.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
prefix=/usr/local
destdir=
user=${SUDO_USER:-${DOAS_USER:-}}
uninstall=
while [ $# -gt 0 ]; do
    case $1 in
        --prefix) prefix=$2; shift 2 ;;
        --destdir) destdir=$2; shift 2 ;;
        --user) user=$2; shift 2 ;;
        --uninstall) uninstall=1; shift ;;
        -h|--help) sed -n '2,24p' "$0"; exit 0 ;;
        *) echo "install.sh: unknown argument $1" >&2; exit 2 ;;
    esac
done
bin=$destdir$prefix/bin
sessions=$destdir$prefix/share/wayland-sessions
applications=$destdir$prefix/share/applications
icons=$destdir$prefix/share/icons/hicolor/scalable/apps
docs=$destdir$prefix/share/doc/vergence       # license and notices (the settings app's About page names it)
docfiles="LICENSE-APACHE LICENSE-GPL THIRD-PARTY-NOTICES.md"
programs="vergence-server vergence-term vergence-settings vbus vshell"
tools="vergence-term vergence-settings"

if [ -n "$uninstall" ]; then
    for p in $programs vctl vergence-session vergence; do rm -f "$bin/$p"; done
    rm -f "$sessions/vergence.desktop"
    for t in $tools; do rm -f "$applications/$t.desktop" "$icons/$t.svg"; done
    for d in $docfiles; do rm -f "$docs/$d"; done
    rmdir "$docs" 2>/dev/null || true
    echo "removed Vergence from $destdir$prefix (user settings and logs kept)"
    exit 0
fi

release=$root/target/release
for p in $programs; do
    [ -x "$release/$p" ] || { echo "install.sh: $release/$p is missing; run cargo build --release first" >&2; exit 1; }
done
if [ -z "$destdir" ] && [ "$(id -u)" -ne 0 ]; then
    echo "install.sh: installing under $prefix needs root (sudo tools/install.sh), or --destdir DIR to stage" >&2
    exit 1
fi

install -d -m 755 "$bin" "$sessions" "$applications" "$icons" "$docs"
for d in $docfiles; do install -m 644 "$root/$d" "$docs/$d"; done
for t in $tools; do
    install -m 644 "$root/data/applications/$t.desktop" "$applications/$t.desktop"
    install -m 644 "$root/data/icons/$t.svg" "$icons/$t.svg"
done
for p in $programs; do install -m 755 "$release/$p" "$bin/$p"; done
install -m 755 "$here/vctl" "$bin/vctl"
install -m 755 "$here/vergence-session" "$bin/vergence-session"
ln -sf vergence-session "$bin/vergence"
tmp=$(mktemp)
cat > "$tmp" <<DESKTOP
[Desktop Entry]
Name=Vergence
Comment=Vergence display server
Exec=$prefix/bin/vergence-session
Type=Application
DesktopNames=Vergence
DESKTOP
install -m 644 "$tmp" "$sessions/vergence.desktop"
rm -f "$tmp"
echo "installed $("$bin/vergence-server" --version 2>/dev/null || echo vergence) under $destdir$prefix"
[ -n "$destdir" ] && exit 0

# ---- system setup: the seat ----
if [ -d /run/systemd/system ]; then
    echo "seat: logind (systemd) manages the seat; nothing to enable"
elif [ "$(uname -s)" = FreeBSD ]; then
    command -v seatd >/dev/null || echo "seat: seatd is not installed (pkg install seatd)" >&2
    sysrc seatd_enable=YES >/dev/null && service seatd start >/dev/null 2>&1 || true
    echo "seat: seatd enabled in rc.conf and started"
    group=video
elif [ -d /etc/sv/seatd ]; then
    [ -e /var/service/seatd ] || ln -s /etc/sv/seatd /var/service/seatd
    echo "seat: seatd enabled (runit, /var/service/seatd)"
    group=_seatd
else
    echo "seat: no logind and no known seatd service; enable seatd for your init system" >&2
fi
if [ -n "${group:-}" ]; then
    if [ -z "$user" ]; then
        echo "seat: add each user who runs Vergence to the $group group (--user NAME does it)"
    elif id -Gn "$user" | tr ' ' '\n' | grep -qx "$group"; then
        echo "seat: $user is in $group"
    else
        if [ "$(uname -s)" = FreeBSD ]; then pw groupmod "$group" -m "$user"; else usermod -aG "$group" "$user"; fi
        echo "seat: $user added to $group (takes effect at the next login)"
    fi
fi
echo "start: 'vergence' from a text console, or choose Vergence at the login screen"
