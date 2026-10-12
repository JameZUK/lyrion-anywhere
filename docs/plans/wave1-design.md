# Wave 1 UX design: agreed details

Agreed with the owner on 2026-10-12, from `docs/plans/ux-research.md` (Wave 1). This covers the web app and the Android app together, with logic shared in Rust (`core`/`mobile`) or TypeScript where possible. iOS takes the same design when its app is built.

## 1. Sleep timer (every player)

- **Options:**
  - fixed lengths: 15 / 30 / 45 / 60 / 90 minutes, plus a custom length;
  - end of track, end of album, end of queue;
  - **sleep all players**: every player the user may control, or the current sync group.
- **Fade-out before stopping:** customisable per user in Settings, from off to 2 minutes, **30 seconds by default**.
- **How it works:**
  - fixed lengths use LMS's own `sleep <seconds>`, already allowed by the gateway, so the timer also works on hardware players and survives the app closing;
  - "end of track/album/queue" is computed from the queue (remaining time) and also set as an LMS `sleep`, re-armed if the queue changes;
  - this device's own player fades in its engine; other players use LMS's sleep fade where the player supports it.
- **UI:**
  - a moon button in Now playing opens a sheet, and shows the remaining time when set;
  - the mini player shows a small moon;
  - one tap cancels.

## 2. Ambient Now playing

- **Background:** a **slow-moving blurred** version of the album art behind Now playing (Plexamp-style):
  - it falls back to a still blur under *reduce motion*, battery saver, or a low-power device;
  - it never reduces text contrast: a scrim keeps WCAG AA, and the existing contrast tests are extended.
- **Scope:** **Now playing only.** The rest of the app keeps the user's theme, with a subtle art tint on the mini player.
- **Rendering:** art changes cross-fade.
  - **Web:** CSS/Canvas with a small blurred bitmap, no WebGL requirement.
  - **Android:** RenderEffect blur on API 31+ and a pre-blurred bitmap below that.

## 3. "Up next" peek and transitions

- A row **under the transport controls**: "Up next · Title — Artist" with a thumbnail.
  - Tap opens the queue.
  - Swipe the row away to remove that track from the queue, with Undo.
- Art and title animate between tracks: a short cross-fade or slide, disabled under reduce motion.

## 4. One mini-player gesture model (web, Android, iOS)

| Gesture | Action |
|---|---|
| Swipe left / right | Next / previous track, with haptic feedback on phones |
| Swipe up (drag) | Open Now playing; drag down to close |
| Long-press | Open *Play on…* (the player picker) |
| Tap | Open Now playing |

All of these are also available as accessibility actions (TalkBack and screen readers) and keyboard shortcuts on the web.

## 5. Quality badge

- Format and resolution on Now playing, e.g. **FLAC 24/96 · Hi-Res**, or **MP3 320**.
- **Hi-Res** means more than 16-bit or more than 48 kHz, on lossless sources only.
- **Tap opens the signal path:** this already exists on Android; the web app gets an equivalent panel showing source, decoding, DSP/volume, and output mode and rate.

## 6. Library stations (endless, refilled as they play)

- **Built-in stations:**
  - **Decade / Time Travel:** a chosen decade, or "my 80s/90s…".
  - **Genre:** one or more genres.
  - **Forgotten favourites:** often played, not heard for a long time.
  - **Fresh:** recently added, barely played.
  - **Deep cuts:** rarely or never played tracks by artists you play a lot.
- **Custom stations, pinned from any menu:** any mix or generator item, such as AudioMuse-AI "mix like this", a Custom Browse mix or a Random Mix setup, gets **Save as station**. It then appears under Stations and on Home, and is refilled by re-running that same (already learned) action when the queue runs low.
- **Refill:** when the queue has fewer than about 5 tracks left, add another batch, avoiding repeats from the last 200 played.
- **Sources:**
  - LMS Random Mix (`randomplay`), for genre and decade;
  - play counts and history (see 8);
  - LMS album/track queries;
  - AudioMuse-AI or other plugins through their own offered actions.

  The gateway allow-list gains a fixed `randomplay` grammar with negative tests. Pinned plugin actions use the existing learned-action check; they are stored as references to offered actions, never as free commands.

## 7. Customisable Home (per user, stored on the server)

- **Default shelves, in this order:**
  1. **Continue / Recently played**
  2. **Stations**
  3. **New Music / Recently added**
  4. **Most played** and **On this day**
- **Editing:** users can **reorder, hide and add** shelves, including pinned plugin menus, filters and stations.
- **Storage:** the layout is saved **per user on the gateway**, so it is the same on every device. It is a small JSON document per user (versioned, size-capped, validated), behind new `GET/PUT /v1/me/home` endpoints with negative tests.

## 8. Listening history (for the shelves and stations above)

- **Sources, combined and weighted:**
  - **the per-user history on the gateway, weighted higher;**
  - **LMS's server-wide play counts and last-played**, which also cover hardware players.
- **Per-user history:**
  - the gateway records what each signed-in user plays through Lyrion Anywhere: track key, time and seconds heard, the same rule as play counting;
  - **kept 13 months**, then pruned;
  - users can view and clear it, and switch it off in Settings;
  - never shared with other users;
  - documented in `docs/PRIVACY.md`.
- **Scoring example:** "Most played" = user plays × 3 + server plays × 1, over the period. The weights are tunable in code.

## 9. Synced lyrics

- **Sources:**
  - lyrics in the files' tags, plain or LRC;
  - the **Music & Artist Information** plugin, when installed.

  No third-party online lyrics services are used by us.
- **Full-screen view in Now playing:**
  - the current line is highlighted and auto-scrolls;
  - tap a line to seek there;
  - plain (unsynced) lyrics scroll normally.
- **Lyrics peek:** the current line is shown small over the ambient background. **On by default when synced lyrics exist**, and can be switched off in Settings. Tapping it opens the full view.
- **Engineering:**
  - an LRC parser in the Rust core (fuzz-tested), shared by both apps;
  - the gateway serves lyrics through an allow-listed, id-only request; no file paths ever reach the client.

## Cross-cutting

- **Accessibility:**
  - every gesture also has a button or accessibility action;
  - reduce motion is respected;
  - contrast is tested;
  - lyrics are readable at 200 % text size.
- **Testing:** unit tests for all shared logic (sleep timing, station refill, scoring, LRC parsing, Home layout validation). Plus negative and fuzz tests for every new gateway endpoint or allow-list entry. Plus UI tests on web (Vitest) and Android (Robolectric).
- **Release rule:** run the manifest diff check, and keep the crash reporter, for every Android build.
