// Playback: the Media3 session around the Rust player (notification, lock screen,
// Bluetooth and headset buttons), audio focus, "becoming noisy", and wake/Wi-Fi locks.
plugins {
    alias(libs.plugins.android.library)
}

android {
    namespace = "io.github.jamezuk.lyrionanywhere.playback"
    compileSdk = 37
    defaultConfig {
        minSdk = 29
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    testOptions {
        unitTests.isIncludeAndroidResources = true
    }
}

dependencies {
    api(project(":data"))
    api(libs.media3.session)
    api(libs.media3.common)
    implementation(libs.androidx.core.ktx)
    implementation(libs.kotlinx.coroutines.android)

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    testImplementation(libs.robolectric)
    testImplementation(libs.androidx.test.junit)
}
