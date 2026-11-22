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
