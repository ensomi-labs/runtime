# Swift-against-Rust parity

The Swift app's sync is the reference for behaviour. This compares it with this crate on recordings, by feeding the same
audio through both extractors and both frame streams through Sonalign.

`main.swift` runs on a Mac, compiled together with the Swift app's sources, so that the decoding and the frames are
the app's own. It reads a fixture directory (recordings with `.ambient-sync-fixture.json` sidecars, as
`AmbientSyncFixtureRecorder` writes them) and writes, under numbered names, the PCM and the frames of each recording
and of its reference. `examples/parity.rs` then reports:

- Frame differences on the same 48 kHz PCM, which test the extractor port alone.
- Frame differences from a reference at its own rate through this crate's conversion, against the app's
  `AVAudioConverter`.
- Lock time and lock position through Sonalign, for each of these, and for a whole `Session` fed 20 ms blocks.
- Final locks of each query against another song's reference.

Both sides run the same Rust Sonalign: the baseline is the Swift app's frames, not its `Aligner` wrapper or its live
capture. `session` keeps its own block cadence, so it compares first locks, not call by call.

Recordings and exports stay private. Keep them outside the repository (`build/` is ignored). The small synthetic case in
`conformance/sync` has no such restriction, and `tests/conformance.rs` holds this crate to it everywhere.
