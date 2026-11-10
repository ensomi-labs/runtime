# Sync frames

`frames.json` holds the feature frames the Swift app's extractor makes from `frames.f32`, a synthetic signal of 8704
little-endian f32 samples at 48 kHz: silence, two tones, a click, quiet noise, loud noise and a low tone.
`crates/sync/parity/main.swift`, with `--conformance`, writes both. Frames are compared within the rounding of a
single-precision FFT, whose order of operations another implementation need not share.
