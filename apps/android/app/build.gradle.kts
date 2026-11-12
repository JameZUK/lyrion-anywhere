plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

// Release signing comes from the environment (CI secrets or a local shell), never from
// the repository. Without it, release builds are unsigned.
val releaseKeystore: String? = System.getenv("ANYWHERE_KEYSTORE")

// The ABIs the Rust library is built for (gradle.properties, or -Panywhere.rust.abis=…). The
// APK carries native code for these only: dependencies such as JNA ship more (armeabi, mips,
// x86), and a device preferring one of those would load that folder, find no Rust library
// and crash.
val rustAbis: List<String> =
    providers.gradleProperty("anywhere.rust.abis").get().split(',').map { it.trim() }.filter { it.isNotEmpty() }
val knownAbis = listOf("arm64-v8a", "armeabi-v7a", "x86_64", "x86", "armeabi", "mips", "mips64", "riscv64")

// GitHub releases: one APK per ABI beside the universal one (-Panywhere.splitApks=true).
// Google Play gets the app bundle, which Play splits by itself.
val splitApks: Boolean = providers.gradleProperty("anywhere.splitApks").orNull == "true"

android {
    namespace = "io.github.jamezuk.lyrionanywhere"
    compileSdk = 37

    defaultConfig {
        applicationId = "io.github.jamezuk.lyrionanywhere"
        minSdk = 29
        targetSdk = 36
        versionCode = 42
        versionName = "1.17.3"
        // Only the ABIs the Rust layer is built for (above). Per-ABI APKs filter through
        // `splits` instead (AGP refuses both at once); the packaging rule below keeps the
        // universal APK to the same ABIs.
        if (!splitApks) {
            ndk {
                abiFilters += rustAbis
            }
        }
    }

    splits {
        abi {
            isEnable = splitApks
            reset()
            include(*rustAbis.toTypedArray())
            isUniversalApk = true
        }
    }

    packaging {
        jniLibs {
            excludes += knownAbis.filterNot { it in rustAbis }.map { "lib/$it/**" }
        }
    }

    signingConfigs {
        if (releaseKeystore != null) {
            create("release") {
                storeFile = file(releaseKeystore)
                storePassword = System.getenv("ANYWHERE_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("ANYWHERE_KEY_ALIAS")
                keyPassword = System.getenv("ANYWHERE_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            if (releaseKeystore != null) signingConfig = signingConfigs.getByName("release")
        }
        debug {
            applicationIdSuffix = ".debug"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    // F-Droid and reproducible builds: no Google dependency-info blob in the APK.
    dependenciesInfo {
        includeInApk = false
        includeInBundle = false
    }

    lint {
        abortOnError = true
        checkDependencies = true
        // Generated UniFFI bindings are not ours to lint.
        checkGeneratedSources = false
        disable += "GradleDependency"
        disable += "NewerVersionAvailable"
        disable += "AndroidGradlePluginVersion"
        // targetSdk stays at 36 for A0 (behaviour changes of 37 are reviewed separately);
        // compileSdk is 37 because current AndroidX releases require it.
        disable += "OldTargetApi"
        // The ABIs come from `anywhere.rust.abis` (x86_64 included by default), which lint
        // can't read; it would warn on every build.
        disable += "ChromeOsAbiSupport"
    }

    testOptions {
        unitTests.isIncludeAndroidResources = true
    }
}

/**
 * What ships, checked on the MERGED manifest: the source manifest alone can't show what the
 * libraries' manifests add.
 * - The androidx.startup initialisers must be exactly [expected]. A new one arriving with a
 *   library (WorkManager's did) runs before Application.onCreate and has crashed the app at
 *   launch.
 * - The exported components must be exactly [exported], each guarded by the permission given
 *   ("" for none). Anything exported is reachable by every other app: a library that exports
 *   something new, or drops a guard, fails the build until it is reviewed.
 * - The permissions asked for must be exactly [permissions], and the app declares none of its
 *   own but signature-level ones. `${applicationId}` stands for the package.
 */
abstract class VerifyMergedManifest : DefaultTask() {
    @get:InputFile
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val mergedManifest: RegularFileProperty

    @get:Input
    abstract val expected: SetProperty<String>

    @get:Input
    abstract val exported: MapProperty<String, String>

    @get:Input
    abstract val permissions: SetProperty<String>

    /** Touched when the check passes, so it is skipped while nothing changes. */
    @get:OutputFile
    abstract val marker: RegularFileProperty

    @TaskAction
    fun verify() {
        val android = "http://schemas.android.com/apk/res/android"
        val doc = javax.xml.parsers.DocumentBuilderFactory.newInstance()
            .apply {
                isNamespaceAware = true
                setFeature("http://apache.org/xml/features/disallow-doctype-decl", true)
            }
            .newDocumentBuilder()
            .parse(mergedManifest.get().asFile)
        fun elements(tag: String) = doc.getElementsByTagName(tag).let { list -> (0 until list.length).map { list.item(it) as org.w3c.dom.Element } }
        val pkg = doc.documentElement.getAttribute("package")
        fun portable(name: String) = if (pkg.isNotEmpty()) name.replace(pkg, "\${applicationId}") else name

        val providers = elements("provider")
        val startup = providers.filter { it.getAttributeNS(android, "name") == "androidx.startup.InitializationProvider" }
        check(startup.size == 1) { "expected one androidx.startup.InitializationProvider in the merged manifest, found ${startup.size}" }
        val metas = startup.single().getElementsByTagName("meta-data")
        val found = (0 until metas.length).map { metas.item(it) as org.w3c.dom.Element }
            .filter { it.getAttributeNS(android, "value") == "androidx.startup" }
            .map { it.getAttributeNS(android, "name") }
            .toSet()
        check(found == expected.get()) {
            "start-up initialisers in the merged manifest changed.\n  expected: ${expected.get().sorted()}\n" +
                "  found:    ${found.sorted()}\nA new initialiser runs before onCreate and has crashed the app " +
                "at launch: remove it with tools:node=\"remove\" (see AndroidManifest.xml) or review it and " +
                "update this list."
        }

        val components = listOf("activity", "activity-alias", "service", "receiver", "provider").flatMap(::elements)
        val open = components.filter { it.getAttributeNS(android, "exported") == "true" }
            .associate { it.getAttributeNS(android, "name") to it.getAttributeNS(android, "permission") }
        check(open == exported.get()) {
            "exported components in the merged manifest changed (name to the permission guarding it).\n" +
                "  expected: ${exported.get().toSortedMap()}\n  found:    ${open.toSortedMap()}\nEvery other app can " +
                "reach an exported component: review what a library added, then remove it with " +
                "tools:node=\"remove\" or update this list."
        }

        val asked = (elements("uses-permission") + elements("uses-permission-sdk-23"))
            .map { portable(it.getAttributeNS(android, "name")) }
            .toSet()
        check(asked == permissions.get()) {
            "permissions in the merged manifest changed.\n  expected: ${permissions.get().sorted()}\n" +
                "  found:    ${asked.sorted()}\nA library asked for a permission: review it, then remove it " +
                "with tools:node=\"remove\" or update this list."
        }
        val legacy = elements("uses-permission").single { it.getAttributeNS(android, "name") == "android.permission.BLUETOOTH" }
        check(legacy.getAttributeNS(android, "maxSdkVersion") == "30") { "the old BLUETOOTH permission must stop at Android 11 (maxSdkVersion 30)" }
        for (p in elements("permission")) {
            check(p.getAttributeNS(android, "protectionLevel") == "signature") {
                "the app declares ${p.getAttributeNS(android, "name")} below signature level: other apps could hold it"
            }
        }
        marker.get().asFile.writeText(
            (found.sorted() + open.toSortedMap().map { (k, v) -> "$k $v" } + asked.sorted()).joinToString("\n"),
        )
    }
}

/** Exported, and what guards each ("" for nothing: the app checks its callers itself). */
val exportedComponents: Map<String, String> = mapOf(
    // The launcher, voice "play …" and pairing links (only their text is read).
    "io.github.jamezuk.lyrionanywhere.MainActivity" to "",
    // Media library for cars: every caller is checked in onConnect (CallerPolicy).
    "io.github.jamezuk.lyrionanywhere.playback.PlaybackService" to "",
    // The launcher's widget host: only the update broadcast, which at worst redraws.
    "io.github.jamezuk.lyrionanywhere.widget.NowPlayingWidgetReceiver" to "",
    "io.github.jamezuk.lyrionanywhere.widget.NowPlayingLargeWidgetReceiver" to "",
    "io.github.jamezuk.lyrionanywhere.widget.PlayPauseTileService" to "android.permission.BIND_QUICK_SETTINGS_TILE",
    // Libraries' own, each behind a permission only the system holds.
    "androidx.media3.session.BluetoothValidationActivity" to "android.permission.BLUETOOTH_PRIVILEGED",
    "androidx.glance.appwidget.GlanceRemoteViewsService" to "android.permission.BIND_REMOTEVIEWS",
    "androidx.work.impl.background.systemjob.SystemJobService" to "android.permission.BIND_JOB_SERVICE",
    "androidx.work.impl.diagnostics.DiagnosticsReceiver" to "android.permission.DUMP",
    "androidx.profileinstaller.ProfileInstallReceiver" to "android.permission.DUMP",
)

/**
 * Debug builds only: Compose's tooling (previews) and test manifests, from the UI module's
 * debug dependencies. Never in a release.
 */
val debugOnlyExported: Map<String, String> = mapOf(
    "androidx.compose.ui.tooling.PreviewActivity" to "",
    "androidx.activity.ComponentActivity" to "",
)

val askedPermissions: Set<String> = setOf(
    "android.permission.INTERNET",
    "android.permission.CAMERA",
    "android.permission.FOREGROUND_SERVICE",
    "android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK",
    "android.permission.WAKE_LOCK",
    "android.permission.FOREGROUND_SERVICE_DATA_SYNC",
    "android.permission.POST_NOTIFICATIONS",
    "android.permission.BLUETOOTH_CONNECT",
    "android.permission.BLUETOOTH",
    // :playback (bit-perfect USB output), :data (Wi-Fi or mobile data).
    "android.permission.MODIFY_AUDIO_SETTINGS",
    "android.permission.ACCESS_NETWORK_STATE",
    // WorkManager's (brought by Glance), for work the app never schedules; normal, install-time.
    "android.permission.RECEIVE_BOOT_COMPLETED",
    // AndroidX core's guard for receivers registered not-exported on Android 12 and older.
    "\${applicationId}.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION",
)

androidComponents {
    onVariants { variant ->
        val name = variant.name.replaceFirstChar { it.uppercase() }
        val verify = tasks.register<VerifyMergedManifest>("verify${name}MergedManifest") {
            mergedManifest.set(variant.artifacts.get(com.android.build.api.artifact.SingleArtifact.MERGED_MANIFEST))
            expected.set(
                setOf(
                    "androidx.emoji2.text.EmojiCompatInitializer",
                    "androidx.lifecycle.ProcessLifecycleInitializer",
                    "androidx.profileinstaller.ProfileInstallerInitializer",
                ),
            )
            exported.set(if (variant.buildType == "debug") exportedComponents + debugOnlyExported else exportedComponents)
            permissions.set(askedPermissions)
            marker.set(layout.buildDirectory.file("merged-manifest-check/${variant.name}.txt"))
        }
        // Every APK, bundle and lint run of the variant checks it (CI runs lintDebug and
        // assembleDebug; releases run assembleRelease/bundleRelease).
        tasks.matching { it.name in setOf("assemble$name", "bundle$name", "lint$name") }
            .configureEach { dependsOn(verify) }
    }
}

dependencies {
    implementation(project(":ui"))
    implementation(project(":playback"))
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.androidx.datastore.preferences)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.compose.material3.adaptive.navigation.suite)
    // The home-screen widget.
    implementation(libs.androidx.glance.appwidget)
    // Foreground or not, for offline mode (its start-up initialiser is already merged in).
    implementation(libs.androidx.lifecycle.process)

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    testImplementation(libs.robolectric)
    testImplementation(libs.androidx.test.junit)
    testImplementation(platform(libs.compose.bom))
    testImplementation(libs.compose.ui.test.junit4)
}
