# Local patches to opus-decoder 0.1.1

This is the published `opus-decoder` 0.1.1 crate (MIT OR Apache-2.0, see `LICENSE-MIT` and `LICENSE-APACHE`; it ports algorithms from libopus, BSD 3-Clause). Lyrion Anywhere uses it through `[patch.crates-io]` in the workspace `Cargo.toml` until a fixed release is published, then this copy should be removed.

## `celt/vq.rs`: `extract_collapse_mask` overflow

The collapse mask was built in a `u8` with `1 << i` for `i` up to the number of short blocks, which can be 16. In debug builds that panicked ("attempt to shift left with overflow"). In release builds the shift amount was masked, so the wrong bit was set and transient frames were decoded with the wrong anti-collapse decisions.

libopus builds the mask in an `unsigned` and keeps only the low 8 bits when it stores it. The patch does the same.

**Found by:** Lyrion Anywhere's `gateway/tests/live.rs`, which decodes `ffmpeg`/libopus output of noise at 96 kb/s.

## `celt/kiss_fft.rs`, `celt/mdct.rs`: a real FFT for the inverse MDCT

The inverse MDCT's FFT (`flat_fft_forward`) was a direct DFT: O(n²) with a `sin_cos` per term, in f64, and with an f32 π. For 20 ms frames that is about 460,000 trigonometric calls per channel per frame, so decoding ran at only about 2× real time on a desktop core. That is not enough on a phone, or in a browser's WebAssembly.

The patch adds `FftPlan`: a mixed-radix forward FFT (radix 4, 2, 3 and 5, with a generic butterfly for any other factor), with its twiddles built once per size. `MdctBackward` keeps one plan for its size and uses it in place of `flat_fft_forward`. The result is closer to the exact transform than before, because the old one's f32 π made its angles drift as the size grew.

**Result:** about 170× real time instead of 2× (160 kb/s stereo, one desktop core). Against libopus decoding the same files, the signal-to-error ratio rose from 80.6 dB to about 100.5 dB.

**Tested by:**
- `plan_matches_exact_dft`, which checks every CELT size (60, 120, 240, 480) and awkward ones (1, 7, 49) against an exact f64 DFT;
- the crate's own tests (the RFC vector tests need `OPUS_TESTVECTORS_DIR` and are skipped without it);
- Lyrion Anywhere's `core/tests/opus.rs`, including a speed floor that the old code fails.

**Found by:** the decoder speed measurement for `docs/PERFORMANCE.md` (`core/examples/decode_speed.rs`).

## `lib.rs`: Hybrid redundancy longer than the frame

In Hybrid packets the redundancy byte count is read from the packet (2 to 257) and the redundant CELT frame was then taken as `&frame[frame.len() - redundancy_bytes..]` with no check, in both the CELT-to-SILK and the SILK-to-CELT branch. A short Hybrid frame with the redundancy flag set and a large count panicked the decoder: an overflow in debug builds, an out-of-range slice in release builds. In the browser build (`panic = "abort"`) that aborts the whole WebAssembly player; native hosts caught it but failed the stream.

libopus drops the redundancy when what is left of the frame after the redundant bytes is shorter than the bits already read. The patch applies the same rule (without shrinking the range decoder's storage) before either branch uses the count.

**Tested by:** `tests/robustness.rs` (added): two crafted packets, one per branch, that panicked the unpatched decoder; random short Hybrid frames; random packets for every TOC configuration, frame-count code and length 0 to 300; truncated and bit-flipped copies of real SILK, Hybrid, CELT and code 3 packets (`tests/fixtures/*.pkts`, made by us with FFmpeg's libopus from a synthetic signal); and the multistream wrapper. Each decode runs under `catch_unwind` and any panic fails the test.

**Found by:** Lyrion Anywhere's code review of 15 October 2026 (H2).

## `Cargo.toml`: standalone tests

The manifest gains an empty `[workspace]` table and a lock file, so `cargo test --locked --manifest-path third_party/opus-decoder/Cargo.toml` runs the crate's own tests (CI does this). The `criterion` dev-dependency is dropped: the benches are not vendored. The `robustness` test target is added.
