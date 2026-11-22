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
