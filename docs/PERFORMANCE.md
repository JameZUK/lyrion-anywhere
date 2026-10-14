# Performance

What has been measured, and the budgets for what still needs a phone to measure.

## Measured (2026-10-14)

Figures from the development machine (8 virtual cores of a QEMU guest, release builds) unless stated otherwise. They are for comparisons and regressions, not promises: a phone's core is several times slower.

### Decoding (Rust core, one core)

`cargo run --release -p anywhere-core --features opus --example decode_speed -- FILE FORMAT`, on 4 minutes of synthetic stereo material (tones and pink noise) encoded with FFmpeg:

| Format | Speed |
|---|---|
| FLAC 16-bit / 44.1 kHz | about 2,000× real time |
| FLAC 24-bit / 96 kHz | about 870× |
| MP3 320 kb/s | about 1,070× |
| Ogg Vorbis (q6) | about 1,010× |
| Opus 160 kb/s | about 170× (was 2× before the FFT fix below) |

**Opus fix:** the vendored Opus decoder computed its inverse MDCT's FFT as a direct O(n²) DFT, with a `sin_cos` per term. A mixed-radix FFT plan made it about 80 times faster and closer to libopus (signal-to-error against libopus rose from 80.6 to about 100.5 dB). See `third_party/opus-decoder/PATCHES.md`. `core/tests/opus.rs` now asserts at least 20× real time in release builds, so a slow path can't come back unnoticed. The DSP chain and track analysis have their own floors in `core/tests/dsp_performance.rs` (about 170× and 400×).

### Gateway

`tools/demo/bench.py [GATEWAY]` starts the demo LMS (an invented library in Python) and a release gateway on loopback, then times requests:

| Measure | Result |
|---|---|
| Start-up to `/healthz` | about 100 ms |
| Memory at rest (resident) | about 10 MB |
| Memory after the run below | about 11 MB (30 MB peak) |
| Sign-in | about 29 ms (mostly the password hash, by design) |
| `GET /v1/me`, one at a time | median 0.33 ms, p99 0.45 ms |
| `POST /v1/rpc` `albums 0 50`, one at a time (to LMS and back) | median 0.93 ms, p99 1.7 ms |
| `GET /v1/me`, 8 / 32 clients | about 2,750 / 2,500 requests a second |
| `POST /v1/rpc`, 8 / 32 clients | about 450 requests a second |

The throughput figures are limits of the test rig, not of the gateway: the Python client (a new connection per request) caps `/v1/me`, and the single-threaded demo LMS caps `/v1/rpc`. Its occasional 1 s outlier at 32 clients comes from the demo LMS too. A real LMS is slower than either, and is the bottleneck in practice.

**Live install:** after 2 hours 18 minutes serving a household (web app, phones, an iPad), the gateway used 10.8 MB resident (24.7 MB peak) on a 2-core VM, with 3 threads and negligible CPU. Opus re-encoding runs in separate `ffmpeg` processes, one per stream, and isn't counted here.

### Web app

From the plugin 0.3.0 release:

| Measure | Size |
|---|---|
| First load (HTML, the JavaScript and CSS the page links) | 322 KB, 109 KB compressed |
| All JavaScript | 759 KB, 255 KB compressed (routes load as they are opened) |
| The player's WebAssembly (decoders and DSP) | 1.9 MB, 595 KB compressed; not part of the first load. It is fetched for the one-off playback self-test and when this browser plays. The service worker leaves it to the browser's HTTP cache |

### Android

From the 1.10.0 release, against the budgets below:

| Measure | Size | Budget |
|---|---|---|
| Per-ABI APK, arm64-v8a | 18.9 MB | ≤ 20 MB |
| Per-ABI APK, armeabi-v7a | 16.2 MB | |
| Universal APK | 36.7 MB | ≤ 40 MB |
| App bundle (AAB) | 22.2 MB | (Play's download size not measured) |

Start-up, frames, memory, underruns and battery need a phone and haven't been measured yet (below).

## Android budgets

The Android app's budgets for 1.0: what is measured, the limit, and how to measure it. They apply to a **release build** (minified, with the baseline profile) on a **mid-range reference phone**: a Pixel 6a or Galaxy A54 class device running Android 14 or later. Debug builds are much slower and are never measured.

**Status:** the budgets are defined and the build is set up for them (R8, resource shrinking, a baseline profile, per-ABI APKs). The sizes are measured above. Everything else needs a device; record it in the table at the end, without serial numbers or other personal details.

| Area | Measure | Budget | How |
|---|---|---|---|
| **Cold start** | Time to first frame (`TotalTime` of `am start -W`), app not in memory | **≤ 500 ms** median of 10 | `StartupBenchmark` or `am start -W` (below) |
| | Time to full display (the library or Now playing drawn; `reportFullyDrawn`) | ≤ 1,200 ms median, signed in, server on the LAN | Macrobenchmark `StartupTimingMetric` |
| **Warm start** | Activity re-created, process alive | ≤ 250 ms median | `am start -W` after Back |
| **Scrolling** | Albums grid (500 albums), queue (500 tracks), artist list: frame duration | P50 ≤ 8 ms, P90 ≤ 12 ms, P99 ≤ 24 ms; **under 1 % janky frames** | Macrobenchmark `FrameTimingMetric`, or `dumpsys gfxinfo` |
| **Memory** | PSS while browsing, after a scroll through 500 albums | ≤ 160 MB | `dumpsys meminfo io.github.jamezuk.lyrionanywhere` |
| | PSS while playing (DSP on, waveform shown) | ≤ 200 MB | as above |
| | PSS in the background, playing, UI gone | ≤ 90 MB | as above, 30 s after leaving the app |
| **Audio** | Underruns while browsing heavily (scrolling, artwork loading) | **0** in 10 minutes | the signal path's underrun count, LMS's `STMo` |
| **Battery** | Playing a FLAC album, screen off, Wi-Fi | ≤ 4 % an hour on the reference phone | Settings → Battery, or `dumpsys batterystats` |
| **Size** | Per-ABI APK (arm64-v8a) | ≤ 20 MB | the release job's file sizes |
| | Universal APK | ≤ 40 MB | as above |
| | Download from Google Play (arm64, from the app bundle) | ≤ 15 MB | Play Console's size report |

The audio thread never waits on the UI by design (the Rust engine has its own thread and a lock-free ring), so the audio budget mostly tests the device's scheduling and the network.

## What the build does for these

- **R8** (`isMinifyEnabled`) and **resource shrinking** (`isShrinkResources`) in release builds. Keep rules exist only where reflection or JNI needs them: the UniFFI/JNA bindings and the certificate verifier (`apps/android/rust/consumer-rules.pro`), serialised navigation routes and Glance callbacks (`apps/android/app/proguard-rules.pro`).
- **The Rust library** is built with LTO and a single codegen unit (Cargo profile `android-release`) and stripped by the Android build. It is most of the APK: about 11 MB per ABI before compression.
- **Per-ABI APKs** for GitHub releases (`-Panywhere.splitApks=true`): a phone downloads one ABI, not three. The universal APK stays, for anyone unsure which to pick. Google Play builds per-device APKs from the app bundle by itself.
- **Native libraries are stored uncompressed and page-aligned** (the Android Gradle plugin's default): they are mapped straight from the APK, with no copy on install. That makes the download larger than compressing them would, but the installed size smaller.
- **A baseline profile** (`apps/android/app/src/main/baseline-prof.txt`) compiles the app's own code ahead of time at install, so start-up and the first scroll don't run interpreted. The libraries (Compose, Media3 and others) bring their own. This first profile is hand-written and coarse (all of the app's code); replace it with a measured one (below).

## Measuring

### Cold and warm start without tooling

```sh
./gradlew assembleRelease          # sign it, or install a release build signed with a test key
adb install -r app/build/outputs/apk/release/app-release.apk
adb shell cmd package compile -f -m speed-profile io.github.jamezuk.lyrionanywhere   # as after a day's use
for i in $(seq 10); do
  adb shell am force-stop io.github.jamezuk.lyrionanywhere
  adb shell am start -W -n io.github.jamezuk.lyrionanywhere/.MainActivity | grep TotalTime
done
```

### Frames and memory

```sh
adb shell dumpsys gfxinfo io.github.jamezuk.lyrionanywhere reset
# scroll the Albums grid for 20 seconds
adb shell dumpsys gfxinfo io.github.jamezuk.lyrionanywhere | grep -E "Janky|percentile"
adb shell dumpsys meminfo io.github.jamezuk.lyrionanywhere | grep -E "TOTAL PSS|Native Heap|Java Heap"
```

### Baseline profiles

Generating a measured profile needs a device or emulator: a rooted one, or a userdebug or `google_apis` emulator image with API 33 or later. There is no device in CI, so the procedure is manual:

1. Add a `com.android.test` module (`:baselineprofile`) targeting `:app`, with `androidx.benchmark:benchmark-macro-junit4` and `androidx.test.uiautomator:uiautomator`. Add a non-debuggable `benchmark` build type in `:app` (`initWith(release)`, signed with the debug key, `matchingFallbacks += "release"`). The `:rust` module builds the optimised library for every non-debug build type.
2. Write a `BaselineProfileRule` test that covers the journeys that matter:
   - start the app, signed in to the demo server (`tools/demo`, through `adb reverse tcp:9444 tcp:9444`);
   - open Albums and scroll to the end;
   - open an album, then Now playing;
   - open the queue and scroll it;
   - go back to Home.
3. Run it: `./gradlew :baselineprofile:connectedBenchmarkAndroidTest -Pandroid.testInstrumentationRunnerArguments.androidx.benchmark.enabledRules=BaselineProfile`.
4. Copy the generated `*-baseline-prof.txt` over `app/src/main/baseline-prof.txt`, keeping the header comment. Commit it.
5. Check it helps: run a `StartupBenchmark` (`StartupTimingMetric`, `CompilationMode.Partial()` against `CompilationMode.None()`) and a scrolling benchmark (`FrameTimingMetric`), and compare with the budgets above.

Regenerate the profile when start-up or the main screens change substantially, and at least before each release.

## Results

| Date | Build | Phone (model, Android) | Cold start (median) | Scroll P90 / jank | PSS browsing / playing | Notes |
|---|---|---|---|---|---|---|
| — | — | — | — | — | — | not measured yet (no device during A7) |
