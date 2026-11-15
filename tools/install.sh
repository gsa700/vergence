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
#   bin/vergence-server, vergence-term, vergence-settings, vergence-calc, vergence-quit, vbus, vnative, vsound, vshell, vctl   the programs
#   bin/vergence-session                     the session launcher (tools/vergence-session)
#   bin/vergence -> vergence-session         start the session from a text console
#   share/wayland-sessions/vergence.desktop  "Vergence" at a display manager's login screen
#   share/applications/vergence-*.desktop    the terminal, the settings app, the calculator and Force Quit, for the shelf and any launcher
#   share/icons/hicolor/scalable/apps/       their icons (data/icons, drawn by the project)
#   share/doc/vergence/                      LICENSE-APACHE, LICENSE-GPL, THIRD-PARTY-NOTICES.md
#   libexec/vergence/sound-default           FreeBSD only: the root helper that sets the default
#                                            sound output (hw.snd.default_unit) and nothing else;
#                                            setuid root, group video, mode 4750
# User files stay in the user's home: ~/.config/vergence (settings), ~/.local/state/vergence (logs).
#
# System setup, unless --destdir: where there is no logind, seatd is enabled and
# started (FreeBSD rc.conf; runit's /var/service), and the invoking user
# (SUDO_USER / DOAS_USER, or --user NAME) joins the group that may talk to it:
# video on FreeBSD, _seatd on Void. A display manager that offers Wayland
# sessions lists Vergence after a restart. Where there is a Wi-Fi card and
# no NetworkManager, wpa_supplicant is enabled (runit; FreeBSD's rc.conf
# wlans_ and WPA DHCP), its control socket for the wheel group.
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
programs="vergence-server vergence-term vergence-settings vergence-calc vergence-quit vbus vnative vsound vshell"
tools="vergence-term vergence-settings vergence-calc vergence-quit"
libexec=$destdir$prefix/libexec/vergence

if [ -n "$uninstall" ]; then
    for p in $programs vctl vergence-session vergence; do rm -f "$bin/$p"; done
    rm -f "$sessions/vergence.desktop"
    for t in $tools; do rm -f "$applications/$t.desktop" "$icons/$t.svg"; done
    for d in $docfiles; do rm -f "$docs/$d"; done
    rm -f "$libexec/sound-default"
    rmdir "$libexec" 2>/dev/null || true
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
if [ "$(uname -s)" = FreeBSD ]; then
    # The sound helper: setuid root, run only by the video group (the seat's,
    # below). Staged (--destdir) without the bits; the package sets them.
    [ -x "$release/sound-default" ] || { echo "install.sh: $release/sound-default is missing; run cargo build --release first" >&2; exit 1; }
    mkdir -p "$libexec"
    if [ -z "$destdir" ]; then
        install -o root -g video -m 4750 "$release/sound-default" "$libexec/sound-default"
    else
        install -m 755 "$release/sound-default" "$libexec/sound-default"
        echo "note: $prefix/libexec/vergence/sound-default needs owner root, group video, mode 4750 in the package"
    fi
fi
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
# ---- system setup: Wi-Fi (settings milestone 1, step 4) ----
# wpa_supplicant is enabled where there is a Wi-Fi card and nothing else
# manages networks; never where there is none (a service with no card to
# manage fails and is restarted over and over). Its control socket is the
# wheel group's, so the settings app's Wi-Fi page needs no root. Whether
# the radio is on is the session's (desktop.toml [network] wifi; default:
# only while no network cable is connected).
if [ -d /run/NetworkManager ]; then
    echo "wifi: NetworkManager manages networks here; nothing to enable"
elif [ "$(uname -s)" = FreeBSD ]; then
    devs=$(sysctl -n net.wlan.devices 2>/dev/null || true)
    if [ -z "$devs" ]; then
        echo "wifi: no Wi-Fi card; nothing to enable"
    else
        dev=${devs%% *}
        conf=/etc/wpa_supplicant.conf
        if [ ! -e "$conf" ]; then
            printf 'ctrl_interface=/var/run/wpa_supplicant\nctrl_interface_group=wheel\nupdate_config=1\n' > "$conf"
            chmod 600 "$conf"
            echo "wifi: wrote $conf (control socket for the wheel group, networks saved there)"
        fi
        if [ -z "$(sysrc -n "wlans_$dev" 2>/dev/null || true)" ]; then
            sysrc "wlans_$dev=wlan0" >/dev/null
            sysrc ifconfig_wlan0="WPA DHCP" >/dev/null
            service netif start wlan0 >/dev/null 2>&1 || true
            echo "wifi: $dev as wlan0 with wpa_supplicant and DHCP (rc.conf), started"
        else
            echo "wifi: $dev already set up in rc.conf"
        fi
    fi
else
    cards=
    for d in /sys/class/net/*; do
        [ -d "$d/wireless" ] || [ -e "$d/phy80211" ] || continue
        cards="$cards ${d##*/}"
    done
    if [ -z "$cards" ]; then
        echo "wifi: no Wi-Fi card; nothing to enable"
    elif [ -d /etc/sv/wpa_supplicant ]; then
        conf=/etc/wpa_supplicant/wpa_supplicant.conf
        [ -e "$conf" ] || { mkdir -p /etc/wpa_supplicant; : > "$conf"; chmod 600 "$conf"; }
        grep -q '^ctrl_interface=' "$conf" || echo 'ctrl_interface=/run/wpa_supplicant' >> "$conf"
        grep -q '^ctrl_interface_group=' "$conf" || echo 'ctrl_interface_group=wheel' >> "$conf"
        grep -q '^update_config=' "$conf" || echo 'update_config=1' >> "$conf"
        [ -e /var/service/wpa_supplicant ] || ln -s /etc/sv/wpa_supplicant /var/service/wpa_supplicant
        echo "wifi: wpa_supplicant enabled (runit) for$cards; control socket for the wheel group"
    else
        echo "wifi: Wi-Fi card$cards: enable wpa_supplicant for your init system (ctrl_interface_group=wheel, update_config=1)" >&2
    fi
fi
if [ -n "$user" ] && ! id -Gn "$user" | tr ' ' '\n' | grep -qx wheel; then
    echo "wifi: $user is not in wheel, so the settings app cannot use Wi-Fi (add them to wheel to allow it)"
fi
# ---- system setup: a driver for every GPU (GPUs milestone 1) ----
# A second GPU shows its screens with a renderer of its own; without that
# GPU's own OpenGL, Mesa falls back to software on the CPU and its screens
# are slow (Hubble's NVIDIA card: 40 to 77 ms a 4K frame, 2026-11-14). Mesa
# runs nouveau's OpenGL through Zink, which needs NVK and the Vulkan loader.
# Said here, not installed: they are the system's packages.
if [ "$(uname -s)" = Linux ]; then
    distro=$(. /etc/os-release 2>/dev/null; echo "${ID:-}")
    for dev in /sys/class/drm/card[0-9]*/device; do
        [ -e "$dev/driver" ] || continue
        drv=$(basename "$(readlink "$dev/driver")")
        case "$drv" in
            nouveau)
                icd=; lib=
                for f in /usr/share/vulkan/icd.d/nouveau_icd*.json; do [ -e "$f" ] && icd=1; done
                for f in /usr/lib/libvulkan.so.1 /usr/lib64/libvulkan.so.1 /usr/lib/*/libvulkan.so.1; do [ -e "$f" ] && lib=1; done
                if [ -n "$icd" ] && [ -n "$lib" ]; then
                    echo "gpu: NVIDIA (nouveau): OpenGL through Zink on NVK, on the GPU"
                else
                    case "$distro" in
                        void) pkgs="mesa-vulkan-nouveau vulkan-loader (xbps-install)" ;;
                        fedora) pkgs="mesa-vulkan-drivers vulkan-loader (dnf install)" ;;
                        debian|ubuntu) pkgs="mesa-vulkan-drivers libvulkan1 (apt install)" ;;
                        *) pkgs="Mesa's NVK Vulkan driver and the Vulkan loader" ;;
                    esac
                    echo "gpu: NVIDIA (nouveau) has no OpenGL of its own: screens on it would be drawn by the CPU. Install $pkgs" >&2
                fi
                ;;
        esac
    done
fi
echo "start: 'vergence' from a text console, or choose Vergence at the login screen"
