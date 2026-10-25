# The quality badge and the signal path, redesigned: agreed design

_Agreed with the owner on 2026-10-25 (questions and answers in the session). Ships with plugin 0.7.3 and Android 1.17.0, in the batch held on main. Supersedes the display parts of `streaming-quality.md` ("Clients")._

## Why

Playing a 24/96 FLAC on mobile data, the Android app's badge by the track said **FLAC 24/96 · Hi-Res** while the sheet it opened said **Lossy source · Opus 48 kHz**. Both were "true" of something: the badge described the library file, the sheet the stream the engine decodes after the gateway re-encoded it. Neither answered the only question most people have: *how good is what I am hearing?* The web badge was changed on 2026-10-18 to name the stream heard; the Android badge was not, and the sheet's verdict wording ("Lossy source") is wrong for a lossless file that was re-encoded on the way. The panel as a whole is a list of facts, not a picture of a path.

## Decisions

1. **The badge states the weakest link, then what is heard.** Verdict word first, stream second: **Hi-Res · FLAC 24/96**, **Lossless · FLAC 16/44.1**, **Transcoded · Opus 160**, **Lossy · MP3 320**. The verdict is decided by codecs and re-encoding only: a lossy file is *Lossy*; a lossless file re-encoded to Opus by the gateway is *Transcoded*; a lossless stream arriving as stored is *Hi-Res* (more than 16 bits or more than 48 kHz) or *Lossless*. Resampling by the device, EQ, levelling, crossfeed, ReplayGain and volume never change the verdict: they are stages in the sheet. Unknown format: no badge.
2. **Colour as well as words, subtly:** Hi-Res and Lossless in the theme's badge colour; Transcoded and Lossy in an amber tint. The word always carries the meaning.
3. **The sheet is a block diagram**, stacked on phones and a row on wide screens, each block a card with a title, the format at that point and one line of what happened there, arrows between. The block that lowers the verdict is tinted amber. Blocks:
   - **File** — the library track as LMS has it: codec, bits/rate or bit rate (status tags `type`, `samplerate`, `samplesize`, `bitrate`).
   - **LMS** — only when LMS changed something on its way out: converted the codec (ALAC → FLAC for our players, any transcoding rule), lowered the rate or depth, or applied server ReplayGain. Otherwise no block.
   - **Lyrion gateway** — "passed through unchanged" or "re-encoded to Opus 160 kb/s", and why: *High, on mobile data* / *Data saver, chosen in Settings* / *overridden here*. Under it, the quality switch (decision 5).
   - **This device** — the decoder (what arrived: "Opus 48 kHz" or "FLAC 24/96"), the output device and its rate, and the bit-perfect or resampling note. Under it, **Details** (collapsed): today's stage list, in order (ReplayGain, volume, DSP stages, resampling, dither, output), the cushion, the output clock, the EQ chip.
4. **Other players and offline:** a remote player (Squeezebox, UPnP) shows **File → Player** from LMS's status, badge from the file (the gateway never transcodes for it), no gateway block and no switch. A download played offline shows **Download → This device** with the download's format (the FLAC original, or the Opus/MP3 it was saved as), no switch.
5. **Switching quality from the sheet, as an override.** Buttons *Original · High · Data saver* under the gateway block; the one in force is selected. A tap overrides the automatic choice for this session: until the app (web page) is restarted or the network type changes (Wi-Fi ↔ mobile data), when the per-network setting applies again. Settings are not changed. The gateway block then says "overridden here", with *Back to automatic*. The change takes effect **now**: the client sends the gateway `{"quality": …}` and seeks to the current position, so the new stream starts there within a second or two (a short gap); for a stream that cannot be sought (radio) it applies from the next stream, and the sheet says so.
6. **Placement:** the badge is on Now playing (both layouts, beside the artist as today) and in the mini player's second line, since what is heard exists only for the playing track. Library and queue rows keep showing the file's format where they do today.
7. **One design, both clients.** The verdict rules, the badge text and the block model live in the shared Rust core, with JSON test vectors both clients must pass, so web and Android cannot drift (as `core::privnet` does for private networks).

## Data model (shared, in `core/`)

```rust
pub enum Verdict { HiRes, Lossless, Transcoded, Lossy }
pub struct Badge { verdict: Verdict, stream: String }      // "Transcoded", "Opus 160"
pub enum BlockKind { File, Lms, Gateway, Device, Download, Player }
pub struct Block { kind, title: String, format: String, note: String, lowers: bool }
pub struct Diagram { badge: Option<Badge>, blocks: Vec<Block>, switch: Option<Switch> }
pub struct Switch { levels: [Level; 3], in_force: Level, origin: Origin /* Setting(network) | Override | Auto */, seekable: bool }
```

Inputs (`QualityInputs`): the file's format from status; what LMS served (codec, and bits/rate when the stream is passed through) and whether server gain applied; what the gateway sent (pass-through, or Opus at N kb/s and the level asked); the decoder's format; the output device, rate and route verdicts (today's `SignalPathInfo`); the quality origin. One pure function `describe(&QualityInputs) -> Diagram`. Vectors: `core/tests/fixtures/quality-vectors.json`, each an input and the expected badge, blocks and `lowers` flags, covering: FLAC 24/96 passed through; FLAC 24/96 re-encoded (High on mobile data; Data saver by setting; override); FLAC 16/44.1 passed through; ALAC file served as FLAC (LMS block); FLAC 24/96 downsampled by LMS to 16/44.1 (LMS block, verdict Lossless); MP3 320 as sent (Lossy, no gateway change); Opus radio stream; server ReplayGain only; remote player; offline download original and offline Opus; unknown type (no badge). The web (TypeScript) and the mobile crate run the same vectors; the Android UI tests render from the model.

## What the gateway must tell the client

The engine sees only the stream it decodes. For the LMS and gateway blocks the client needs what LMS served and what the gateway did. The stream relay (`GET /v1/stream…`) adds response headers, which old clients ignore and which need no socket protocol change:

- `Anywhere-Served: flc` — the codec LMS sent the gateway, from the `strm` format byte (`f` FLAC, `p` PCM with rate and depth from the `strm` fields, `m` MP3, `o` Ogg, `a` AAC, `l` ALAC).
- `Anywhere-Sent: passthrough` or `Anywhere-Sent: opus; kbps=160; level=high` — what the gateway did to it.

Both are plain tokens the clients validate like every other header (letters, digits, `;`, `=`, at most 64 bytes). `GET /v1/me` lists `"stream_info": true` so clients know the headers are meant (absent: infer from the decoder and the level asked, as today). PROTOCOL.md gets the two headers under *Streams*; tests: the headers for a pass-through, an Opus re-encode at each level, a PCM stream, and that they survive `?offset=` resumes and `?limit=` chunks.

## Clients

**Engines (mobile crate, web worker):** read the two headers at stream start and keep them with the track's audio (`TrackAudio.served`, `TrackAudio.sent`); expose them in the signal-path snapshot. The web worker passes them to the page with the existing `source` status.

**Android:**
- `QualityBadge` takes the `Badge` from the diagram, never `track.quality`, when this phone is the player; for a remote player the badge is the file's (as today, through the same `describe`). Mini player: the same text, compact.
- `SignalPathSheet` draws the diagram: `Column` of block cards on compact widths, `Row` on medium and expanded; arrows as a small icon between; the lowering block tinted `tertiaryContainer`-amber; *Details* an expandable section under the device block holding today's `StageRow` list, `CushionLine`, `OutputClockLine`, `EqChip`.
- The switch: a `SegmentedButton` row under the gateway block; selection calls `PlayerViewModel.overrideQuality(level)`, which sets `LocalPlayerRepository.qualityOverride` (in memory, cleared on process start and on a metered-state change), sends the control message through the existing path and seeks to the current position (`time <t>`) when the track is seekable. "Back to automatic" clears it. The gateway block's reason line reads the origin.
- Strings in British English; `verdict_*`, `block_*`, `switch_*`.

**Web:**
- `quality.ts` keeps its codec table but the badge comes from the shared `describe` (TypeScript port in `lib/player/quality-path.ts`, vector-tested). `QualityBadge`/the title row and the mini player use it.
- The sheet (`SignalPath.svelte`) draws the blocks with CSS grid: one column under 600 px, a row above; same tint and Details section.
- The switch: three buttons; `device.overrideQuality(level)` sets an in-memory override (cleared on reload and when `navigator.connection.type` changes), sends `{"quality"}` and seeks via `api.rpc(mac, ['time', t])`. The Settings page's "Streaming quality in this browser" says when an override is in force.

## Tests

- Core: the vectors; `describe` never panics on junk (fuzz over random inputs); badge text matches the vectors byte for byte.
- Gateway: the two headers (see above); the quality control message followed by a seek yields a stream at the new level (an integration test with the fake LMS and a transcode stub, as in the streaming-quality tests).
- Mobile crate: headers parsed and carried through a resume; the snapshot carries them; junk headers ignored.
- Android (unit): the badge uses the heard stream (the exact case reported: FLAC 24/96 on mobile data → "Transcoded · Opus 160", amber); the diagram's blocks for each vector; the LMS block appears only when LMS changed something; the switch sends the message and seeks; the override clears on a metered change and survives a track change; a remote player and offline show their shapes; TalkBack reads the blocks in order.
- Web (Vitest): the same cases on the components; the override's lifetime; Safari (no Network Information API) behaves.
- Live: on the owner's setup, the reported track on mobile data shows "Transcoded · Opus 160" and the diagram File → Gateway (amber) → Phone; switching to Original in the sheet restarts the stream at the same position with FLAC 24/96 and the badge turns Hi-Res; back to automatic returns to Opus on the next stream.

## Docs

SPEC §7.2 "Quality badge and signal path" rewritten, decision **D30**; FEATURES rows *Quality badge and signal path* and *Streaming quality* updated; PROTOCOL (headers, `stream_info`); CHANGELOG; `streaming-quality.md` "Clients" section marked superseded by this plan.

## Out of scope

Changing the quality levels or bit rates (D25 stands); LMS's own transcoding rules (shown, not controlled); a per-track permanent setting; queue rows predicting a verdict.
