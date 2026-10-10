# Phase 0 report — tunnel spike

_In progress · started 2026-10-10_

Phase 0 proves the riskiest assumption: a clean-room Rust core can act as a real LMS player, then do the same through the gateway's WebSocket tunnel and in a browser.

## Gate criteria

| Gate | Status |
|---|---|
| Core plays as a real LMS player (native, direct TCP) | **Passed** |
| Gapless track transition | **Passed** at the protocol level and in a sample-exact unit test. A listening test is still to do. |
| Plays through gateway WSS tunnel behind a reverse proxy | **Passed**: live through Caddy (TLS, HTTP/2) against LMS 9.1.x |
| Browser player (WASM + AudioWorklet), gapless | Not started |
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

## Gateway spike results

The gateway was rewritten from Go to Rust (spec D2) so it shares the core's fuzzed Slimproto parser and stream-request policy.

Test path: LMS ← `anywhere-gateway` (loopback) ← Caddy (`tls internal`, HTTP/2) ← `anywhere-cli --gateway`. The token was read from a git-ignored file, never argv.

- **Registration:** the player registered through the tunnel. HELO was checked against the device's assigned MAC.
- **Playback:** the album played, with LMS time 3.0 → 6.0 → 9.0 s.
- **Seek:** produced gateway stream #2.
- **Gapless transition:** LMS sent track 2 (stream #3) while track 1 was still playing, and moved to index 1 at the boundary.
- **Audio relay:** all audio went through `/v1/stream/<session>/<n>`. The gateway replayed only the request LMS itself issued, after exact-match validation.
- **Blocked paths:** through the proxy, `/jsonrpc.js`, `/music/1/download`, `/stream.mp3`, `/settings/index.html` and `/cometd` all returned 404.
- **Abrupt disconnect:** killing the client released the session and closed the LMS connection, and LMS marked the player disconnected. (Phase 1 adds the resume grace period.)

## Test suite at this point

- `cargo test --workspace`: 47 tests, all passing. Clippy is clean with `-D warnings`.
- **Gateway: 16 integration tests against a fake LMS** that records every frame and HTTP path it receives. They cover:
  - missing, wrong, near-miss, oversize and binary tokens;
  - cross-origin upgrades;
  - wrong-MAC, missing, truncated and repeated HELO frames, and oversize frames;
  - `serv` dropping;
  - stream requests for another player, LMS internals or direct streams;
  - stream-endpoint authorisation (no token, another device's token, token in the query string, basic auth, stale or future sequence numbers);
  - no LMS path reachable;
  - security headers;
  - TLS/proxy-header enforcement, including IPv4-mapped addresses.
- **Mutation checks:** removing the HELO MAC check, the `serv` drop or the Origin check each makes the suite fail.
- **Licence audit:** every dependency was checked for GPL compatibility, which led to the relicence to GPL-2.0-or-later (spec D11). `aws-lc`/OpenSSL are kept out; TLS uses rustls with ring.
- `cargo test -p anywhere-core`: 27 tests, all passing.
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

1. WASM build of the core + AudioWorklet output + a minimal SvelteKit page.
2. nginx example config for real deployments, tested the same way.
3. Mobile Safari: AudioWorklet vs Managed Media Source with the screen locked.
