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
