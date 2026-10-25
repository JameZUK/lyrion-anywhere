# Streaming quality on mobile data: agreed design

Agreed with the owner on 2026-10-12 (task #33). This covers the gateway, the Android app and the web app; iOS takes the same design when its app is built.

## Choices made

- **How:** the gateway re-encodes the live stream to **Opus**, with the `ffmpeg` it already uses for transcoded downloads.
  - Not LMS's own bitrate limit: that transcodes to MP3 through `lame`, which many servers don't have (the owner's doesn't).
  - Opus also sounds better than MP3 at the same rate.
- **Levels:**

  | Level | What is sent | About |
  |---|---|---|
  | **Original** | the stream as LMS sends it (lossless stays lossless) | ~400 MB/hour for CD-quality FLAC |
  | **High** | Opus 160 kbps | ~70 MB/hour |
  | **Data saver** | Opus 96 kbps | ~45 MB/hour |

- **Defaults:** Original on Wi-Fi (and other unmetered networks), **High on mobile data** (metered). Both can be changed in Settings.

## What is re-encoded

- **FLAC streams only** (`strm` format `f`), in this first version. LMS sends FLAC to our players for FLAC files, and also converts ALAC and other lossless formats to FLAC for them.
- **Everything else passes through as it is:** MP3, AAC, Ogg and Opus are already compact; WAV/AIFF arrive rarely.
- **Fallback to Original:** when the gateway has no `ffmpeg`, or is already running its limit of live encodes, the stream is sent as it is. The client always knows which it got: the `strm` it receives says Ogg Opus (`u`) or the original format.

## How it works

1. **The client says what it wants:**
   - when it connects, in the player socket's sign-in message: `{"token": …, "quality": "high"}`;
   - when that changes (for example, Wi-Fi to mobile data), with a control message: `{"quality": "low"}`. The change takes effect from the next stream: the next track, or a seek.
   - Clients send this only when `GET /v1/me` lists `"stream_quality": true` under `features`, because older gateways refuse unknown messages.
2. **Rewriting the stream command:** when LMS sends `strm s` with FLAC and the session wants High or Data saver, the gateway rewrites the format byte to Ogg Opus (`u`) before passing the frame on. The core probes the content, so it decodes it as Opus.
3. **The relay** puts `ffmpeg` between LMS and the client:
   - LMS's bytes go to `ffmpeg`'s input, and its Ogg Opus output is what the relay counts, keeps for replay and serves.
   - `?offset=`, acknowledgements and resume all work on the Opus stream unchanged, because LMS is asked for a stream only once and its output is produced once.
   - **Back-pressure** still reaches LMS: a client that stops reading stops the relay reading `ffmpeg`, which stops `ffmpeg` reading LMS.
4. **Status reports (STAT):** LMS hears the byte count it sent, worked out in proportion from what the client has received (as for FLAC artwork stripping); once the client has everything, LMS hears the full count. Elapsed time comes from the decoded audio, so sync and position are unaffected.
5. **`ffmpeg` is run as hardened as for downloads:**
   - a fixed argument list, no shell, an empty environment;
   - the input demuxer is forced (`-f flac`), and only the `pipe` protocol is allowed;
   - output is Ogg Opus at 48 kHz, stereo;
   - the process is killed when the stream ends;
   - at most N live encodes run at once (`--live-transcode-jobs`, default 4).

## Clients

_The display parts of this section (the badge and the signal path) are superseded by `signal-path-redesign.md` (2026-10-25): the badge states the weakest link then the stream heard, the sheet is a block diagram, and the facts come from the gateway's `stream` message (protocol 2). The settings and the automatic choice below still stand._

- **Android:**
  - Settings › Playback › *Streaming quality*: separate choices for *On Wi-Fi* and *On mobile data*;
  - whether the network is metered comes from `ConnectivityManager` (`NET_CAPABILITY_NOT_METERED`) and follows changes;
  - the signal path shows "Opus 160 (High, on mobile data)".
- **Web:**
  - Settings › *Streaming quality in this browser*: *Automatic*, Original, High or Data saver;
  - *Automatic* uses the Network Information API where the browser has it: mobile connection → High, *Save data* → Data saver, otherwise Original;
  - browsers without that API (Firefox, and Safari, so iPhone and iPad) use Original unless a level is chosen, and the setting's note says so there ("Always Original in this browser, which can't tell mobile data from Wi-Fi…");
  - the quality badge and the signal path name the stream actually heard, from the engine's decoded format rather than the library track's: the badge says "Opus 160", the signal path has a *Stream to this device* row ("Opus 160 kb/s, re-encoded by the gateway for the High setting", or "FLAC 24-bit, as stored (not re-encoded)") and a verdict *Re-encoded to Opus 160 kb/s*, and its rate row adds "(Opus is always 48 kHz)". Added 2026-10-18 after forum users on iPhones read the old panel's "96 kHz FLAC … 48 kHz … FLAC in MP4" as a conversion error;
  - Opus decoding is added to the browser player's WebAssembly core.

## Tests

- **Gateway:**
  - the quality message: grammar, limits and unknown values, with negative tests;
  - the `strm` rewrite: FLAC only, Original unchanged, fallback when there is no `ffmpeg` or no free slot;
  - the STAT conversion;
  - the relay through a real `ffmpeg` (when present): the output is Ogg Opus that the core decodes, resume with `?offset=` gives identical bytes, back-pressure holds upstream, and the process is killed when a session ends;
  - hostile input: a corrupt FLAC ends the stream as an error, never hangs, never panics.
- **Mobile and web:** the choice of level from the network type and settings, sending only when supported, and the setting UIs.
- **Live (opt-in):** stream one track at each level to the test player and check the bytes per second.
