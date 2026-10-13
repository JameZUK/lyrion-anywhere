# Wave 3 design: signature features

Started on 2026-10-13, from `docs/plans/ux-research.md` §6 (Wave 3). The aim is features no other LMS app offers. Web and Android together; iOS takes the same design when its app is built. Each part is detailed before it starts.

## Order

1. **Listening stats and "Your year"** (D7). Built on the per-user listening history the gateway already keeps.
2. **Smart filters v2** (X7): "any" rules, track-level rules (rating, play count, last played, format, sample rate, length, year range), and filters as download rules.
3. **Smart Shuffle and DJ styles** (X3), with Sonic path (X2) only if AudioMuse-AI offers it.
4. **Guest queue ("Jam")** (D8). It needs a threat model first, as it is a new kind of access.
5. **Album booklets** (X5 part 2). It needs a security design for resolving an album's folder without exposing paths.
6. **Investigate the rolling cache** (H3).

Deferred: timed lyrics from LRCLIB (see `ux-research.md`, Deferred).

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
