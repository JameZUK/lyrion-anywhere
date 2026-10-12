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

## 3–4 (outline; detailed before each part starts)

- **Car mode:** opens automatically when the phone connects to a car's Bluetooth, configurable in Settings:
  - *Off*;
  - *Any car audio*;
  - *This device*, chosen from paired devices.

  This needs Android's `BLUETOOTH_CONNECT` permission (Android 12+). It is asked for only when automatic opening is first used, and the privacy policy says so. Without it, car mode is a button.
- **The rest follow `ux-research.md` §4,** with the usual rules: negative tests for every new gateway grammar, fuzz tests for every parser, British English, and no real names in fixtures.
