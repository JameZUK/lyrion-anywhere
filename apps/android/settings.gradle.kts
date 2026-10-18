pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

// F-Droid (and anyone avoiding prebuilt binaries) builds rustls-platform-verifier's Kotlin half
// from source: -Panywhere.verifier.source=<checkout>/android/rustls-platform-verifier.
val verifierFromSource: Boolean = providers.gradleProperty("anywhere.verifier.source").isPresent

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
        // BEGIN prebuilt-verifier (F-Droid's recipe deletes these lines)
        // The Kotlin half of rustls-platform-verifier (Android's certificate verifier, called
        // from Rust). Published only here; limited to that one artefact. The branch can change,
        // so the AAR (like every dependency) is pinned by its SHA-256 in
        // gradle/verification-metadata.xml, and Gradle refuses anything else.
        if (!verifierFromSource) {
            exclusiveContent {
                forRepository {
                    maven {
                        name = "rustlsPlatformVerifier"
                        url = uri("https://github.com/rustls/rustls-platform-verifier/raw/maven-archive/android-release-support/maven/")
                    }
                }
                filter { includeModule("org.rustls", "rustls-platform-verifier") }
            }
        }
        // END prebuilt-verifier
    }
}

rootProject.name = "LyrionAnywhere"

include(":app", ":ui", ":data", ":rust", ":playback")
if (verifierFromSource) include(":verifier-source")
