# Phase 0 report — tunnel spike

_In progress · started 2026-10-10_

Phase 0 proves the riskiest assumption: a clean-room Rust core can act as a real LMS player, then do the same through the gateway's WebSocket tunnel and in a browser.

## Gate criteria

| Gate | Status |
|---|---|
| Core plays as a real LMS player (native, direct TCP) | **Passed** |
| Gapless track transition | **Passed** at the protocol level and in a sample-exact unit test. A listening test is still to do. |
| Plays through gateway WSS tunnel behind a reverse proxy | Not started (needs Go and Caddy/nginx) |
| Browser player (WASM + AudioWorklet), gapless | Not started (needs wasm-pack and the Rust wasm32 target) |
| Mobile Safari background-audio path chosen | Not started |

## Native spike results (LMS 9.1.x)

Test setup:
- `anywhere-cli` with `--null`, a silent real-time sink, so nothing was audible on the network.
- A dedicated test player MAC (`02:4c:41:00:00:01`).
- Playback driven over JSON-RPC, against that player only.

Findings:

- **Registration:** LMS lists the player as model `squeezelite`, model name "Lyrion Anywhere". HELO and SETD (name) are accepted.
- **Playback:** a 10-track FLAC album played at 44.1 kHz. LMS's reported time (3.0, 6.0, 9.0, 12.1 s) matched the core's own elapsed time.
- **Back-pressure:** the compressed input buffer held at about 8 MiB, the high-water mark. PCM stayed at about 10 s ahead.
- **Seek:** `time 160` produced `STMf`, a new stream (LMS seeks server-side), then `STMs`.
- **Gapless transition:** the sequence was `DSCO` → `STMd` → LMS sent the next `strm s` while track 1 was still playing → `STMs` at the boundary. LMS advanced to index 1 with output never stopping.
- **Transport:**
  - pause → `STMp`, and LMS shows `pause` with time frozen;
  - resume → `STMr`;
  - stop → `STMf`.
- **Volume:** LMS volume 50 arrived as a linear gain of about 0.06 (about −24.5 dB). This is LMS's own volume curve for squeezelite-class players. The plugin will fix remote-device players at 100% (spec §4).

## Test suite at this point

- `cargo test -p anywhere-core`: 25 tests, all passing.
  - Protocol framing and STAT layout.
  - Streaming decode under partial data.
  - The player state machine, including a sample-exact two-track gapless test.
  - Robustness tests that drive the frame readers, player and decoder with arbitrary and adversarial input.
- **Bugs the robustness tests found:**
  - A clock that went backwards would panic in the heartbeat timer. Fixed with a saturating subtraction.
  - The server frame-size guard could never fire, because a `u16` length can't exceed 64 KiB. The limit is now 32 KiB, with a regression test.

## Gotchas

- LMS sends volume as 16.16 gain values already shaped by its dB curve. Don't apply a second curve on the client.
- After `strm s` LMS expects `STMc` promptly, then `RESP` with the HTTP headers. `STMd` is what makes LMS send the next track, so sending it late breaks gapless playback.
- Symphonia treats a zero-byte read as end of stream. The decoder therefore demuxes only with at least 128 KiB of lookahead, or after the stream has ended, and it re-probes when large embedded artwork arrives in pieces.
- The core outputs PCM at the track's native sample rate and doesn't resample. Hosts reopen their output on a rate change, so gapless playback holds only between tracks with the same rate. That covers albums in practice.

## Next

1. Gateway spike (Go): `WS /v1/player`, `/v1/stream/{session}`, HELO MAC enforcement, `serv` dropping. Security tests come first.
2. WASM build of the core + AudioWorklet output + a minimal SvelteKit page.
3. A test through a TLS reverse proxy (Caddy locally; nginx config for real deployments).
4. Mobile Safari: AudioWorklet vs Managed Media Source with the screen locked.
