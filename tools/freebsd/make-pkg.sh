#!/bin/sh
# Make a FreeBSD package of Vergence from a release build: no root needed.
#
#   cargo build --release
#   tools/freebsd/make-pkg.sh [--version V] [--out DIR]
#
# Then, as root:  pkg install ./dist/vergence-<version>.pkg
#
# One package for the server and every app (they speak one exact bus
# protocol version, CLAUDE.md). The files are staged by tools/install.sh
# --destdir; this adds what install.sh writes straight into the system (the
# rc.d script, PAM files in /usr/local/etc/pam.d, the fontconfig entry) and
# the setuid helpers' owner and mode. What stays the administrator's choice
# (services, who may use the seat, the login screen at boot, /media) is
# said in the message pkg shows after installing (docs/packaging.md).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
prefix=/usr/local
out=$root/dist
cargo_version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$root/Cargo.toml")
# A later build sorts after an earlier one (pkg compares the parts as numbers).
version=$cargo_version.$(date -u +%Y%m%d%H%M)
while [ $# -gt 0 ]; do
    case $1 in
        --version) version=$2; shift 2 ;;
        --out) out=$2; shift 2 ;;
        -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
        *) echo "make-pkg.sh: unknown argument $1" >&2; exit 2 ;;
    esac
done
[ "$(uname -s)" = FreeBSD ] || { echo "make-pkg.sh: FreeBSD only" >&2; exit 1; }

work=$(mktemp -d -t vergence-pkg)
trap 'rm -rf "$work"' EXIT
stage=$work/stage
meta=$work/meta
mkdir -p "$stage" "$meta" "$out"

# ---- the files ----
"$root/tools/install.sh" --destdir "$stage" --prefix "$prefix" > "$work/install.log"
grep -v '^note: ' "$work/install.log" || true
P=$stage$prefix
install -d "$P/etc/rc.d" "$P/etc/pam.d" "$P/etc/fonts/conf.d"
sed "s|@PREFIX@|$prefix|g" "$here/vdm.rc" > "$P/etc/rc.d/vdm"
chmod 555 "$P/etc/rc.d/vdm"
# PAM, as install.sh writes it on FreeBSD (the login stack for logging in,
# the system stack for unlocking and elevation), in the package's own
# directory; vdm looks in both (vergence-dm paths::PAM_DIRS).
pam() { f=$P/etc/pam.d/$1; shift; printf '%s\n' "$@" > "$f"; chmod 644 "$f"; }
pam vdm-unlock "# vdm-unlock: the Vergence lock screen's password check (package)" \
    "auth		include		system" "account		include		system"
pam vdm-elevate "# vdm-elevate: an administrator approving one named action (package)" \
    "auth		include		system" "account		include		system"
pam vdm "# vdm: logging in at the Vergence login screen (package)" \
    "auth		include		login" "account		include		login" \
    "password	include		login" "session		include		login"
pam vdm-greeter "# vdm-greeter: the Vergence login screen's own session (package)" \
    "auth		required	pam_permit.so" "account		required	pam_permit.so" \
    "password	required	pam_deny.so" "session		required	pam_permit.so"
pam vdm-autologin "# vdm-autologin: Vergence's automatic login, off unless 'vdm autologin on' (package)" \
    "auth		required	pam_permit.so" "account		required	pam_nologin.so" "account		include		system" \
    "password	required	pam_deny.so" "session		include		system"
printf '%s\n' '<?xml version="1.0"?>' '<!DOCTYPE fontconfig SYSTEM "fonts.dtd">' \
    "<!-- Vergence's own fonts, Inter and JetBrains Mono (package) -->" \
    "<fontconfig><dir>$prefix/share/vergence/fonts</dir></fontconfig>" > "$P/etc/fonts/conf.d/90-vergence-fonts.conf"
chmod 644 "$P/etc/fonts/conf.d/90-vergence-fonts.conf"

# ---- the packing list: every staged file; the helpers setuid root ----
plist=$meta/plist
(cd "$stage" && find . -type f -o -type l | sed 's|^\./|/|' | sort) | while read -r f; do
    case $f in
        "$prefix"/libexec/vergence/sound-default|"$prefix"/libexec/vergence/media-eject)
            echo "@(root,vergence,4750) ${f#"$prefix"/}" ;;
        "$prefix"/*) echo "${f#"$prefix"/}" ;;
        *) echo "$f" ;;
    esac
done > "$plist"
for d in libexec/vergence share/vergence/words share/vergence/fonts share/vergence share/doc/vergence; do
    echo "@dir $d" >> "$plist"
done

# ---- dependencies: the packages that own the libraries our programs load,
# and what runs beside them (the seat, the bus, Mesa's drivers, keymaps) ----
deps=$work/deps
: > "$deps"
# Only the libraries a program names itself (NEEDED): pkg brings in what
# those need in turn.
for f in "$P"/bin/* "$P"/sbin/* "$P"/libexec/vergence/*; do
    file -b "$f" | grep -q '^ELF' || continue
    for so in $(readelf -d "$f" 2>/dev/null | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'); do
        ldd -f '%o %p\n' "$f" 2>/dev/null | awk -v so="$so" '$1 == so { print $2 }'
    done
done | sort -u | while read -r lib; do
    case $lib in /usr/local/*) pkg which -q "$lib" 2>/dev/null || true ;; esac
done | sort -u > "$work/owners"
for name in seatd dbus mesa-dri xkeyboard-config; do
    pkg info -e "$name" && pkg query '%n-%v' "$name"
done >> "$work/owners"
sort -u "$work/owners" | while read -r nv; do
    [ -n "$nv" ] || continue
    pkg query '%n "%o" "%v"' "$nv"
done | sort -u > "$deps"
{
    echo "deps {"
    while read -r n o v; do
        echo "  $n { origin = $o; version = $v; }"
    done < "$deps"
    echo "}"
} > "$meta/deps.ucl"

# ---- install scripts ----
# Before the files: the helpers' group (pkg sets their owner and mode from
# the list, so it must be there), and _vdm, the login screen's account.
cat > "$meta/pre-install" <<'SH'
pw groupshow vergence >/dev/null 2>&1 || pw groupadd vergence
if ! id _vdm >/dev/null 2>&1; then
    u=900
    while id "$u" >/dev/null 2>&1 || pw groupshow -g "$u" >/dev/null 2>&1; do u=$((u + 1)); done
    pw useradd _vdm -u "$u" -c "Vergence display manager (greeter)" -d /var/empty -s /usr/sbin/nologin
fi
id -Gn _vdm | tr ' ' '\n' | grep -qx video || pw groupmod video -m _vdm
SH
cat > "$meta/post-install" <<SH
mkdir -p /var/db/vdm && chmod 700 /var/db/vdm
command -v fc-cache >/dev/null 2>&1 && fc-cache -f $prefix/share/vergence/fonts >/dev/null 2>&1 || true
SH

cat > "$meta/message" <<MSG
Vergence is installed. As root, once:

  The seat and the system bus (if not on already):
    sysrc seatd_enable=YES dbus_enable=YES && service seatd start && service dbus start

  Each person who uses Vergence: the seat's group, and the helpers'
  (sound output, ejecting drives), from their next login:
    pw groupmod video -m USER && pw groupmod vergence -m USER

  vdm (the lock screen; the login screen too, if on):
    sysrc vdm_enable=YES && service vdm start
    vdm login-screen on        the login screen at boot (off: log in on a
                               console and type 'vergence')

  Removable drives under /media: the automounter with Vergence's map
    sysrc autofs_enable=YES, and in /etc/auto_master replace the /media line with
    /media		-vergence_media	-nosuid,noatime,autoro
    then: service automountd start && service autounmountd start

A GPU needs its kernel driver (Intel/AMD: drm-kmod, kld_list="i915kms" or
"amdgpu"). After an upgrade, vdm keeps running the old version until it is
restarted ('service vdm restart' ends every session it started).
MSG

# ---- the manifest ----
abi=$(pkg config abi)
desc=$(printf '%s' "Vergence: a Wayland desktop written in Rust, with its own display server, login and lock screen (vdm), dock, terminal, settings, file viewer and text editor." )
{
    echo "name = vergence;"
    echo "version = \"$version\";"
    echo "origin = \"x11-wm/vergence\";"
    echo "comment = \"Wayland desktop with its own server, login screen and apps\";"
    echo "desc = \"$desc\";"
    echo "maintainer = \"vergence@localhost\";"
    echo "www = \"https://github.com/gsa700/vergence\";"
    echo "prefix = \"$prefix\";"
    echo "abi = \"$abi\";"
    echo "licenselogic = \"dual\";"
    echo "licenses = [ \"APACHE20\", \"GPLv2+\" ];"
    cat "$meta/deps.ucl"
    echo "scripts {"
    for s in pre-install post-install; do
        printf '  %s = <<EOD\n%s\nEOD\n' "$s" "$(cat "$meta/$s")"
    done
    echo "}"
    printf 'messages [ { message = <<EOD\n%s\nEOD\n } ]\n' "$(cat "$meta/message")"
} > "$meta/+MANIFEST"

pkg create -M "$meta/+MANIFEST" -p "$plist" -r "$stage" -o "$out"
pkgfile=$out/vergence-$version.pkg
[ -f "$pkgfile" ] || pkgfile=$(ls -t "$out"/vergence-"$version".* | head -1)
echo
echo "made $pkgfile ($(du -h "$pkgfile" | cut -f1), $(wc -l < "$plist" | tr -d ' ') entries, $(wc -l < "$deps" | tr -d ' ') dependencies)"
echo "install as root: pkg install $pkgfile"
