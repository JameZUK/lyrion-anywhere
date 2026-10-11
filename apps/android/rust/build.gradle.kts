// The Rust host layer (`mobile/` in the repository root) as an Android library:
// - `cargoBuild<Variant>` cross-compiles libanywhere_mobile.so with cargo-ndk for each ABI in
//   `anywhere.rust.abis`, into a generated jniLibs directory;
// - `uniffiBindgen<Variant>` generates the Kotlin bindings from that library (UniFFI
//   library mode) into a generated source directory.
// Both are wired as generated sources of the variant, so `./gradlew assembleDebug` builds
// everything from a clean checkout. Needs `cargo` and `cargo-ndk` on PATH and the Android
// Rust targets installed (see apps/android/README.md).

import javax.inject.Inject
import org.gradle.process.ExecOperations

plugins {
    alias(libs.plugins.android.library)
}

val workspaceDir: Directory = rootProject.layout.projectDirectory.dir("../..")
val rustAbis: List<String> =
    providers.gradleProperty("anywhere.rust.abis").get().split(',').map { it.trim() }.filter { it.isNotEmpty() }
val ndkVersionPinned = "29.0.14206865"


android {
    namespace = "io.github.jamezuk.lyrionanywhere.rust"
    compileSdk = 37
    ndkVersion = ndkVersionPinned
    defaultConfig {
        minSdk = 29
        consumerProguardFiles("consumer-rules.pro")
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

/** The Rust sources that affect the library (for up-to-date checks). */
fun rustSources(): FileTree = workspaceDir.asFileTree.matching {
    include("Cargo.toml", "Cargo.lock", "mobile/**", "core/src/**", "core/Cargo.toml")
    exclude("**/target/**")
}

/** Checks that the Kotlin half of rustls-platform-verifier matches the Rust half. */
abstract class VerifyVerifierVersion : DefaultTask() {
    @get:InputFile
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val lockFile: RegularFileProperty

    @get:Input
    abstract val expected: Property<String>

    @TaskAction
    fun verify() {
        val lines = lockFile.get().asFile.readLines()
        val idx = lines.indexOfFirst { it.trim() == "name = \"rustls-platform-verifier-android\"" }
        val version = lines.drop(idx + 1).firstOrNull { it.trim().startsWith("version = ") }
            ?.substringAfter('"')?.substringBefore('"')
        check(idx >= 0 && version == expected.get()) {
            "rustls-platform-verifier-android is $version in Cargo.lock but ${expected.get()} in " +
                "gradle/libs.versions.toml; they must match"
        }
    }
}

abstract class CargoNdkBuild : DefaultTask() {
    @get:Inject
    abstract val exec: ExecOperations

    @get:Input
    abstract val abis: ListProperty<String>

    @get:Input
    abstract val profile: Property<String>

    @get:Input
    abstract val apiLevel: Property<Int>

    @get:Internal
    abstract val workspace: DirectoryProperty

    @get:Internal
    abstract val ndkDir: DirectoryProperty

    /**
     * Strip build-machine paths from the library (reproducible builds): the workspace and
     * Cargo's home are rewritten to fixed names in panic locations and debug data, so the
     * same commit builds the same library wherever it is checked out.
     */
    @get:Input
    abstract val remapPaths: Property<Boolean>

    /** The `cargo` to run (`anywhere.cargo`, e.g. for a toolchain not on PATH). */
    @get:Input
    abstract val cargoExecutable: Property<String>

    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val sources: ConfigurableFileCollection

    @get:OutputDirectory
    abstract val outputDir: DirectoryProperty

    @TaskAction
    fun build() {
        val out = outputDir.get().asFile
        out.deleteRecursively()
        out.mkdirs()
        exec.exec {
            workingDir = workspace.get().asFile
            environment("ANDROID_NDK_HOME", ndkDir.get().asFile.absolutePath)
            environment("CARGO_INCREMENTAL", "0")
            if (remapPaths.get()) {
                val cargoHome = System.getenv("CARGO_HOME") ?: "${System.getProperty("user.home")}/.cargo"
                val targetDir = System.getenv("CARGO_TARGET_DIR")
                val flags = listOfNotNull(
                    "--remap-path-prefix=${workspace.get().asFile.canonicalPath}=/build",
                    "--remap-path-prefix=$cargoHome=/cargo",
                    targetDir?.let { "--remap-path-prefix=$it=/target" },
                ).joinToString(" ")
                // Per-target flags, so host build scripts (and any RUSTFLAGS set for other
                // targets) are left alone.
                for (triple in listOf("aarch64-linux-android", "armv7-linux-androideabi", "x86_64-linux-android", "i686-linux-android")) {
                    environment("CARGO_TARGET_${triple.uppercase().replace('-', '_')}_RUSTFLAGS", flags)
                }
            }
            val targets = abis.get().flatMap { listOf("-t", it) }
            commandLine(
                listOf(cargoExecutable.get(), "ndk") + targets + listOf(
                    "--platform", apiLevel.get().toString(),
                    "-o", out.absolutePath,
                    "build", "-p", "anywhere-mobile", "--locked", "--profile", profile.get(),
                ),
            )
        }
    }
}

abstract class UniffiBindgen : DefaultTask() {
    @get:Inject
    abstract val exec: ExecOperations

    @get:InputFile
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val library: RegularFileProperty

    @get:InputFile
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val config: RegularFileProperty

    @get:Internal
    abstract val workspace: DirectoryProperty

    @get:Input
    abstract val cargoExecutable: Property<String>

    @get:OutputDirectory
    abstract val outputDir: DirectoryProperty

    @TaskAction
    fun generate() {
        val out = outputDir.get().asFile
        out.deleteRecursively()
        out.mkdirs()
        exec.exec {
            workingDir = workspace.get().asFile
            environment("CARGO_INCREMENTAL", "0")
            commandLine(
                cargoExecutable.get(), "run", "-q", "--locked", "-p", "anywhere-uniffi-bindgen", "--",
                "generate", "--library", library.get().asFile.absolutePath,
                "--language", "kotlin", "--no-format", "--out-dir", out.absolutePath,
            )
        }
    }
}

// `cargo` from PATH unless -Panywhere.cargo=/path/to/cargo says otherwise (F-Droid).
val cargoCommand: Provider<String> = providers.gradleProperty("anywhere.cargo").orElse("cargo")

val verifyVerifier = tasks.register<VerifyVerifierVersion>("verifyRustlsVerifierVersion") {
    lockFile.set(workspaceDir.file("Cargo.lock"))
    expected.set(libs.versions.rustls.platform.verifier)
}

androidComponents {
    onVariants { variant ->
        val name = variant.name.replaceFirstChar { it.uppercase() }
        // Debug APKs use a quick, stripped build; release builds the optimised library
        // (Cargo.toml's `android-release`: LTO, one codegen unit) without build paths.
        val isRelease = variant.buildType != "debug"
        val cargoProfile = if (isRelease) "android-release" else "android-dev"
        val cargo = tasks.register<CargoNdkBuild>("cargoBuild$name") {
            dependsOn(verifyVerifier)
            abis.set(rustAbis)
            profile.set(cargoProfile)
            remapPaths.set(isRelease)
            cargoExecutable.set(cargoCommand)
            apiLevel.set(29)
            workspace.set(workspaceDir)
            ndkDir.set(androidComponents.sdkComponents.ndkDirectory)
            sources.from(rustSources())
            outputDir.set(layout.buildDirectory.dir("rust/${variant.name}/jniLibs"))
        }
        variant.sources.jniLibs?.addGeneratedSourceDirectory(cargo, CargoNdkBuild::outputDir)

        val bindgen = tasks.register<UniffiBindgen>("uniffiBindgen$name") {
            library.set(cargo.flatMap { it.outputDir.file("${rustAbis.first()}/libanywhere_mobile.so") })
            config.set(workspaceDir.file("mobile/uniffi.toml"))
            cargoExecutable.set(cargoCommand)
            workspace.set(workspaceDir)
            outputDir.set(layout.buildDirectory.dir("generated/uniffi/${variant.name}/kotlin"))
        }
        variant.sources.kotlin?.addGeneratedSourceDirectory(bindgen, UniffiBindgen::outputDir)
    }
}

dependencies {
    // UniFFI's Kotlin bindings call the library through JNA.
    api("${libs.jna.get()}@aar")
    // Android's certificate verifier, called by the Rust TLS stack through JNI.
    // From source when `anywhere.verifier.source` is set (F-Droid), else the prebuilt artefact.
    if (providers.gradleProperty("anywhere.verifier.source").isPresent) {
        implementation(project(":verifier-source"))
    } else {
        implementation(libs.rustls.platform.verifier)
    }
    implementation(libs.androidx.core.ktx)
    api(libs.kotlinx.coroutines.android)
}
