# Local patches to opus-decoder 0.1.1

This is the published `opus-decoder` 0.1.1 crate (MIT OR Apache-2.0, see `LICENSE-MIT` and `LICENSE-APACHE`; it ports algorithms from libopus, BSD 3-Clause). Lyrion Anywhere uses it through `[patch.crates-io]` in the workspace `Cargo.toml` until a fixed release is published, then this copy should be removed.

## `celt/vq.rs`: `extract_collapse_mask` overflow

The collapse mask was built in a `u8` with `1 << i` for `i` up to the number of short blocks, which can be 16. In debug builds that panicked ("attempt to shift left with overflow"). In release builds the shift amount was masked, so the wrong bit was set and transient frames were decoded with the wrong anti-collapse decisions.

libopus builds the mask in an `unsigned` and keeps only the low 8 bits when it stores it. The patch does the same.

**Found by:** Lyrion Anywhere's `gateway/tests/live.rs`, which decodes `ffmpeg`/libopus output of noise at 96 kb/s.
