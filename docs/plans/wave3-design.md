# Wave 3 design: signature features

Started on 2026-10-13, from `docs/plans/ux-research.md` §6 (Wave 3). The aim is features no other LMS app offers. Web and Android together; iOS takes the same design when its app is built. Each part is detailed before it starts.

## Order

Agreed with the owner on 2026-10-13:

1. **Listening stats and "Your year"** (D7). Built on the per-user listening history the gateway already keeps.
2. **Rolling cache** (H3), for the Android player on patchy mobile data. It starts as an investigation, written up here before it is built.

**Later**, by the owner's choice:
- **Smart filters v2** (X7): "any" rules, track-level rules and download rules. Picked up if a need for track-level rules appears; v1 covers albums by genre, decade, artist and compilation.
- **Smart Shuffle and DJ styles** (X3): largely covered by Start radio, More like this and stations.
- **Guest queue ("Jam")** (D8): it needs a threat model first, as it is a new kind of access.
- **Album booklets** (X5 part 2): it needs a security design for resolving an album's folder without exposing paths.
- **Timed lyrics from LRCLIB** (see `ux-research.md`, Deferred).

## 1. Listening stats and "Your year" (part 1)

### What the user sees

- **A "Your listening" page:**
  - **Opening it:** from a card at the end of Home ("This month: 23 h, 412 plays") and from Settings › Listening history.
  - **Period:** a picker for this month, last 30 days, this year, a past year (where history reaches) or everything kept.
  - **Totals:** hours listened, plays, and how many different tracks, albums and artists.
  - **Most played:** artists, albums and tracks (top 10, "Show 20"), each tappable to its page, with plays and hours.
  - **New this period:** artists first played in it.
  - **When you listen:** a 24-hour chart, a weekday chart and, for a year, a month-by-month chart.
  - **Streaks:** the longest run of days with music, and the busiest day.
- **"Your year in sound":**
  - **The story:** swipeable full-screen cards: total hours, top artist, top album, top track, top five artists, a new discovery, your busiest hour, your longest streak, and a closing summary.
  - **When:** offered from 1 December to 31 January for the year, and on demand for any year with history.
  - **Sharing:** "Share" makes an image of the current card on the device itself (a canvas on the web; a bitmap on Android). Nothing is uploaded; the platform's share sheet takes it from there.
- **Empty and switched-off states:** with history switched off, the page says so and links to the setting. With too little history, it says how long until there is something to show.
- **Accessibility:** charts have text alternatives (a table or a summary sentence). The story cards are a carousel with buttons, and have no autoplay.

### The contract (gateway, web and Android build to it)

`GET /v1/me/stats?period=<p>&tz=<minutes>`:

- **`period`:**
  - `days-N`: the last N days, N from 7 to 400 (default `days-30`);
  - `month-YYYY-MM`;
  - `year-YYYY`;
  - `all`: everything the history keeps.
- **`tz`:** minutes east of UTC, from −840 to 840 (default 0), for the hour, day and streak buckets. Clients send the device's offset.
- **Any other parameter, or a malformed value:** 400.
- **Answer:**

```json
{
  "enabled": true,
  "retention_days": 396,
  "period": {"from": 1790000000, "to": 1792600000},
  "totals": {"plays": 412, "seconds": 82800, "tracks": 301, "albums": 64, "artists": 41, "days": 27},
  "top_artists": [{"id": 12, "name": "An Example Band", "plays": 40, "seconds": 9100}],
  "top_albums": [{"id": 7, "plays": 22, "seconds": 5200}],
  "top_tracks": [{"id": 9, "plays": 6, "seconds": 1300}],
  "new_artists": [{"id": 31, "name": "Another Example", "first_played": 1791000000}],
  "by_hour": [0, 0, 1, …],
  "by_weekday": [12, 30, …],
  "by_month": [{"month": "2026-10", "plays": 412, "seconds": 82800}],
  "longest_streak": {"days": 9, "from": "2026-10-02", "to": "2026-10-10"},
  "busiest_day": {"date": "2026-10-04", "seconds": 14400}
}
```

- **Ids and lists:**
  - **Albums and tracks by id:** clients load their names and art by id, as for shelves.
  - **Artists come with their names:** LMS can look an artist up by one id only (`artists … artist_id:<id>`), not a list. So the gateway adds `name`, looked up per artist, cleaned like any LMS text, and cached. Library ids can change after a rescan, so the history's stable track key is used for counting and the current id is reported.
  - **List lengths:** at most 20 in each top list and in `new_artists`.
  - **`by_hour` and `by_weekday`:** 24 and 7 entries. The weekday list starts on Monday.
  - **`by_month`:** only for `year-*`, `all`, or `days-N` with N > 60.
  - **`longest_streak` and `busiest_day`:** `null` with no plays.
- **History off:** `enabled: false`, with zeros and empty lists.
- **New artists:** an artist's first play in the history falls inside the period. The history only reaches back `retention_days`, so very old favourites can look new; clients say "first played since …" for `all` and long periods.
- **Limits:**
  - It counts under the `/v1/me` allowance.
  - It reads at most 200 000 history rows per answer (the newest first).
  - An answer is cached per user, period and tz for 5 minutes, and dropped when the history is cleared or switched off.
- **Privacy:** a user's own history only, never anyone else's. No new data is kept: it reads the history that already exists, with its retention, opt-out, clear and export. `docs/PRIVACY.md` says the stats page exists.

### Rules

- **Tests:**
  - every period and tz edge, including DST-free maths on fixed offsets, the 400s, and other users' data never counted;
  - fuzz-style tests for the period parser;
  - integration tests with a fake history;
  - UI tests for the page, the charts' text alternatives, the story cards and image sharing (on the web, the canvas export is checked with a fake canvas);
  - the accessibility audit on Android.
- **Language and fixtures:** British English; invented names in fixtures.

## 2. Rolling cache (part 2)

**Status:** design agreed on 2026-10-13 (see the decisions at the end). Spike S1 done; Phase A next.

Investigation written up on 2026-10-13, for the Android player (D14: the phone is a real LMS player). The web player is unchanged. iOS takes the same design when its app is built.

### The goal

When the phone is the player and streams over mobile data:
- **Short gaps don't stop the music:** tunnels, lifts and trains, from a few seconds to a few minutes.
- **Replays don't re-download:** going back to a recent track, or replaying one, doesn't fetch it again.

LMS stays in charge of the queue, sync, seeking, play counts and scrobbling throughout.

### What happens today (from the code)

**How a track arrives:**
1. LMS sends `strm s` over the player socket (`gateway/src/tunnel.rs`, `session.rs`).
2. The gateway checks the request in `Session::note_strm`. The request is always `GET /stream.mp3?player=<mac>`, so it never names a track. The gateway may rewrite the format byte to Ogg Opus (`live.rs`).
3. The core raises `Action::OpenStream`. `Engine::open_stream` then fetches `/v1/stream/<session>/<n>` (`stream_task` in `mobile/src/player/engine.rs`).
4. The gateway relays LMS's stream (`relay.rs`). It reads at most one replay window (4 MiB) beyond the client's last ack, and the client acks every 256 KiB (`ACK_EVERY`).
5. The gateway keeps one relay per session. A new `strm s` cancels the previous relay.

**How much is held ahead:**

| Where | How much |
|---|---|
| Core input (compressed) | Up to 8 MiB (`INPUT_HIGH_WATER`). The engine stops reading when `wants_stream_data()` is false. |
| Decoded PCM | 10 s (`OUTPUT_TARGET_SECS`) |
| Ring and output | About 0.4 s (`RING_AHEAD`) |
| Gateway | At most 4 MiB in flight; nothing held for the client beyond that |

In listening time, 8 MiB is:
- about 7 minutes at High (Opus 160), so usually the whole track, and it arrives in seconds;
- about 60–90 s for CD-quality FLAC (Original);
- about 20 s for 24/96 FLAC.

**At a track boundary:**
- `STMd` is sent when the decoder finishes (`Player::decode_some`). Because the core decodes only 10 s ahead, that is about 10 s before the end of the track.
- LMS answers `STMd` with the next `strm s`.
- So the next track's bytes can't arrive until roughly the last 10 s of the current one. Nothing earlier lets the phone buffer it.

**When the network drops (walk-through):**
- **Player socket:**
  - The client treats 25 s without anything arriving as a dead socket (`SOCKET_IDLE`) and resumes.
  - After 8 s of silence the gateway starts heartbeating LMS. It replays the last STAT, with elapsed time advanced by wall time (`Session::synthesised_stat`).
  - The client's STATs wait in the outbox (`OUTBOX_LIMIT` 2000) and are sent on resume.
- **Stream fetch:**
  - It retries up to 12 times (`STREAM_RETRIES`), with 0.5 s to 5 s backoff plus a 10 s connect timeout per try.
  - That is roughly 50 s to 3 min. After that, `StreamFailed` → `on_stream_error` → `DSCO(1)`, and the track is cut short at what was received.
  - At High this rarely matters, because the track is already downloaded. At Original it does.
- **Gap that spans a track boundary:**
  - `STMd` sits in the outbox, so LMS sends no `strm s`.
  - The phone plays out its 10 s, sends `STMo`, and goes silent until the socket resumes.
- **Gap of more than 55 s (`protocol::RESUME_FOR`; the gateway's `resume_grace` is 60 s and has no CLI flag):**
  - The client starts a new session, and `Engine::on_greeting` with `fresh` drops all queued audio.
  - LMS sees a reconnect and playback stops, whatever was buffered.
- **The gateway's listening history (`history.rs` `Tracker::on_stat`):**
  - It accepts an elapsed step only if the step is no bigger than the wall time since the last STAT, plus some slack.
  - The burst of STATs sent on resume therefore counts for almost nothing.
  - Meanwhile the heartbeats keep adding time to the old track, even past its end.

**What the player can learn about a stream:**
- **From the `strm` itself:** the stream number, the format byte, autostart, the threshold and the replay gain. It never says which track.
- **From LMS `status - 2 tags:xed`** (`protocol::stream_status_command` / `stream_track`, already used for levelling and transitions):
  - the current and next track ids;
  - it races with LMS and refuses anything doubtful.
- **From the stream bytes:**
  - a FLAC stream starts with `fLaC` + STREAMINFO, which includes the audio's MD5 and sample count;
  - pass-through and FLAC-stripped streams are the same bytes every time;
  - a stream after a seek starts somewhere else, so its first bytes differ.

### Options considered

**(a) Pre-fetch the next N queue tracks with `/v1/download` and play them from the file.** Not recommended.
- **No `strm` during a gap:** LMS can't send it, so the phone would have to move to the next track on its own. On resume it would then have to reconcile with LMS (`playlist index` + `time`, which makes LMS restart the stream from that point). That breaks sync groups and gapless playback, and puts LMS's position briefly out of step.
- **Guessing the queue:** shuffle, Don't Stop The Music, random mix and queue edits all change what comes next.
- **The download endpoint is the wrong tool:**
  - it needs the download permission;
  - an Opus download is transcoded in full before the first byte arrives;
  - it allows 3 at once per user;
  - each download writes an audit-log entry;
  - the data is fetched twice, because LMS still streams the track as well.

**(b) A bigger read-ahead of the live stream (memory or disk).**
- **Alone:** it only helps in the middle of a track, and at High that is already covered. It does nothing at the boundary.
- **Combined with reporting `STMd` once the stream is fully received:** the read-ahead then runs a whole track ahead, with LMS still choosing every track. This is the core of the recommendation.

**(c) Rely on the gateway's resume, with a bigger window and grace.**
- **The window:** making it bigger changes nothing, because the client already pulls ahead and the window only bounds bytes in flight.
- **The grace:** a longer grace is needed, so that a long buffer can outlast a gap longer than 55 s. It is part of the recommendation, but not enough on its own.

### Recommendation

**Phase A: whole-track read-ahead** (the owner's main goal):
- spool each live stream to disk as fast as the network allows;
- report `STMd` once a stream is fully received;
- negotiate a longer resume grace.

Worst-case cushion: about one track. At any moment it is the rest of the current track plus the whole next one, once its `strm` has arrived.

**Phase B: replay cache** (the "rolling cache" proper):
- keep finished spools in a bounded LRU cache;
- when a new stream starts, compare its first bytes with the cache and, on a match checked by hash, play the rest from the cache;
- a replay, a skip back, or LMS re-sending a track after a skip then costs about 64 KiB instead of the whole track.

**Phase C (optional): reuse downloaded FLAC originals** for streams at the Original level, matched on STREAMINFO.

### Phase A in detail

**Spike S1 first** (opt-in live test against the test player). Confirm on LMS 9.x:
- **Early `STMd`:** LMS accepts a `STMd` that comes long before the end of the track and sends the next `strm s` at once.
- **Waiting for the start:** LMS then waits for that track to start (`STMs`) before streaming the one after it (the "TRACKWAIT" behaviour I believe it has). If it doesn't, the phone limits itself to one track ahead.
- **Queue edits:** what LMS does when the already-streamed next track is deleted, moved, or the queue is shuffled.
- **`STMu` after a gap:** what a `STMu` arriving late after a gap does, compared with `STMo`.
- **Background:** squeezelite users run very large output buffers, which send `STMd` early in the same way, so LMS is expected to cope.

**S1 results (2026-10-13, LMS 9.x, test player; `tools/live/lms_read_ahead.py`):**
- **Early `STMd` works.** The next `strm s` came within about 40 ms. LMS then held the track after it until the next track's `STMs`, so the phone is never more than one track ahead. LMS kept showing the current track as playing throughout.
- **`STMc` and `STMh` matter.** LMS ignores `STMd` for a stream it never saw connect.
- **Queue edits:** deleting or moving the received-but-not-started next track brings `strm f` and then a new `strm s` for the new next track, while LMS still shows the current track playing. So **`strm f` must drop only what was fetched beyond the audible track**, and `strm q` stops everything. Today the core treats `f` like `q`; that already cuts the last few seconds when the next track is edited, and must change in A2.
- **`STMu` is harmful here.** With the next track received, `STMu` moved LMS to the next track and stopped it.
- **`STMo` is safe.** LMS kept playing and sent nothing; a later `STMs` moved it on and released the next stream. A4's rule is confirmed.

**A1. Disk spool for the live stream** (`mobile/src/player/engine.rs`, new `player/spool.rs`). Effort: M.
- **Writing:**
  - `stream_task` writes what it receives to an app-private spool file and acks the network bytes as today.
  - It no longer waits for `wants_stream_data()`. A separate feeder hands bytes from the spool to the core when the core wants them.
- **Counting:**
  - "received" (the resume `?offset=`, acks, STAT `bytes_received`) is counted separately from "fed to the decoder".
  - `DSCO` is sent when the network stream ends; the decoder sees end of input when the spool is fully fed.
- **Limits and fallback:**
  - a cap per stream (e.g. 256 MiB) and in total (e.g. 512 MiB);
  - over a cap, on a full disk, or with no cache directory, it falls back to today's back-pressure in memory.
- **Configuration:** `LocalPlayerConfig` gains `cache_dir: Option<String>`. Android passes a folder under `noBackupFilesDir`, scoped like downloads (`store::scope_name`). Tests and the web pass `None`.
- **Retry budget:** fetch retries become time-based and last until the session's grace runs out, instead of 12 tries. A gap no longer cuts a track short.

**A2. Ready once received, with a queue of tracks in the core** (`core/src/player.rs`). Effort: M–L.
- **Switch:** a new `Player::set_ready_when_received(bool)`, off by default (web and CLI unchanged).
- **When it is on:**
  - **`STMd`:** sent once per track, when the host says the whole stream is in (`stream_received`).
  - **Next `strm s`:** a `strm s` that arrives while the current track is still decoding becomes `next`; today's `start_stream` would overwrite the current track.
  - **Promotion:** `next` is promoted when the current track finishes decoding.
  - **Which track gets the bytes:** `open_stream()`, `wants_stream_data()` and STAT `bytes_received` refer to the track being fetched.
  - **Elapsed, `STMs`, segments, gapless:** unchanged, because they follow the audible track.
  - **`STMu`:** only when every track is drained and nothing is queued.
  - **`strm q`:** clears everything. **`strm f`:** drops the queued and fetching tracks, keeps the audible one (S1).
- **Not before 20 s:** don't report ready before 20 s of the current track have been heard (to be tuned). This saves data for people who skip fast.

**A3. Longer resume grace.** Effort: M.
- **Gateway:**
  - the sign-in message accepts `"grace": <seconds>` (an integer, 0–3600);
  - the gateway grants `min(asked, --max-resume-grace)` (new flag, default 600 s) and returns `"grace"` in the greeting;
  - `/v1/me` lists `"resume_grace": true` under `features`;
  - an unknown or malformed value is a policy violation (1008), with negative tests;
  - the detached frame log (1 MiB) is enough for 10 minutes of `strm t` polls.
- **Client:**
  - `RESUME_FOR` becomes the granted grace minus 5 s; with an older gateway it stays at 55 s;
  - the outbox drops `STMt` before event STATs, so `STMs`/`STMd`/`STMo` survive.
- **Battery:** the app passes network changes in (`Cmd::Network`, from `ConnectivityManager`). With no network, the engine stops trying to reconnect and tries again as soon as the network returns.

**A4. Keeping LMS and the history right.** Effort: S.
- **No `STMu` with the link down:** if the phone runs out of audio while disconnected, the core sends `STMo` instead of `STMu` (`set_link_up(false)` from the host). On resume LMS then sends the next `strm` rather than treating playback as finished. To be confirmed in S1.
- **History plausibility** (`gateway/src/history.rs`): elapsed steps are checked against the STAT's own jiffies, not the time it arrived. Plays heard during a gap then count when the burst arrives.
- **Heartbeats:** while detached, the synthesised elapsed time never passes the resolved track's duration.
- **Sync groups:** LMS releases the next track to a group only when every member is ready, so a grouped phone gets no extra cushion but nothing breaks.

**A5. Android.** Effort: S–M.
- **Settings › Playback › *Gaps in the signal*:**
  - "Read ahead the next track": on, with a separate switch for mobile data, also on by default;
  - the signal path and the player sheet show the cushion ("4 min buffered ahead"), estimated from spooled bytes and the decoded byte rate.

### Phase B: replay cache

**Spike S3:** add `-fflags +bitexact -flags:a +bitexact` to the live Opus arguments in `gateway/src/live.rs`. Today ffmpeg's Ogg muxer picks a random stream serial, so two encodes of one track differ. Check that two encodes of the same FLAC give the same bytes. Effort: S.

**B1. The cache store** (`mobile/src/player/cache.rs`). Effort: M.
- **Files:** `<root>/<scope>/stream-cache/<64 hex>.bin` and `index.json`.
- **Each entry:** the fingerprint, the level, the format byte, the length, the SHA-256 of the whole stream, last used, and (optionally) the track id from `stream_track`, for labelling only.
- **Index file:** checked as strictly as the offline index (bounded, malformed entries dropped, atomic writes).
- **What is kept:** only spools that ended cleanly; partial ones are deleted.
- **Eviction:**
  - least recently used, under a cap (Off / 500 MB / 1 GB / 2 GB / 5 GB; default 1 GB);
  - spools in use are never evicted;
  - the space counts separately from downloads.

**B2. Matching** (in `stream_task`). Effort: M.
- **Fingerprint:** once the first 64 KiB (or the whole stream, if shorter) has arrived, take SHA-256(level, format byte, those bytes).
- **On a hit:**
  - check the entry's length and full hash;
  - stop the HTTP fetch and feed the rest from the file;
  - `bytes_received` keeps counting the whole stream, and the next `strm` cancels the gateway's half-read relay.
- **On a miss, or any doubt:** carry on from the network.
- **Seeks:** they start mid-stream, so they never match.
- **Signal path:** "from cache".
- **Spike S4:** confirm that a STAT `bytes_received` beyond what the relay sent is harmless, and that `upstream_bytes` clamps it.

**Phase C (optional, M): reuse downloaded FLAC originals.**
- **When:** a FLAC stream at the Original level whose STREAMINFO (MD5 and samples) equals a downloaded original's.
- **How:** splice at the first audio frame (stream offset = the file's header length + bytes received − the stripped header length). The frames are identical, so the decoded audio is bit-identical.
- **Not for Opus:** Opus downloads aren't byte-identical to live Opus, so they aren't reused.

### Mobile data, battery and storage

- **Extra data:** only next tracks fetched and then skipped (High: about 4–6 MB each; Original: about 30 MB). The 20 s wait reduces this, and the mobile-data switch turns it off. The replay cache costs no extra data.
- **Battery:** better or the same.
  - Downloading a track in one burst lets the modem sleep; a steady trickle doesn't.
  - Disk writes are a few MB per track.
  - Hashing a 40 MB entry is well under a second.
  - Decoding is unchanged (still 10 s ahead).
  - No reconnect attempts while Android reports no network.
- **Format:** the cache holds whatever was streamed, at the session's level. A track cached on Wi-Fi at Original won't serve a High stream on mobile data.

### Security

- **App-private storage:** files live under `noBackupFilesDir` in a per-server, per-user hashed scope.
  - Names are hex fingerprints with a fixed extension.
  - No titles, paths or tokens are stored.
  - The root must be absolute with no `..` (`store::scope_dir`'s check).
  - Symlinks and non-files are refused.
- **Nothing exposed:** UniFFI shows only totals (bytes, count), a "from cache" flag and "Clear cache", never paths.
- **Untrusted on reading:** the index is bounded and validated, every hit is hash-checked before use, and the decoder is already fuzzed.
- **Deleted** on sign-out, on revocation (at the next connection, as downloads are) and on a server change.
- **Gateway:** no new endpoint. The grace request is bounded and capped; a longer grace holds one LMS connection per device and still counts against the existing limits.

### Failure modes

| Case | What happens |
|---|---|
| Disk full or a cap reached | Back-pressure in memory, as today; the cache evicts first |
| Cache entry corrupt or tampered | Hash mismatch: the entry is deleted and the stream comes from the network |
| Gap longer than the buffer | `STMo` and silence; on resume LMS sends the next `strm` and playback continues |
| Gap longer than the grace | New session, as today; playback stops (see question 3) |
| Token expired during the gap | Renewed on reconnect (existing) |
| Device revoked | 1008 stop; the cache is deleted |
| LMS restarted | 1001, new session |
| Skip or seek during the read-ahead | Flush; a complete spool moves to the cache, a partial one is deleted |
| Queue edited after the next track was streamed | LMS's own handling (spike S1) |
| App killed in a tunnel | As today |

### Tests

**Unit:**
- **Core track queue:**
  - one `STMd` per track;
  - the next track queued while the current one decodes;
  - flush clears everything;
  - `STMu` only when everything is drained;
  - `STMo` instead of `STMu` with the link down;
  - offsets and STAT bytes for the track being fetched;
  - with the switch off, byte-for-byte today's behaviour (the existing suite);
  - random event sequences never panic.
- **Spool:** caps, a full disk, truncated files.
- **Cache index:** parser robustness and fuzzing, bounds, LRU eviction, entries in use never evicted.
- **Fingerprints:** same, different, short and post-seek streams.
- **Grace:** request grammar and negative tests on the gateway; the client's resume window following the grant.
- **History:** plausibility from jiffies.
- **Live Opus:** deterministic output when `ffmpeg` is present.

**Integration** (`mobile/tests/player.rs`: FakeLms + the in-process gateway + `Proxy`):
- `STMd` comes early and track 2 is fully fetched long before track 1 ends;
- the proxy cut for 90 s across a boundary (grace 120 s): the audio stays continuous and bit-identical, there is no new HELO, and LMS gets `STMs` for track 2 after resume;
- a gap longer than the buffer: `STMo`, then recovery;
- a gap longer than the grace: a new session;
- a replay: at most 64 KiB is served by `/stream.mp3`, the output is bit-identical, and the signal path says "from cache";
- a tampered entry falls back to the network;
- an old gateway: 55 s;
- sign-out and revocation clear the cache.

**Live (opt-in):** spikes S1 to S4 kept as regression tests against the test player.

**Manual (Android, real SIM):**
- **Airplane mode:**
  - for 20 s, 90 s and 6 min;
  - mid-track and 5 s before a boundary;
  - at High and at Original.
  - Check that the music carries on, the LMS web UI shows the right track after reconnecting, and history and play counts are right.
- **Real journeys:** an Underground ride, a lift, a motorway tunnel.
- **Wi-Fi ↔ mobile handover during a gap.**
- **Network tests:** `tc netem` loss and outage on a test router.
- **Data use:** skip back and check the app's data use in Android settings.
- **Sync group:** with a squeezelite player, no regression.
- **Cache:** fill it past the cap and check eviction.
- **Battery:** an hour with read-ahead on and off.

### Effort

| Piece | Effort |
|---|---|
| Spikes S1–S4 | S–M (2–3 days) |
| A1 disk spool and retry budget | M |
| A2 core track queue, ready once received | M–L |
| A3 grace negotiation, outbox priority, network-aware reconnect | M |
| A4 `STMo`/`STMu` rule, history jiffies, heartbeat cap | S |
| A5 Android settings and cushion indicator | S–M |
| B1 cache store | M |
| B2 matching and switch-over, B3 bitexact Opus | M + S |
| C reuse of downloaded FLAC | M (optional) |

Phase A comes to about 3 weeks, Phase B about 1.5–2 weeks and Phase C about 1 week, tests included.


### Open questions for the owner

**Decided by the owner on 2026-10-13:**
1. **Download permission:** yes. The replay cache needs it. Without it, only the current and next track are spooled, and they are deleted after playing.
2. **Mobile data:** read-ahead is on by default on mobile data, at every level including Original, with the 20 s wait.
3. **Grace and recovery:** the gateway's maximum is 10 min. After a gap longer than the grace, the app restarts the same track at the same position on the new session.
4. **Cache size and label:** 1 GB by default. "From cache" shows in the signal path.
5. **Recently played offline:** no.
6. **Phase C:** yes, built after Phase B.

The questions as asked:

1. **Download permission and the replay cache:** should the cache (Phase B) need the download permission? An admin who turned downloads off may not expect copies kept on the device. My suggestion is yes. Without the permission, only the current and next track are spooled, and they are deleted after playing.
2. **Data on mobile data:** is reading ahead acceptable on mobile data by default (about 5 MB wasted per early skip at High), and is the 20 s wait before fetching the next track right? At Original on mobile data, should read-ahead stay on?
3. **Grace and recovery:** what maximum grace on the gateway (my suggestion is 10 min)? While the phone is away, LMS shows it as "playing" for that long. And after a gap longer than the grace, should the app restart playback itself (same track, same position) on the new session?
4. **Default cache size and the "from cache" label:** what default size (my suggestion is 1 GB)? Should "from cache" show only in the signal path, or also as a badge on the player?
5. **Recently played offline:** should cached tracks be playable in offline mode as "Recently played"? That means keeping titles and artwork with each entry, a bigger privacy and storage footprint. My suggestion is no for now.
6. **Phase C:** is reusing downloaded FLAC originals worth about a week now, or later?
