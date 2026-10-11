# Releasing

Two things are released from this repository, from different tags:

| What | Tag | CI job | Publishes |
|---|---|---|---|
| The LMS plugin (gateway, web app, plugin) | `v<version>`, e.g. `v0.2.0` | `release` | A GitHub release with the plugin zips, `repo.xml` and `SHA256SUMS` |
| The Android app | `android-v<version>`, e.g. `android-v1.0.0` | `android-release` | A GitHub **pre-release** with the signed APKs (universal and one per ABI), the app bundle, `BUILDINFO.txt` and `SHA256SUMS` |

- The plugin job publishes only on `refs/tags/v*`, which an `android-v*` tag never matches.
- The Android job runs only on `refs/tags/android-v*`.
- Both publish only after the other jobs pass for that commit.

Choosing a version number is the owner's decision; nothing here picks one.

## Plugin releases (`v*`)

1. **Version:** set `<version>` in `plugin/Plugins/LyrionAnywhere/install.xml`. The tag must equal `v` plus that version; the job checks this.
2. **Notes:**
   - write `docs/release-notes/v<version>.md` (the job refuses a tag without it);
   - add the release to `CHANGELOG.md`.
3. **Checks:** run every suite (see `CLAUDE.md`): Rust, web, plugin, deploy. CI runs them again on the tag.
4. **Public-repo hygiene:** grep the diff since the last release for addresses, hostnames and names.
5. **Tag and push:** `git tag v<version> && git push origin v<version>`.
6. **After CI:**
   - check the release's files against `SHA256SUMS`;
   - install the zip on a test LMS, and update one through the repository URL;
   - check the gateway starts, and a sign-in and a stream work.

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

The release job signs with a key held in four repository secrets. Make the key once and keep a backup of it offline: losing it means no more updates for existing installs.

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

- The job writes the keystore to the runner's temporary folder (mode 600) only for the build, then deletes it.
- Locally, the same build reads `ANYWHERE_KEYSTORE` (a path), `ANYWHERE_KEYSTORE_PASSWORD`, `ANYWHERE_KEY_ALIAS` and `ANYWHERE_KEY_PASSWORD`. Without them, release builds are unsigned.
- Earlier test builds were signed with a key whose certificate is in `docs/release-notes/android-0.1.0-alpha.1.md`. Keep using that key if existing installs should update in place.

### Tagging and publishing

1. `git tag android-v<versionName> && git push origin android-v<versionName>`.
2. CI runs the tests and then `android-release`, which:
   - checks the tag against `versionName` and the release notes;
   - builds from scratch (no shared caches) with `-Panywhere.splitApks=true assembleRelease bundleRelease`;
   - verifies every signature;
   - checks the native libraries carry no build paths;
   - writes `BUILDINFO.txt` (commit, Rust, cargo-ndk, NDK and JDK versions) and `SHA256SUMS`;
   - creates a **pre-release** with:
     - `LyrionAnywhere-android-<v>.apk` (universal);
     - `LyrionAnywhere-android-<v>-arm64-v8a.apk`, `-armeabi-v7a.apk` and `-x86_64.apk`;
     - `LyrionAnywhere-android-<v>.aab`.
3. Check the pre-release:
   - `sha256sum -c SHA256SUMS`;
   - `apksigner verify --print-certs` shows the expected certificate;
   - install the per-ABI APK on a phone over the previous version (it must update in place);
   - sign in, play, download, and go offline.
4. Promote it to a full release on GitHub when happy.

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
- the baseline profile is generated from a checked-in text file.

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
