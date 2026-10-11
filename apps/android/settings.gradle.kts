pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
        // The Kotlin half of rustls-platform-verifier (Android's certificate verifier, called
        // from Rust). Published only here; limited to that one artefact.
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
}

rootProject.name = "LyrionAnywhere"

include(":app", ":ui", ":data", ":rust", ":playback")
