plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

// Release signing comes from the environment (CI secrets or a local shell), never from
// the repository. Without it, release builds are unsigned.
val releaseKeystore: String? = System.getenv("ANYWHERE_KEYSTORE")

android {
    namespace = "io.github.jamezuk.lyrionanywhere"
    compileSdk = 37

    defaultConfig {
        applicationId = "io.github.jamezuk.lyrionanywhere"
        minSdk = 29
        targetSdk = 36
        versionCode = 10
        versionName = "0.1.0-alpha.10"
        // Only the ABIs the Rust layer is built for (keep in step with `anywhere.rust.abis`
        // in gradle.properties): dependencies such as JNA ship more (armeabi, mips, x86),
        // and a device preferring one of those would load that folder, find no Rust library
        // and crash.
        ndk {
            abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64")
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
    }

    testOptions {
        unitTests.isIncludeAndroidResources = true
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

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
}
