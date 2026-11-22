Test fixtures for vergence-hymie, generated (no recordings): sine tones made
by ffmpeg on techbench, 2026-11-21.

- tone16.flac: 1.5 s, 44.1 kHz, stereo, 16-bit (440 Hz left, 660 Hz right,
  ffmpeg's sine at 1/8 amplitude times 0.8: peaks about 0.1), compression level 8; title "Test Tone", artist "Vergence",
  album "Hymie Fixtures", ReplayGain track gain -6.00 dB, peak 0.8; a 64x64
  blue PNG as its front cover.
- tone24.flac: 1 s, 48 kHz, mono, 24-bit, 1 kHz, 1/8 amplitude times 0.5, compression
  level 5.

Both keep the MD5 of their decoded samples in STREAMINFO, which the tests
check the decoder against.

MP3s, encoded by LAME 4.0 on hubble, 2026-11-21, from WAVs written by a
Python script (16-bit, the tones at 0.5 amplitude starting at sample 0).
Each keeps the Info frame with LAME's tag (frame count, encoder delay 576
and padding), from which the tests check gapless length and placement:

- tone44.mp3: 1.5 s (66,150 samples), 44.1 kHz, stereo (440 Hz left, 660 Hz
  right), MPEG-1, CBR 128 kbps, joint stereo; ID3v2.3 title "Test Tone MP3",
  artist "Vergence", album "Hymie Fixtures". LAME wrote a track gain
  (-8.2 dB) without a peak, so it has no ReplayGain as FFmpeg reads it.
- tone22.mp3: 1 s (22,050 samples), 22.05 kHz, mono, 1 kHz, MPEG-2, CBR
  64 kbps (at 32 kbps the frames are too small for LAME's tag); title
  "Mono Tone".
- burst48.mp3: 1 s (48,000 samples), 48 kHz, stereo, MPEG-1, VBR (-V 2),
  joint stereo: a 300 Hz tone at 0.2 under bursts of noise (a fixed linear
  congruential generator, 0.4 left and 0.3 right) for 1,500 samples every
  12,000, so that short blocks and mid/side stereo are decoded; title
  "Bursts".

FFmpeg 9.0.2 (techbench) decodes all three to the same length, and
`HYMIE_MP3_REFERENCE=<folder of ffmpeg -f f32le decodes> cargo test` compares
them sample by sample (about 2e-7 RMS apart, 2026-11-21).

MP4s (.m4a), encoded by FFmpeg 9.0.2 on techbench, 2026-11-21 (muxer
Lavf63.1.102: an edit list starting at 0 for ALAC, at 1024 for AAC; the
moov after the frames):

- alac16.m4a: ALAC from tone16.flac (16-bit stereo, 44.1 kHz, 66,150
  samples), its title changed to "ALAC Tone", artist and album and the PNG
  cover carried over (covr, data type 14). Four freeform (----) atoms were
  then added to its ilst by a Python script, named in lower case as
  foobar2000 writes them: replaygain_track_gain "-7.50 dB",
  replaygain_track_peak "0.800000", replaygain_album_gain "-8.25 dB",
  replaygain_album_peak "0.900000".
- alac24.m4a: ALAC from tone24.flac (24-bit mono, 48 kHz, 48,000 samples;
  `-sample_fmt s32p`), title "ALAC 24". Its frames shift off one byte.
- noise16.m4a: ALAC from a WAV of 11,025 stereo 16-bit frames at 44.1 kHz,
  each sample the top 16 bits of the generator x = x * 1103515245 + 12345
  (from x = 1; left, then right), which the tests make again. Full-scale
  noise, so ALAC stores its frames uncompressed (escaped); the last frame is
  short. Title "Noise".
- aac.m4a: AAC-LC, 64 kbps, 0.5 s of FFmpeg's 440 Hz sine, mono, 44.1 kHz;
  title "AAC Tone", artist "Vergence", album "Hymie Fixtures", a 32x32 red
  JPEG cover (covr, data type 13). Not played by this crate (AAC has its own
  crate and helper, vergence-hymie-aac): its tags and cover are read here.

AAC-LC with real sound, for vergence-hymie-aac's tests (they live here with
the other sound fixtures), encoded by FFmpeg 9.0.2's own AAC encoder on
techbench, 2026-11-21, from its aevalsrc (sines at 0.5 starting at sample 0),
with `-aac_pns 0` (PNS noise is random by design, so a seek could not be
compared with a straight read); intensity stereo, M/S and TNS left on. An
edit list starting at 1024 (the encoder's priming) and as long as the
samples given:

- aac44.m4a: 1.5 s (66,150 samples), 44.1 kHz, stereo (440 Hz left, 660 Hz
  right), 128 kbps; title "AAC Stereo", artist "Vergence", album "Hymie
  Fixtures".
- aac48.m4a: 1 s (48,000 samples), 48 kHz, mono, 1 kHz, 96 kbps; title "AAC
  Mono".
- aacburst.m4a: 1 s (48,000 samples), 48 kHz, stereo, 128 kbps: a 300 Hz
  tone at 0.2 under bursts of aevalsrc's random() noise (0.4 left, 0.3
  right) for 1,500 samples every 12,000: its frames use short windows, M/S,
  intensity stereo and TNS (vergence-hymie-aac's tests count them);
  title "AAC Bursts".

FFmpeg decodes all three to the same length, and `HYMIE_AAC_REFERENCE=<folder
of ffmpeg -f f32le decodes> cargo test -p vergence-hymie-aac` compares them.

FFmpeg decodes the three ALAC files to exactly the samples they were made
from, and so do the tests (against tone16.flac and tone24.flac, whose own
decode matches their MD5).
