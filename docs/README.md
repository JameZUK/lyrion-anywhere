# Documentation

Start with the project [`README`](../README.md) for an introduction.

## Using and running it

| Document | For | What it covers |
|---|---|---|
| [`INSTALL.md`](INSTALL.md) | Everyone setting it up | Step by step: the plugin or Docker, the reverse proxy, users, first sign-in, checks, everyday administration, troubleshooting |
| [`DEPLOY.md`](DEPLOY.md) | Administrators | The deployment reference: plugin, Docker networking, reverse proxy requirements, CDNs, a checklist |
| [`FEATURES.md`](FEATURES.md) | Everyone | Every feature, by area, with the platforms it runs on |
| [`PRIVACY.md`](PRIVACY.md) | Everyone | What the apps and the gateway store, where and for how long; Google Play's Data safety answers |

## How it works

| Document | What it covers |
|---|---|
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | Components, repository layout, trust boundary, request flows with sequence diagrams, the core and its hosts, the Android app, the plugin, storage, testing |
| [`PROTOCOL.md`](PROTOCOL.md) | The public API (`/v1`) and player protocol: authentication, the player socket and resume, streams, the command policy, transfer, sync, downloads, offline plays, your own data, analysis, lyrics, events |
| [`../SPEC.md`](../SPEC.md) | The specification: every design decision (D1–D26), goals, the threat model and the testing strategy |
| [`PERFORMANCE.md`](PERFORMANCE.md) | Android performance budgets and how to measure them |
| [`security/`](security) | Security reviews and penetration tests |

## Building and releasing

| Document | What it covers |
|---|---|
| [`../README.md#building-from-source`](../README.md#building-from-source) | Building and testing everything from source |
| [`../apps/android/README.md`](../apps/android/README.md) | Building the Android app, and its internals in detail |
| [`RELEASING.md`](RELEASING.md) | Plugin and Android releases, signing, Google Play and F-Droid |
| [`../CHANGELOG.md`](../CHANGELOG.md) | What changed and when, newest first |
| [`release-notes/`](release-notes) | Notes for each release |

## Design plans and history

These record the agreed designs and what was built, milestone by milestone. The code and the documents above are the reference for what exists now.

| Document | What it covers |
|---|---|
| [`plans/native-apps.md`](plans/native-apps.md) | The native apps plan (Android first, iOS designed in), with the status of each Android milestone (A0–A7) |
| [`plans/streaming-quality.md`](plans/streaming-quality.md) | Lower-data streams on mobile data (live Opus) |
| [`plans/ux-research.md`](plans/ux-research.md) | Research into other players, and the waves of UX work that followed |
| [`plans/wave1-design.md`](plans/wave1-design.md) | Wave 1: sleep timer, Now playing, listening history, Home, stations, lyrics |
| [`plans/wave2-design.md`](plans/wave2-design.md) | Wave 2: alarms, ratings, library and queue, Now playing layouts, visualiser, Rooms, car mode, smart filters |
| [`plans/wave3-design.md`](plans/wave3-design.md) | Wave 3: listening stats and Your year; the rolling cache (Phases A and B built, Phase C in progress) |
| [`phase0-report.md`](phase0-report.md) | The Phase 0 spike that proved the approach |
