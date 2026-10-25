# Under the bonnet

How Lyrion Anywhere is built, for the curious. The request flows, sequence diagrams and storage details are in [`ARCHITECTURE.md`](ARCHITECTURE.md); the wire protocol in [`PROTOCOL.md`](PROTOCOL.md); every design decision in [`../SPEC.md`](../SPEC.md).

## One player core, no I/O of its own

The player is one Rust library: the Slimproto client that makes a device look like a Squeezebox to Lyrion, the player state machine, the decoders, the DSP and the track analysis. It does no networking and no audio output itself. The same core runs in the browser as WebAssembly inside a worker, and in the Android app as the native engine; each host only moves bytes in from the network and samples out to the speaker. One implementation, tested once, is what keeps the web app and the Android app behaving the same.

## Its own decoders

FLAC, ALAC, MP3, AAC-LC, Vorbis and WAV/AIFF are decoded through Symphonia, and Opus through a pure-Rust decoder with no `unsafe` code and no C, patched here to decode at about 170 times real time on one desktop core. Every decoder is fed random, truncated and oversize input in tests and must never panic: a corrupt file or a hostile stream can only ever stop a track, not the app.

## DSP that steps aside

On Android, loudness levelling (ReplayGain, or the server's EBU R128 analysis), parametric EQ with AutoEq profiles, loudness-compensated volume, crossfeed, a four-times oversampled true-peak limiter, dither and transitions all work on 64-bit samples. Everything is off by default, and a bypassed stage leaves the samples untouched, so with nothing turned on the chain is bit-transparent, which tests prove at every common rate and depth. The output then opens at the track's own rate, bit-perfect to a USB DAC on Android 14 and later where the phone and the DAC offer it.

## What you hear, shown honestly

The quality badge and the signal path are drawn from one shared model in the core, held to the same set of test vectors on the web, in the Rust engine and in the Android screens, so the two apps cannot drift apart. The badge names the weakest link first (Hi-Res, Lossless, Transcoded or Lossy) and then the stream actually heard. The path is a chain of blocks, file to server to gateway to device, and the block that lowered the quality is the one that is tinted. The gateway tells the player what Lyrion served and what it did to the stream, for every stream, so the diagram is fact rather than inference. The gateway also analyses each track once (a thousand-point waveform, R128 loudness, peaks, silences and fades) for the waveform seek bar and for levelling.

## One small gateway

A single Rust process behind your reverse proxy, tunnel or VPN, installed, started and supervised by the Lyrion plugin, and at rest it is small: one measurement found it using 10.8 MB after two hours of serving a household. It keeps its own user accounts rather than Lyrion's: argon2id password hashes, optional time-based two-step codes, fifteen-minute access tokens and rotating refresh tokens, and a device list you can see and revoke from. Everything a device asks of Lyrion passes through an allow-list of commands, and every answer is scrubbed of file paths and home-network addresses before it leaves.

## A player link that survives the network

Each device's Squeezebox-style connection to Lyrion is carried over a secure WebSocket. When that drops, the gateway holds the player's place in Lyrion, for up to ten minutes on Android, while the phone plays on from what it has already read ahead; when the link returns the connection is resumed where it was and the player rejoins its group in step, without pausing the rest. On Android a rolling cache (a disk spool, read-ahead of the next track, a replay cache, and downloaded albums standing in for their streams) rides out tunnels and dead spots.

## Both sides say their version

The player protocol carries a version number from both sides. A change to it means the plugin and the apps move together: an app that is too old for a server is told to update in plain words, and an app that is newer than its server says which plugin version it needs, while remote control of the other players keeps working. Doing it properly is preferred to carrying compatibility shims; the release notes say what to update.

## Measured, not guessed

Performance budgets for the Android app, and the measurements behind the claims above, are in [`PERFORMANCE.md`](PERFORMANCE.md). Security reviews and penetration tests are in [`security/`](security).
