#!/bin/sh
# Install Vergence on this machine from a release build, or stage it for a package.
#
#   sudo tools/install.sh                  under /usr/local
#   sudo tools/install.sh --prefix /usr    another prefix
#   tools/install.sh --destdir DIR         stage under DIR (packaging; no root, no system setup)
#   sudo tools/install.sh --uninstall      remove what an earlier run put there
#   sudo tools/install.sh --vdm            also: the login screen at boot (VDM)
#   sudo tools/install.sh --no-vdm         also: no login screen at boot (vdm still
#                                          runs for the lock); neither: as it was
#
# Build first, on this machine or one with the same OS and architecture:
#   cargo build --release
#
# What goes where (PREFIX=/usr/local):
#   bin/vergence-server, vergence-term, vergence-settings, vergence-calc, vergence-quit, vergence-hw, vergence-edit, vergence-closeup, vergence-reader, vergence-files, vergence-screenshot, vergence-open, vbus, vtile, vagent, vergence-agent-mcp, vnative, vsound, vshell, vergence-remote (SSH runs it for Vergence Remote), vergence-remote-probe, vergence-remote-viewer, vergence-albumwall, vctl   the programs
#   bin/vergence-session                     the session launcher (tools/vergence-session)
#   bin/vergence -> vergence-session         start the session from a text console
#   sbin/vergence-setup                      the system setup, run once as root (install.sh runs it with --yes)
#   share/wayland-sessions/vergence.desktop  "Vergence" at a display manager's login screen
#   share/applications/vergence-*.desktop    the terminal, the settings app, the calculator, Force Quit, Hardware, the text editor, FileView and its Trash window (NoDisplay: the dock's icon only), AlbumWall, for the shelf and any launcher
#   share/icons/hicolor/scalable/apps/       their icons (data/icons, drawn by the project)
#   share/doc/vergence/                      LICENSE-APACHE, LICENSE-GPL, THIRD-PARTY-NOTICES.md
#   share/vergence/words/                    the spelling list (data/words/en_US.txt, from SCOWL) and its terms
#   share/vergence/fonts/                    Inter, JetBrains Mono and Liberation (data/fonts), with their licenses
#   share/vergence/syntax/                   languages described in files, for Edit's colors (data/syntax)
#   share/vergence/defaults/desktop.toml     every account's defaults: wallpaper, the dock's apps, energy (data/defaults)
#   share/backgrounds/vergence/              Vergence's wallpapers (data/backgrounds, drawn by the project)
#   share/vergence/icons/{places,kinds}/     Vergence's own icons for places and files (data/icons)
#   share/icons/Vergence/                    the Vergence icon theme: our apps, places, kinds of files, and the pointers (drawn at install)
#   libexec/vergence/ups-tile, vtile-demo  lab tiles on the dock from scripts (tools/lab; not with --no-lab, as packages)
#   libexec/vergence/vergence-choose         the Open/Save dialog the portal shows for apps not ours
#   libexec/vergence/vergence-decode         pictures decoded apart, limited (Capsicum on FreeBSD)
#   libexec/vergence/vergence-hymie          sound files decoded apart (Hymie), confined the same way
#   libexec/vergence/vergence-hymie-aac      AAC sound files decoded apart (GPL, FAAD2's), confined the same way
#   libexec/vergence/vergence-pdf            PDF documents read and drawn apart, confined the same way
#   libexec/vergence/vergence-unpack         archives read apart for FileView, confined the same way
#   libexec/vergence/vergence-tls            HTTPS through the system's OpenSSL, confined the same way
#   libexec/vergence/media-eject             FreeBSD only: the root helper that unmounts a drive
#                                            under /media (setuid root, group vergence, mode 4750)
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
login_screen=
no_lab=
while [ $# -gt 0 ]; do
    case $1 in
        --no-lab) no_lab=1; shift ;;
        --prefix) prefix=$2; shift 2 ;;
        --destdir) destdir=$2; shift 2 ;;
        --user) user=$2; shift 2 ;;
        --uninstall) uninstall=1; shift ;;
        --vdm) login_screen=on; shift ;;
        --no-vdm) login_screen=off; shift ;;
        -h|--help) sed -n '2,24p' "$0"; exit 0 ;;
        *) echo "install.sh: unknown argument $1" >&2; exit 2 ;;
    esac
done
# The person checked before anything uses the name as root (packaging
# review, L1).
[ -z "$user" ] || id -- "$user" >/dev/null 2>&1 || { echo "install.sh: no account $user" >&2; exit 2; }
bin=$destdir$prefix/bin
sessions=$destdir$prefix/share/wayland-sessions
applications=$destdir$prefix/share/applications
icons=$destdir$prefix/share/icons/hicolor/scalable/apps
docs=$destdir$prefix/share/doc/vergence       # license and notices (the settings app's About page names it)
docfiles="LICENSE-APACHE LICENSE-GPL THIRD-PARTY-NOTICES.md"
programs="vergence-server vergence-term vergence-settings vergence-calc vergence-quit vergence-hw vergence-edit vergence-closeup vergence-reader vergence-files vergence-screenshot vergence-open vbus vtile vagent vergence-agent-mcp vnative vsound vshell vdm-try vergence-remote vergence-remote-probe vergence-remote-viewer vergence-albumwall"
tools="vergence-term vergence-settings vergence-calc vergence-quit vergence-hw vergence-edit vergence-closeup vergence-reader vergence-files vergence-trash vergence-screenshot vergence-albumwall"
libexec=$destdir$prefix/libexec/vergence
sbin=$destdir$prefix/sbin
words=$destdir$prefix/share/vergence/words
fonts=$destdir$prefix/share/vergence/fonts
syntax=$destdir$prefix/share/vergence/syntax
defaults=$destdir$prefix/share/vergence/defaults
backgrounds=$destdir$prefix/share/backgrounds/vergence
# Vergence's own icons for places and kinds of files (tools/make-icons.py).
ownicons=$destdir$prefix/share/vergence/icons
# Vergence's pointers, an Xcursor theme (vergence-harness cursors).
cursortheme=$destdir$prefix/share/icons/Vergence

if [ -n "$uninstall" ]; then
    for p in $programs vctl vergence-session vergence; do rm -f "$bin/$p"; done
    rm -f "$sbin/vdm" "$sbin/vergence-setup"
    # vdm's PAM files and the login screen at boot (the service itself is the
    # init system's to stop: vdm will not start without its binary).
    # Only the files install.sh itself wrote (they say "tools/install.sh"):
    # a package's own copies of them are never removed (packaging review,
    # 2026-11-22, H1: removing the package's PAM files locked people out).
    ours() { [ -f "$1" ] && [ ! -L "$1" ] && grep -q 'tools/install.sh' "$1"; }
    if [ -z "$destdir" ]; then
        for f in /etc/pam.d/vdm /etc/pam.d/vdm-unlock /etc/pam.d/vdm-greeter /etc/pam.d/vdm-autologin /etc/pam.d/vdm-elevate; do
            ours "$f" && rm -f "$f"
        done
        if ! command -v xbps-query >/dev/null 2>&1 || ! xbps-query vergence >/dev/null 2>&1; then
            rm -f /var/lib/vdm/login-screen /var/db/vdm/login-screen /var/lib/vdm/autologin /var/db/vdm/autologin
        fi
    fi
    rm -f "$sessions/vergence.desktop"
    for t in $tools; do rm -f "$applications/$t.desktop" "$icons/$t.svg"; done
    for d in $docfiles; do rm -f "$docs/$d"; done
    rm -f "$libexec/media-eject"
    [ -z "$destdir" ] && [ "$(uname -s)" = FreeBSD ] && rm -f /etc/autofs/special_vergence_media
    # acpid's handler as it was (the power key's guard taken out).
    [ -z "$destdir" ] && [ -f /etc/acpi/handler.sh.vergence-orig ] && mv -f /etc/acpi/handler.sh.vergence-orig /etc/acpi/handler.sh
    rm -f "$libexec/ups-tile" "$libexec/vtile-demo"
    rm -f "$libexec/sound-default" "$libexec/vergence-choose" "$libexec/vergence-decode" "$libexec/vergence-hymie" "$libexec/vergence-hymie-aac" "$libexec/vergence-pdf" "$libexec/vergence-unpack" "$libexec/vergence-tls" "$words/en_US.txt" "$words/SCOWL-COPYRIGHT.txt"
    # The service files and hooks it wrote, never a package's (the service's
    # link in /var/service, the person's choice, is left).
    if [ -z "$destdir" ]; then
        for f in /etc/fonts/conf.d/90-vergence-fonts.conf /usr/local/etc/fonts/conf.d/90-vergence-fonts.conf \
            /etc/zzz.d/suspend/50-vdm /etc/polkit-1/rules.d/50-vergence-drives.rules; do
            ours "$f" && rm -f "$f"
        done
        [ -f /etc/sv/vdm/run ] && grep -q "$prefix/sbin/vdm" /etc/sv/vdm/run && rm -rf /etc/sv/vdm/run /etc/sv/vdm/log
    fi
    rm -f "$fonts"/InterVariable*.ttf "$fonts"/JetBrainsMono-Variable*.ttf "$fonts"/Liberation*.ttf "$fonts"/Inter-OFL.txt "$fonts"/JetBrainsMono-OFL.txt "$fonts"/Liberation-OFL.txt
    rm -rf "$ownicons" "$cursortheme" "$backgrounds" "$syntax"
    rm -f "$defaults/desktop.toml"; rmdir "$defaults" 2>/dev/null || true
    rmdir "$words" "$fonts" "$destdir$prefix/share/vergence" 2>/dev/null || true
    rmdir "$libexec" 2>/dev/null || true
    rmdir "$docs" 2>/dev/null || true
    echo "removed Vergence from $destdir$prefix (user settings and logs kept)"
    exit 0
fi

# The build to install: cargo's target/release, or where a package's build put
# it (the xbps template: target/<triple>/release).
release=${VERGENCE_RELEASE_DIR:-$root/target/release}
case $release in /*) ;; *) release=$root/$release ;; esac
for p in $programs; do
    [ -x "$release/$p" ] || { echo "install.sh: $release/$p is missing; run cargo build --release first" >&2; exit 1; }
done
# A machine has the package or a checkout's install, not both: installing over
# the package would rewrite its PAM files and service to point at /usr/local.
if [ -z "$destdir" ] && command -v xbps-query >/dev/null 2>&1 && xbps-query vergence >/dev/null 2>&1; then
    echo "install.sh: the vergence package is installed here; remove it first (sudo xbps-remove vergence) or use it" >&2
    exit 1
fi
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
# Restarting vdm ends every session it started (they are its children), so
# it is restarted only when its binary changed and no one is logged in
# (otherwise an install for an app change would log the user out).
vdm_changed=no
cmp -s "$release/vdm" "$sbin/vdm" 2>/dev/null || vdm_changed=yes
install -m 755 "$release/vdm" "$sbin/vdm"
# The spelling list every native app checks with (vergence-spell finds it beside bin/).
install -d -m 755 "$words"
install -m 644 "$root/data/words/en_US.txt" "$words/en_US.txt"
install -m 644 "$root/data/words/SCOWL-COPYRIGHT.txt" "$words/SCOWL-COPYRIGHT.txt"
# Vergence's own fonts, for the same look everywhere: Inter and JetBrains
# Mono with their licenses (SIL OFL 1.1); a family the system has itself is
# the system's copy, these the fallback (vergence-text db.rs). Liberation
# Sans, Serif and Mono: the PDF reader's metric-compatible
# stand-ins for Helvetica, Times and Courier when a PDF does not embed them.
install -d -m 755 "$fonts"
# The pointers: drawn by vergence-harness from their own SVG (the theme the
# server picks when none is chosen; it inherits Adwaita for rare shapes).
if [ -x "$release/vergence-harness" ]; then
    rm -rf "$cursortheme"
    "$release/vergence-harness" cursors "$cursortheme" >/dev/null || echo "install.sh: the Vergence pointers could not be drawn (Adwaita's are used)" >&2
    chmod -R a+rX "$cursortheme" 2>/dev/null
else
    echo "install.sh: no vergence-harness in $release: the Vergence pointers are not installed" >&2
fi
# Places and kinds: the SVGs, and their pixels at five sizes drawn by
# vergence-harness (the toolkit shows those, with no SVG code of its own).
for g in places kinds; do
    install -d -m 755 "$ownicons/$g"
    for f in "$root/data/icons/$g"/*.svg; do install -m 644 "$f" "$ownicons/$g/"; done
done
if [ -x "$release/vergence-harness" ]; then
    "$release/vergence-harness" icons "$root/data/icons" "$ownicons" >/dev/null || echo "install.sh: the icons' pixels could not be drawn (drawn marks are shown)" >&2
    chmod -R a+rX "$ownicons" 2>/dev/null
    # The Vergence icon theme (with the pointers above): Settings > Theme and
    # fonts > Icons, which our apps follow, Vergence's when none is chosen.
    "$release/vergence-harness" icon-theme "$root/data/icons" "$cursortheme" >/dev/null || echo "install.sh: the Vergence icon theme could not be laid out" >&2
    chmod -R a+rX "$cursortheme" 2>/dev/null
fi
for f in "$root"/data/fonts/*; do install -m 644 "$f" "$fonts/"; done
# Languages described in files (vergence-syntax rules.rs, docs/syntax-files.md).
mkdir -p "$syntax"
for f in "$root"/data/syntax/*.toml; do install -m 644 "$f" "$syntax/"; done
# Every account's defaults (read under /etc's and each person's own) and the
# wallpapers they name; the prefix written in, so the default picture is found.
mkdir -p "$defaults" "$backgrounds"
sed "s#@PREFIX@#$prefix#g" "$root/data/defaults/desktop.toml" > "$defaults/desktop.toml" && chmod 644 "$defaults/desktop.toml"
for f in "$root"/data/backgrounds/*.svg; do install -m 644 "$f" "$backgrounds/"; done
# Every app on the machine may use them (GTK, Qt, Firefox find fonts through
# fontconfig): its conf.d gets our directory, after the system's own.
if [ -z "$destdir" ]; then
    for conf in /etc/fonts/conf.d /usr/local/etc/fonts/conf.d; do
        [ -d "$conf" ] || continue
        printf '%s\n' '<?xml version="1.0"?>' '<!DOCTYPE fontconfig SYSTEM "fonts.dtd">' \
            "<!-- Vergence's own fonts, Inter, JetBrains Mono and Liberation (tools/install.sh writes it) -->" \
            "<fontconfig><dir>$prefix/share/vergence/fonts</dir></fontconfig>" > "$conf/90-vergence-fonts.conf"
        chmod 644 "$conf/90-vergence-fonts.conf"
        command -v fc-cache >/dev/null 2>&1 && fc-cache -f "$prefix/share/vergence/fonts" >/dev/null 2>&1
        echo "fonts: Inter, JetBrains Mono and Liberation registered with fontconfig ($conf/90-vergence-fonts.conf)"
    done
fi
install -m 755 "$here/vctl" "$bin/vctl"
# vctl talks to the control socket through nc (Fedora installs none by default).
command -v nc >/dev/null 2>&1 || echo "vctl: needs nc (Fedora: dnf install nmap-ncat; Void: xbps-install openbsd-netcat)" >&2
# Lab tiles (dock milestone 1): a NUT UPS and a demonstration tile, scripts on vtile.
install -d -m 755 "$libexec"
if [ -z "$no_lab" ]; then
    install -m 755 "$here/lab/ups-tile" "$libexec/ups-tile"
    install -m 755 "$here/lab/vtile-demo" "$libexec/vtile-demo"
fi
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
[ -x "$release/vergence-hymie" ] || { echo "install.sh: $release/vergence-hymie is missing; run cargo build --release first" >&2; exit 1; }
install -m 755 "$release/vergence-hymie" "$libexec/vergence-hymie"
# AAC in its own helper: FAAD2's code is GPL-2.0-or-later only, so it is kept out
# of everything dual-licensed (crates/vergence-hymie-aac).
[ -x "$release/vergence-hymie-aac" ] || { echo "install.sh: $release/vergence-hymie-aac is missing; run cargo build --release first" >&2; exit 1; }
install -m 755 "$release/vergence-hymie-aac" "$libexec/vergence-hymie-aac"
# PDF documents read and drawn apart, confined (PDF milestone 1, step 6).
[ -x "$release/vergence-pdf" ] || { echo "install.sh: $release/vergence-pdf is missing; run cargo build --release first" >&2; exit 1; }
install -m 755 "$release/vergence-pdf" "$libexec/vergence-pdf"
[ -x "$release/vergence-unpack" ] || { echo "install.sh: $release/vergence-unpack is missing; run cargo build --release first" >&2; exit 1; }
install -m 755 "$release/vergence-unpack" "$libexec/vergence-unpack"
[ -x "$release/vergence-tls" ] || { echo "install.sh: $release/vergence-tls is missing; run cargo build --release first" >&2; exit 1; }
install -m 755 "$release/vergence-tls" "$libexec/vergence-tls"
install -m 755 "$here/vergence-session" "$bin/vergence-session"
# The system setup, run once as root after installing (packaging milestone 1):
# a package enables nothing, this does, asking first; install.sh runs it with
# --yes below.
install -d -m 755 "$sbin"
install -m 755 "$here/vergence-setup" "$sbin/vergence-setup"
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

# ---- system setup: the files vergence-setup's steps need ----
# The seat's group where there is no logind (polkit's rule names it).
group=
if [ -d /run/systemd/system ]; then group=
elif [ "$(uname -s)" = FreeBSD ]; then group=video
elif [ -d /etc/sv/seatd ]; then group=_seatd
fi
# ---- system setup: _vdm, the greeter's account (VDM milestone 1, review I10) ----
# (Before vdm starts: vdm looks for it once, when it starts.)
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
    # Elevation prompts (elevation milestone 1): the person's own password,
    # by the same stack; the administrators' group is vdm's check.
    printf '%s\n' "# vdm-elevate: an administrator approving one named action (tools/install.sh writes it)" \
        "auth		include		$stack" "account		include		$stack" > /etc/pam.d/vdm-elevate
    chmod 644 /etc/pam.d/vdm-elevate
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
# The login screen's own session (no check, nothing to answer): on Fedora
# logind gives the seat only to a process in a session (class greeter, as
# GDM's), so the login screen gets one; elsewhere a session that does nothing.
if [ -d /run/systemd/system ]; then
    printf '%s\n' "# vdm-greeter: the Vergence login screen's own session (tools/install.sh writes it)" \
        "auth		required	pam_permit.so" "account		required	pam_permit.so" "password	required	pam_deny.so" \
        "session		required	pam_loginuid.so" "session		optional	pam_keyinit.so force revoke" \
        "session		required	pam_systemd.so" "session		required	pam_limits.so" > /etc/pam.d/vdm-greeter
elif [ "$(uname -s)" = FreeBSD ]; then
    printf '%s\n' "# vdm-greeter: the Vergence login screen's own session (tools/install.sh writes it)" \
        "auth		required	pam_permit.so" "account		required	pam_permit.so" "password	required	pam_deny.so" \
        "session		required	pam_permit.so" > /etc/pam.d/vdm-greeter
else
    printf '%s\n' "# vdm-greeter: the Vergence login screen's own session (tools/install.sh writes it)" \
        "auth		required	pam_permit.so" "account		required	pam_permit.so" "password	required	pam_deny.so" \
        "session		required	pam_limits.so" "-session	optional	pam_elogind.so" > /etc/pam.d/vdm-greeter
fi
chmod 644 /etc/pam.d/vdm-greeter
# Automatic login (VDM step 8, off unless `vdm autologin on`): no check, the
# account and session stacks of the system's login.
if [ -f /etc/pam.d/password-auth ]; then
    printf '%s\n' "# vdm-autologin: Vergence's automatic login (tools/install.sh writes it)" \
        "auth		required	pam_permit.so" "account		required	pam_nologin.so" "account		include		password-auth" \
        "password	required	pam_deny.so" \
        "session		required	pam_loginuid.so" "session		include		password-auth" "session		include		postlogin" > /etc/pam.d/vdm-autologin
elif [ -f /etc/pam.d/system-local-login ]; then
    printf '%s\n' "# vdm-autologin: Vergence's automatic login (tools/install.sh writes it)" \
        "auth		required	pam_permit.so" "account		include		system-local-login" \
        "password	required	pam_deny.so" "session		include		system-local-login" > /etc/pam.d/vdm-autologin
elif [ "$(uname -s)" = FreeBSD ] && [ -f /etc/pam.d/login ]; then
    printf '%s\n' "# vdm-autologin: Vergence's automatic login (tools/install.sh writes it)" \
        "auth		required	pam_permit.so" "account		required	pam_nologin.so" "account		include		system" \
        "password	required	pam_deny.so" "session		include		system" > /etc/pam.d/vdm-autologin
fi
[ -f /etc/pam.d/vdm-autologin ] && chmod 644 /etc/pam.d/vdm-autologin
# The login screen at boot (--vdm), or not (--no-vdm); vdm reads the file
# when it starts. Neither: left as it was.
if [ "$(uname -s)" = FreeBSD ]; then vdm_state=/var/db/vdm; else vdm_state=/var/lib/vdm; fi
mkdir -p "$vdm_state" && chmod 700 "$vdm_state"
# (--vdm / --no-vdm are passed on to vergence-setup, which sets it.)
# The service, by init system; started now and at every boot. What to do
# with a running vdm: leave it (unchanged), restart it (changed, no Vergence
# session running), or leave it and say how (changed, sessions running).
if ps -ax -o command= | grep -E '(^|/)vergence-server( |$)' | grep -v -- '--greeter' | grep -q .; then
    sessions=yes
else
    sessions=no
fi
if [ "$vdm_changed" = no ]; then vdm_do=start
elif [ "$sessions" = yes ]; then vdm_do=defer
else vdm_do=restart
fi
vdm_deferred() {
    echo "vdm: updated, but a Vergence session is running, so the running vdm was left alone (restarting it ends every session); the new one starts at the next boot, or now, logging everyone out, with: sudo $1" >&2
}
if [ -d /run/systemd/system ]; then
    printf '%s\n' "[Unit]" "Description=VDM, the Vergence display manager" "After=systemd-user-sessions.service systemd-logind.service" "" "[Service]" \
        "ExecStart=$prefix/sbin/vdm" "Restart=always" "" "[Install]" "WantedBy=multi-user.target" > /etc/systemd/system/vdm.service
    systemctl daemon-reload
    # Enabled by vergence-setup (below); a changed vdm restarted here.
    if [ "$vdm_do" = restart ] && systemctl is-active vdm >/dev/null 2>&1; then systemctl restart vdm; fi
    if [ "$vdm_do" = defer ]; then vdm_deferred "systemctl restart vdm"; fi
    # Before any sleep, every session locks (VDM step 2): systemd runs
    # system-sleep hooks with "pre" before the machine sleeps.
    printf '%s\n' "#!/bin/sh" "# vdm: lock every Vergence session before sleeping (tools/install.sh)" \
        'case "$1" in pre) exec '"$prefix"'/sbin/vdm before-sleep ;; esac' > /usr/lib/systemd/system-sleep/vdm
    chmod 755 /usr/lib/systemd/system-sleep/vdm
    echo "vdm: service written (systemd); sessions lock before sleep (system-sleep hook)"
elif [ "$(uname -s)" = FreeBSD ]; then
    install -m 555 "$here/freebsd/vdm.rc" /usr/local/etc/rc.d/vdm
    sed -i '' "s|@PREFIX@|$prefix|g" /usr/local/etc/rc.d/vdm
    # Enabled by vergence-setup (below); a changed vdm restarted here.
    if [ "$vdm_do" = restart ] && service vdm status >/dev/null 2>&1; then service vdm restart >/dev/null 2>&1 || true; fi
    if [ "$vdm_do" = defer ]; then vdm_deferred "service vdm restart"; fi
    echo "vdm: rc.d script written"
elif [ -d /etc/sv ] && [ -d /var/service ]; then
    mkdir -p /etc/sv/vdm/log
    printf '%s\n' "#!/bin/sh" "exec 2>&1" "exec $prefix/sbin/vdm" > /etc/sv/vdm/run
    printf '%s\n' "#!/bin/sh" "exec vlogger -t vdm -p auth" > /etc/sv/vdm/log/run
    chmod 755 /etc/sv/vdm/run /etc/sv/vdm/log/run
    # runit's state in /run, as Void's packages lay it out (vsv): a package
    # installed later cannot unpack over a real supervise folder (techbench,
    # 2026-11-22). One runsv already made is left until it is restarted.
    [ -e /etc/sv/vdm/supervise ] || ln -s /run/runit/supervise.vdm /etc/sv/vdm/supervise
    [ -e /etc/sv/vdm/log/supervise ] || ln -s /run/runit/supervise.vdm-log /etc/sv/vdm/log/supervise
    # Enabled by vergence-setup (below); a changed vdm restarted here.
    if [ -e /var/service/vdm ] && [ "$vdm_do" = restart ]; then sv restart vdm >/dev/null 2>&1 || true; fi
    if [ "$vdm_do" = defer ]; then vdm_deferred "sv restart vdm"; fi
    # Before any sleep, every session locks (VDM step 2): zzz runs these first.
    if [ -d /etc/zzz.d/suspend ]; then
        printf '%s\n' "#!/bin/sh" "# vdm: lock every Vergence session before sleeping (tools/install.sh)" "$prefix/sbin/vdm before-sleep" > /etc/zzz.d/suspend/50-vdm
        chmod 755 /etc/zzz.d/suspend/50-vdm
        echo "vdm: sessions lock before sleep (/etc/zzz.d/suspend/50-vdm)"
    fi
    echo "vdm: service written (runit, /etc/sv/vdm)"
else
    echo "vdm: no known init system; start $prefix/sbin/vdm as root at boot" >&2
fi
# ---- system setup: removable drives (files milestone 1) ----
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
    # autofs keeps an answer that mounted nothing (a stick's folder looked
    # into while it was out) for vfs.autofs.cache seconds, 600 by default,
    # and `automount -c` clears only /media itself: the stick plugged in
    # meanwhile opened empty. Five seconds (see docs/drives-milestone-1.md).
    sysctl vfs.autofs.cache=5 >/dev/null 2>&1 || true
    if ! grep -q '^vfs\.autofs\.cache=' /etc/sysctl.conf 2>/dev/null; then
        printf '# Vergence (tools/install.sh): a drive plugged in is mounted at once\nvfs.autofs.cache=5\n' >> /etc/sysctl.conf
    fi
    echo "drives: the automounter is enabled (rc.conf autofs_enable) and running; its cache 5 s (sysctl.conf)"
elif command -v udisksctl >/dev/null || [ -x /usr/libexec/udisks2/udisksd ] || [ -x /usr/lib/udisks2/udisksd ]; then
    # (The system bus and polkitd are enabled by vergence-setup.)
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
fi
# The desktop's icons (vergence-files --desktop) run the program they
# started with until they are started again: stopped here, a server that
# keeps them running starts the new one at once, so the desktop and FileView
# never differ (see docs/drives-milestone-1.md). Only
# with such a server (its log says so); an older one would leave no desktop.
if [ -z "$destdir" ] && [ -n "$user" ] && command -v pgrep >/dev/null 2>&1; then
    home_of=$(getent passwd "$user" 2>/dev/null | cut -d: -f6)
    pids=$(pgrep -u "$user" -f '^(/usr/local/bin/)?vergence-files --desktop' 2>/dev/null || true)
    if [ -n "$pids" ]; then
        if grep -q 'started again if it exits' "$home_of/.local/state/vergence/server.log" 2>/dev/null; then
            kill $pids 2>/dev/null && echo "desktop: the icons started again with the new program"
        else
            echo "desktop: the icons keep the old program until the next session (this session's server does not start them again)"
        fi
    fi
fi
# The steps that turn things on (the seat, vdm, the login screen with --vdm,
# drives, Wi-Fi, the power key, a system log), one copy of them, in
# vergence-setup; install.sh asks nothing, as before.
set -- --yes
[ -n "$user" ] && set -- "$@" --user "$user"
[ -n "$login_screen" ] && set -- "$@" --login-screen "$login_screen"
"$sbin/vergence-setup" "$@"
