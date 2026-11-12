# Local patches to opus-decoder 0.1.1

This is the published `opus-decoder` 0.1.1 crate (MIT OR Apache-2.0, see `LICENSE-MIT` and `LICENSE-APACHE`; it ports algorithms from libopus, BSD 3-Clause, whose notice is in `COPYING-libopus`). Lyrion Anywhere uses it through `[patch.crates-io]` in the workspace `Cargo.toml` until a fixed release is published, then this copy should be removed.

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

## Debug logging and trace scaffolding removed (no I/O)

Upstream 0.1.1 shipped its author's debugging code. Two blocks in `celt/mod.rs` opened a log file at a hard-coded path on the author's machine (`OpenOptions::new().create(true).append(true)`) on every CELT frame, under no flag: a failing `open(2)` per frame on Linux and Android, and on any machine where that directory exists, a JSON line per frame for ever. Twenty-seven more such blocks in `celt/mod.rs`, `celt/bands.rs` and `celt/mdct.rs` sat behind trace flags hard-coded to `false`. Elsewhere JSON strings and diagnostic sums were built for loggers that did nothing: on every stereo CELT frame, every intra-energy frame, and at fixed indices of a process-wide packet counter in `silk/mod.rs`.

The patch removes all of it: every `#region agent log` block, the no-op loggers (`append_debug_log`, `append_runtime_debug_log`, `append_anti_collapse_debug_log`, `append_silk_debug_log`, `trace_checkpoint`, `trace_lpc`, `trace_core`), the `debug_trace!` macro and its calls, the trace flags and the blocks they gated, the diagnostic values only those blocks read, the `trace_*` parameters, and the `TRACE_*` global counters (one `fetch_add(SeqCst)` per packet and per CELT frame, shared by every decoder in the process). The `packet_idx` the internals still pass around is now a constant that nothing reads.

Decoding is unchanged: the same PCM and final range, bit for bit, at every output rate and channel count, for the SILK, Hybrid, CELT and code 3 fixtures and the core's Ogg fixtures, including packet loss concealment (checked before and after with a digest of the output), and the crate's unit tests (among them `plan_matches_exact_dft`) pass as before.

**Tested by:** `tests/no_io.rs` (added), which fails if `OpenOptions`, `std::fs`, `std::io::Write`, `File::create`, `File::open`, `/Users/` or `#region agent log` appears in the library sources outside their test modules and doc comments.

**Found by:** Lyrion Anywhere's code review of 15 October 2026 (M22). Worth reporting upstream.

## `packet.rs`: packets up to RFC 6716's limits

Packets over 1,500 bytes were refused (`MAX_PACKET_SIZE`), but RFC 6716 allows frames of up to 1,275 bytes and up to 48 of them in a packet (120 ms of 2.5 ms frames). So a 40 or 60 ms packet above about 200 kb/s failed, and after 64 such packets in a row Lyrion Anywhere's core stopped the stream.

The patch accepts packets of up to 61,440 bytes: 48 frames of 1,275 bytes with all their header bytes fit in 61,296, so anything longer is padding, and RFC 7845 §6 has Ogg Opus demuxers treat a packet over 61,440 bytes per stream as invalid. In its place it applies RFC 6716's own rule, as libopus does: a frame whose length is implied (code 0, code 1, CBR code 3, the last frame of code 2 and of VBR code 3) may not be longer than 1,275 bytes. Explicit frame lengths can't say more than that.

**Tested by:** `packets_up_to_the_rfc_limits_parse` in `packet.rs` (48 frames of 1,275 bytes, a 2,102-byte 60 ms packet, and one byte over in each place a length is implied) and `long_packets_within_the_rfc_decode_and_never_panic` in `tests/robustness.rs` (a 2,102-byte packet decodes; random packets of every frame count up to the limit never panic).

**Found by:** Lyrion Anywhere's code review of 15 October 2026 (still open on 12 November).

## `COPYING-libopus`: libopus's licence notice

The crate ports libopus function by function, and libopus is under the BSD 3-Clause licence, which asks that redistributed source keep its copyright notice, conditions and disclaimer. Upstream links to it but does not include it. `COPYING-libopus` is libopus's `COPYING` file, unchanged (as shipped with libopus 1.6.1).

**Found by:** Lyrion Anywhere's code review of 15 October 2026.
