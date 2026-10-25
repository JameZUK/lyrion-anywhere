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
pub struct Badge { verdict: Verdict, stream: String, text: String }   // "Transcoded", "Opus 160", "Transcoded · Opus 160"
pub enum BlockKind { File, Lms, Gateway, Device, Download, Player }
pub struct Block { kind, title: String, format: String, note: String, lowers: bool }
pub struct Diagram { badge: Option<Badge>, blocks: Vec<Block>, switch: Option<Switch> }
pub struct Switch { levels: [Level; 3], in_force: Level /* the level the current stream was sent at, else the origin's */, origin_text: String, overridden: bool, seekable: bool }
```

Inputs (`QualityInputs`): the file's format from status; what LMS served (codec, and bits/rate when the stream is passed through) and whether server gain applied; what the gateway sent (pass-through, or Opus at N kb/s and the level asked); the decoder's format; the output device, rate and route verdicts (today's `SignalPathInfo`); the quality origin. One pure function `describe(&QualityInputs) -> Diagram`. Vectors: `core/tests/fixtures/quality-vectors.json`, each an input and the expected badge, blocks and `lowers` flags, covering: FLAC 24/96 passed through; FLAC 24/96 re-encoded (High on mobile data; Data saver by setting; override); FLAC 16/44.1 passed through; ALAC file served as FLAC (LMS block); FLAC 24/96 downsampled by LMS to 16/44.1 (LMS block, verdict Lossless); MP3 320 as sent (Lossy, no gateway change); Opus radio stream; server ReplayGain only; remote player; offline download original and offline Opus; unknown type (no badge). The web (TypeScript) and the mobile crate run the same vectors; the Android UI tests render from the model.

## Clarifications from the build (2026-10-25)

- **`lowers` means "lowers the verdict".** The LMS block is tinted only when LMS's conversion turned a Hi-Res file into a Lossless stream, or sent a lossy codec. A 24/96 file served as 24/48 shows "Lowered to 48 kHz" in the block's note but is still Hi-Res by decision 1, so it is not tinted. Exactly one block is tinted at most, never the device.
- **Android passes its DSP-chain flag** (`dsp_chain`), not `dsp_active`, to the device input: `dsp_active` also covers the resampler, and the note would otherwise blame the listener's DSP for resampling.
- **`Switch.overridden`** was added so a client knows when to offer *Back to automatic*.
- **After the review (2026-10-25, 29 vectors):** the switch's `in_force` is the level the current stream was sent at (Opus 96 = Data saver, any other Opus = High, a passed-through FLAC = Original; a passed-through MP3 or PCM says nothing, since the gateway only re-encodes FLAC), falling back to the origin's level; when the chosen level differs from the one heard the reason reads "{Level} applies from the next track" and the gateway note ends with it ("Passed through unchanged; High applies from the next track", "Re-encoded to Opus 160 kb/s; Original applies from the next track"). A Hi-Res file that LMS turns into a non-Hi-Res lossless stream tints the LMS block (DSD64 → FLAC 16/44.1 included). With no file type, the served stream's losslessness decides the verdict; a lossy decode of a lossless source before the stream message arrives tints the gateway block with "Re-encoded (details to follow)". A headerless passed-through stream takes its rate and depth from the decoder before the file. The device note keeps the output's own name as given ("iBasso DC03, bit-perfect"). Names lose every default-ignorable and bidi character (the whole `Cf` category, Hangul fillers, variation selectors), then are cut to 40 characters.
- **Wording, as implemented in `core::quality`** (the vectors are the contract): titles "File", "LMS", "Lyrion gateway", "This device", "Download", a remote player's name or "Player"; file notes "Hi-Res lossless", "Lossless", "Lossy", "Format not reported"; LMS notes "Converted from ALAC", "Lowered to 16-bit, 44.1 kHz", "ReplayGain −6.5 dB", joined with ", "; gateway notes "Passed through unchanged" or "Re-encoded to Opus 160 kb/s (High, on mobile data)"; reasons "{Level}, on Wi-Fi" / "on mobile data" / "chosen in Settings" / "overridden here" / "chosen automatically"; device note the output's name then "bit-perfect", "resampled to 48 kHz", "processed by DSP you turned on"; player block "Plays the stream from LMS"; download block "Saved on this device". A gain under 0.05 dB counts as none; WAV or AIFF arriving as PCM is not a conversion; names are cleaned of control, zero-width and bidi characters and cut to 40 characters.

## What the gateway tells the client, and the protocol version

_Decided 2026-10-25: no backwards compatibility for the player protocol. Both sides move to **protocol 2** together (plugin 0.7.3, Android 1.17.0, the web app that ships inside the plugin), and the release notes say so. Everything that does not need the new protocol (sign-in, browsing, remote control over `/v1/rpc` and `/v1/events`) is unchanged, so a mismatched pair still controls other players; only the device's own player needs both sides current._

**The stream message.** The engine sees only the stream it decodes; the LMS and gateway blocks need what LMS served and what the gateway did. The gateway sends a text message on the player socket for every stream it relays:

```json
{"stream":{"seq":12,
  "served":{"codec":"flac","rate":96000,"bits":24,"channels":2,"gain_db":-3.2},
  "sent":{"mode":"opus","kbps":160,"level":"high"}}}
```

- `seq` is the count of `strm s` frames the gateway has forwarded in this session, so the client ties the message to the stream it is fetching; both sides count the same frames (replayed frames after a resume are not counted twice, as the client never processes a frame twice).
- `served` is LMS's stream as it reached the gateway: the codec from the `strm` format byte (`f` FLAC, `p` PCM with rate, depth and channels from the `strm` fields, `m` MP3, `o` Ogg, `a` AAC, `l` ALAC, `u` Opus), the rate, depth and channels of a FLAC stream from its STREAMINFO (the gateway reads the first 42 bytes before passing them on or to `ffmpeg`), and the server ReplayGain from the `strm` (`gain_db`, absent when none). Unknown fields are `null`.
- `sent` is what the gateway did: `{"mode":"passthrough"}` or `{"mode":"opus","kbps":160,"level":"high"|"low"}`.
- It is sent when the gateway has the upstream response (so it knows the codec and can read STREAMINFO), which is after the client asked for the stream; the badge and sheet update when it arrives, within the first second. After a resume greeting the gateway sends it again for the stream in progress, so a resumed client is never without it.
- It is not a frame (not binary) and is never counted in `received`, like the text keepalive.
- After the review of 2026-10-25: the gateway re-sends the last 4 messages of the session after a resume greeting, and answers a `{"quality":…}` control message with a `{"quality":<level>}` echo once applied, so a client seeks for an override only after the echo.

**Protocol version.**
- `GET /v1/me` answers `"protocol": 2` (the gateway's).
- The player socket's sign-in form carries `"protocol": 2`. A form without it, or with a lower number, is an old client: the gateway closes with **4426** and the reason `update the app` after checking the token and before opening a session, and audits `player.too_old` (device and the number given). A higher number than the gateway knows is accepted (the newer client decides what to do).
- **Old app, new server:** the Android app shows, where the player would appear, "This server needs Lyrion Anywhere 1.17.0 or later. Update the app." (the close reason is mapped, never shown raw). Remote control keeps working. A stale web page (an old cached app) gets the same close; its error says to reload, and the boot guard and content ETags already make the next load fresh.
- **New app, old server:** the app reads `protocol` from `/v1/me` at start-up and sign-in; below 2 (or absent) it does not open the player socket and says "This phone can't play from this server until its Lyrion Anywhere plugin is updated to 0.7.3 or later. You can still control other players." Settings › About shows the server's protocol and version.
- **Admins:** the devices table records each device's protocol at attach; *Your devices* and the plugin's Devices list show "needs updating" beside an old one, so an admin can see who is behind after upgrading.
- PROTOCOL.md gains a *Versions* section: protocol 1 (0.1 to 0.7.2 / Android to 1.16.0), protocol 2 (0.7.3 / 1.17.0: the stream message, the version in the sign-in form, close 4426).

**Why this and not headers or inference.** Response headers would be readable only by code that performs the fetch and would be a second channel beside the socket; inference from the decoder cannot see LMS's own conversion under a re-encode. A first-class message on the socket, with a version number that lets each side say plainly that the other is too old, is the proper design, and the owner chose it over compatibility shims.

**Tests (protocol):** the message for a pass-through FLAC (STREAMINFO read: 24/96), an Opus re-encode at each level, a PCM stream (fields from the `strm`), an MP3 as sent, server gain present and absent; `seq` across a seek, a track change and a resume (re-sent after the greeting); a client without `protocol`, with 1, with junk (`"2"`, `2.0`, `-1`, `99999999999`, an object) and with 3; the audit line; `/v1/me` carries `protocol`. Robustness: junk STREAMINFO bytes never panic and leave `rate`/`bits` null; the message is at most 512 bytes.

## Clients

**Engines (mobile crate, web worker):** send `"protocol": 2` in the sign-in form; keep the latest `stream` message per `seq` and attach it to the matching `strm s` (`TrackAudio.served`, `TrackAudio.sent`); expose it in the signal-path snapshot. The web worker passes it to the page with the existing `source` status. Close 4426 is reported as "update needed", never retried.

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
- Gateway: the stream message and the protocol version (see above); the quality control message followed by a seek yields a stream at the new level (an integration test with the fake LMS and a transcode stub, as in the streaming-quality tests).
- Mobile crate: the stream message parsed and tied to its `strm` by `seq`, carried through a resume; the snapshot carries it; junk messages ignored; close 4426 mapped to the update message; `/v1/me` without `protocol` 2 keeps the player off with the server message.
- Android (unit): the badge uses the heard stream (the exact case reported: FLAC 24/96 on mobile data → "Transcoded · Opus 160", amber); the diagram's blocks for each vector; the LMS block appears only when LMS changed something; the switch sends the message and seeks; the override clears on a metered change and survives a track change; a remote player and offline show their shapes; TalkBack reads the blocks in order.
- Web (Vitest): the same cases on the components; the override's lifetime; Safari (no Network Information API) behaves.
- Live: on the owner's setup, the reported track on mobile data shows "Transcoded · Opus 160" and the diagram File → Gateway (amber) → Phone; switching to Original in the sheet restarts the stream at the same position with FLAC 24/96 and the badge turns Hi-Res; back to automatic returns to Opus on the next stream.

## Docs

SPEC §7.2 "Quality badge and signal path" rewritten, decision **D30**; FEATURES rows *Quality badge and signal path* and *Streaming quality* updated; PROTOCOL (the stream message, the version in the sign-in form, close 4426, `/v1/me` `protocol`, a *Versions* section); release notes for 0.7.3 and Android 1.17.0 open with "update both: the app needs the plugin 0.7.3 or later, and the plugin's player needs app 1.17.0 or later"; README's compatibility line; CHANGELOG; `streaming-quality.md` "Clients" section marked superseded by this plan.

## Out of scope

Changing the quality levels or bit rates (D25 stands); LMS's own transcoding rules (shown, not controlled); a per-track permanent setting; queue rows predicting a verdict.
