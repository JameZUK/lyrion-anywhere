# Privacy policy

**Lyrion Anywhere** (the Android app, the web app and the gateway plugin for Lyrion Music Server) is free software for listening to **your own** music library on **your own** server. This policy says what each part keeps, where, and for how long.

*Last updated: 11 October 2026.*

## In short

- **Nothing goes to us.** The app talks only to the server you type in or scan (your Lyrion Anywhere gateway). The project has no servers, accounts, analytics, advertising, tracking or third-party SDKs, and receives no data from the app.
- **Your server keeps what it needs to sign you in and keep you safe:** your account, your devices, and an audit log. Its administrator (usually you or your household) can see it.
- **Your phone keeps your sign-in (sealed by the Android Keystore), your settings and your downloads.** Signing out removes the sign-in and, if you choose, the downloads. A device the server revokes deletes its downloads.

## What the Android app stores on your phone

All of this is in the app's private storage. Other apps can't read it, and none of it is backed up or copied to a new phone (backups and device-to-device transfers are turned off for the app).

| What | Why | How long |
|---|---|---|
| **Your server's address** | To connect | Until you sign out of that server |
| **Your sign-in:** the refresh token and the server's id for this phone | To stay signed in | Until you sign out or the server revokes the device |
| **Settings:** sound, downloads, default player, library choice, the last user signed in on this server (for offline mode) | Your preferences | Until you uninstall, or sign out (the offline owner) |
| **Downloads:** audio files, covers, and an index of titles, artists, albums, durations and formats | Offline listening | Until you remove them, choose to remove them at sign-out, the server revokes the device, or you uninstall |
| **Plays heard offline:** track id, when it started and how long it played | To update play counts on your server when you're back online | Until the server has them (then deleted) |
| **Crash reports** (only after a crash) | So you can see what went wrong and choose to share it | Until you dismiss the report |

- **The sign-in is sealed:** encrypted with AES-256-GCM under a key kept in the Android Keystore, which never leaves the phone. Access tokens are kept in memory only, never on disk. Your password is never stored; it is dropped from memory after you sign in.
- **Downloads** are in a folder named by a one-way hash of the server and user name. Files are named by the server's track keys, not by title or path. The index holds no tokens, addresses or file paths.
- **Crash reports** stay on the phone. The app never sends them anywhere: you see the report the next time the app opens, and only you can copy or share it. Reports hold the app version, Android version, phone model and the error's stack trace. Before a report is saved or shown, the app removes tokens, secrets, server addresses, IP addresses and folder names. From a native crash, it keeps only the names of the code that crashed, never the memory dump Android records.
- **Artwork** is cached in memory only.
- **The camera** is used only to scan a pairing QR code, when you choose to. Frames are read on the phone and never stored or sent.

## What the app sends, and to whom

The app sends data **only to your gateway**, over HTTPS (plain HTTP is refused, except to the phone itself during development). That covers:
- signing in;
- browsing and searching your library;
- controlling players;
- streaming and downloading music;
- waveform analysis;
- sending plays heard offline.

When it signs in, the app also tells your gateway what it is, so its admins can recognise the device:
- the client type (`android`);
- the phone's name (for example "Pixel 8"; you can rename it in Settings);
- the app's own User-Agent (`LyrionAnywhere-Mobile/<version>`).

Nothing is sent to the project, Google or anyone else. The app contains no analytics, advertising or crash-reporting services, and no Google Play Services.

**Android Auto:** when you use the app in a car, Android Auto shows your library's titles, which the app hands to it on the phone. It is told the ids and names of items only, never addresses or files.

## What your gateway (the server) stores

The gateway runs on your server, next to Lyrion Music Server. Its data stays there, in its data directory.

| What | Detail | How long |
|---|---|---|
| **Accounts** | Username, role (admin or listener), a password hash (argon2id; the password itself is never stored), the two-step secret if you turn it on, download permission and player grants | Until an admin deletes the account |
| **Devices** | Per signed-in device: its name, its LMS player id (a made-up MAC address), when it was created and last seen, its **last IP address**, its **client type** and platform (for example "android" and "Pixel 8"), and its browser or app **User-Agent** | While the device is active. A revoked device's address and User-Agent are erased at once, and the device itself is deleted 30 days after revocation |
| **Sign-ins** | Hashes of refresh tokens (never the tokens) and of pairing codes | Until they expire (90 days of inactivity by default; the admin can change it) or are used |
| **Play keys** | For plays sent from offline listening: a random key per play, the track id and when it was played, so a play retried after a lost answer is never counted twice | 91 days, at most 50,000 per user |
| **Audit log** | Security events: sign-ins and failures (with the client's address), new and revoked devices, renames, settings changes, plays recorded (a count) | One year, at most the newest 100,000 entries |
| **Track analysis cache** | Waveform and loudness of tracks, named by a salted hash of the file | Until the cache is full (least recently used go first) |

- **Who sees it:** the gateway's admins, through Lyrion Music Server's settings page (which needs a token only that page has). Listeners see only their own account and devices. Nothing is shared with anyone else.
- **Play counts** themselves are kept by Lyrion Music Server, as when you play a track on any LMS player.
- **The reverse proxy** (for example nginx) that you put in front of the gateway may keep its own access logs. That is your proxy's configuration, not this software's.

## Deleting your data

- **Sign out** (Settings → Sign out): the phone deletes its sign-in, sends any plays still waiting, and keeps or deletes your downloads as you choose. The server revokes the device, erasing its address and User-Agent.
- **Revoked by the server** (an admin revokes the device, or too many devices signed in): the phone deletes its sign-in and all its downloads the next time it connects.
- **Uninstall:** Android deletes everything the app stored.
- **On the server:** ask its admin to delete your account. That deletes your devices, sign-ins and play keys at once. The audit log keeps its entries naming your username until they expire (one year), as a security record.

## Children

The app has no content of its own and no age-specific features. What it plays is your own library.

## Changes

Changes to this policy are made in the project's public repository, with the date above updated. The app's About screen links here.

## Contact

Please open an issue on the project's GitHub page: https://github.com/JameZUK/lyrion-anywhere/issues

---

## Google Play: Data safety answers

For the Play Console's *Data safety* form. Google defines "collected" as data **transmitted off the device by the app** to the developer or a third party. This app sends data only to a server the user runs or is given access to, which the developer doesn't operate and has no access to.

**Recommended answers**
- **Does your app collect or share any of the required user data types?** No.
  - The app sends account and library data only to the user's own server, chosen by the user, and the developer receives nothing.
  - Crash reports are never transmitted by the app; the user may share one by hand.
  - There are no analytics or SDKs.
- **Is all of the user data collected by your app encrypted in transit?** Yes. HTTPS only; plain HTTP is refused.
- **Do you provide a way for users to request that their data is deleted?** Yes:
  - signing out deletes the app's data;
  - uninstalling deletes everything;
  - server-side data is deleted by the server's admin (see *Deleting your data*).
- **Independent security review:** not claimed. The project's own reviews are in `docs/security/`.

**If Google asks for a stricter reading** (treating the user's own server as "collection"), declare the following:

| Data type | Collected | Shared | Purpose | Optional |
|---|---|---|---|---|
| Personal info → User IDs (username) | Yes | No | Account management, app functionality | No |
| Device or other IDs (device id) | Yes | No | App functionality, security | No |
| App activity → Other actions (offline plays) | Yes | No | App functionality | Yes (only when offline playback is used) |
| Audio files (downloads are received, not sent) | No | No | — | — |
| App info and performance → Crash logs | No (never sent by the app) | No | — | — |

All of these are processed only on the user's own server, are encrypted in transit, and can be deleted as above.

**Other Play Console declarations**
- **Ads:** none.
- **Target audience:** 13+, a music player for one's own library.
- **Content rating:** no user-generated content shared between users, and no purchases.
- **Government, financial and health apps:** not applicable.
- **Permissions:**
  - camera: QR pairing, on request only;
  - notifications: playback and download progress;
  - foreground services: media playback and data sync (downloads).
