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
#   bin/vergence-server, vergence-term, vergence-settings, vergence-calc, vergence-quit, vergence-hw, vergence-edit, vergence-closeup, vergence-files, vergence-open, vbus, vtile, vnative, vsound, vshell, vctl   the programs
#   bin/vergence-session                     the session launcher (tools/vergence-session)
#   bin/vergence -> vergence-session         start the session from a text console
#   share/wayland-sessions/vergence.desktop  "Vergence" at a display manager's login screen
#   share/applications/vergence-*.desktop    the terminal, the settings app, the calculator, Force Quit, Hardware and the text editor, for the shelf and any launcher
#   share/icons/hicolor/scalable/apps/       their icons (data/icons, drawn by the project)
#   share/doc/vergence/                      LICENSE-APACHE, LICENSE-GPL, THIRD-PARTY-NOTICES.md
#   share/vergence/words/                    the spelling list (data/words/en_US.txt, from SCOWL) and its terms
#   libexec/vergence/ups-tile, vtile-demo  lab tiles on the dock from scripts (tools/lab)
#   libexec/vergence/vergence-choose         the Open/Save dialog the portal shows for apps not ours
#   libexec/vergence/vergence-decode         pictures decoded apart, limited (Capsicum on FreeBSD)
#   libexec/vergence/media-eject             FreeBSD only: the root helper that unmounts a drive
#                                            under /media (setuid root, video group)
#   /etc/autofs/special_vergence_media       FreeBSD only: the automounter's /media map (FAT
#                                            drives writable by video; a special map, so it lists)
#   libexec/vergence/sound-default           FreeBSD only: the root helper that sets the default
#                                            sound output (hw.snd.default_unit) and nothing else;
#                                            setuid root, group vergence, mode 4750 (its own group,
#                                            not the seat's: VDM review I5)
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
programs="vergence-server vergence-term vergence-settings vergence-calc vergence-quit vergence-hw vergence-edit vergence-closeup vergence-files vergence-open vbus vtile vnative vsound vshell vdm-try"
tools="vergence-term vergence-settings vergence-calc vergence-quit vergence-hw vergence-edit vergence-closeup vergence-files"
libexec=$destdir$prefix/libexec/vergence
sbin=$destdir$prefix/sbin
words=$destdir$prefix/share/vergence/words

if [ -n "$uninstall" ]; then
    for p in $programs vctl vergence-session vergence; do rm -f "$bin/$p"; done
    rm -f "$sbin/vdm"
    rm -f "$sessions/vergence.desktop"
    for t in $tools; do rm -f "$applications/$t.desktop" "$icons/$t.svg"; done
    for d in $docfiles; do rm -f "$docs/$d"; done
    rm -f "$libexec/media-eject"
    [ -z "$destdir" ] && [ "$(uname -s)" = FreeBSD ] && rm -f /etc/autofs/special_vergence_media
    rm -f "$libexec/ups-tile" "$libexec/vtile-demo"
    rm -f "$libexec/sound-default" "$libexec/vergence-choose" "$libexec/vergence-decode" "$words/en_US.txt" "$words/SCOWL-COPYRIGHT.txt"
    rmdir "$words" "$destdir$prefix/share/vergence" 2>/dev/null || true
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
# vdm, VDM's root daemon (docs/vdm-milestone-1.md): it checks the lock
# screen's password for the account that asks, and always runs (question 3).
[ -x "$release/vdm" ] || { echo "install.sh: $release/vdm is missing; run cargo build --release first" >&2; exit 1; }
install -d -m 755 "$sbin"
install -m 755 "$release/vdm" "$sbin/vdm"
# The spelling list every native app checks with (vergence-spell finds it beside bin/).
install -d -m 755 "$words"
install -m 644 "$root/data/words/en_US.txt" "$words/en_US.txt"
install -m 644 "$root/data/words/SCOWL-COPYRIGHT.txt" "$words/SCOWL-COPYRIGHT.txt"
install -m 755 "$here/vctl" "$bin/vctl"
# Lab tiles (dock milestone 1): a NUT UPS and a demonstration tile, scripts on vtile.
install -d -m 755 "$libexec"
install -m 755 "$here/lab/ups-tile" "$libexec/ups-tile"
install -m 755 "$here/lab/vtile-demo" "$libexec/vtile-demo"
if [ "$(uname -s)" = FreeBSD ]; then
    # The sound helper: setuid root, run only by the vergence group, a group of
    # its own (VDM review I5: the seat's group, video, will also have the
    # greeter's account in it, which must not run these). The installing user
    # joins it (from the next login). Staged (--destdir) without the bits; the
    # package sets them.
    [ -x "$release/sound-default" ] || { echo "install.sh: $release/sound-default is missing; run cargo build --release first" >&2; exit 1; }
    mkdir -p "$libexec"
    if [ -z "$destdir" ]; then
        pw groupshow vergence >/dev/null 2>&1 || pw groupadd vergence
        if [ -n "$user" ] && ! id -Gn "$user" | tr ' ' '\n' | grep -qx vergence; then
            pw groupmod vergence -m "$user"
            echo "helpers: $user added to the vergence group (sound output and eject, from the next login)"
        fi
    fi
    if [ -z "$destdir" ]; then
        install -o root -g vergence -m 4750 "$release/sound-default" "$libexec/sound-default"
    else
        install -m 755 "$release/sound-default" "$libexec/sound-default"
        echo "note: $prefix/libexec/vergence/sound-default needs owner root, group vergence, mode 4750 in the package"
    fi
    # Removable drives (files milestone 1): the eject helper, as the sound
    # helper; the automounter's /media map.
    [ -x "$release/media-eject" ] || { echo "install.sh: $release/media-eject is missing; run cargo build --release first" >&2; exit 1; }
    if [ -z "$destdir" ]; then
        install -o root -g vergence -m 4750 "$release/media-eject" "$libexec/media-eject"
    else
        install -m 755 "$release/media-eject" "$libexec/media-eject"
        echo "note: $prefix/libexec/vergence/media-eject needs owner root, group vergence, mode 4750 in the package"
    fi
    # A special map (auto_master's -vergence_media) lives in /etc/autofs:
    # only special maps are asked for the list of what is there.
    if [ -z "$destdir" ]; then
        install -m 755 "$here/freebsd/media-map" /etc/autofs/special_vergence_media
    else
        mkdir -p "$destdir/etc/autofs" && install -m 755 "$here/freebsd/media-map" "$destdir/etc/autofs/special_vergence_media"
    fi
fi
# Our Open/Save dialog for apps not ours (the portal's FileChooser; files milestone 1).
[ -x "$release/vergence-choose" ] || { echo "install.sh: $release/vergence-choose is missing; run cargo build --release first" >&2; exit 1; }
mkdir -p "$libexec"
install -m 755 "$release/vergence-choose" "$libexec/vergence-choose"
# Pictures decoded apart, limited (files milestone 1, step 4).
[ -x "$release/vergence-decode" ] || { echo "install.sh: $release/vergence-decode is missing; run cargo build --release first" >&2; exit 1; }
install -m 755 "$release/vergence-decode" "$libexec/vergence-decode"
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
# ---- system setup: vdm (VDM milestone 1) ----
# Our own PAM service for unlocking (review I2): the system's usual password
# stack, never "other". Fedora's system-auth asks for a fingerprint first and
# waits for it, so there it is password-auth (a fingerprint becomes a second
# service of its own later, review I9). vdm refuses to start without this file.
if [ -f /etc/pam.d/password-auth ]; then stack=password-auth
elif [ -f /etc/pam.d/system-auth ]; then stack=system-auth
elif [ -f /etc/pam.d/system ]; then stack=system
else stack=; fi
if [ -n "$stack" ]; then
    printf '%s\n' "# vdm-unlock: the Vergence lock screen's password check (tools/install.sh writes it)" \
        "auth		include		$stack" "account		include		$stack" > /etc/pam.d/vdm-unlock
    chmod 644 /etc/pam.d/vdm-unlock
    echo "vdm: /etc/pam.d/vdm-unlock uses the $stack stack"
else
    echo "vdm: no system PAM stack found (password-auth, system-auth or system); write /etc/pam.d/vdm-unlock by hand" >&2
fi
# And for logging in at the login screen (VDM step 4): the system's own
# login stack, so whatever it asks (a password, a security key, a code) the
# login screen asks too.
if [ -f /etc/pam.d/password-auth ]; then
    printf '%s\n' "# vdm: logging in at the Vergence login screen (tools/install.sh writes it)" \
        "auth		substack	password-auth" "auth		include		postlogin" \
        "account		required	pam_nologin.so" "account		include		password-auth" \
        "password	include		password-auth" \
        "session		required	pam_loginuid.so" "session		include		password-auth" "session		include		postlogin" > /etc/pam.d/vdm
elif [ -f /etc/pam.d/system-local-login ]; then
    printf '%s\n' "# vdm: logging in at the Vergence login screen (tools/install.sh writes it)" \
        "auth		include		system-local-login" "account		include		system-local-login" \
        "password	include		system-local-login" "session		include		system-local-login" > /etc/pam.d/vdm
elif [ "$(uname -s)" = FreeBSD ] && [ -f /etc/pam.d/login ]; then
    printf '%s\n' "# vdm: logging in at the Vergence login screen (tools/install.sh writes it)" \
        "auth		include		login" "account		include		login" \
        "password	include		login" "session		include		login" > /etc/pam.d/vdm
fi
[ -f /etc/pam.d/vdm ] && chmod 644 /etc/pam.d/vdm && echo "vdm: /etc/pam.d/vdm written for the login screen"
# The service, by init system; started now and at every boot.
if [ -d /run/systemd/system ]; then
    printf '%s\n' "[Unit]" "Description=VDM, the Vergence display manager (unlock checks)" "" "[Service]" \
        "ExecStart=$prefix/sbin/vdm" "Restart=always" "" "[Install]" "WantedBy=multi-user.target" > /etc/systemd/system/vdm.service
    systemctl daemon-reload
    systemctl enable vdm >/dev/null 2>&1 || true
    systemctl restart vdm
    # Before any sleep, every session locks (VDM step 2): systemd runs
    # system-sleep hooks with "pre" before the machine sleeps.
    printf '%s\n' "#!/bin/sh" "# vdm: lock every Vergence session before sleeping (tools/install.sh)" \
        'case "$1" in pre) exec '"$prefix"'/sbin/vdm before-sleep ;; esac' > /usr/lib/systemd/system-sleep/vdm
    chmod 755 /usr/lib/systemd/system-sleep/vdm
    echo "vdm: enabled and started (systemd); sessions lock before sleep (system-sleep hook)"
elif [ "$(uname -s)" = FreeBSD ]; then
    install -m 555 "$here/freebsd/vdm.rc" /usr/local/etc/rc.d/vdm
    sed -i '' "s|@PREFIX@|$prefix|g" /usr/local/etc/rc.d/vdm
    sysrc vdm_enable=YES >/dev/null
    service vdm restart >/dev/null 2>&1 || service vdm start >/dev/null 2>&1 || true
    echo "vdm: enabled in rc.conf and started"
elif [ -d /etc/sv ] && [ -d /var/service ]; then
    mkdir -p /etc/sv/vdm/log
    printf '%s\n' "#!/bin/sh" "exec 2>&1" "exec $prefix/sbin/vdm" > /etc/sv/vdm/run
    printf '%s\n' "#!/bin/sh" "exec vlogger -t vdm -p auth" > /etc/sv/vdm/log/run
    chmod 755 /etc/sv/vdm/run /etc/sv/vdm/log/run
    if [ -e /var/service/vdm ]; then sv restart vdm >/dev/null 2>&1 || true; else ln -s /etc/sv/vdm /var/service/vdm; fi
    # Before any sleep, every session locks (VDM step 2): zzz runs these first.
    if [ -d /etc/zzz.d/suspend ]; then
        printf '%s\n' "#!/bin/sh" "# vdm: lock every Vergence session before sleeping (tools/install.sh)" "$prefix/sbin/vdm before-sleep" > /etc/zzz.d/suspend/50-vdm
        chmod 755 /etc/zzz.d/suspend/50-vdm
        echo "vdm: sessions lock before sleep (/etc/zzz.d/suspend/50-vdm)"
    fi
    echo "vdm: enabled and started (runit, /var/service/vdm)"
else
    echo "vdm: no known init system; start $prefix/sbin/vdm as root at boot" >&2
fi
# ---- system setup: _vdm, the greeter's account (VDM milestone 1, review I10) ----
# The login screen runs as _vdm: a system account that owns nothing, logs in
# nowhere (nologin) and has a home it cannot write (/var/empty); its state
# lives on a tmpfs vdm makes fresh at each start (step 4). It may use the
# seat (seatd's group where there is no logind), never the setuid helpers.
if ! id _vdm >/dev/null 2>&1; then
    if [ "$(uname -s)" = FreeBSD ]; then
        u=900
        while id "$u" >/dev/null 2>&1 || pw groupshow -g "$u" >/dev/null 2>&1; do u=$((u + 1)); done
        pw useradd _vdm -u "$u" -c "Vergence display manager (greeter)" -d /var/empty -s /usr/sbin/nologin
    else
        useradd -r -M -d /var/empty -s "$(command -v nologin || echo /sbin/nologin)" -c "Vergence display manager (greeter)" _vdm
    fi
    echo "vdm: the greeter's account _vdm made (uid $(id -u _vdm))"
fi
if [ -n "${group:-}" ] && ! id -Gn _vdm | tr ' ' '\n' | grep -qx "$group"; then
    if [ "$(uname -s)" = FreeBSD ]; then pw groupmod "$group" -m _vdm; else usermod -aG "$group" _vdm; fi
    echo "vdm: _vdm may use the seat ($group)"
fi
# ---- system setup: removable drives (files milestone 1; Dave, 2026-11-15) ----
# Linux: udisks2 mounts a drive plugged in when the session asks (the
# server does); nothing to set up but the daemon itself. FreeBSD: the base
# system's automounter, /media pointed at our map (FAT drives writable by
# the video group); the original auto_master is kept beside it.
if [ "$(uname -s)" = FreeBSD ]; then
    line="/media		-vergence_media	-nosuid,noatime,autoro"
    if grep -q "^/media[[:space:]]*-vergence_media" /etc/auto_master; then
        echo "drives: /media is already Vergence's map"
    else
        [ -e /etc/auto_master.before-vergence ] || cp /etc/auto_master /etc/auto_master.before-vergence
        grep -v '^#\{0,1\}/media[[:space:]]' /etc/auto_master > /etc/auto_master.new
        printf '%s\n' "$line" >> /etc/auto_master.new
        mv /etc/auto_master.new /etc/auto_master
        echo "drives: /media in /etc/auto_master is Vergence's map (the original is /etc/auto_master.before-vergence)"
    fi
    sysrc autofs_enable=YES >/dev/null
    for s in automountd autounmountd; do service "$s" status >/dev/null 2>&1 || service "$s" start >/dev/null 2>&1 || true; done
    # An autofs mount keeps the map it was made with: /media made from
    # another map is unmounted first, so it is mounted again with ours.
    if mount -p | grep "[[:space:]]/media[[:space:]]*autofs" | grep -vq "^map -vergence_media"; then umount /media 2>/dev/null || true; fi
    automount >/dev/null 2>&1 || true
    automount -c >/dev/null 2>&1 || true
    echo "drives: the automounter is enabled (rc.conf autofs_enable) and running"
elif command -v udisksctl >/dev/null || [ -x /usr/libexec/udisks2/udisksd ] || [ -x /usr/lib/udisks2/udisksd ]; then
    echo "drives: udisks2 is installed; the session mounts drives through it"
    # Without logind polkit cannot see who sits at the machine and udisks2
    # refuses every mount: a rule lets the seat group mount, eject and power
    # off removable drives (docs/packaging.md). polkitd runs as a service,
    # and the system bus reloads its configuration so polkitd can claim its
    # name (it starts and exits every second until then).
    if [ ! -d /run/systemd/system ] && [ -n "${group:-}" ] && [ -d /etc/polkit-1/rules.d ]; then
        sed "s/\"_seatd\"/\"$group\"/" "$here/linux/50-vergence-drives.rules" > /etc/polkit-1/rules.d/50-vergence-drives.rules
        chmod 644 /etc/polkit-1/rules.d/50-vergence-drives.rules
        echo "drives: polkit lets the $group group mount removable drives (/etc/polkit-1/rules.d/50-vergence-drives.rules)"
        dbus-send --system --type=method_call --dest=org.freedesktop.DBus / org.freedesktop.DBus.ReloadConfig 2>/dev/null || true
        if [ -d /etc/sv/polkitd ] && [ ! -e /var/service/polkitd ]; then
            ln -s /etc/sv/polkitd /var/service/polkitd
            echo "drives: polkitd enabled (runit, /var/service/polkitd)"
        fi
    fi
else
    echo "drives: udisks2 is not installed, so drives plugged in are not mounted (Void: xbps-install udisks2)" >&2
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
