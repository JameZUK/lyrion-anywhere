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
        versionCode = 40
        versionName = "1.17.1"
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
 * The androidx.startup initialisers in the MERGED manifest must be exactly [expected]. A new
 * one arriving with a library (WorkManager's did) runs before Application.onCreate and has
 * crashed the app at launch; the source manifest alone can't show what the merge adds.
 */
abstract class VerifyStartupInitialisers : DefaultTask() {
    @get:InputFile
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val mergedManifest: RegularFileProperty

    @get:Input
    abstract val expected: SetProperty<String>

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
        val providers = doc.getElementsByTagName("provider").let { list -> (0 until list.length).map { list.item(it) as org.w3c.dom.Element } }
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
        marker.get().asFile.writeText(found.sorted().joinToString("\n"))
    }
}

androidComponents {
    onVariants { variant ->
        val name = variant.name.replaceFirstChar { it.uppercase() }
        val verify = tasks.register<VerifyStartupInitialisers>("verify${name}StartupInitialisers") {
            mergedManifest.set(variant.artifacts.get(com.android.build.api.artifact.SingleArtifact.MERGED_MANIFEST))
            expected.set(
                setOf(
                    "androidx.emoji2.text.EmojiCompatInitializer",
                    "androidx.lifecycle.ProcessLifecycleInitializer",
                    "androidx.profileinstaller.ProfileInstallerInitializer",
                ),
            )
            marker.set(layout.buildDirectory.file("startup-initialisers/${variant.name}.txt"))
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
