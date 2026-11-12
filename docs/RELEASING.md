# Releasing

Two things are released from this repository, from different tags:

| What | Tag | CI job | Publishes |
|---|---|---|---|
| The LMS plugin (gateway, web app, plugin) | `v<version>`, e.g. `v0.2.0` | `release` (build), then `release-publish` | A GitHub release with the plugin zips, `repo.xml` and `SHA256SUMS`; build provenance for the zips and `repo.xml` |
| The Android app | `android-v<version>`, e.g. `android-v1.0.0` | `android-release` (build), `android-sign`, then `android-publish` | A GitHub release, **never marked Latest**, with the signed APKs (universal and one per ABI), the app bundle, `BUILDINFO.txt` and `SHA256SUMS`; build provenance for the APKs and the bundle |

- The plugin's publish job runs only on `refs/tags/v*`, which an `android-v*` tag never matches. Its build job also runs on every push to `main`, building and checking without publishing.
- The Android jobs run only on `refs/tags/android-v*`.
- Both publish only after the other jobs pass for that commit.
- **Building and publishing are separate jobs.** The build jobs run third-party code (cargo and every crate's build script, Gradle and its plugins, npm, Docker images) with read access only, no secrets, and no credentials left in the checkout (`persist-credentials: false`). Only the publish jobs hold write access (contents for the release, id-token and attestations for the provenance), and they run nothing but GitHub's CLI and actions on the files the build handed over.
- **The Android release key is used in a job of its own** (`android-sign`), on a fresh runner where no build code has run, so no build script or Gradle daemon can be about to read it (below, *Signing keys*).

### Android releases are never Latest

LMS installs find plugin updates at `https://github.com/JameZUK/lyrion-anywhere/releases/latest/download/repo.xml`. GitHub's *Latest* is one release. The plugin's `v*` releases are published with `--latest` and take it; Android releases are full releases published with `--latest=false`, which leaves *Latest* where it is. **Never tick "Set as the latest release" on an `android-v*` release.** If one becomes Latest, that URL is a 404 and every LMS install silently stops seeing plugin updates, security fixes included. (Until 2026-10-23 Android releases were pre-releases for the same reason; a full release that is never Latest is as safe, and lists the app properly.) One more way it can go wrong: if the plugin release that is Latest is ever deleted, GitHub chooses Latest afresh by date and may pick an Android release.

Guards:
- `tools/check-latest.sh [--expect-version <v>]` fetches that URL and fails unless it is the plugin's `repo.xml` (and that version). Run it after any change on the releases page.
- CI runs it after publishing a plugin release (expecting the new version) and after publishing an Android one; the Android job also fails if the release it just made is Latest.
- The *Plugin repository URL* workflow (`.github/workflows/repo-xml.yml`) runs it whenever a release is published, promoted, edited or deleted, weekly, and on demand.

If it fails: mark the newest `v*` release as Latest (`gh release edit v<version> --latest`), then run `tools/check-latest.sh --expect-version <version>`.

Choosing a version number is the owner's decision; nothing here picks one.

## Verifying a release

LMS checks a plugin download only against the SHA1 in `repo.xml`, and `SHA256SUMS` sits in the same release. Both are written by whoever publishes the release, so they prove the download is intact, not who built it. The release jobs therefore also publish signed **build provenance** ([artifact attestations](https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations), signed through Sigstore with the workflow's own identity) for:
- the plugin zips and `repo.xml` (`release-publish`, on `v*` tags);
- the APKs and the app bundle (`android-publish`, on `android-v*` tags).

Anyone can check that a file was built by this repository's CI workflow, from a given commit and tag, with the [GitHub CLI](https://cli.github.com/):

```sh
gh attestation verify LyrionAnywhere-<version>.zip --repo JameZUK/lyrion-anywhere \
  --signer-workflow JameZUK/lyrion-anywhere/.github/workflows/ci.yml --source-ref refs/tags/v<version> --deny-self-hosted-runners
gh attestation verify repo.xml --repo JameZUK/lyrion-anywhere \
  --signer-workflow JameZUK/lyrion-anywhere/.github/workflows/ci.yml --source-ref refs/tags/v<version> --deny-self-hosted-runners
gh attestation verify LyrionAnywhere-android-<version>.apk --repo JameZUK/lyrion-anywhere \
  --signer-workflow JameZUK/lyrion-anywhere/.github/workflows/ci.yml --source-ref refs/tags/android-v<version> --deny-self-hosted-runners
```

- It prints the workflow, commit and ref that built the file.
- `--signer-workflow` pins the workflow file, so a file attested by any other workflow in the repository fails.
- `--source-ref` pins the release's tag, so a file built from another branch or tag fails.
- `--deny-self-hosted-runners` refuses a build that ran anywhere but GitHub's own runners.
- `tools/release/verify-plugin.sh` and `verify-android.sh` check all three.
- A modified file, or one built anywhere else, fails with "no matching attestations".
- Releases published before CI started attesting them (18 October 2026) have none.
- Only the files on the GitHub release are attested. Copies installed from Google Play or F-Droid are built or signed by those stores' pipelines and are not.

## Helper scripts

The steps below are scripted under `tools/release/` (they need the [GitHub CLI](https://cli.github.com/) signed in; the Android one also the SDK's build-tools). Each prints what it checked and stops at the first problem.

| Script | What it does |
|---|---|
| `tools/release/ci-wait.sh run <id>` \| `branch <branch> [<commit>]` \| `tag <tag>` | Waits for a CI run to finish (polling every 45 s) and fails unless it succeeded, listing the failed jobs. A tag's run is found by its head branch, a commit's by its full sha (`gh run list --commit` with a short sha finds nothing). |
| `tools/release/tag.sh [--push] [plugin <v>] [android <v>]` | Checks the tree is ready (clean, on `origin/main`, the versions in `install.xml` / `build.gradle.kts`, the release notes, the F-Droid changelog for the versionCode) and makes the annotated tags; prints the push command, or pushes with `--push`. |
| `tools/release/verify-plugin.sh <v> [<dir>]` | Downloads the `v<v>` release, checks `SHA256SUMS`, the build provenance of the zip and `repo.xml`, the plugin repository URL (`tools/check-latest.sh`), unpacks the zip (into `dist/releases/v<v>/unpacked` by default) and checks `install.xml` and `Bin/SHA256SUMS` inside; prints the unpacked directory last, for the deploy. |
| `tools/release/sign-android.sh <unsigned dir> <out dir> <v>` | The `android-sign` job's signing step: signs an unsigned build (`app/build/outputs`, or a copy of it) with the key named by the `ANYWHERE_KEY*` variables, using only the SDK's `apksigner` (v2, alignment kept, as the Gradle plugin signed) and the JDK's `jarsigner`, writes the release's file names into `<out dir>`, and checks every signature and the certificate. `tools/release/test-sign-android.sh <apk>` tests it with a throwaway key (CI's `android` job runs it on the debug APK). |
| `tools/release/verify-android.sh <v> [<previous>]` | Downloads the `android-v<v>` release into `dist/android/<v>/`, checks `SHA256SUMS`, the build provenance of every APK and the bundle, the signing certificate, the version against `build.gradle.kts`, the merged manifest against the previous release's APK (only the version lines may differ; the providers and `androidx.startup` initialisers are listed), and that the release is not marked *Latest*. |

A release, end to end: push `main` and `tools/release/ci-wait.sh branch main <commit>`; `tools/release/tag.sh plugin <v> android <v>` and push the tags; `tools/release/ci-wait.sh tag v<v>` (and `tag android-v<v>`); `tools/release/verify-plugin.sh <v>` and `tools/release/verify-android.sh <v> <previous>`; then deploy. Deploying to a particular installation and checking it live are specific to that installation, so those scripts live outside the repository (gitignored `dev/local.*` files, see `CLAUDE.md`): a deploy script that installs the verified zip's web app, gateway binary and plugin files and restarts LMS (resuming whatever was playing), and a live check (health, static files, the content-hash ETag, a sign-in with a test account, an image through the image proxy, the transcoding set-up).

## Plugin releases (`v*`)

1. **Version:** set `<version>` in `plugin/Plugins/LyrionAnywhere/install.xml`. The tag must equal `v` plus that version; the job checks this.
   - **`cloudflared` pin:** before a release, run `tools/update-cloudflared-pin.sh <version>` with the newest [`cloudflared` release](https://github.com/cloudflare/cloudflared/releases) (it downloads the three Linux assets, sums them and rewrites `gateway/src/cftunnel/pins.rs`), and note the version in the changelog. `tools/test-release.sh` checks the pin is well formed.
2. **Notes:**
   - write `docs/release-notes/v<version>.md` (the job refuses a tag without it);
   - add the release to `CHANGELOG.md`.
3. **Checks:** run every suite (see `CLAUDE.md`): Rust, web, plugin, deploy. CI runs them again on the tag.
4. **Public-repo hygiene:** grep the diff since the last release for addresses, hostnames and names.
5. **Tag and push:** `git tag v<version> && git push origin v<version>`.
6. **After CI:**
   - check the release's files against `SHA256SUMS`, and their provenance with `gh attestation verify` (see *Verifying a release* below);
   - install the zip on a test LMS, and update one through the repository URL;
   - check the gateway starts, and a sign-in and a stream work.

A release published by hand first (built locally with `tools/build-release.sh`, which is quicker than waiting for CI) is left alone: the job sees the release already exists and stops without replacing it. The same applies to Android releases.

A hand-published release has no matching provenance: CI still attests the files *it* built, but unless the local build is byte-for-byte identical, `gh attestation verify` fails for the published files. Prefer letting CI publish; if a release must be published by hand, say so in its notes.

## Android releases (`android-v*`)

### Before tagging

1. **Version:** set `versionCode` (always higher than the last one published anywhere) and `versionName` in `apps/android/app/build.gradle.kts`.
2. **Notes:**
   - `docs/release-notes/android-<versionName>.md` (the job refuses a tag without it). Include the signing certificate's SHA-256, as earlier releases did.
   - `apps/android/fastlane/metadata/android/en-GB/changelogs/<versionCode>.txt`: at most 500 characters, shown by Google Play and F-Droid.
3. **Tests:** everything in `CLAUDE.md`, including:
   - `./gradlew assembleDebug testDebugUnitTest lint assembleRelease bundleRelease`, with no lint issues;
   - clippy for Android.
4. **Manifest diff check** (see below), with every change justified in the A-milestone status or the release notes.
5. **Crash-reporter rule** (see below).
6. **Privacy:**
   - re-read `docs/PRIVACY.md` against what the release stores or sends;
   - if anything changed, update it and Google Play's *Data safety* form together.
7. **Performance:** if start-up or the main screens changed, regenerate the baseline profile and record a measurement (`docs/PERFORMANCE.md`).
8. **Public-repo hygiene:** grep the diff for addresses, hostnames and names.

### Dependency verification

Gradle checks every dependency, plugins included, against the SHA-256 recorded in `apps/android/gradle/verification-metadata.xml`, and fails the build on anything unknown or changed. This matters most for the rustls-platform-verifier AAR (the JNI bridge every certificate check goes through), which comes from a GitHub branch that can change.

- After adding or updating a dependency, refresh the file **from a clean module cache**: stop the daemon (`./gradlew --stop`), move `~/.gradle/caches/modules-2` aside, then from `apps/android` run `./gradlew --write-verification-metadata sha256 help testDebugUnitTest lint assembleDebug assembleRelease`, then review the diff: only the components you changed should appear. A warm cache leaves out metadata files (parent POMs, BOM `.module` files) that a clean resolution fetches, and CI, which always starts clean, then fails verification; the 1.15.5 release run did exactly that.
- A changed checksum for a version already listed is never accepted by regenerating: find out why the artefact changed first.
- `DependencyVerificationTest` checks the file is in force and that the verifier AAR is pinned.

### The manifest diff check

The release APK's merged manifest is what users install, and dependencies can add to it silently. In alpha 5 and 6, WorkManager's start-up initialiser, brought in by a library, crashed the app on launch.

Compare the new release with the last one published:

```sh
cd apps/android
./gradlew assembleRelease
aapt2=$ANDROID_HOME/build-tools/36.1.0/aapt2
$aapt2 dump xmltree --file AndroidManifest.xml app/build/outputs/apk/release/app-release.apk > /tmp/new.txt
$aapt2 dump xmltree --file AndroidManifest.xml LyrionAnywhere-android-<previous>.apk > /tmp/old.txt
diff /tmp/old.txt /tmp/new.txt
```

Check, and justify every difference in:
- **Providers:** only `androidx.startup.InitializationProvider`, not exported, unless a change is justified. **No new `androidx.startup` initialiser**, and `androidx.work.WorkManagerInitializer` stays removed. The initialisers today are `EmojiCompatInitializer`, `ProcessLifecycleInitializer` and `ProfileInstallerInitializer`.
- **Exported components** (`android:exported="true"`), each with its reason. Today:
  - the launcher activity (with voice search);
  - the media library service (Android Auto; every caller is checked in code);
  - the widget receiver (update broadcasts only);
  - the Quick Settings tile (system-bound).
- **Permissions:** the list in `SecurityConfigTest`/the manifest tests. A new permission needs a reason and a privacy-policy check.
- **Services, receivers and activities** added by libraries.
- `allowBackup="false"`, the data extraction rules and the network security configuration are unchanged.

The `:app` manifest tests catch most of this on every build, but they test the source manifest. The diff is what actually ships.

### The crash-reporter rule

- **No crash-reporting service, analytics or SDK** is ever added: no Firebase, Sentry or ACRA with a network sender, and no Play Services.
- Crash reports stay on the phone. The user sees them at the next start and may copy or share them; the app never sends them.
- **Reports must never hold secrets:**
  - `CrashLog` redacts every report before it is saved or shown (tokens, secrets, server addresses, IP addresses, home folders), and keeps only backtrace names from native tombstones;
  - a new kind of secret (a new token format, a new header) must be added to `CrashLog.redact`, with a case in `CrashLogTest`;
  - exception messages must never include tokens, passwords or URLs with credentials, in Kotlin and in Rust (`mobile/`).
- `CrashLogTest` must pass.

### Signing keys and repository secrets (one-time set-up)

The `android-sign` job signs with a key held in four repository secrets. Make the key once and keep a backup of it offline: losing it means no more updates for existing installs.

```sh
keytool -genkeypair -v -keystore lyrion-anywhere-release.jks -alias lyrion-anywhere \
  -keyalg RSA -keysize 4096 -validity 10000
base64 -w0 lyrion-anywhere-release.jks > keystore.b64
gh secret set ANDROID_KEYSTORE_B64 < keystore.b64
gh secret set ANDROID_KEYSTORE_PASSWORD     # prompts; the keystore's password
gh secret set ANDROID_KEY_ALIAS             # e.g. lyrion-anywhere
gh secret set ANDROID_KEY_PASSWORD          # the key's password
shred -u keystore.b64
```

Alternatively, use *Settings → Secrets and variables → Actions → New repository secret* on GitHub.

- **Only `android-sign` sees the key.** The build job (`android-release`) has no secrets and builds unsigned. `android-sign` runs on a fresh runner, where no build code has run: it fetches the unsigned files, writes the keystore to the runner's temporary folder (mode 600), signs with `tools/release/sign-android.sh` (the SDK's `apksigner` and the JDK's `jarsigner`, passwords passed by variable name, never on a command line), checks every signature, and deletes the keystore. `android-publish` then attests and publishes the signed files.
- Locally, either let Gradle sign: the release build reads `ANYWHERE_KEYSTORE` (a path), `ANYWHERE_KEYSTORE_PASSWORD`, `ANYWHERE_KEY_ALIAS` and `ANYWHERE_KEY_PASSWORD`, and without them builds unsigned. Or sign as CI does: build without them, then `tools/release/sign-android.sh apps/android/app/build/outputs dist-android <versionName>` with the same four variables set.
- Earlier test builds were signed with a key whose certificate is in `docs/release-notes/android-0.1.0-alpha.1.md`. Keep using that key if existing installs should update in place.

### Tagging and publishing

1. `git tag android-v<versionName> && git push origin android-v<versionName>`.
2. CI runs the tests and then `android-release`, which:
   - checks the tag against `versionName` and the release notes;
   - builds from scratch (no shared caches), unsigned and with no secrets: `-Panywhere.splitApks=true assembleRelease`, then `bundleRelease` in a separate run (the Android Gradle plugin can't do both in one);
   - checks nothing came out signed and the native libraries carry no build paths, and writes `BUILDINFO.txt` (commit, Rust, cargo-ndk, NDK and JDK versions);
   - in `android-sign`, signs the APKs and the bundle, verifies every signature and that one certificate signed them all, and writes `SHA256SUMS`;
   - in `android-publish`, attests the build provenance and creates a release (`--latest=false`, and fails if it came out as Latest) with:
     - `LyrionAnywhere-android-<v>.apk` (universal);
     - `LyrionAnywhere-android-<v>-arm64-v8a.apk`, `-armeabi-v7a.apk` and `-x86_64.apk`;
     - `LyrionAnywhere-android-<v>.aab`.
3. Check the release:
   - `sha256sum -c SHA256SUMS`;
   - `gh attestation verify` passes for each APK and the bundle (see *Verifying a release* below);
   - `apksigner verify --print-certs` shows the expected certificate;
   - install the per-ABI APK on a phone over the previous version (it must update in place);
   - sign in, play, download, and go offline.
4. Never mark it Latest (see *Android releases are never Latest* above). The publish job's last step checks the plugin repository URL still works.

### Google Play

**One-time set-up (owner):**
- a Google Play developer account (one-off fee, identity verification);
- create the app `io.github.jamezuk.lyrionanywhere`.
- **App signing:** choose to use **your own app signing key**, the same key as GitHub and F-Droid, uploaded with Play's encryption tool. Then the three channels install as updates of one another. If Play generates its own key, Play installs can't be updated from GitHub or F-Droid builds, or the reverse.
- **Store listing:** from `apps/android/fastlane/metadata/android/en-GB/`:
  - `title.txt`, `short_description.txt` and `full_description.txt`;
  - `images/icon.png` (512 px);
  - screenshots (below);
  - a feature graphic (1024 × 500), still to make.
- **Policy declarations:**
  - Data safety: the answers in `docs/PRIVACY.md` › *Google Play: Data safety answers*;
  - privacy policy URL: the GitHub link to `docs/PRIVACY.md`;
  - content rating questionnaire;
  - target audience 13+;
  - no ads.
- **Foreground service declarations:**
  - media playback (playing music);
  - data sync (downloads the user starts).
- **App access for review:** a bring-your-own-server app needs a demo. Host the `tools/demo` stand-in LMS behind a test gateway (read-only library, invented content) and give reviewers its address and a listener account.
- Android Auto: opt in under *Advanced settings → Form factors*. The app must pass the car app quality checks.

**Each release:** upload the `.aab` from the GitHub release to the internal testing track, then closed testing, then production. Alternatively, `fastlane supply` uses the same metadata folder. Changelogs come from `changelogs/<versionCode>.txt`.

**Screenshots** (no device in CI, so made by hand):

1. Run the demo LMS and a gateway locally (`tools/demo`). Its library and artwork are invented, so nothing personal is shown.
2. On a phone or emulator: `adb reverse tcp:9444 tcp:9444`. Sign in to `http://127.0.0.1:9444` with a debug build, which allows loopback HTTP.
3. Use the system's demo mode for a clean status bar:
   ```sh
   adb shell settings put global sysui_demo_allowed 1
   adb shell am broadcast -a com.android.systemui.demo -e command enter
   adb shell am broadcast -a com.android.systemui.demo -e command clock -e hhmm 1000
   ```
4. Capture Home, an album, Now playing with the signal path, Downloads and Settings → Sound, in light and dark: `adb exec-out screencap -p > 01-home.png`. Repeat on a 10-inch tablet emulator for `tenInchScreenshots`.
5. Save them under `fastlane/metadata/android/en-GB/images/phoneScreenshots/` (and `tenInchScreenshots/`), numbered in display order.

### Android screenshots

The documentation's Android screenshots (`docs/images/android/*.png`) need no device or emulator. The `DocScreenshots*` tests in `:ui` render the app's real screens and theme on the JVM (Robolectric native graphics), fed with the web demo's invented library (`ui/src/test/.../docs/DocLibrary.kt`) and generated cover art (`DocArt.kt`, drawn as `tools/demo/fake_lms.py` draws it). The status bar and gesture bar are drawn over the app, and the app is given matching window insets. Phones are 411×914 dp at 420 dpi (1078×2399 px); the tablet is 1280×800 dp, scaled to 1080 px wide. Each file is an opaque RGB PNG, written at the strongest lossless compression.

To regenerate them, from `apps/android`:

```sh
./gradlew :ui:testDebugUnitTest --tests '*DocScreenshots*' -PrecordDocScreenshots
```

Every normal `testDebugUnitTest` run also renders them all (about 20 seconds) and fails if one comes out blank, but writes nothing. Look at each image before committing.

### F-Droid

F-Droid builds the app from source. The draft recipe is `apps/android/fdroid/io.github.jamezuk.lyrionanywhere.yml`, with the srclib `apps/android/fdroid/srclibs/rustls-platform-verifier.yml`.

**Dependencies:**
- All free software: AndroidX, Compose, Media3, Glance, CameraX, Coil, ZXing, kotlinx, JNA and Rust crates. There are no Google Play Services, Firebase or other proprietary libraries; `cargo deny` checks the Rust licences in CI.
- Everything comes from Maven Central, Google's Maven repository or crates.io, except one artefact: the Kotlin half of **rustls-platform-verifier**. Its project publishes it only in a Maven repository on GitHub.
  - F-Droid builds it from source instead: the srclib checks out the rustls-platform-verifier tag matching `Cargo.lock` (today `v/0.7.1`, whose `rustls-platform-verifier-android` is 0.2.0).
  - `-Panywhere.verifier.source=<srclib>/android/rustls-platform-verifier` makes the build compile it as the `:verifier-source` module, instead of downloading it.
  - The recipe also deletes the GitHub repository's declaration from `settings.gradle.kts` (between the `BEGIN`/`END prebuilt-verifier` markers), so the scanner sees no unknown repository.
  - The module was checked to compile to the same three classes and resources as the published artefact.
- JNA's native dispatch library comes prebuilt inside its Maven Central artefact, as for every UniFFI app F-Droid builds. If a reviewer asks, JNA can be built from source as another srclib.
- `com.google.android.gms.car.application` in the manifest is only a metadata name that Android Auto reads. It isn't a Google Play Services dependency.

**Toolchain:**
- Rust is installed by the recipe through F-Droid's `rustup` srclib, pinned to the version in the GitHub release's `BUILDINFO.txt`, with `cargo-ndk` 4.1.2.
- Gradle finds it through `-Panywhere.cargo=$HOME/.cargo/bin/cargo`.
- The NDK is 29.0.14206865 and the JDK is 21.

**Submitting (owner):**
1. Fork https://gitlab.com/fdroid/fdroiddata.
2. Copy the two files into `metadata/` and `srclibs/`.
3. Fill in:
   - the version and commit;
   - the Rust version;
   - `AllowedAPKSigningKeys` (the release certificate's SHA-256, lower case without colons).
4. Run `fdroid lint io.github.jamezuk.lyrionanywhere` and `fdroid build -v -l io.github.jamezuk.lyrionanywhere` (with `fdroidserver` and its build VM, or the `registry.gitlab.com/fdroid/docker-executable-fdroidserver` image).
5. Open a merge request. Later versions are picked up from `android-v*` tags (`UpdateCheckMode: Tags ^android-v`).

**Reproducible builds:** with `Binaries:` and `AllowedAPKSigningKeys:` set, F-Droid publishes the project's own signed APK when its unsigned build matches it byte for byte. What the build does for that:
- the Rust library's build paths are remapped (workspace, Cargo home and target folder);
- `Cargo.lock` is used as it is (`--locked`);
- the profile is fixed (`android-release`);
- AGP writes no dependency-info blob (`dependenciesInfo` off);
- the baseline profile is generated from a checked-in text file;
- the APKs are built unsigned and signed afterwards by `apksigner`, which keeps the build's entries and central directory byte for byte and adds only the signing block, after zero padding that starts it on a 4 KiB boundary (checked by stripping the block again, 2026-11-12). Releases up to 1.17.3 were signed by the Gradle plugin, without that padding: check that the first F-Droid build after the change still matches.

What must match between the GitHub build and F-Droid's:
- the Rust version (CI uses the stable toolchain of the day; `BUILDINFO.txt` records it, and the recipe pins it);
- cargo-ndk;
- the NDK;
- the JDK major version;
- the commit (the tag).

If F-Droid's build doesn't match:
1. Compare the two APKs with `diffoscope`.
2. Check the native libraries first (toolchain differences).
3. Then `classes.dex` (Kotlin, AGP or JDK differences).
4. Then `META-INF/version-control-info.textproto`, which records the commit.

Until the builds match, F-Droid can still publish under its own key. It then can't update over GitHub or Play installs, and the reverse.
