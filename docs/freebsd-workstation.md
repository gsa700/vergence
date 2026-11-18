# The FreeBSD workstation: install plan

Status: **planned** (Dave, 2026-11-17). Nothing here is done yet.

## Why

Hambench's hardware becomes the FreeBSD workstation and the primary place
Vergence is developed. Techbench's ITX hardware becomes the hamshack's Windows
PC in its place. The ATX board takes a GPU and the 10 Gb card at once (the ITX
board's single slot forced a choice), and its several NVMe slots let each
system have its own drive. NVIDIA on FreeBSD is where the most unbuilt work is
(whitepaper §8.3: NVIDIA's own driver and explicit sync), so the NVIDIA card
goes here. Linux stays covered by Hubble (Void, no systemd; an Intel + GTX 1650
hybrid laptop) and a Lenovo Tiny running Fedora (systemd and logind).

## The machine

- ASUS TUF GAMING Z690-PLUS WIFI (full ATX, DDR5), Intel i9-13900K with UHD 770 graphics.
- Supermicro dual-port SFP+ card with an Intel chip (FreeBSD: `ix` or `ixl`;
  the NAS runs the same kind of card on FreeBSD already).
- GPU: the GTX 960 to start (Maxwell: NVIDIA's legacy 580 branch only), a
  Turing or newer card later (a GTX 1650 matches Hubble's chip; a T1000 has
  four outputs).
- Several NVMe slots: FreeBSD on the first. Fedora goes on a second drive as
  boot option 2 when one is found (Dave, 2026-11-17: "reliability always
  matters to me"): a fallback when FreeBSD hits a wall, the same GPU and
  monitors under Linux to tell an OS bug from ours, and the systemd/logind
  side.

## Before installing (BIOS)

1. Raptor Lake microcode 0x12B or later, and Intel's default power limits
   (not the board's "unlimited" profile): release builds load all cores for
   minutes at a time. Microcode: done (BIOS 4301 has 0x12F, Intel's final
   fix; the newer 4505/4601/4701 are security and ME only, and Dave is
   waiting on them, 2026-11-17). Power limits: check.
2. Storage in plain NVMe/AHCI mode, Intel VMD off.
3. "iGPU Multi-Monitor" on, so the Intel outputs work beside the NVIDIA card.
4. Secure Boot off (FreeBSD's loader isn't signed for it).

## Install

- FreeBSD 15.1, the installer's ZFS layout with "Encrypt disks" (GELI) on the
  first NVMe. Encrypted, so the login screen never logs in by itself (the same
  rule as Techbench's LUKS).
- Boot environments (`bectl`) before every driver change.
- Network: the SFP+ card's port by DHCP in `rc.conf` (`ifconfig_ix0="DHCP"` or
  `ixl0`, whichever `pciconf -lv` shows). The card brings Hambench's MAC
  address, so the DHCP reservation and DNS name need changing on pfSense
  (Dave's step). Its name: **techbench** (Dave, 2026-11-17), already the reservation for that MAC on pfSense.

## Packages

kepler's list (2026-11-17) as the start:

    bash dbus drm-kmod firefox foot git htop libdrm libinput libudev-devd
    libxkbcommon mc mesa-dri mesa-libs nano noto-basic noto-emoji pixman
    pkgconf rsync seatd sudo wayland wayland-protocols xkeyboard-config
    xwayland zsh

plus `gpu-firmware-intel-kmod-alderlake` (Raptor Lake's UHD 770 uses Alder
Lake's firmware; check the name), and Rust by rustup (`. ~/.cargo/env` over
ssh, as on kepler).

`/etc/rc.conf`, as on kepler: `kld_list="i915kms"`, `seatd_enable`,
`dbus_enable`, `autofs_enable`, `sshd_enable`, `ntpd_enable`; `vdm_enable`
comes from `tools/install.sh`.

## Vergence

`tools/sync-hubble.sh <host>` (add the host), `cargo test --workspace`,
`cargo build --release`, `sudo tools/install.sh --vdm` (the login screen at
boot). `~/.config/vergence/control-socket` for `vctl`, if Dave wants it driven
remotely.

## First stage: Intel only

Two or three monitors on the UHD 770's outputs, no NVIDIA card. This makes
FreeBSD a daily workstation before NVIDIA brings its unknowns, and it's
kepler's setup with more screens.

## Second stage: the NVIDIA card

- Packages: `nvidia-driver-580` and `nvidia-drm-612-kmod-580` (the GTX 960;
  the kmod must match drm-kmod's 6.12 and be built for 1501000: 15.0's kmods
  panic on 15.1). A Turing or newer card later: the current branch, whose
  drm-612 kmod is only `-devel` (615) so far.
- `/boot/loader.conf`: `hw.nvidiadrm.modeset="1"`; `kld_list` gains
  `nvidia-drm`.
- GBM is there: both packages ship `/usr/local/lib/gbm/nvidia-drm_gbm.so`,
  `libnvidia-allocator` and `libnvidia-egl-gbm` (checked 2026-11-17 by
  listing the packages).

What to find out first, in this order:

1. Can i915 and nvidia-drm share buffers (dma-buf/PRIME)? That decides
   whether screens on both GPUs work (the GPUs milestone's relay), or whether
   every screen goes on the NVIDIA card to start.
2. "NVIDIA draws": on a desktop the card should draw and Intel's screens get
   the copies, the reverse of Hubble. Planned, not built.
3. Explicit sync (DRM syncobj), which NVIDIA's driver leans on.
4. Sleep and wake (an older note says NVIDIA's FreeBSD driver keeps no video
   memory over suspend under Wayland).
5. The hardware cursor (KDE users report it invisible on NVIDIA and fall back
   to a software cursor).

## Moving in

- Dave's scripts: `~/Projects/techbench-scripts` on Control (Techbench's
  `~/.local/bin`, crontab and backup script). The Linux-only ones are listed
  in its README.
- Not on FreeBSD: CD ripping (whipper), AppImages (Arduino IDE, Raspberry Pi
  Imager, nRF Connect), .NET 10, podman. These go to the Linux Tiny or stay
  on Control.
