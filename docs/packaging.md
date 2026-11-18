# Packaging notes

What a package of Vergence needs from the system, collected as it comes up
(Dave, 2026-11-15: "lets make note of that for deps which will come up
whenever we want to package this for distribution"). `tools/install.sh` does
these things on a machine by hand today; a package (an xbps template, a
FreeBSD port, a Fedora spec) has to do the same through its own means:
dependencies, files it owns, and steps after installing.

## FreeBSD: the package (2026-11-17)

`tools/freebsd/make-pkg.sh` makes `dist/vergence-<version>.pkg` from a
release build as an ordinary user (`pkg create`; the version is the
workspace's plus a UTC date stamp, so a later build sorts after an earlier
one). Installed with `pkg install ./dist/vergence-….pkg` as root. It
carries:

- the files `install.sh --destdir` stages, plus what install.sh writes
  into the system: `etc/rc.d/vdm`, the PAM files in
  `/usr/local/etc/pam.d` (vdm looks there after `/etc/pam.d` on FreeBSD,
  vergence-dm `paths::PAM_DIRS`), `etc/fonts/conf.d/90-vergence-fonts.conf`,
  `/etc/autofs/special_vergence_media`;
- the setuid helpers as `root:vergence 4750` (in the packing list);
- dependencies: the packages owning the libraries our programs link
  themselves, plus seatd, dbus, mesa-dri and xkeyboard-config;
- a pre-install script (the `vergence` group, the `_vdm` account in
  `video`) and a post-install script (`/var/db/vdm`, the font cache).

What stays the administrator's, said in the message pkg shows: enabling
seatd, dbus, vdm and autofs (`sysrc`), each user into `video` and
`vergence`, `vdm login-screen on` for the login screen at boot, and the
`/media` line in `/etc/auto_master`. Techbench (Dave's workstation, no
root for Claude) is where this list is checked: every step that needs root
is one a real user meets too.

A machine set up earlier with `install.sh` has PAM files in `/etc/pam.d`;
they are found first, so remove `/etc/pam.d/vdm*` after installing the
package.

## One rule for every platform

- **The server and the apps are one version.** They speak one exact bus
  protocol version and refuse any other (CLAUDE.md), so they are one
  package, or packages that each require the other at the same version.
- **Two setuid helpers** (FreeBSD only): `libexec/vergence/sound-default`
  and `libexec/vergence/media-eject`, owner root, group video, mode 4750.
- `libexec/vergence/vergence-decode` (the confined picture helper) and
  `libexec/vergence/vergence-choose` (the file dialog for other apps) are
  plain programs.

## Removable drives

The session mounts a USB stick or card when it is plugged in
(files milestone 1), `noexec` and `nosuid` on every system.

**Linux** (udisks2, over the system bus):

- Needs: `udisks2` (which brings `polkit`) and a running system D-Bus.
- **With logind** (Fedora, systemd distributions): nothing more. polkit
  sees the active local session and udisks2's own rules allow the mount.
- **Without logind** (Void with seatd, Hubble): polkit cannot tell the
  person at the machine from anyone else, so udisks2 refuses every mount
  ("Not authorized") until a rule allows it. Install
  `tools/linux/50-vergence-drives.rules` as
  `/etc/polkit-1/rules.d/50-vergence-drives.rules` (mode 644). It lets the
  seat group (`_seatd` on Void) mount, eject and power off removable
  drives only; internal disks (udisks2's `*-system` actions) still need an
  administrator.
- **Without logind**, polkitd must run as a service (Void:
  `/var/service/polkitd`); udisksd starts on demand over D-Bus.
- **After installing polkit**, the system bus has to reload its
  configuration (`dbus-send --system --dest=org.freedesktop.DBus /
  org.freedesktop.DBus.ReloadConfig`, or a reboot) before polkitd can claim
  its name; until then polkitd starts and exits again every second ("Lost
  the name org.freedesktop.PolicyKit1"). Found on Hubble, 2026-11-15.

**FreeBSD** (the base system's automounter):

- Nothing to install: autofs is in the base system.
- `/etc/autofs/special_vergence_media` from `tools/freebsd/media-map`
  (mode 755): only removable devices (`da`, `mmcsd`, `sdda`, `cd`), FAT
  writable by the video group.
- `/etc/auto_master`: `/media  -vergence_media  -nosuid,noatime,autoro`
  in place of the system's `/media` line (install.sh keeps the original as
  `/etc/auto_master.before-vergence`).
- `autofs_enable=YES` in rc.conf; automountd and autounmountd running.
- exFAT and NTFS sticks need `fusefs-exfat` and `fusefs-ntfs` (FreeBSD's
  own map says so in the system log when one is plugged in).

## The seat

- **With logind**: nothing.
- **Without logind**: `seatd` enabled (Void: `/var/service/seatd`;
  FreeBSD: `seatd_enable` in rc.conf), and each user of the session in the
  seat group (`_seatd` on Void, `video` on FreeBSD).

## Packages a Void machine needed (Hubble, 2026-11-09 onward)

For building: `base-devel pkg-config` and the `-devel` packages of
`libseat libinput libxkbcommon MesaLib libgbm wayland eudev-libudev
libdrm`. At run time: `seatd libinput libxkbcommon xkeyboard-config
mesa-dri mesa-vulkan-intel wayland xorg-server-xwayland (only for X11
apps, off by default) fontconfig noto-fonts-ttf adwaita-icon-theme
linux-firmware-intel openbsd-netcat (vctl) xrdb (Xft.dpi for X apps)
udisks2 polkit`; for screens on an NVIDIA GPU, `mesa-vulkan-nouveau
vulkan-loader` (install.sh says so when it finds one).

## Wi-Fi

Without NetworkManager, `wpa_supplicant` enabled for each card, its
control socket for the `wheel` group (`ctrl_interface_group=wheel`,
`update_config=1`); the settings app's Wi-Fi page works for users in wheel.
