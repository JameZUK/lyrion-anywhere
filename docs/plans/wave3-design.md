# Wave 3 design: signature features

Started on 2026-10-13, from `docs/plans/ux-research.md` §6 (Wave 3). The aim is features no other LMS app offers. Web and Android together; iOS takes the same design when its app is built. Each part is detailed before it starts.

## Order

Agreed with the owner on 2026-10-13:

1. **Listening stats and "Your year"** (D7). Built on the per-user listening history the gateway already keeps.
2. **Rolling cache** (H3), for the Android player on patchy mobile data. It starts as an investigation, written up here before it is built.

3. **Wave 3.5: declutter** (agreed on 2026-10-14; plan in `wave3_5-declutter.md`). The waves added features faster than the screens absorbed them. A review of every screen on Android and the web (rendered, at phone, tablet and desktop sizes) finds what crowds each one, and an agreed plan removes, merges or moves it, keeping both apps consistent. No new features.

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

**Status:** design agreed on 2026-10-13 (see the decisions at the end). Spike S1 done; Phase A built on 2026-10-13. What remains for Phase A is the manual testing on a real phone (see "Manual" under Tests). The live check of A6's rejoin is built (`mobile/tests/live_sync.rs`): the phone rejoins in step. LMS paused the rest of the group meanwhile, and a long stall moved the whole group back; both fixed on 2026-10-15 (see "A6 live check"), and the live check now asserts both, but has yet to be run against the fix.
- **Gateway** (A3, A4): grace negotiation, `--max-resume-grace`, history from jiffies, heartbeats capped at the track's end. Built.
- **Core** (A2, A4, A6 in the core): built (see "Core built" under A2).
- **Mobile engine** (A1, the client's A3, A6, `strm f`): built on 2026-10-13 (see "Mobile engine built" under A1).
- **Android** (A5) and the restart after a gap longer than the grace (decision 3): built on 2026-10-13 (see "Android built" under A5).
- **Phase B** (S3, S4, B1, B2, the replay cache on Android): built on 2026-10-13 (see "Phase B built" under Phase B). What remains is the manual check on a real phone ("Cache" under Manual).
- **Phase C** (downloaded FLAC originals as a source): built on 2026-10-13 (see "Phase C built" under Phase C). What remains is the manual check on a real phone ("Downloads" under Manual).

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
- **Sync groups** (a second test player, removed afterwards):
  - LMS sends the next stream to a group only once every member has sent `STMd`, so a grouped phone gets no extra cushion from reading ahead. Sync itself is unaffected: starts are still timed by LMS, and decoding is still 10 s ahead.
  - One member's `STMo` leaves the group playing and sends the others nothing. (Here the stream was already received in full. While LMS is still streaming the track, a member's `STMo` makes it pause and rebuffer the whole group: see "A6 live check".)
  - **LMS does not pull a member that fell behind back into line** (no `strm p`, `a` or `u` in 15 s with a member 8 s behind). Within a group, later tracks start gaplessly on each player, so the offset would last.
  - **`STMs` after `STMo` for the same track moves the whole group on a track.** `STMs` means a new track started; resuming must send `STMr`.

**A6. Rejoining a group after a gap** (from S1). Effort: S.
- The engine knows how long the output was stalled. If the player is in a sync group (`sync ?` on resume) and the stall exceeded about 50 ms, it drops that much audio from the buffer (`skip_ahead`) on resume, so it is back in step with the others; LMS's usual small corrections do the rest.
- If the buffer holds less than the skip, it flushes and lets LMS restart it at the group's position.
- Alone, it carries on from where it stopped, as now.
- After `STMo`, recovery sends `STMr`, never a second `STMs` for the same track.
- Tests: the stall measure and skip in the core, and the sync scenario of `tools/live/lms_read_ahead.py` extended to check that a rejoining member is within 100 ms of the group.

**A6 live check (2026-10-13, LMS 9.1, `mobile/tests/live_sync.rs`).**
- **The check:**
  - **Set-up:** the real engine, behind an in-process gateway and a proxy that can be taken down, is the test player. A fake Slimproto player is the second member of the group; it follows LMS's timing exactly, so its elapsed time is the group's position.
  - **Streams in memory** (no spool), so a stall needs a gap of about 1.5 minutes: 10 s decoded plus up to 8 MiB undecoded.
  - **The gap:** the network goes (`set_network_available(false)` and the proxy down) until the engine has run dry. It stays down 3 s more (short) or 14 s more (long), then comes back.
  - **The measure:** the engine's elapsed less the other member's, the median of 40 paired samples over 2 s, before the gap and 5 s after the rejoin.
  - **Running it:** `ANYWHERE_LIVE=1 cargo test -p anywhere-mobile --test live_sync -- --nocapture` with `dev/local.env`; it takes about 4 minutes. Without both it passes without touching anything.
- **Offsets measured** (three runs). The engine's figure comes from its last 20 ms tick, so it reads about 10 ms early on average; the 100 ms bound allows for that.

  | Stall | Before the gap | After the rejoin | Stalled, until playing again |
  |---|---|---|---|
  | Short (skipped in place) | −4 to −11 ms | −1 to −9 ms | 6.8 s |
  | Long (`NotEnoughBuffered`, `time`) | −8 to −12 ms | −8 ms | 16.5 s |

- **What the other members hear: the open problem.** The engine is in step afterwards, but S1's "a member's `STMo` leaves the group playing" holds only once the track has been streamed in full.
  - **LMS rebuffers the whole group.** While LMS is still streaming the track (`STREAMING` or `STREAMOUT`), a member's `STMo` runs LMS's `_Rebuffer`:
    - every member is paused (`strm p`);
    - LMS waits until each reports its buffer full again (30 s at most, then it restarts at the song position);
    - it resumes all of them with a timed `strm u`, which holds the others back until the stalled member's reported position.
  - **The `STMo` arrives late.** It is sent while the link is down, so it reaches LMS on reconnect, after the others have played on through the gap.
  - **Short stall:** the others paused for 3.7 to 4 s at the reconnect, then got a skip correction of 0.15 to 0.55 s. Both the engine's skip and LMS's hold-back happened, and LMS's correction settled the difference.
  - **Long stall:** the others paused for 14.7 s, then the engine's `time ?` / `time <t>` restarted the group 13.7 to 13.9 s back, at the engine's own stall point.
    - **Why:** during a rebuffer, `time ?` gives the position LMS paused the master at. The engine was the master, so that was its own stale position.
    - **Result:** the group replays the stretch the phone missed. The design wanted the phone to jump forward to the group, not the group to go back.
  - **Decided and fixed (2026-10-15, approved by the owner):**
    - in a group, a mid-track `STMo` still queued when the link comes back is dropped (with the `STMr` for its recovery), since the engine handles the stall itself; LMS then never pauses the others for it. Alone, the end-of-track `STMo` and any underrun with the link up are unchanged;
    - the group restarts at its own position (the stall point plus the time stalled), checked against LMS's current track, rather than at the answer to `time ?`. Nothing is sent if LMS isn't playing, the track changed, or the point is at or past the track's end;
    - details in docs/PROTOCOL.md, "Rejoining after a gap". The live check asserts both (the others paused at most 1 s; a restart moves them at most 1.5 s).

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
- **Mobile engine built (2026-10-13)** (`mobile/src/player/spool.rs`, `engine.rs`):
  - **Spools:** `<cache_dir>/<scope>/spool/<n>.spool`, with the scope as for downloads. Each stream gets one when it opens. The engine keeps a spool for exactly the streams the core still takes input for (a new core call, `Player::fed_streams()`), so a flush, a `strm f` or a replaced stream deletes the others' files at once. The feeder follows the oldest; the fetch writes the newest.
  - **Fallback in memory:** with no cache folder, over a cap (256 MiB per stream, 512 MiB in all) or after a write error (a full disk), the rest of that stream stays in memory, and the fetch pauses while more than 256 KiB waits for the decoder: today's back-pressure. A spool file shorter than what was written (truncated or replaced) ends the track with what was fed. The root must be absolute, without `..`, and not a symlink; spool files are created exclusively and never opened through a symlink.
  - **Deleted:** when the track has been fed, on a flush, on stop, on sign-out or revocation (the socket's 1008), on a new session, and at start-up (every scope's spool folder, which covers a crash and a change of server or user). `remove_stream_spools(cache_dir)` does the same for the app when no player runs.
  - **Read-ahead** (`LocalPlayerConfig.read_ahead`, default on; `LocalPlayer::set_read_ahead`) is in force only with a cache folder.
  - **Retry budget:** the granted grace, counted from the first failure in a row (60 s without a grant).
  - **Client A3:** grace asked for only when `/v1/me` says `resume_grace` (the first connection waits up to 2 s for `/v1/me`); the resume window is the grant less 5 s (55 s without one). The outbox drops `STMt` first. `LocalPlayer::set_network_available` (from `ConnectivityManager`): with no network the socket is dropped, nothing is tried, and the core's link is down (so running dry sends `STMo`); when it returns, the player reconnects at once. The core's link also follows the socket.
  - **`strm f`:** on `trim_count()` the engine drops what it holds of tracks after `keep_track`: a hold or crossfade in the transitions, the pipeline's pending block, and the ring from the first sample of the later track (a new partial discard in the ring: the output plays up to that point, then jumps past it).
  - **A6:** sync membership comes from the status query already made for each new stream (and again on resume). After a stall, a lone player carries on at once; a grouped one waits up to 2 s for enough audio to skip the stall, then calls `resume_after_stall`. On `NotEnoughBuffered` it asks LMS for the group's position (`time ?`) and sends `time <that>`, so LMS restarts the group in step (the others hear a short pause). Minimal by design; checked live (see "A6 live check" under A6).
  - **For Android (A5):** `LocalPlayerStatus.cushion_ms` (and `LocalPlayer::cushion_ms()`): decoded audio plus spooled bytes at the byte rate seen so far. Also `resume_window_ms` and `read_ahead`.
  - **The 20 s wait** before read-ahead is the core's default.
  - **Restart after a gap longer than the grace** (decision 3, built with A5):
    - **What is remembered:** the status query made for each new stream also gives the track's queue position (`protocol::stream_position`: the item's `playlist index`, checked against `playlist_cur_index`, and for a fresh stream LMS's `time`, so a stream that began part-way after a seek is placed right). While the link is down, the engine notes the point being heard every tick (`protocol::watch_gap`): queue position, track id, and time into the track. The point follows playback through the gap, so a track the phone carried on into from its spool is the one restarted.
    - **Blocked:** if the player wasn't playing when the link went (the user had paused or stopped), or the user held the output during the gap.
    - **On the new session** (a greeting with `resumed=false` replacing a lost one), after 1 s for LMS to settle, the engine asks `status <i> 1 tags:x` and restarts (`playlist index <i>`, then `time <s>` from 1 s in) only when `protocol::restart_commands` agrees: LMS shows the player stopped (not playing, not paused), the item at `<i>` is the same library track, and LMS's current index is `<i>` or the one before it. Anything malformed means no restart.
    - **Limit:** a stop from another controller during the gap looks the same to the phone as LMS dropping the player, so such a track is restarted.

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
- **Core built (2026-10-13), with these refinements:**
  - **Fetched and fed are separate.** `open_stream()`, STAT `bytes_received` and `HELO` follow the newest track (the one being fetched). `on_stream_data()` and `wants_stream_data()` follow the oldest track whose input hasn't ended, so a spool can keep feeding the current track while the next one downloads. A queued track is held to `INPUT_HIGH_WATER` until it decodes.
  - **Spool calls:** `stream_received()` (network done: `DSCO`, and `STMd` may go) and `stream_bytes_received(total)` (the network count, for STAT and resume). `on_stream_end()` still ends the decoder's input; it sends `DSCO` only if `stream_received()` didn't.
  - **`STMd` fallback:** if the track finishes decoding first (a short track), `STMd` goes then, as before. Never twice.
  - **`strm f` with nothing audible** (not playing or paused, or the audible track fully played) is a full flush, as before. Otherwise hosts follow it through `trim_count()` / `trim()`, not `flush_count()`.
  - **Web player (2026-10-13):** the WASM host logs which track each pulled frame came from and hands the worker `take_trim()` (`keep_track`, `keep_frames`, `drop_frames`) once per `strm f`. Web Audio drops the last `drop_frames` frames queued in the worklet. The media element's audio is pulled a track at a time (`pull_one`) and its segmenter is told whose it is (`push_track`), so `trim_after(keep_track)` cuts the timeline at the FLAC frame where the kept track ends: the page removes the segments from there, and the segmenter queues the kept part of that frame again (and the kept track's init segment, if the rate changes back).
  - **A `strm s` while the current track is still being fetched** replaces it, as before (LMS doesn't do this without a `strm q`).
  - **`STMr` after `STMo`** is sent only in read-ahead mode, so the web and CLI players are unchanged.
- **A4 and A6 in the core:** `set_link_up(up, now)`; with the link down, running out of audio sends `STMo`, and `STMu` waits until the link has been back 5 s (`LINK_SETTLE_MS`) with nothing new from LMS. `stall_ms()`, `resume_after_stall(in_sync_group, now)` (skips a stall over 50 ms in a group, or reports `NotEnoughBuffered`) and `skip_audio(ms)`.

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
- **Android built (2026-10-13):**
  - **Settings:** `ReadAheadSettings` (DataStore keys `read_ahead`, `read_ahead_mobile_data`, both on by default) under Settings › Playback on this phone › Gaps in the signal. The mobile-data switch can be changed only while reading ahead is on.
  - **The rule:** read ahead = the setting on AND (not on a metered network OR the mobile-data switch on). `RustLocalPlayerRepository` applies it with `set_read_ahead` whenever the settings or the network type change, and before a new engine starts.
  - **Network:** the app already follows the default network with one `ConnectivityManager.NetworkCallback` (`AndroidNetworkMonitor`, created with the app container, not an androidx.startup initialiser); the repository passes "anything but offline" to `set_network_available`, and the metered state to the rule above. A second callback in the playback service would only duplicate it.
  - **Cache folder:** `noBackupFilesDir/stream-cache`, beside downloads (`noBackupFilesDir/offline`); the engine creates it and scopes it by server and user.
  - **The cushion:** from `LocalPlayerStatus.cushion_ms` while `read_ahead` is in force; shown from 1 s up as "45 s", "4 min" or "1 h 5 min buffered ahead" in the signal path and under "This phone" in the "Play on…" sheet, read out in full words by TalkBack.
  - **Clean-up:** when the signed-in session ends or changes (sign-out, revocation, another server or user), the repository stops the player and calls `remove_stream_spools`. Switching the player off and on clears nothing (the engine deletes its own spools).

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

**Phase B built (2026-10-13):**
- **S3** (`gateway/src/live.rs`): `-fflags +bitexact -flags:a +bitexact` as output options. With them ffmpeg writes no version strings and numbers the Ogg stream serially instead of at random, so two encodes of one FLAC are byte-identical. A test with the real `ffmpeg` checks it (run in CI, which sets `ANYWHERE_REQUIRE_FFMPEG`), and fails without the flags.
- **S4** (`gateway/src/relay.rs`, `session.rs`): a STAT `bytes_received` beyond what the relay sent was harmless for live Opus (`Counts::upstream_bytes` never passes what was fed) but passed through unchanged for other streams. `Relay::upstream_bytes` now clamps the client's count to what the relay has sent before mapping it, so LMS is never told of bytes it didn't send. Gateway tests with FakeLms: a relay read to 64 KiB and abandoned, then a STAT for the whole stream (MP3 and FLAC), and the session's next stream still relayed.
- **`?limit=` (new, not in the plan):** `/v1/stream/<s>/<n>?limit=m` (1 to 16 MiB) sends at most `m` bytes and ends cleanly. Without it the relay sends up to a replay window (4 MiB) ahead, so on a fast link most of a track arrived before the lookup could stop it. When the cache holds anything at the stream's level and format, the first request asks for 96 KiB (64 KiB of fingerprint after a usual header); a miss fetches the rest with `?offset=` (one more request, from the same upstream); an older gateway ignores the limit. A replay then downloads about 98 KiB.
- **B1 store** (`mobile/src/player/cache.rs`): `<cache_dir>/<scope>/stream-cache/<64 hex>.bin` and `index.json` (version 1). Each entry: fingerprint, level (0 original, 1 high, 2 low), format byte, length, SHA-256 of the whole stream, a last-used counter, and the LMS track id (labelling only). The index is read with `O_NOFOLLOW`, at most 8 MiB and 20,000 entries; each entry is checked field by field and dropped if malformed, duplicated, or its file is missing, the wrong size, a symlink or not a plain file; stray files (and anything not a plain file) are removed. Writes go to a fresh temporary file renamed over the index. A spool is moved in (renamed: same folder tree) only when its stream ended cleanly and every byte is in its file; it is then read from the cache, held in use (a pin) until fed. LRU eviction under the cap; pinned entries are never evicted (held over the cap until released). The cache counts apart from downloads and spools.
- **Fingerprint (a refinement):** SHA-256 of a fixed label, the level, the format byte and the stream's first bytes: its header plus 64 KiB, or the whole stream if shorter. The header is the FLAC metadata, an ID3v2 tag, or the Ogg header pages (before the first page with a granule position); a header over 1 MiB is never matched. Covering audio past the header means a stream after a seek can't match the whole track even when its header is long (embedded artwork with `--keep-flac-pictures`). Known limit: a seek into a run of byte-identical audio frames longer than the window (an MP3's silent intro) would match from the start.
- **B2 matching** (in `stream_task`): every stream is fingerprinted and hashed as it arrives while the cache may be used; it is looked up only while the cache is in use. On a hit the whole cached file is read on a blocking thread, checked against the entry's length and hash and against the bytes already received, and only then is the download dropped and the rest fed from that file (at the decoder's pace). The hash is checked again at the end; a mismatch or a short read deletes the entry (and fails the track there, as a broken download would). A failed check deletes the entry and the stream carries on from the network. `bytes_received` counts the whole stream; acks stop at what the network delivered. The status and the signal path say "from cache" while the track heard came from it.
- **Permission (decision 1):** the engine asks `/v1/account` when it starts and with each new session; the cache is used only once `downloads.enabled` is true, and is emptied when it is false. Without it, spools behave as in Phase A.
- **Deleted:** the user's "Clear", the size set to off, the permission gone, the player socket refused or revoked (1008), and `remove_stream_spools` (now spools and every scope's cache: sign-out, revocation, another server or user). At start the engine deletes every other scope's cache.
- **UniFFI:** `LocalPlayerConfig.stream_cache_bytes` (0 off, at most 64 GiB), `LocalPlayer::set_stream_cache_bytes`, `stream_cache_totals()` and `clear_stream_cache()`, the free functions `stream_cache_totals(session, cache_dir)` and `clear_stream_cache(session, cache_dir)` for when no player runs, `LocalPlayerStatus.from_cache` and `SignalPathInfo.from_cache`. Only totals (bytes, count) are exposed, never paths.
- **Android:** Settings › Playback on this phone › Gaps in the signal › *Replay cache* (beside read-ahead rather than under Storage, which exists only with downloads): Off, 500 MB, 1 GB (default), 2 GB, 5 GB (DataStore `replay_cache_bytes`, anything else read as 1 GB), a usage line ("512.0 MB for 12 tracks") and *Clear*. Without the download permission the choices are disabled and it says why. "Off" with no player running empties the cache at once. The signal path shows a *From cache* stage.

**Phase C (optional, M): reuse downloaded FLAC originals.**
- **When:** a FLAC stream at the Original level whose STREAMINFO (MD5 and samples) equals a downloaded original's.
- **How:** splice at the first audio frame (stream offset = the file's header length + bytes received − the stripped header length). The frames are identical, so the decoded audio is bit-identical.
- **Not for Opus:** Opus downloads aren't byte-identical to live Opus, so they aren't reused.

**Phase C built (2026-10-13)** (`mobile/src/player/downloads.rs`, `stream_task` in `engine.rs`, `anywhere_core::decode::verify_flac`):
- **When:** the session's level is Original and the `strm` format byte is `f` (pass-through or FLAC-stripped by the gateway), and the offline index lists at least one Original download with the extension `flac`. MP3, Opus and other originals (ALAC in MP4) are never candidates.
- **The probe:** such a stream's first request asks for `?limit=` 96 KiB, as Phase B does. Its first bytes are held until the FLAC header has ended (`downloads::flac_head`: `fLaC`, a 34-byte STREAMINFO first and only there, defined block types, at most 16 MiB and 1024 blocks, the last flagged, then a frame sync). Anything else, and the stream simply carries on.
- **Matching:** STREAMINFO equal in full: MD5, total samples, sample rate, channels and bit depth. An MD5 of all zeros or an unknown length never matches. The downloads' headers are read once and remembered while each file's size and time stay the same.
- **Checks before use** (on a blocking thread; any failure means the network):
  - the offline store's own check: a plain file, never a symlink, of the size the index recorded (`store::file_fits`), opened with `O_NOFOLLOW`; the index read the same way, bounded and parsed by the offline index's parser; the scope and `tracks` folders real folders;
  - the file's STREAMINFO equal to the stream's;
  - the audio received so far equal to the file's bytes at the same place (`Mf + (N − Ms)`), which also rules out a re-encode of the same audio;
  - the whole file decoded with every frame intact, to its STREAMINFO's MD5 and sample count (`verify_flac`), through the same open file. Its other metadata (tags, artwork) is replaced by a bare STREAMINFO for this, so an odd tag block doesn't rule a file out. Done once per file version (size and time). It costs one decode of the file before the switch (the stream waits for it); read-ahead usually hides it.
- **Splice:** the rest of the stream is the file from `Mf + (N − Ms)`, where `Mf` and `Ms` are the file's and the stream's header lengths and `N` the bytes received. Fed at the decoder's pace. The frames are the same bytes, so the decoded audio is bit-identical.
- **Counting:** `bytes_received` (STAT, spool) counts the whole stream; acks stop at what the network delivered; the gateway clamps what LMS hears (S4). A file that comes up short ends the track there. Such streams aren't kept in the replay cache (the download is already on the phone).
- **Scope and clean-up:** `LocalPlayerConfig.downloads_dir` (Android: `noBackupFilesDir/offline`, the offline library's folder) under the signed-in user's hashed scope. Only read, never written: the download manager owns the files. No download permission check: a download that exists implies it. Revocation and sign-out delete the downloads as before; a file that has gone is not used. The engine stops reading them after a 1008.
- **Reporting:** `LocalPlayerStatus.from_downloads` and `SignalPathInfo.from_downloads`. Android shows a *From downloads* stage in the signal path.

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
| Gap longer than the grace | New session; the track heard last restarts where it was, if LMS still has it there and nobody paused or stopped it (decision 3) |
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
- an Original FLAC stream of a downloaded album fetches only the probe and plays bit-identical "from downloads"; a modified download falls back to the network; a different master (same tags, another MD5) is not matched;
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
- **Downloads:** download an album as originals, then play it at Original on mobile data: the signal path says "From downloads" and the app's data use stays small.
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
| A6 rejoining a sync group after a gap | S |
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
