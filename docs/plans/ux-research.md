# UX and feature research: what to borrow next

**Status:** research for review (2026-10-12). Nothing here is decided. Proposals that are adopted should become SPEC decisions or plan items.

**Scope:** the web app (PWA), the Android app and the planned iOS app. It compares Lyrion Anywhere with Plexamp, Symfonium, Spotify, audiophile players (Roon/ARC, UAPP, Neutron, Poweramp), Apple Music, iOS library players (Doppler, Marvis Pro), Qobuz and Tidal, and the LMS ecosystem (Material Skin, Squeezer, iPeng and the common plugins).

**Method:** official sites, store listings, release notes, help pages and community forums, gathered in October 2026. Store listings are developer claims, and some sources are a few years old. Where something couldn't be verified it says so. Everything is summarised in our own words.

---

## 1. Executive summary: the ten most valuable ideas

1. **Sleep timer on every player:** LMS's `sleep` is server-side, so it works on hardware players too, which Spotify's timer [doesn't do over Connect](https://community.spotify.com/t5/Android/Sleep-timer-only-works-when-not-casting-to-another-device/td-p/5322690). The gateway already allows it, so only the UI is missing.
2. **An ambient Now playing:** an animated, blurred album-art background (Plexamp's UltraBlur), large art, a format badge ("FLAC 24/96") and an "Up next" peek.
3. **Synced lyrics:** these come from LMS track tags and the Music & Artist Information (MAI) plugin, which Material Skin already shows as timed lyrics. They'd scroll with the waveform and support tap-to-seek.
4. **Library stations:** one-tap radio from your own library (Time Travel, Decade, Genre, Deep cuts, Forgotten favourites, Random album), built on LMS Random Mix, play counts and AudioMuse-AI. Plexamp's best-loved discovery feature.
5. **A customisable Home:** reorderable and hideable shelves, as in Plexamp and Symfonium, including On this day, Most played, Recently played, Mixes for you, Downloads and pinned plugin menus.
6. **Ratings, and smart playlists and filters:** stars or hearts through Ratings Light, and rule-based views (genre, year, rating, play count, last played, added). Symfonium's and Marvis Pro's most-praised power feature.
7. **Alarms:** wake to music on any player, as Material Skin and Squeezer manage today. A clear LMS-native advantage over streaming apps.
8. **Credits, bios and booklets:** contributor roles you can tap (Tidal), artist biographies and album reviews (MAI), and the album folder's PDF booklet (Qobuz-style), via MAI's `localfiles`.
9. **Car mode and CarPlay:** a big-target driving screen in every app, CarPlay on iOS, and artwork in Android Auto.
10. **A visualiser, plus "Your year" stats:** a battery-aware spectrum, oscilloscope or "fluid" visualiser drawn from the decoded PCM, and a private, on-server listening summary built from LMS play counts and the gateway's play log.

---

## 2. Per-app findings

### 2.1 Plexamp (Android, iOS, desktop)

**Sound and transitions**
- **Loudness levelling:** EBU R128 analysis on the server. The seek bar is drawn from the loudness analysis, not a raw waveform ([how-to guide](https://forums.plex.tv/t/577024)).
- **Sweet Fades:** overlaps tracks using the loudness data and avoids fading inside albums. It's free, as are gapless playback and pre-caching ([Plexamp page](https://www.plex.tv/plexamp/)).

**Sonic features** (Plex Pass; the server must have run sonic analysis)
- **Sonically similar tracks, albums and artists.**
- **Sonic Adventure:** pick a start track and a destination track, and the app fills in a gradual path between them ([summary](https://www.techspot.com/downloads/7055-plexamp.html)).
- **Guest DJ:** DJs that reshape the queue as it plays. *Stretch* inserts a mini Sonic Adventure between songs; *Contempo* favours the same era; *Groupie* favours the same artist; others exist.
- **Sonic Sage:** a short-lived experiment with an LLM to suggest playlists ([Sound & Vision](https://www.soundandvision.com/content/plexamp-s-sonic-sage-uses-chatgpt-improved-discovery)).

**Library stations and mixes**
- **Stations** ([how-to guide](https://forums.plex.tv/t/577024)): Library, Time Travel (oldest to newest), Random Album, Genre/Style, Mood, Decade, and the Artist and Album Mix Builders. Selection weighs popularity, your ratings and how recently a track was played.
- **Home shelves:** Mixes for you, On this day, Top albums, Most played and Artists on tour. Shelves can be reordered and hidden.

**Look and feel**
- **UltraBlur:** a soft, multi-colour gradient taken from the album art behind the player. It's widely praised and was tuned again in 4.4 ([release notes](https://www.apkmirror.com/apk/plex-inc/plexamp/plexamp-4-4-0-release)).
- **Visualisers:** "over a dozen" visualisers and four visual themes. On touch devices, swipe the visualiser to change it.
- **Gestures:**
  - swipe the player left or right to change track;
  - swipe up to peek at the queue;
  - long-press play to stop;
  - long-press next to skip the album.
- **Lyrics:** interactive (tap a line to seek), licensed from LyricFind.
- **Driving mode:** a simplified, large-control screen. A long press on play stops. Swipe-to-skip was requested for car view but [never shipped](https://forums.plex.tv/t/swipe-gestures-for-plex-amp/831909).
- **Android Auto:** Home, Library, Charts and Downloads, plus a star button on Now playing to rate the track.

**Downloads**
- Downloads by duration (30 minutes to 24 hours of music).
- Short playlists download automatically.
- Smart playlists refresh themselves.
- An option to prefer downloaded copies.

**Pain points** ([forum threads](https://forums.plex.tv/c/plex-labs/16?page=7))
- Complaints about shuffle quality, search, stutter on some FLAC files, Bluetooth output switching and mobile data use.
- The best features need a subscription and the Plex cloud account.
- No bit-perfect USB output.

### 2.2 Symfonium (Android)

Sources: [the Symfonium site](https://symfonium.app/) and [its docs](https://docs.symfonium.app/wiki/).

**Customisable UI**
- Configurable tabs, home menus and shortcuts.
- Per-screen layouts (list, grid, carousel) and Material You.
- A shared catalogue of "Styles": themes, Now playing styles and whole-app styles ([configurable interface](https://docs.symfonium.app/wiki/other/symfonium-configurable-interface/)).
- Power users rate this customisation above everything else.

**Smart filters and smart playlists**
- Rule-based views over any tag: rating, play history, genre, year and custom tags. They can be combined and pinned ([smart filters](https://docs.symfonium.app/wiki/other/smart-filters/)).

**Cache and offline**
- Three tiers: a playback cache, a rolling cache and permanent downloads ([cache guide](https://docs.symfonium.app/wiki/other/offline-media-cache-downloads-rolling-cache-playback-cache-automatic-cache-and-more/)).
- Automatic rules decide what is kept.

**Sources and casting**
- **Sources:** Plex, Jellyfin, Emby, Subsonic/Navidrome, Kodi, SMB, WebDAV and cloud drives, merged into one library.
- **Casting:** to Chromecast, UPnP (gapless where supported), Kodi, Plex and Sonos (groups and volume).

**Sound**
- Bit-perfect USB on Android 14+.
- A 64-bit DSP path.
- PEQ and GEQ with AutoEq and APO import, and several EQ profiles.
- Crossfeed.
- "Smart Fades" (music-aware crossfades).

**Discovery and the rest**
- **Mixes:** instant and personal mixes from songs, artists, genres, history and likes ([mixes explained](https://docs.symfonium.app/wiki/other/mixes-radio-shuffle-shuffled-what-the-random/)).
- **Lyrics:** synced, embedded or server-provided, on a configurable lyrics screen.
- **Queues:** several Now playing queues, each keeping its own position, shuffle state and speed.
- **Android Auto:** deep customisation and voice commands, with alphabetical splitting for long lists.
- **Other platforms:** Wear OS with music sync, and Android TV.

**Pain points**
- A steep learning curve and an overwhelming number of settings.
- One developer; Android only.

### 2.3 Spotify

**Personalisation and discovery**
- **Home:** feed filter chips (Music, Podcasts and so on) and shelves personalised by time of day.
- **DJ:** a voiced AI host that mixes recent listening, recommendations and throwbacks ([Bustle](https://www.bustle.com/life/spotify-updates-2023-smart-shuffle-discovery-mode-dj)).
- **Smart Shuffle:** a third state of the shuffle button that adds suggestions to the user's own playlist ([community](https://community.spotify.com/t5/Your-Library/Introducing-Smart-Shuffle/td-p/5516566/page/33)). Some users find it intrusive, so it can be turned off.
- **Blend:** shared playlists made from two or more people's tastes.
- **Wrapped:** a yearly, shareable story of your listening.

**Listening together**
- **Jam:** a live group session in which guests add to one queue and the host decides whether they can reorder it ([BGR](https://bgr.com/tech/spotifys-new-jam-feature-turns-everyone-at-the-party-into-a-dj)). Guests join by QR code or by being near the host.

**Devices**
- **Connect device picker:** a single device icon, with the current device highlighted in green and a bar showing where it plays. This is the model our *Play on…* follows.

**Now playing**
- **Lyrics:** time-synced and scrolling, opened by swiping up from Now playing, licensed from Musixmatch ([TechCrunch](https://techcrunch.com/2021/11/18/spotify-finally-rolls-out-real-time-lyrics-to-global-users)).
- **Canvas:** short looping visuals behind Now playing (artist-supplied).

**Library, search and queue**
- **Your Library:** type chips (Playlists, Albums, Artists, Downloaded), sort (recents, recently added, alphabetical, creator), grid or list, and pinned items.
- **Search:** recent searches, genre and mood tiles, and instant typed results grouped by type.
- **Queue:** "Next in queue" (user-added) is kept apart from "Next from" (context).

**Timers**
- **Sleep timer:** preset durations and "end of track". It [doesn't work reliably over Connect](https://community.spotify.com/t5/Android/Sleep-timer-only-works-when-not-casting-to-another-device/td-p/5322690), a long-standing complaint.

**Pain points**
- The feed is crowded with podcasts and promotion.
- The Smart Shuffle state confuses people.
- Device switching is sometimes flaky.

### 2.4 Audiophile and other notable players

**Roon / Roon ARC**
- **Signal path:** shows every stage with a quality light, and ARC shows the chain on mobile too ([TechHive review](https://www.techhive.com/article/1443872/roon-arc-review.html)).
- **Focus:** multi-criteria filtering with sort and text, e.g. composition by period and form ([Roon 1.8](https://blog.roonlabs.com/roon-1-8-summer-release/)).
- **Discovery:** Valence (Roon's metadata and recommendation engine), Roon Radio and Daily Mixes.
- **Credits:** rich, cross-linked credits and bios; everything is a link.
- **Pain points:** price, and a heavyweight "core". ARC can't use local outputs other than the phone.

**USB Audio Player Pro (UAPP)**
- **Output:** its own USB driver for bit-perfect output (now also via Android 14's API), hardware volume, DSD and MQA.
- **Extras:** ToneBoosters PEQ (an add-on) and UPnP as renderer and server ([Audiophile Style](https://audiophilestyle.com/polestar/software/extream-usb-audio-player-pro-reviews-r271/)).
- **Pain points:** a dated UI, and some stability and DAC regressions after updates.

**Neutron**
- **Engine:** its own 32/64-bit engine.
- **DSP:** a 4–60 band parametric EQ, ambiophonic surround, crossfeed, a compressor, speaker time alignment and dither ([App Store](https://apps.apple.com/app/id766858884)).
- **Pain points:** the UI is notoriously hard to learn. Power without polish.

**Poweramp (Android)**
- **EQ:** a graphic EQ of up to 32 bands, plus a parametric mode.
- **Direct Volume Control:** EQ without clipping.
- **Visualisers:** Milkdrop presets and a spectrum.
- **Look:** a wave seek bar and third-party skins ([release notes](https://www.apkmirror.com/?p=10821559)).
- **Lyrics:** synced lyrics were added recently.

**Apple Music**
- **Badges:** small "Lossless", "Hi-Res Lossless" and "Dolby Atmos" badges on Now playing and albums.
- **Lyrics:** full-screen and time-synced, with per-voice alignment.
- **iOS 26:** AutoMix (beat-matched transitions), lyrics translation and pronunciation guides ([MusicTech](https://musictech.com/news/music/apple-music-ios-26-updates/)).
- **Pain points:** AutoMix reportedly doesn't work over AirPlay, and it can sound gimmicky on albums.

**Doppler (iOS)**
- **Library:** a clean, local-first library with album collections.
- **System surfaces:** interactive Now playing widgets and CarPlay.
- **Stats:** monthly and yearly listening reports ([App Store](https://apps.apple.com/no/app/id1468459747)).
- Shows that a private, on-device "year in review" is valued without any cloud.

**Marvis Pro (iOS)**
- **Layouts:** a highly customisable Home, including the metadata rows and how lists and Now playing look.
- **Smart Rules:** filter, sort and limit rules with "maximum items" ([App Store](https://apps.apple.com/sg/app/id1447768809)). Reviewers single out how much can be customised.

**Tidal and Qobuz**
- **Tidal:** interactive credits. Tap a role on an artist page to see everything they played on, produced or wrote ([Engadget](https://www.engadget.com/2019-07-03-tidals-interactive-credits-let-you-explore-artists-other-proje.html)).
- **Qobuz:** digital booklets, liner notes and editorial articles ([App Store](https://apps.apple.com/app/id946429340)).

### 2.5 The LMS ecosystem (what LMS users expect)

**Material Skin** ([README](https://github.com/CDrummond/lms-material), [ChangeLog](https://github.com/CDrummond/lms-material/blob/master/ChangeLog))
- **Player tools:** a sleep timer per player, and "sleep all" in the drawer.
- **Alarms:** an alarm editor with shuffle options, and the next alarm shown in the toolbar (from `status` alarmData, LMS 8.5+).
- **Ratings:** shown in the queue and lists, settable through Ratings Light, with half-star support.
- **MAI integration:** biographies, album reviews and *timed lyrics* that highlight and scroll with playback (the default).
- **Party mode:** a restricted guest UI with no skipping (Material dropped it from the LMS skins list, but it still exists as a mode).
- **Look:** blurred art backgrounds behind toolbars.
- **Gestures:** swipe to change track, and swipe on the cover for volume.

**Squeezer (Android)** ([F-Droid](https://f-droid.org/packages/uk.org.ngo.squeezer/))
- **Player tools:** sync groups, a sleep timer, alarm management and favourites editing.
- **Extras:** lyrics, widgets, Wear OS controls and downloads.
- **Swipe gestures in lists:** right for play next, left to add to the queue.

**iPeng (iOS)**
- A long-standing paid controller with an optional on-device player.
- Couldn't be verified online this round; treat it as background only.
- LMS users coming from it expect alarms, sleep, artwork and plugin menus.

**Plugins that matter**
- **Music & Artist Information** ([CLI](https://github.com/michaelherger/MusicArtistInfo/blob/master/CLI.md)):
  - `musicartistinfo lyrics`, `biography`, `albumreview`, `artistphoto(s)` and `albumcovers`;
  - `localfiles folder:…`, which lists PDF, TXT, HTML, MD and NFO files in an album folder (booklets).
- **Ratings Light** ([repo](https://github.com/af-1/lms-ratingslight)) for ratings, and **Alternative Play Count** for better play and skip counts.
- **Dynamic Playlists, Virtual Library Creator and Custom Skip** (all by [AF-1](https://github.com/AF-1)): LMS's "smart playlist" ecosystem.
- **Random Mix, Don't Stop The Music and AudioMuse-AI:** mixes and radio (we already use DSTM and AudioMuse-AI).
- **Spotty, podcasts and radio:** these already appear through plugin menus.

**LMS core** ([CLI reference](https://lyrion.org/reference/cli/introduction/))
- `sleep`, `alarm` and `alarms`, and `favorites`;
- `songinfo` and `titles` with rating and lyrics tags (from file tags);
- `albums sort:recentlyplayed|playcount` (9.1);
- contributor roles.
- LMS keeps a play count and last-played time per track, but not a per-play history.

**What LMS users notice is missing in new clients:** alarms, sleep, ratings, MAI info, and a way to see *all* players at a glance with power and sleep status.

---

## 3. Gap analysis

Key: ✅ has it · ◐ partial · ✗ not present · — not applicable.

| Feature | Plexamp | Symfonium | Spotify | Others (best) | Lyrion Anywhere today |
|---|---|---|---|---|---|
| Play on device / pick output | ✅ (cast to Plexamp players) | ✅ (UPnP, Cast, Sonos) | ✅ Connect | Roon zones | ✅ *Play on…*, Transfer, sync groups |
| Sleep timer | ◐ | ✅ | ◐ (fails over Connect) | Material, Squeezer: per player | ✗ (gateway allows `sleep`; no UI) |
| Alarms | ✗ | ✗ | ✗ | Material, Squeezer | ✗ |
| Blurred or animated art background | ✅ UltraBlur | ✅ styles | ◐ Canvas | Material: toolbar blur | ◐ album-art colour only |
| Visualiser | ✅ 12+ | ◐ | ✗ | Poweramp: Milkdrop | ✗ (planned A8) |
| Synced lyrics | ✅ | ✅ | ✅ | Apple: translation; Material: MAI timed | ✗ |
| Waveform or loudness seek bar | ✅ | ◐ | ✗ | Poweramp: wave bar | ✅ |
| Loudness levelling | ✅ R128 | ✅ RG | ✅ | Roon | ✅ R128/RG (Android) |
| Smart crossfades | ✅ Sweet Fades | ✅ Smart Fades | ✅ (crossfade, AutoMix-like) | Apple AutoMix | ✅ (Android) |
| Library stations (time travel, decade, genre…) | ✅ | ✅ mixes | ◐ (no own library) | Roon Radio | ◐ Start radio, plugin menus |
| Sonic path / guest DJ | ✅ | ✗ | ◐ DJ | — | ✗ (AudioMuse-AI could supply) |
| Smart Shuffle style suggestions | ◐ DJ | ✗ | ✅ | — | ✗ |
| Customisable Home | ✅ | ✅✅ | ◐ chips | Marvis Pro | ✗ fixed shelves |
| Smart filters / playlists | ✅ | ✅ | ✗ | Marvis rules, Roon Focus, LMS Dynamic Playlists | ◐ sort and role chips; plugin menus |
| Ratings | ✅ stars | ✅ | ◐ like | Material via Ratings Light | ✗ |
| Credits / roles | ◐ | ◐ | ◐ | Tidal, Roon | ◐ roles in browse; not tappable from a track |
| Bios / reviews / booklets | ✅ bios | ✅ bios | ◐ | Qobuz booklets, MAI | ✗ (plugin menus only) |
| Format and quality badges | ✅ | ✅ | ✗ | Apple badges | ◐ signal path (Android) |
| Signal path | ✗ | ◐ | ✗ | Roon | ✅ (Android) |
| Bit-perfect USB | ✗ | ✅ | ✗ | UAPP | ✅ (Android 14+) |
| EQ with AutoEq | ✅ basic | ✅ | ◐ | Neutron, Poweramp | ✅ (Android) |
| Offline / smart downloads | ✅ | ✅✅ (rolling cache) | ✅ | Roon ARC smart downloads | ✅ (rolling cache ✗) |
| Car mode in app | ✅ | ◐ | ✅ | — | ✗ |
| Android Auto / CarPlay | ✅ / ✅ | ✅ / — | ✅ / ✅ | — | ✅ AA (no browse art) / iOS planned |
| Widgets / tile | ✅ | ✅✅ | ✅ | Doppler: interactive | ✅ one widget, tile |
| Gestures / queue peek | ✅ | ✅ | ✅ | Squeezer list swipes | ◐ art swipe, mini player swipe |
| Group listening (Jam) | ✗ | ✗ | ✅ | Material party mode | ✗ (accounts exist) |
| Listening stats / year in review | ◐ | ◐ | ✅ Wrapped | Doppler reports | ✗ (play counts in LMS) |
| Up next preview | ✅ | ✅ | ✅ | — | ◐ queue only |
| Search UX (recents, scopes) | ◐ | ✅ | ✅ | — | ◐ |
| Wear OS | ✗ | ✅ | ✅ | Squeezer | ✗ (planned) |
| Haptics | ◐ | ✅ | ✅ | — | ◐ art swipe tick |
| Self-hosted, no cloud account | ✗ (Plex account) | ✅ | ✗ | Roon (licence) | ✅ |

**Where we already lead:** player-first control of real hi-fi players, Transfer, sync with hardware players, bit-perfect output with a visible signal path, a waveform from server analysis, a hardened internet-facing gateway, and no third-party cloud.

**The largest gaps:** feel (backgrounds, a visualiser, lyrics), daily utilities (sleep, alarms, ratings), personalisation (Home, smart filters, stations) and the car.

---

## 4. Proposals

Each proposal gives platforms (W = web, A = Android, I = iOS), LMS feasibility, effort (S/M/L) and priority (P1 = next, P2 = soon, P3 = later).

**Security note for every proposal touching the gateway:** each new command is a fixed allow-list entry, scoped to permitted players, with negative tests. Each new parser of server text (LRC, booklets, plugin answers) gets fuzz tests, as `CLAUDE.md` requires.

### 4.1 Delight

**D1. Ambient Now playing ("glow" background)**
- **What:**
  - a slow, animated mesh gradient made from 3–5 colours taken from the art (we already extract an accent), drawn behind Now playing and, faintly, behind the mini player and queue;
  - a static blurred variant when "reduce motion" is set;
  - optional "Art fills the screen" mode.
- **Why:** Plexamp UltraBlur; Apple Music's animated art; Material's blurred toolbars.
- **Platforms:** W A I.
- **Feasibility:** client only. Use the existing artwork endpoint at a small size (64–128 px) for colour extraction.
  - *Web:* CSS gradients or a small canvas, at 15–30 fps, paused when hidden.
  - *Android:* Compose `Brush` animation or `RenderEffect` blur (API 31+, fall back to gradients).
  - *iOS:* `MeshGradient` (iOS 18).
- **Effort:** S–M. **Priority:** P1.

**D2. Now playing layouts**
- **What:** three switchable layouts, chosen with a swipe or a layout button and remembered per device:
  1. *Art* (today's layout, larger art and the waveform);
  2. *Lyrics* (D3);
  3. *Info* (format badge, credits, MAI bio snippet, "Up next" card).
  - On tablets and desktop: art left, lyrics or info right, with the queue as a pinned side panel.
- **Why:** Plexamp and Spotify swipe-up panels; Symfonium Now playing styles.
- **Platforms:** W A I.
- **Feasibility:** client only, apart from D3 and D9.
- **Effort:** M. **Priority:** P2.

**D3. Synced lyrics**
- **What:**
  - lyrics as a Now playing layout and a full-screen view, with the current line highlighted and auto-scrolled;
  - tap a line to seek, which reuses the waveform seek path;
  - plain lyrics when untimed;
  - offline copies saved with downloads;
  - a large-text karaoke mode for the TV or car.
- **Why:** Spotify, Apple Music, Plexamp and Symfonium; LMS users already see timed lyrics in Material.
- **Platforms:** W A I.
- **Feasibility:**
  - LMS returns embedded lyrics in track info (lyrics tag in `songinfo`/`titles`).
  - MAI's `musicartistinfo lyrics track_id:…` fetches lyrics, which Material renders timed when they carry LRC timestamps.
  - *Gateway:* allow `musicartistinfo lyrics` with `track_id` only (no free `url:` or artist/title from listeners), when MAI is installed (found dynamically, like AudioMuse-AI), plus a size cap.
  - *Client:* an LRC parser in Rust `core` (no I/O; fuzzed), shared through WASM and UniFFI.
- **Effort:** M. **Priority:** P1.

**D4. Visualiser**
- **What:** an optional full-screen or in-art visualiser: spectrum bars, an oscilloscope, "glow" (D1's gradient pulsing with bass energy) and a VU-style meter for audiophiles.
  - Swipe to change, as in Plexamp.
  - Automatically off on battery saver, and capped at 30 fps.
- **Why:** Plexamp, Poweramp and Squeezebox Touch nostalgia (VU meters).
- **Platforms:** W A I (only when playing locally; on a remote player, show D1 instead).
- **Feasibility:**
  - Android and iOS: an FFT in Rust `core` on the samples the engine already decodes, sent to the UI at a low rate.
  - Web: the decoded PCM in the player worker (both playback modes), or an `AnalyserNode` in Web Audio mode.
  - Nothing on the server.
- **Effort:** M. **Priority:** P2.

**D5. Up next card and transitions**
- **What:**
  - In the last 15 s of a track, a small "Up next: Title · Artist" chip slides in on Now playing and the mini player.
  - Album changes cross-fade the art and background; Now playing opens from the mini player with a shared-element transition.
  - Track changes animate the waveform in.
- **Why:** Spotify's "Up next" previews; the fluidity of Plexamp and Apple Music.
- **Platforms:** W A I.
- **Feasibility:** client only; LMS `status` already carries the next track.
  - *Web:* the View Transitions API (with a fallback).
  - *Android:* Compose shared elements.
  - *iOS:* `matchedGeometryEffect`.
- **Effort:** S. **Priority:** P1.

**D6. Haptics vocabulary**
- **What:** consistent light ticks on these events:
  - track skip by swipe;
  - waveform scrub detents (each 10 %);
  - volume steps;
  - drag-to-reorder pick-up and drop;
  - rating set;
  - a sleep timer set.
  - A single "Haptics" switch.
- **Why:** Spotify and iOS conventions; we already tick on art swipes.
- **Platforms:** A I (W: `navigator.vibrate` on Android browsers only; no iOS Safari support).
- **Feasibility:** client only.
- **Effort:** S. **Priority:** P2.

**D7. "Your year" and listening stats**
- **What:**
  - a private stats page: most played artists, albums and tracks, hours listened, new discoveries (first played this year), the busiest hour and the longest streak;
  - a December "year in sound" story (cards you can swipe; sharing exports an image made on the device).
- **Why:** Spotify Wrapped; Doppler's yearly reports.
- **Platforms:** W A I.
- **Feasibility:**
  - LMS gives play count and last played per track, and album sorts by play count (9.1). That covers most-played lists today.
  - Per-play history isn't kept by LMS, so the gateway would keep an opt-in, per-user play log. It already accepts `POST /v1/plays` for offline plays; it would also record plays for this user's own devices.
  - The log needs a retention setting, a privacy policy update and an export or delete option.
- **Effort:** L (the most-played lists alone are S). **Priority:** P3 (most-played shelf P2).

**D8. Guest queue ("Jam" for a house party)**
- **What:**
  - The host shows a QR code; guests open a web page with no account and can search the library and *suggest* tracks (play next or add to the end) on one chosen player.
  - Optional host approval, a per-guest cap, and an expiry (a few hours).
  - Guests never control transport or volume.
- **Why:** Spotify Jam; Material's party mode.
- **Platforms:** W (guest); host on W A I.
- **Feasibility:**
  - *Gateway:* a new, very narrow guest credential, bound to one player and to add-only commands, with rate limits and its own audit trail.
  - This is security-heavy: threat-model it first, with negative tests for every command outside the guest set.
  - QR pairing already exists to build on.
- **Effort:** L. **Priority:** P3.

### 4.2 Usability

**U1. Sleep timer**
- **What:**
  - a moon icon in Now playing's overflow, with presets (15/30/45/60/90 min), "End of this track" and "End of the queue";
  - a remaining-time chip on Now playing and the mini player;
  - "+10 min" from the notification on Android;
  - "Sleep all players" for admins, as in Material.
- **Why:** Material, Squeezer and Spotify. LMS's sleep runs on the server, so it works on hardware players and survives the phone locking, which Spotify's doesn't over Connect.
- **Platforms:** W A I.
- **Feasibility:**
  - LMS `sleep <seconds>` and `sleep ?` are **already in the gateway allow-list** for permitted players.
  - "End of track" and "end of queue" are computed on the client from `status` (remaining time), and re-armed on skips.
  - Offline queue (Android and iOS): a local timer in the engine, with a short fade.
- **Effort:** S. **Priority:** P1.

**U2. Alarms**
- **What:**
  - an Alarms page per player: time, days, volume, what to play (favourite, playlist or the current queue), shuffle, and an on/off switch;
  - the next alarm shown on the player row in *Play on…*.
- **Why:** Material and Squeezer; a core reason people keep Squeezebox hardware.
- **Platforms:** W A I.
- **Feasibility:**
  - LMS `alarm add/update/delete/enableall/disableall` and the `alarms` query; `status` alarmData (LMS 8.5+).
  - *Gateway:* both commands are reserved today (not callable). They need allow-list entries scoped to permitted players, with strict parameter grammar (time, days bitmask, volume range, URL only from favourites or playlists the user can see), plus negative tests.
  - Listeners: alarms only on players they're granted.
- **Effort:** M. **Priority:** P2.

**U3. Mini player gesture model** (made the same on every platform)
- **Gestures:**
  - tap opens Now playing;
  - swipe up opens Now playing;
  - swipe left or right changes track, with a haptic tick and the next or previous art peeking in as you drag;
  - long-press opens *Play on…*;
  - swipe down (or a close button) hides it while paused.
- **Feedback:**
  - a thin progress line along the top edge;
  - the "Playing on Kitchen" strip stays.
- **Why:** Plexamp, Spotify and Apple Music conventions.
- **Platforms:** W A I.
- **Feasibility:** client only. Android already has the swipe up and the art swipe. The web needs pointer-event gestures with keyboard and screen-reader equivalents.
- **Effort:** S. **Priority:** P1.

**U4. Queue: "Up next" sections and quick actions**
- **What:**
  - Show the queue in sections: *Now playing*, *Up next* (tracks you added with Play next) and *Then* (the rest of the album or playlist).
  - Swipe a row right for play next and left to remove, with undo.
  - Squeezer-style swipes in browse lists (right = play next, left = add to queue).
  - A "Clear upcoming" action.
  - A queue peek from Now playing by swiping up.
- **Why:** Spotify's "Next in queue" split; Squeezer and Plexamp.
- **Platforms:** W A I.
- **Feasibility:**
  - LMS has one flat playlist, so the client would track which positions it inserted with `playlist insert`, best effort and per device.
  - Otherwise just presentation, plus list swipes using commands already allowed.
- **Effort:** S–M. **Priority:** P2.

**U5. Customisable Home**
- **What:**
  - Home is an editable list of shelves: reorder, hide, change size (carousel, grid or compact) and add more.
  - Built-in shelves:
    - carry on (recently played);
    - recently added / New Music;
    - On this day (albums from this year *n* years ago, or added on this date);
    - most played;
    - forgotten favourites (high play count, not played for 6 months);
    - random albums;
    - downloads;
    - stations (D-stations, §4.3);
    - favourites;
    - playlists.
  - **Pin anything:** any plugin menu, smart filter or radio station can be pinned as a shelf or tile.
  - Layouts sync between a user's devices.
- **Why:** Plexamp, Symfonium and Marvis Pro; Spotify's filter chips.
- **Platforms:** W A I.
- **Feasibility:**
  - Data from existing queries:
    - `albums sort:new|random|recentlyplayed|playcount` (the last two need LMS 9.1);
    - `titles` with year filters;
    - favourites.
  - *Gateway:* an optional per-user preferences document (small, size-capped, schema-checked) so the layout follows the user. Otherwise store it locally.
- **Effort:** M. **Priority:** P1.

**U6. Library filters and sorting, in Spotify style**
- **What:**
  - type chips on Library (Albums, Artists, Playlists, Downloaded, Favourites) that combine with sort (recently added, recently played, A–Z, year, artist);
  - grid or list toggle;
  - an A–Z fast scroller;
  - a remembered choice.
- **Why:** Spotify Your Library; Roon Focus.
- **Platforms:** W A I.
- **Feasibility:** LMS `albums`/`artists` sort options and the existing role and genre filters. Client only.
- **Effort:** S. **Priority:** P2.

**U7. Search UX**
- **What:**
  - recent searches (stored locally, clearable);
  - results grouped by type, with a "Top result" card;
  - filter chips (Artists, Albums, Tracks, Playlists, Radio, Plugins);
  - typo-tolerant suggestions from LMS search;
  - "Search on <plugin>" rows that hand the query to plugin search menus;
  - play straight from a result row.
- **Why:** Spotify and Symfonium.
- **Platforms:** W A I.
- **Feasibility:** LMS `search` plus per-type queries (already allowed); plugin searches through their menus. Recents stay on the device (never sent to the gateway).
- **Effort:** S–M. **Priority:** P2.

**U8. Car mode**
- **What:**
  - a driving screen with huge art, three giant buttons (previous, play/pause, next), a "Start radio" button, a big clock-style elapsed time, voice search, and long-press play to stop;
  - no lists deeper than one level;
  - opens automatically on car Bluetooth (Android: an optional trigger by Bluetooth device).
- **Why:** Plexamp driving mode; Spotify car view.
- **Platforms:** W (useful for mounted tablets) A I.
- **Feasibility:** client only.
- **Effort:** S–M. **Priority:** P2.

**U9. CarPlay (iOS) and Android Auto artwork**
- **What:**
  - CarPlay: the same tree as Android Auto (Home, Library, Downloads), the Now playing template, and a rate button when ratings exist.
  - Android Auto: browse artwork through a content provider granted per URI.
- **Why:** platform parity; Plexamp and Spotify.
- **Platforms:** A I.
- **Feasibility:**
  - CarPlay needs Apple's audio entitlement and `CPTemplate` work on the shared tree grammar (`mobile/src/auto.rs`).
  - The Android Auto artwork provider needs careful security tests: grant per URI, no export.
- **Effort:** M each. **Priority:** P2 (CarPlay comes with the iOS app).

**U10. All players at a glance**
- **What:** a "Rooms" view: each permitted player as a card with art, title, volume slider, power, sleep remaining and next alarm. Tap to control, drag one card onto another to sync, and long-press for Transfer.
- **Why:** Material's player list; Roon zones; Sonos rooms.
- **Platforms:** W A I (tablet and desktop first).
- **Feasibility:** existing `players` and `status` subscriptions; the gateway already filters by permission.
- **Effort:** M. **Priority:** P2.

**U11. System surfaces**
- **Android:**
  - a second, larger resizable widget (art, title, a waveform progress line, transport);
  - "Start radio" and "Sleep 30 min" app shortcuts;
  - Material You dynamic colour as a theme option;
  - Wear OS remote (already planned).
- **iOS:**
  - lock-screen and home-screen widgets;
  - Control Centre controls;
  - App Intents/Shortcuts ("Play my morning playlist on Kitchen");
  - a Live Activity showing a running sleep timer or a Transfer in progress.
- **Web:** manifest shortcuts (Resume, Radio, Search), full Media Session actions, and keyboard shortcuts (space, arrow keys, `/` for search, `L` for lyrics).
- **Why:** Doppler's interactive widgets; Symfonium's widget range; Spotify's shortcuts.
- **Feasibility:** client only. The Shortcuts and widget actions call the same player controller (D20).
- **Effort:** S–M each. **Priority:** P2.

### 4.3 Discovery

**X1. Library stations**
- **What:** a Stations shelf of one-tap radios over your own library, each never-ending (with DSTM) or as a fixed 50-track mix:
  - **Time Travel:** oldest to newest decade;
  - **Decade** and **Genre** radio;
  - **Deep cuts:** tracks with low play counts from albums you like;
  - **Forgotten favourites:** high play count, not played recently;
  - **Fresh:** recently added, never played;
  - **Random album;**
  - **Artist mix builder:** pick 2–5 artists, then similar music between them.
- **Why:** Plexamp library stations; Symfonium mixes; Roon Radio.
- **Platforms:** W A I.
- **Feasibility:**
  - LMS **Random Mix** (`randomplay tracks|albums|year|contributors` with genre and year selection) covers Random and the genre and decade radios.
  - Play-count filters need `titles`/`albums` sorts and filters (LMS 9.1 sorts). The client assembles a batch and keeps it topped up, like a DSTM provider.
  - AudioMuse-AI and DSTM handle "similar to" (already integrated).
  - *Gateway:* allow-list `randomplay` parameters as a fixed grammar (it's partly reachable through plugin menus today).
- **Effort:** M. **Priority:** P1 for Random, Decade and Genre; P2 for the rest.

**X2. Sonic path ("from this to that")**
- **What:** pick a start and an end track; the queue drifts gradually from one to the other.
- **Why:** Plexamp Sonic Adventure.
- **Platforms:** W A I.
- **Feasibility:**
  - Needs similarity vectors. AudioMuse-AI (self-hosted) has path-style features. **Check** whether its LMS plugin exposes them as a menu we can discover.
  - Otherwise this is out of reach without our own embedding analysis (a big job: the analysis store would grow).
  - Only shown when available.
- **Effort:** S if AudioMuse-AI exposes it; L otherwise. **Priority:** P3.

**X3. Smart Shuffle and DJ modes**
- **What:**
  - "Shuffle +" on a playlist or album: every few tracks, insert one similar track (visibly marked, with a one-tap remove and "add to playlist").
  - "DJ styles" as queue policies:
    - *Same era* (year ±3);
    - *Stretch* (a similar track between each pair);
    - *Artist focus*.
- **Why:** Spotify Smart Shuffle; Plexamp Guest DJ.
- **Platforms:** W A I (applies to whichever player is being controlled).
- **Feasibility:**
  - The client inserts with `playlist insert` using AudioMuse-AI or DSTM "similar" answers (already integrated) and `titles year:`.
  - It must be resilient when the queue changes elsewhere: a single controller "owns" the mode, with a visible indicator.
- **Effort:** M. **Priority:** P3.

**X4. Credits you can tap**
- **What:**
  - On track and album info, list every role LMS knows (composer, conductor, band, track artist, album artist and other roles), each tappable to that contributor's works *in that role*.
  - On artist pages, "Appears as: Composer (12) · Conductor (3)".
- **Why:** Tidal and Roon credits.
- **Platforms:** W A I.
- **Feasibility:** LMS contributor roles are in the database, and role filters are already used in browse. MAI can add Discogs or MusicBrainz credits for releases.
- **Effort:** S–M. **Priority:** P2.

**X5. Artist and album stories, and booklets**
- **What:**
  - Artist pages: portrait (MAI `artistphoto`), biography and related artists.
  - Album pages: review (MAI `albumreview`) and **"Booklet"**: PDFs and text files from the album folder, viewed in the app.
- **Why:** Qobuz booklets; Roon; Material's MAI integration.
- **Platforms:** W A I.
- **Feasibility:**
  - MAI's `biography`, `albumreview`, `artistphoto` and `localfiles`.
  - *Gateway:* allow-list them with id-only parameters.
  - Booklets are the hard part: `localfiles` takes a *folder path* and returns file URLs, and we never expose paths (SPEC).
  - The gateway would need its own album-id → booklet lookup: resolve the folder server-side, fetch only listed files of allowed types and sizes, serve them under opaque ids, and strip paths.
  - PDFs are rendered with the platform viewer (pdf.js would be third-party code on the web, so consider images or a download instead).
  - Needs path-traversal and type-confusion negative tests.
- **Effort:** S (bios and reviews), M–L (booklets). **Priority:** P2 for bios, P3 for booklets.

**X6. Ratings and loving**
- **What:**
  - a heart or 1–5 stars (user's choice) on Now playing, the queue and the lock screen or notification, and in Android Auto/CarPlay;
  - filters and stations that use ratings.
- **Why:** Plexamp's rate button in the car; Material and Ratings Light.
- **Platforms:** W A I.
- **Feasibility:**
  - Read: the LMS rating tag in `songinfo`/`titles`.
  - Write: the **Ratings Light** CLI, when installed (discovered dynamically; hidden otherwise).
  - *Gateway:* a fixed `ratingslight setrating` grammar (track id and a bounded value).
  - Offline: queue ratings and sync them back like plays (closes an A5 gap).
  - Without the plugin, offer "Add to favourites" as the "love" action.
- **Effort:** M. **Priority:** P2.

**X7. Smart filters and smart playlists**
- **What:**
  - A rule builder: field, operator and value.
    - Fields: genre, year, decade, artist, role, rating, play count, last played, added, format, sample rate, duration, compilation.
    - Rules combine with all or any, plus sort and limit.
  - Save a filter as a Library view or Home shelf, play or shuffle it, or **make it a download rule** (feeds smart downloads).
- **Why:** Symfonium smart filters; Marvis Smart Rules; Roon Focus; Plexamp smart playlists.
- **Platforms:** W A I.
- **Feasibility:**
  - LMS queries filter by genre, year, artist, role and library, with LMS 9.1 sorts. Play-count, rating and last-played *ranges* aren't all filterable server-side.
  - Options:
    - (a) a client-side evaluator over a cached track index (the Rust `core` holds the rule engine, with no I/O);
    - (b) use **Virtual Library Creator** or **Dynamic Playlists** when installed;
    - (c) a gateway query endpoint that translates a rule AST into allowed LMS queries.
  - Start with (a) for small and medium libraries; measure memory.
  - Rules saved per user on the gateway, size-capped.
- **Effort:** L. **Priority:** P2 (a simple version), P3 (full).

### 4.4 Audiophile

**H1. Quality badges and a signal-path chip everywhere**
- **What:**
  - a compact badge on Now playing ("FLAC 24/96", "Hi-Res", "Lossy 320", "DSD64"), coloured by the signal path's verdict (bit-perfect, lossless, resampled or lossy), and tappable to open the signal-path panel;
  - small badges on album tiles (Hi-Res);
  - a web signal path, limited to what the browser lets us know: decode lossless; output rate per `AudioContext.sampleRate`; "the browser may resample".
- **Why:** Apple's badges; Roon's signal path light.
- **Platforms:** W A I.
- **Feasibility:** LMS tags for type, sample rate, sample size and bitrate are already fetched for analysis. Android has the panel.
- **Effort:** S. **Priority:** P1.

**H2. Per-output EQ profiles and quick switching**
- **What:**
  - EQ profiles bound to output devices (this headphone, this DAC, car Bluetooth), switched automatically;
  - an EQ curve preview with the AutoEq target;
  - an A/B toggle;
  - a "loudness-compensated" volume option.
- **Why:** Symfonium, Neutron and UAPP; already planned in A8.
- **Platforms:** A I.
- **Feasibility:** client and Rust `core`. Output identification per platform (USB product, Bluetooth name kept only on the device).
- **Effort:** M. **Priority:** P2.

**H3. Rolling cache for streaming**
- **What:** keep the last *N* GB of streamed tracks (original quality) as an automatic cache, so replays and flaky connections cost nothing. The signal path says "from cache".
- **Why:** Symfonium's rolling cache; Plexamp's pre-caching.
- **Platforms:** A I (W: not with the current Slimproto stream model).
- **Feasibility:**
  - Streaming goes through LMS (Slimproto), which decides what to send, so a cache hit would require LMS to play the local copy instead.
  - Feasible in the engine only if it can match the stream to a cached file by track id and verify it (hash or size) before substituting. That interacts with sync and seeking.
  - Investigate first; may be too invasive. Pre-downloading the next track for the *offline* queue is easier.
- **Effort:** L. **Priority:** P3.

**H4. Album-aware everything**
- **What:**
  - an "Album mode" switch on shuffle (shuffle by album, never inside one);
  - gapless-sequence indicators (a small link icon between gapless tracks in the queue);
  - per-album levelling noted in the signal path.
- **Why:** audiophile expectations (Roon, Plexamp's album awareness).
- **Platforms:** W A I.
- **Feasibility:** LMS shuffle by album (`playlist shuffle 2`) is already a mode. Gapless detection reuses the smart-transition logic.
- **Effort:** S. **Priority:** P2.

### 4.5 Proposal summary

| # | Proposal | Platforms | Feasibility (LMS / gateway / client) | Effort | Priority |
|---|---|---|---|---|---|
| U1 | Sleep timer (+ end of track, sleep all) | W A I | LMS `sleep`, already allowed; client only | S | P1 |
| D1 | Ambient blurred-art background | W A I | Client only | S–M | P1 |
| D5 | Up next card and transitions | W A I | Client only (`status`) | S | P1 |
| U3 | Mini player gesture model | W A I | Client only | S | P1 |
| H1 | Quality badges and web signal path | W A I | LMS tags; client | S | P1 |
| D3 | Synced lyrics | W A I | LMS tags + MAI; gateway allow-list; LRC parser in core | M | P1 |
| U5 | Customisable Home + new shelves | W A I | LMS 9.1 sorts; optional gateway prefs doc | M | P1 |
| X1 | Library stations | W A I | Random Mix, DSTM, AudioMuse-AI; `randomplay` grammar | M | P1/P2 |
| U2 | Alarms | W A I | LMS `alarm`/`alarms`; new gateway grammar | M | P2 |
| X6 | Ratings | W A I | Rating tag + Ratings Light; gateway grammar | M | P2 |
| D4 | Visualiser | W A I | FFT in core; client | M | P2 |
| U8 | Car mode | W A I | Client only | S–M | P2 |
| U9 | CarPlay; AA artwork | A I | Shared auto tree; content provider | M | P2 |
| X4 | Tappable credits | W A I | LMS roles (+ MAI) | S–M | P2 |
| X5 | Bios and reviews (booklets later) | W A I | MAI; gateway id-only grammar; booklet resolver | S / M–L | P2 / P3 |
| U4 | Queue sections and list swipes | W A I | Client | S–M | P2 |
| U10 | Rooms (all players) view | W A I | Existing subscriptions | M | P2 |
| U6, U7 | Library chips; search UX | W A I | Client | S, S–M | P2 |
| D2 | Now playing layouts | W A I | Client | M | P2 |
| D6 | Haptics vocabulary | A I (W partial) | Client | S | P2 |
| H2 | Per-output EQ profiles | A I | Core and client | M | P2 |
| H4 | Album-aware shuffle and gapless markers | W A I | LMS shuffle modes | S | P2 |
| U11 | Widgets, Shortcuts, Control Centre, keys | W A I | Client | S–M | P2 |
| X7 | Smart filters and playlists | W A I | Client rule engine in core; optional plugins | L | P2/P3 |
| D7 | Stats and "your year" | W A I | LMS counts; opt-in gateway play log | L | P3 |
| X3 | Smart Shuffle and DJ styles | W A I | AudioMuse-AI/DSTM + client queue policy | M | P3 |
| D8 | Guest queue (Jam) | W | New guest credential; heavy security work | L | P3 |
| X2 | Sonic path | W A I | Depends on AudioMuse-AI | S/L | P3 |
| H3 | Rolling cache | A I | Engine and LMS interplay; investigate | L | P3 |

---

## 5. Accessibility and performance

**Motion and visuals**
- **Reduced motion:** the animated background (D1), visualiser (D4), transitions (D5) and lyric auto-scroll all honour `prefers-reduced-motion`, Android's "Remove animations" and iOS's Reduce Motion. They fall back to static blur and a highlighted line without smooth scrolling.
- **Contrast over art:** text over D1 backgrounds uses a scrim, and the contrast check runs against the *actual* extracted colours at runtime, not just theme tokens. We already test theme legibility. High contrast disables the background effect.
- **No flashing:** visualisers stay below three flashes a second at full-screen luminance change (WCAG 2.3.1), with an "intensity" setting and a calm default.

**Input and assistive technology**
- **Gestures always have an alternative:** every swipe (U3, U4, D2) has a visible button or menu item, a TalkBack or VoiceOver custom action, and a keyboard shortcut on the web. Thresholds respect accessibility touch settings.
- **Haptics:** off when the system has haptics disabled, and never the only feedback.
- **Lyrics:** the text is real text (selectable and readable by screen readers), sizes up to 200 %, and the current line is announced only on request (no live-region spam).
- **Sleep timer and alarms:** large, labelled pickers, times read in the user's locale, and a 24-hour option.
- **Car mode:** targets of at least 76 dp and one-handed reach; respects Android Auto and CarPlay driver-distraction rules (list depth and lengths).
- **Ratings:** controls labelled "Rated 4 of 5", adjustable with swipe up and down in TalkBack and VoiceOver.

**Performance**
- **Battery and CPU:** D1 and D4 pause when hidden, in background or on battery saver; the frame rate is capped (30 fps visualiser, 15 fps ambient); and colours come from ≤128 px images. The FFT is computed only while visible, on the audio thread's copy, and never blocks output.
- **Web:** the lyrics parser and visualiser FFT run in the existing worker or WASM. View Transitions are used only where supported.
- **Network:** lyrics, bios and badges are fetched lazily and cached per track id. Home shelves load progressively, with skeletons. Station top-ups request small batches.
- **Data and storage:** lyrics and booklets in downloads count towards storage limits. The stats play log is bounded by retention.

**Security as a performance constraint**
- New gateway grammars (lyrics, alarms, ratings, randomplay, MAI) are fixed and bounded, so a slow plugin can't stall a gateway worker. Use the existing time limits.

---

## 6. Suggested roadmap

### Wave 1: quick wins with high impact (≈ 2–3 weeks)

The aim is to make it *feel* premium, and to add the LMS utilities people expect.

| Item | Proposal | Notes |
|---|---|---|
| Sleep timer | U1 | The gateway is ready; the UI is on three platforms |
| Ambient background | D1 | Static blur first, then animated |
| Up next chip and art transitions | D5 | |
| Mini player gesture model | U3 | Bring the web up to Android's level |
| Quality badges and web signal path | H1 | |
| Library stations, first set (Random, Decade, Genre, Fresh) | X1 | Random Mix plus LMS 9.1 sorts |
| Home: reorder and hide shelves, plus On this day and Most played | U5 (local storage) | Gateway sync later |
| Synced lyrics (embedded first, then MAI) | D3 | LRC parser with fuzz tests |

### Wave 2: depth (≈ 4–6 weeks)

The aim is power users, the car and the daily routine.

- **The daily routine:** alarms (U2) and ratings (X6), including offline sync-back.
- **The car:** car mode (U8), Android Auto artwork, and CarPlay with the iOS app (U9).
- **Now playing:** the visualiser (D4) and layouts (D2).
- **Players:** the Rooms view (U10).
- **Library:** tappable credits (X4), MAI bios and reviews (X5 part 1), library chips and search UX (U6, U7), and queue sections and list swipes (U4).
- **Platform:** per-output EQ profiles (H2), album-aware shuffle (H4), haptics (D6) and system surfaces (U11).
- **Simple smart filters (X7 v1):** genre, year, rating and play count, saved as shelves and download rules.

### Deferred

- **Timed lyrics from LRCLIB (2026-10-13, deferred):** many tracks only have plain lyrics, which can't follow the music. The gateway could ask LRCLIB, a free open database of timed lyrics, by artist, title, album and duration, and cache the answer. The apps already handle timed lyrics. It would be an admin setting, off by default, because it sends what is played to an outside service. Judged overkill for now.

### Wave 3: signature features (larger, needing design reviews)

The aim is features nobody else offers for LMS.

- Full smart playlists (X7) and gateway-synced Home layouts.
- "Your year" stats with an opt-in play log (D7).
- Smart Shuffle and DJ styles (X3); sonic path if AudioMuse-AI supports it (X2).
- Guest queue / Jam (D8), after a threat model.
- Album booklets (X5 part 2), after a security design for path-free folder resolution.
- Investigate the rolling cache (H3).

**Every wave** follows the standing rules: tests first (negative tests for each new gateway grammar, fuzz tests for each new parser), British English strings, and no real library or player names in fixtures or screenshots.
