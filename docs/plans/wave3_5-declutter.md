# Wave 3.5: declutter (Android and web)

Started on 2026-10-14, from a rendered review of every screen on both apps: phone, small phone, tablet and desktop, light and dark, against the demo library. The finding is the same on both. The features are good, but each wave added its control **beside** the existing ones rather than **instead of** them. Almost every fix is a deletion, a merge or a move, not a redesign. No feature is lost: anything taken off a screen gets a named new home.

## Decisions (owner, 2026-10-14)

- **All three phases** go ahead. Phase 3's merges start with a short design section written here before they are built.
- **Ratings:** one favourite/rating toggle by the title on Now playing; the full 1–5 rating stays in ⋮.
- **The "Playing on" strip goes.** The player shows in the mini player; moving playback lives in *Play on…*; a strip appears only while a move is in progress.

## Rules both apps follow from now on

1. **One status slot.**
   - Lasting states (out of reach, busy, reconnecting, moving playback) share one line at the top, in priority order: moving > out of reach > busy > reconnecting.
   - Which player you control is shown *in* the mini player ("on Lounge"), never as an extra band.
   - Offers ("Continue on…") are one-line banners with an action. Results are snackbars.
   - Nothing else stacks.
2. **One mark per meaning.**
   - A single "available offline" mark is shown on track rows when the track is downloaded.
   - The "On this device" cache mark appears only while out of reach. While the server is reachable, the cache is invisible.
   - Quality shows on Now playing only. Gapless shows in track info only.
   - ✕ means close or dismiss and nothing else.
3. **At most two buttons and ⋮.**
   - Page headers: Play, Shuffle and ⋮ (plus Download where it applies).
   - Rows: tapping plays or opens, and the trailing ⋮ is the menu. A row has at most one trailing control besides a drag handle.
   - Menus never open other menus. Destructive actions live only inside ⋮.
4. **One way in.** Each feature has one primary entry point. Shortcuts are allowed only where a competitor would have them (the queue from Now playing, for instance).
5. **Settings is an index.**
   - Seven or eight top-level rows, each with a one-line summary of its current value.
   - Option lists open a picker instead of being laid out inline.
   - At most one sentence of help per control.
   - Engine and diagnostic options go under Advanced.
6. **One word per concept**, the same in both apps (see *Naming* below).

## Bugs found by the renders (being fixed now, ahead of the plan)

| App | Bug |
|---|---|
| Android | Queue title collapses to a vertical "Q/u/e/u/e" on phones when the header has three actions |
| Android | The out-of-reach/busy line on Now playing is painted over by the blurred cover background (unclipped, about 1.4× scale) |
| Android | The ⋮ wraps onto its own line on album and playlist headers |
| Android | The selected Library chip can be scrolled off-screen |
| Web | Navigating inside the app while the server is out of reach reloads the page into the bare "Can't reach the music server" splash |
| Web | While the music server is busy, Now playing says "Nothing to control yet" |
| Web | Out of reach, Download, ⋯ and the "Playing on" strip stay live while Play is greyed, and nothing says why |

## Phase 1: quick wins (each under a day; both apps)

| # | Change | Android | Web |
|---|---|---|---|
| 1 | **Remove the "Playing on X · Bring here · Back to this device" strip.** The player name moves into the mini player; moving playback moves into *Play on…*. The strip is kept only while playback is moving. | ✓ (about 96 dp on every screen) | ✓ (the most saturated element, on every screen) |
| 2 | **Page headers become Play · Shuffle · Download · ⋯.** Play next, Add to queue and the download format move into ⋯; the format default lives in Settings › Downloads. | ✓ | ✓ (album header wraps to three rows on a phone) |
| 3 | **Queue tidy.** Header actions go into ⋯. Rows lose the ✕ and the Move up/down buttons (into ⋯ and drag). The 🔗 gapless marks go. | ✓ | ✓ (about 6 targets per row become 2) |
| 4 | **Now playing tidy.** One favourite/rating toggle instead of five stars (the full rating stays in ⋮). The *Up next* card becomes a one-line link. The bottom row is Play on · Lyrics · Queue. ⋮ is trimmed to about four items. | ✓, and hide the tab bar | ✓; desktop: hide the bottom player bar (a second full transport) and cut the action row from 9 buttons to Queue + ⋯ |
| 5 | **Home.** No bin icons on station cards (delete goes into ⋮). *Edit Home* becomes a quiet text button. | ✓ | ✓ |
| 6 | **Continue on…** becomes a one-line banner with a player picker. | ✓ (replaces a block about 230 dp tall) | ✓ (becomes a snackbar with an action) |
| 7 | **Marks.** One offline glyph; the cache mark only while out of reach; ✕ only for close. | ✓ | ✓ |

## Phase 2: structure (a few days each)

| # | Change | Notes |
|---|---|---|
| 8 | **Settings index with sub-pages.** Account · Appearance (web) · Playback · Now playing · Downloads · Listening history · This device (Android) · About, with Advanced under About. Inline radio lists become value rows with pickers (one shared component). | Android: about 45 rows and 21 inline radio lists become 7 rows. Web: one scroll of about 5,900 px with 16 cards becomes one screen. Account asks for the password once per action, not three times. |
| 9 | **One status-slot component** per app (rule 1), replacing the six or seven separate channels each has today. | |
| 10 | **Library: one model.** One section list (Albums, Artists, Playlists, Genres, Years, Folders, Stations…). New music and Random become sorts. The duplicate chips/pills and the second album browser go. The web's library select moves into ⋯; `/albums` and `/artists` redirect. | Removes about 420 px of controls above the first album on the web |
| 11 | **Desktop web navigation:** the sidebar goes from 14 items to about 7 (Library's seven repeat entries go). The Now playing icon is fixed. | |
| 12 | **Play on…:** tapping a row switches player. Move playback is a secondary action with a correct label. Grouping becomes one *Group players…* row. The paragraph goes. | |

## Phase 3: merges (larger; schedule after phases 1 and 2)

| # | Change | Notes |
|---|---|---|
| 13 | **Rooms folds into *Play on…*** (one player manager instead of two; Rooms remains as the expanded view on tablet and desktop). Card controls go from 6 to 3. | Needs a short design first |
| 14 | **Downloads playback uses the main mini player and Now playing** (the engines stay separate; the UI is one). Downloads then appear in the main Queue. | Removes the "two apps" feel offline |

## Phase 3 design (Android, 2026-10-14)

Written before building items 13 and 14, as the decisions asked.

### 13. Rooms folds into *Play on…*

- ***Play on…* is the one player manager on phones.** Each player is one row: a tap controls it (as in phase 2). Its ⋮ holds what Rooms' card had: *Move playback to <player>*, *Play <player>* or *Pause <player>*, *Alarms* and *Switch <player> on/off*. The controlled player, and any player that is playing, has its volume slider under its name. Sync stays one *Group players…* row (phase 2); a grouped player says who it plays with.
- **The data:** the sheet reads every player's status, alarms and volume through the same view model Rooms uses, only while it is open (the 5-second refresh and the 20-player cap stay).
- **Rooms stays as the expanded view on large windows** (a rail or drawer instead of the tab bar). *All rooms* is offered in *Play on…* there only; on a phone the sheet has everything.
- **A Rooms card goes from six controls to three:** play/pause, the volume slider and ⋮. Power moves into ⋮ (*Switch Kitchen off*); the drag handle goes (drag the cover onto another card instead; *Sync with…* and the TalkBack actions stay); a tap on the card still controls the room and a long press still moves playback there.
- **Now playing's ⋮ loses Alarms and Power:** they are the player's row in *Play on…* (one way in).
- **The one status slot** carries a move in progress for Rooms too (Rooms' own band goes).

### 14. Downloads playback through the main mini player and Now playing

- **Two engines, one face.** The downloads engine (`OfflinePlayerRepository`) stays its own. An adapter (`DownloadsPlayer`) shows what it plays as the main screens read a player: a player called *Downloads*, its status (track, position, shuffle, repeat), its queue in play order, and covers from the phone.
- **The main mini player** shows downloads while they play ("artist · Downloads"), with play/pause, next, previous and *Play on…*; a tap opens the main **Now playing** (cover, lyrics and info pages, waveform-less seek bar, transport, Up next, Play on · Lyrics · Queue, the visualiser and the phone's volume in ⋮). LMS-only controls are left out for downloads: the rating, the sleep timer, the signal path row, alarms and power. **Stop** (end the downloads queue) is in ⋮.
- **Downloads appear in the main Queue tab** and the queue sheet: a row plays from that track; the queue can't be edited (no clear, move or remove), as the engine has no such operations.
- **Offline mode** (the server out of reach, the whole app) uses the same mini player and Now playing; the downloads-only Now playing and mini player go.
- The old route to the downloads' Now playing opens the main one, so notification taps and shortcuts still land.

## Phase 3 design (web, 2026-10-14)

Written before building items 13 and 14. It follows the Android design (merged separately) wherever the web can, so the two apps match; the differences are listed.

### 13. Rooms folds into *Play on…* (web)

- ***Play on…* is the one player manager on phones.** Each player is one row: a tap plays on it (phase 1). Its ⋯ holds what a Rooms card had: *Move playback to <player>*, *Play <player>* or *Pause <player>*, *Alarms* (that player's, on the Alarms page) and *Switch <player> on* or *off*. The volume of the player in use sits at the top of the sheet (phase 1). Sync stays one *Group players…* row; a grouped player says who it plays with.
- **The data:** the sheet and Rooms share one player manager (`lib/rooms/manager.svelte.ts`): every player's status read through `RoomsPoller` only while the sheet or the page is open (every 5 s and on events, at most 20 players), and the same actions (play/pause, volume, power, sync, move).
- **Rooms stays the expanded view on tablets and desktops**, in the sidebar. *Play on…* links to it there only; on a phone (760 px and under) the sheet has everything.
- **A Rooms card goes from six controls to three:** play/pause, the volume slider and ⋯ (*Play on <player>*, *Move playback to <player>*, *Sync with…*, *Play on its own*, *Alarms*, *Switch <player> off*). A switched-off player shows one *Switch on* button in place of play/pause. The grip goes: a pointer drags a card by its cover onto another to sync them, and *Sync with…* in ⋯ is the keyboard and screen-reader way. The instructions paragraph goes. A tap on the card still plays on that room.
- **Now playing's ⋯ loses Alarms:** it is the player's row in *Play on…*, and a Rooms card's ⋯ (one way in per place).
- **The one status slot** carries a move in progress for Rooms too.
- **Differences from Android:** the web has no long press to move playback on a card (it is in ⋯); the controlled player's volume is at the top of *Play on…* rather than under its row.

### 14. Downloads playback through the main mini player and Now playing (web)

- **Two engines, one face.** The downloads engine (`lib/offline/player.svelte.ts`) stays its own. An adapter (`lib/player/now.svelte.ts`) gives the main screens one view of what plays: from the downloads engine while its queue is active, otherwise from the selected LMS player. `controls` sends play/pause, next, previous, seek and a queue jump to the downloads engine while it plays.
- **The main mini player** shows downloads while they play (the title, then "Downloads · artist"), with play/pause, next and previous (swipes too) and *Play on…*; a tap opens the main **Now playing**: the cover from the device, the title, artist and album, a plain seek bar (no waveform), the transport, Up next from the downloads queue, and *Play on · Lyrics · Queue*. LMS-only controls are left out for downloads: the rating, shuffle and repeat, the sleep timer, the quality badge and signal path, This track, Car mode and Alarms. **Stop playing downloads** (ending the downloads queue) is in ⋮.
- **Downloads appear in the main Queue** (the page and the sheet): a row plays from that track; the downloads queue can't be edited there (the engine has no such operations), so no ⋯, handles or clear.
- **The separate downloads bar goes** from the signed-in app. Offline mode's splash (a device where nobody is signed in, with no server to sign in to) keeps its own small bar: there is no main app around it to show.
- **Differences from Android:** web downloads have no visualiser (the browser's audio element feeds no analyser) and no shuffle or repeat (the web engine plays its queue in order).

## Naming (one term each, both apps)

| Concept | Today | Use |
|---|---|---|
| Choosing a player | Play on…, Choose a player, Playing on, This device | **Play on** (the action); the player's name (the state) |
| Moving playback | Move here, Bring here, Bring to this phone, Back to this phone, Move playback here, Transfer | **Move playback to …** |
| The server | LMS, music server, your server | **your music server** (LMS only in Advanced) |
| Downloads | Downloads, Downloaded, Downloaded music, Downloads and storage, On this phone | **Downloads** (the page); **Downloaded** (the state) |
| Smart filters | Filters, Smart filters | **Smart playlists** |
| Listening history | Your listening, Listening history, History | **Your listening** (the stats page); **Listening history** (the setting) |
| Play all | Play, Play all | **Play** |
| No player chosen | four wordings | **No player chosen · Choose a player** |

## Accessibility (rides along)

- Web: restore input focus rings (removed in `app.css`).
- Web: targets of at least 44 px on touch.
- Web: one switch component; power state in text, not colour alone.
- Android: keep the TalkBack custom actions when buttons move into menus.

## Tests

- Each phase updates the screenshot tests: Roborazzi on Android, the docs screenshots on the web.
- Each phase adds layout assertions for the rules above: a header never wraps, a row has at most one trailing control, and at most one status line shows.
- Moved actions keep their existing behaviour tests, now reached through ⋮.

## Source reviews

The full per-screen reviews, with file and line references and screenshots, were written outside the repository. They show the demo library only.
