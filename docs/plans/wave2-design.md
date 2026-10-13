# Wave 2 UX design: agreed details

Agreed with the owner on 2026-10-12, from `docs/plans/ux-research.md` (Wave 2). Web and Android together; iOS takes the same design when its app is built.

## Order

1. **Daily routine:** alarms (U2) and ratings (X6).
2. **Library and queue:**
   - tappable credits (X4);
   - library chips, sort and A–Z (U6);
   - search (U7);
   - queue sections and list swipes (U4);
   - album-aware shuffle (H4).
3. **Now playing and players:**
   - the visualiser (D4);
   - Now playing layouts (D2);
   - the Rooms view (U10).
4. **Car, platform and filters:**
   - car mode (U8) and Android Auto artwork (U9);
   - per-output EQ (H2);
   - haptics (D6);
   - system surfaces (U11);
   - simple smart filters (X7 v1).

## 1. Alarms (LMS's own)

- **What:** an Alarms page for each player:
  - time; days (or once); volume; what to play; shuffle; on/off;
  - an "all alarms on/off" switch for the player.

  The next alarm shows on the player's row in *Play on…* and in the Rooms view. Alarms are LMS's own, so they also ring on hardware players and with no app open.
- **Who:** anyone who may control the player (admins: every player). Every other player is refused, with negative tests.
- **Gateway grammar:**
  - **Read:** `<player> alarms <start> <count> filter:all`. LMS's `url` is never returned: each alarm's sound is described as a category and title, matched against the player's sound list.
  - **Change:** `<player> alarm add|update|delete|enableall|disableall …` with strict parameters:
    - `id` (LMS's own, 8 hex; 1–32 lowercase hex accepted);
    - `time` (0–86 399 seconds after midnight);
    - `dow` (digits 0–6, comma-separated, each once);
    - `enabled`, `repeat` (0/1);
    - `volume` (0–100);
    - `shufflemode` (0, 1, 2).
  - **Sounds** are never URLs from the client. The gateway lists the player's choices from LMS (`alarm playlists`: current queue, favourites, saved playlists, random mixes, natural sounds), gives each an opaque id (a salted hash of LMS's URL), and maps an id back to LMS's own URL only by looking the list up again. A URL LMS doesn't offer can't be set.
- **Endpoints:**
  - `GET /v1/players/{player}/alarms`: the alarms and sounds;
  - `POST`, `PATCH`, `DELETE` on `…/alarms[/{id}]`;
  - `PUT …/alarms/enabled`.

  Typed JSON, rather than raw RPC, so the grammar lives in one place.

## 2. Ratings: per user, blended with LMS's shared rating

- **What:** stars (½ to 5, in half steps) or a heart, as each user chooses in Settings (a heart is 5 stars).
  - **Where:** on Now playing, track menus, the queue, the lock screen/notification, and Android Auto.
  - **Uses:** shelves, stations and (Wave 2.4) smart filters.
- **Storage:**
  - **Per user on the gateway** (like listening history): track key, rating 0–100, time. Private to the user; view and clear in Settings; deleted with the account. Up to 100 000 per user.
  - **Also written to LMS's shared rating** through Ratings Light, where it is installed (`ratingslight setratingpercentnoclient`). The last user to rate a track sets the shared value.
  - **Reading LMS's shared rating:** tag `R` in `titles`/`songinfo`, which works without the plugin, for ratings set elsewhere (Material, other apps).
- **Blending:** where both exist, the user's own rating counts **×3** and LMS's shared rating ×1 (as history). Where only one exists, that one. The weights are tunable in code.
- **Offline:** ratings made offline are queued and sent back with plays (idempotent by a client key, as `/v1/plays`).
- **Endpoints:**
  - `PUT /v1/me/ratings/{track}`: 0–100, in steps of 10; 0 clears;
  - `GET /v1/me/ratings?tracks=…`: at most 200 ids;
  - `POST /v1/me/ratings`: an offline batch, keyed;
  - `DELETE /v1/me/ratings`: all of them.

## 3. Library and queue (part 2)

Detailed on 2026-10-13. Web and Android together. The gateway needs one change: `role_id` is now allowed on `titles`, with the same fixed role list as `albums` and `artists`. Everything else uses commands that are already allowed.

### 3.1 Tappable credits (X4)

- **Track credits:** track info lists every role LMS reports for the track (`tags:AS`): album artist, artist, track artist, composer, conductor and band. Each name is its own link (and its own TalkBack/VoiceOver item).
- **Following a credit:** a name opens that contributor's page *in that role*: the albums where they are, say, the composer (`albums artist_id:… role_id:COMPOSER`), with the role in the title ("Bach · Composer").
- **Artist pages:** under the name, "Appears as" chips for each role with a count, for example *Composer (12) · Conductor (3)*. The counts come from `count` on a one-item `albums` query per role; roles with none are hidden. The default view is unchanged.
- **Out of scope:** MAI credits (X5) stay out of this part.

### 3.2 Library chips, sort and A–Z (U6)

- **Type chips:**
  - **Albums, Artists, Playlists and Favourites** on both apps.
  - **Downloaded** on Android only (the web has no downloads).
- **Album sort:** Recently added (`new`), Recently played (the user's own, from their listening history on the gateway; LMS's `recentlyplayed` is server-wide, so it would show other people's listening, and stays refused), A–Z (`album`), Artist (`artistalbum`) and Year (`yearalbum`).
- **Grid or list:** a toggle for albums.
- **A–Z fast scroller:** for albums sorted A–Z and for artists. It is built from LMS's `indexList` (`tags:Z`: each first character and its count), so a letter jumps straight to its offset without loading the whole list. Using it, it shows a letter bubble, and it is reachable with TalkBack and the keyboard.
- **Remembered choice:** the chip, sort and view are remembered per device.

### 3.3 Search (U7)

- **Recent searches:** kept on the device only, never sent to the gateway. At most 20, each removable, with "Clear all".
- **Grouped results:** one `search` (`extended:1`) gives artists, albums and tracks. Playlists come from `playlists search:`.
- **Top result:** an exact (or else prefix) name match, preferring an artist, then an album, then a track.
- **Filter chips:** All, Artists, Albums, Tracks and Playlists. Picking a type pages through that type alone.
- **Playing from results:** a track plays from its row. An album row has a play button. A long press or the menu offers Play next and Add to queue.
- **Out of scope:**
  - plugin searches ("Search on …") need each plugin's search menu replayed and are left to a later part;
  - LMS search has no fuzzy matching, so there are no typo suggestions.

### 3.4 Queue sections and list swipes (U4)

- **Queue sections:** *Now playing*, *Up next* and *Then*.
  - **Up next** holds the tracks this device added with Play next. LMS keeps one flat list, so the app tracks those positions itself: per device, best effort, shifted by its own edits.
  - **Resetting:** if the queue changes in a way the app can't account for, the sections are dropped and the queue shows flat.
- **Queue swipes:**
  - right: play next (moved to just after the current track);
  - left: remove, with Undo.
- **Browse list swipes** (tracks and albums), Squeezer style:
  - right: play next;
  - left: add to queue.
- **Alternatives to every swipe:**
  - the row's menu;
  - a TalkBack/VoiceOver custom action;
  - a keyboard shortcut on the web.
- **Swipe feel:** thresholds follow the platform's touch settings, and a swipe is never the only way to do something.
- **Clear upcoming:** removes every track after the current one, after asking. It deletes from the end, using the existing `playlist delete` checked against each entry. For a long queue it offers *Clear all* instead.
- **Queue peek:** on Now playing, a swipe up (and a button) shows the queue in a sheet.

### 3.5 Album-aware shuffle (H4)

- **Shuffle button:** cycles *Off → Tracks → Albums* (LMS shuffle 0, 1, 2). Its label and state say which, and long-pressing it offers the three.
- **Gapless markers:** a small link mark between consecutive queue rows from the same album with consecutive track numbers. It is presentation only, has a label for screen readers, and comes with no promise about the player.

### 3.6 Rules

- **Tests:** negative tests for the new `role_id` on `titles`. Fuzz/robustness tests for any new parsing (`indexList`, search results, credit roles). UI and accessibility tests for every new control.
- **Language and fixtures:** British English; no real names in fixtures (use placeholders).

## 4. Now playing and players (part 3)

Detailed on 2026-10-13. Web and Android together. No gateway change: player status, `power`, `mixer`, `sync`, `syncgroups` and the alarms endpoint are already allowed.

### 4.1 Visualiser (D4)

- **Where it runs:** only while this device is the player (Android's own player or the browser). On a remote player, Now playing keeps the blurred-art background (D1).
- **One analyser for everyone:** `core::visual`, sans-IO and shared by Android (through the mobile engine) and the web (through WASM).
  - **Input:** the decoded PCM the player already has.
  - **Output:** at most 30 frames a second of audio. Each frame holds:
    - 32 log-spaced spectrum bands (40 Hz–16 kHz, 0–1 on a 70 dB scale);
    - 128 oscilloscope points;
    - left and right RMS and peak in dBFS;
    - a bass level (0–1) for the glow.
  - **Implementation:** a small radix-2 FFT (2048 points, Hann window) written in the core. No new dependency.
  - **Bad input:** NaN or infinite samples count as silence. Any input is bounded and never panics.
- **Styles:** *Bars*, *Scope*, *Glow* (the blurred-art background pulsing with the bass) and *VU* (two needle meters with ballistics drawn by the client: about 300 ms rise and fall, peak hold).
  - **Choosing:** swipe across the visualiser, or use its button.
  - **Remembered:** per device.
- **Off unless chosen:** the visualiser is off by default (Settings, or the Now playing layout button).
  - It stops automatically in battery saver (Android) and with *reduce motion* (both apps).
  - It draws at most 30 fps, and only while Now playing is visible.
  - The analysis runs only while something is shown, so it costs nothing otherwise.
- **Accessibility:** decorative. Hidden from screen readers apart from its on/off and style controls.

### 4.2 Now playing layouts (D2)

- **Three layouts:**
  - *Art*: today's layout;
  - *Lyrics*: synced lyrics large (Wave 1);
  - *Info*:
    - the quality badge and signal path;
    - credits (part 2), as links;
    - the rating;
    - an "Up next" card.
- **Choosing:** a layout button (a segmented control), and a horizontal swipe on the art area. Remembered per device. Swipes keep their alternatives (the buttons, TalkBack/VoiceOver actions, and the keys `1`, `2`, `3` on the web).
- **Wide windows** (tablets, desktop, landscape over 840 dp):
  - art on the left, lyrics or info on the right;
  - the queue as a pinned side panel that can be hidden.
- **Visualiser:** where the art is, when it is on.
- **Out of scope:** the MAI biography (X5) is left out, as the server has no MAI.

### 4.3 Rooms (U10)

- **What it shows:** a *Rooms* view of every player the user may control. Each card shows:
  - art, title and artist;
  - play/pause;
  - a volume slider;
  - power;
  - sleep remaining;
  - the next alarm.
- **Where:**
  - **Android:** from Now playing's player chip ("All rooms") and the "Play on…" sheet.
  - **Web:** a Rooms page in the menu, and the player picker.
- **Actions:**
  - **Tap a card:** control that player.
  - **Sync:** drag one card onto another. Every drag has an alternative: a "Sync with…" menu item and a TalkBack/VoiceOver action.
  - **Unsync:** from the card's menu.
  - **Long-press:** *Transfer* (Wave 1).
- **Keeping it fresh:**
  - **While open:** each card's status is read every 5 s (event-driven where events arrive), for at most 20 players.
  - **Alarms:** read once on opening, then when the alarms change.
  - **When it closes:** nothing is polled.
- **Layout:** a grid on wide screens, a list on phones. Each card is one accessible group with its controls labelled with the room's name ("Kitchen volume").

### 4.4 Rules

As before:
- tests at every level;
- robustness tests for the analyser (random, silent, NaN, clipped and odd-rate input), plus timing so it stays cheap;
- UI and accessibility tests for every new control;
- British English;
- placeholder names in fixtures.

## 5 (outline; detailed before it starts)

- **Car mode:** opens automatically when the phone connects to a car's Bluetooth, configurable in Settings:
  - *Off*;
  - *Any car audio*;
  - *This device*, chosen from paired devices.

  This needs Android's `BLUETOOTH_CONNECT` permission (Android 12+). It is asked for only when automatic opening is first used, and the privacy policy says so. Without it, car mode is a button.
- **The rest follow `ux-research.md` §4,** with the usual rules: negative tests for every new gateway grammar, fuzz tests for every parser, British English, and no real names in fixtures.
