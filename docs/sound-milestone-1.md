# Sound milestone 1: a Sound page and volume keys

Drafted 2026-11-11 (Dave: "we might need a sound tab in settings"; first
raised 2026-11-10, when kepler's HDMI codec filled its console and he said
to leave it "until a SOUND settings page exists"). **Accepted
2026-11-11** with the three answers under Decided.

The goal: the settings app gets a **Sound** page that lists the machine's
outputs by the names people know ("Front headphones", "Internal speaker",
"HDMI: ASUS VS229"), says which one sound goes to, and sets each one's
volume and mute; the keyboard's volume and mute keys change the same
volume, everywhere in the session. Our own code, on FreeBSD and Linux.

## Where things stand (read 2026-11-11)

- **Nothing in Vergence knows about sound yet**: no page, no volume keys,
  no sound code of our own. Programs play to whatever the system's
  default is.
- **FreeBSD (kepler)** has OSS (`snd_hda`): one `pcm` device per output
  group the codec reports (kepler: pcm0 the internal speaker, pcm1 the
  front headphone jack, pcm2 HDMI). Each has a mixer, `/dev/mixerN`, mode
  0666: volume and mute by plain ioctls, no privilege. Which device is the
  default is the sysctl `hw.snd.default_unit`, which only root may write.
  The outputs' names come from the codec's pin descriptions (the
  `dev.hdaa.N.nidM` sysctls, and `/dev/sndstat`).
- **kepler's front headphone jack is silent under FreeBSD** (found
  2026-11-11): the jack senses a plug, audio reaches it, nothing comes out.
  The Realtek ALC294 seems to want a vendor setup for its headphone
  amplifier that FreeBSD's driver does not do. Not this milestone's to
  fix, but the page must be honest about an output that does not work.
- **Void (Hubble)** has plain ALSA, no PipeWire or PulseAudio: one card,
  `/dev/snd/controlC0` (group `audio`) for its controls. The default
  output is per user, in `~/.asoundrc`: no privilege.
- **Fedora (Techbench)** runs PipeWire, which owns the devices: volume and
  the default output go through it, by its library (borrowing) or its
  socket protocol (ours to write, as the D-Bus edge was).

## Design

**`vergence-sound`, a crate of our own**, used by the settings app (the
page) and the server (volume keys, and the little volume display that
shows when they are pressed). One interface, a back end per system:

- **OSS** (FreeBSD): the devices from `/dev/sndstat` and the codec's pin
  names; volume and mute with the mixer ioctls (`SOUND_MIXER_READ_*`,
  `SOUND_MIXER_WRITE_*`); the default from `hw.snd.default_unit`.
- **ALSA** (Linux without a sound server): the card's controls with
  ALSA's control ioctls (`SNDRV_CTL_IOCTL_ELEM_LIST`, `_INFO`, `_READ`,
  `_WRITE`), our own code as the wire protocols are; the default output in
  `~/.asoundrc`.
- **PipeWire** (Linux with it): see Decided, 2: not yet.

Each output: a stable id, its name, whether it is plugged (jack sense,
where the system says), volume (0 to 100%), mute, and whether it is the
default. The back end is chosen at run time, as the server picks seatd or
logind.

**The page**: the outputs as rows, the default one marked, each with a
volume slider and a mute switch; choosing another output makes it the
default. An output that is present but has never been heard to work is
not hidden (we cannot know); a jack with nothing plugged in says so where
the system tells us. Later: inputs, a test sound, per-program volume.

**Volume keys**: the server reads the keyboard's volume up, volume down
and mute keys (`XF86AudioRaiseVolume` and the rest), changes the default
output's volume in steps of 5%, and draws a small display of the new level
for a moment, as notifications are drawn now. The settings page follows
when it is open.

**Settings**: what is chosen goes in `desktop.toml` (`[sound]`), so the
session puts it back at login; the system's own state (FreeBSD's mixer
service, ALSA's `alsactl`) still restores levels at boot.

## Steps

1. `vergence-sound` with the OSS back end; a `vsound` tool (list, set
   volume, mute, default) to try it on kepler.
2. The ALSA back end; `vsound` on Hubble.
3. The settings app's Sound page, on both.
4. Volume keys and their display in the server.
5. The FreeBSD root helper for the default output (Decided 1), installed by `tools/install.sh`.

## Acceptance

1. On kepler and Hubble, the page lists every output by name, shows which
   is the default, and its sliders and switches change what is heard (the
   level the system reports matches).
2. Choosing another output makes new sound go there.
3. The volume keys change the default output's volume and show it; the
   page, when open, follows.
4. A chosen output and its volume come back after logging in again.
5. Nothing of the sound system is borrowed without Dave's word; no root is
   needed except the helper Decided 1 names.

## Security (whitepaper §7.3)

- Volume and mute need no privilege on either system.
- Choosing FreeBSD's default output needs root: the only privileged step
  in this milestone, and it must stay that narrow (Decided, 1).
- A program cannot change another's volume through us; only the settings
  app and the server's keys do, as the user.

## Not in this milestone

- Inputs (microphones), a test sound, per-program volume, Bluetooth audio.
- Fixing kepler's headphone jack (a codec setup FreeBSD lacks; possibly
  upstream work later).
- Sound for X11 applications beyond what the system gives them.

## Decided (Dave, 2026-11-11: each as proposed)

1. FreeBSD's default output: **(a), a tiny root helper** that sets
   `hw.snd.default_unit` and nothing else, installed by
   `tools/install.sh`.
2. PipeWire: **(c), not yet**; the page says so on PipeWire systems and
   the volume keys wait for our own client in a later milestone.
3. Volume keys: **5% a press**.

## The questions as asked

1. **FreeBSD's default output needs root** (`hw.snd.default_unit`). Ways:
   (a) a tiny root helper that can set that one sysctl and nothing else,
   installed by `tools/install.sh`, run by the settings app; (b) a
   `sudoers`/`doas` rule for that one command, written at install; (c)
   leave the default to the system on FreeBSD and only set volumes there.
   Proposed: (a), the narrowest and the same on every FreeBSD.
2. **PipeWire on Fedora**: (a) our own client of its socket protocol
   (our code, larger); (b) its library (borrowing); (c) not yet: Fedora's
   page says "managed by PipeWire" and the volume keys do nothing there
   until later. Proposed: (c) now, (a) later; Fedora is not the target.
3. **Volume keys' step**: 5% a press, proposed; or the system's own
   steps (ALSA cards often have 64 or so).
