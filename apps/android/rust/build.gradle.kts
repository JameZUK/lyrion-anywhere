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
            val targets = abis.get().flatMap { listOf("-t", it) }
            commandLine(
                listOf("cargo", "ndk") + targets + listOf(
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
                "cargo", "run", "-q", "--locked", "-p", "anywhere-uniffi-bindgen", "--",
                "generate", "--library", library.get().asFile.absolutePath,
                "--language", "kotlin", "--no-format", "--out-dir", out.absolutePath,
            )
        }
    }
}

val verifyVerifier = tasks.register<VerifyVerifierVersion>("verifyRustlsVerifierVersion") {
    lockFile.set(workspaceDir.file("Cargo.lock"))
    expected.set(libs.versions.rustls.platform.verifier)
}

androidComponents {
    onVariants { variant ->
        val name = variant.name.replaceFirstChar { it.uppercase() }
        // Debug APKs use a quick, stripped build; release builds the optimised library.
        val cargoProfile = if (variant.buildType == "release") "release" else "android-dev"
        val cargo = tasks.register<CargoNdkBuild>("cargoBuild$name") {
            dependsOn(verifyVerifier)
            abis.set(rustAbis)
            profile.set(cargoProfile)
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
    implementation(libs.rustls.platform.verifier)
    implementation(libs.androidx.core.ktx)
    api(libs.kotlinx.coroutines.android)
}
