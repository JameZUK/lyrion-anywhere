// The Kotlin half of rustls-platform-verifier, built from its source instead of the
// project's prebuilt Maven artefact. Used only when `anywhere.verifier.source` names the
// `android/rustls-platform-verifier` folder of a rustls-platform-verifier checkout at the tag
// matching Cargo.lock (F-Droid builds it this way, from a srclib: see apps/android/fdroid/).
plugins {
    alias(libs.plugins.android.library)
}

val verifierSource: String = providers.gradleProperty("anywhere.verifier.source").get()

android {
    namespace = "org.rustls.platformverifier"
    compileSdk = 37
    defaultConfig {
        minSdk = 29
        // No consumer rules from upstream (they hold a global -dontobfuscate): the :rust
        // module's consumer-rules.pro keeps every org.rustls.platformverifier class, which
        // Rust calls by name.
        // Upstream's test hooks are compiled out of release builds through this flag.
        buildConfigField("boolean", "TEST", "false")
    }
    buildFeatures {
        buildConfig = true
    }
    sourceSets {
        getByName("main") {
            manifest.srcFile("$verifierSource/src/main/AndroidManifest.xml")
            // Upstream keeps its Kotlin under java/.
            kotlin.srcDir("$verifierSource/src/main/java")
            res.srcDir("$verifierSource/src/main/res")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    lint {
        // Upstream code, linted upstream.
        abortOnError = false
        checkReleaseBuilds = false
    }
}
