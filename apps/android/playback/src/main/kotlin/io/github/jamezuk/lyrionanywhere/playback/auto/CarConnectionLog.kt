package io.github.jamezuk.lyrionanywhere.playback.auto

import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** What [CallerPolicy] decided for a caller, as the diagnostic says it. */
enum class CarOutcome {
    /** Library and playback ([CallerAccess.BROWSE]). */
    ALLOWED,

    /** Transport controls only ([CallerAccess.CONTROL]). */
    CONTROLS,

    /** Nothing ([CallerAccess.REJECT]). */
    REFUSED,

    /** A known car or assistant package, refused because no certificate it offered is one we know. */
    REFUSED_CERTIFICATE,
}

/**
 * One caller decision. [certificatePrefixes]: for [CarOutcome.REFUSED_CERTIFICATE] only, the
 * first [CarConnectionLog.PREFIX_BYTES] bytes of each certificate digest the caller offered
 * (enough to tell a debug or sideloaded build from the release one, nothing more).
 */
data class CarConnection(
    val atMillis: Long,
    val packageName: String,
    val outcome: CarOutcome,
    val certificatePrefixes: List<String> = emptyList(),
)

/**
 * "Car connections" (Settings › About): the last few callers the media session decided on,
 * newest first, so a car that doesn't show the app can be told apart from one that was
 * refused. In memory only, held by the Application for its life: never persisted, never sent.
 */
class CarConnectionLog(
    private val capacity: Int = CAPACITY,
    private val clock: () -> Long = System::currentTimeMillis,
) {
    private val _entries = MutableStateFlow<List<CarConnection>>(emptyList())

    /** Newest first; at most [capacity]. */
    val entries: StateFlow<List<CarConnection>> = _entries.asStateFlow()

    /** Note what [CallerPolicy] decided for [facts]. */
    fun record(facts: CallerFacts, access: CallerAccess, known: List<KnownCaller> = CallerPolicy.KNOWN) {
        val knownPackage = known.any { it.packageName == facts.packageName }
        val outcome = when (access) {
            CallerAccess.BROWSE -> CarOutcome.ALLOWED
            CallerAccess.CONTROL -> CarOutcome.CONTROLS
            CallerAccess.REJECT ->
                if (knownPackage && facts.packageMatchesUid) CarOutcome.REFUSED_CERTIFICATE else CarOutcome.REFUSED
        }
        val prefixes = if (outcome == CarOutcome.REFUSED_CERTIFICATE) {
            facts.certificates.map(::digestPrefix).sorted().take(MAX_CERTIFICATES)
        } else {
            emptyList()
        }
        // Package names are bounded (Android allows far longer than any real one).
        val entry = CarConnection(clock(), facts.packageName.take(MAX_PACKAGE), outcome, prefixes)
        synchronized(this) {
            _entries.value = (listOf(entry) + _entries.value).take(capacity)
        }
    }

    companion object {
        const val CAPACITY = 10
        const val PREFIX_BYTES = 4
        const val MAX_CERTIFICATES = 4
        const val MAX_PACKAGE = 128

        /** "fd:b0:0c:43" from "fd:b0:0c:43:db:…" (only the first [PREFIX_BYTES] bytes). */
        fun digestPrefix(digest: String): String =
            digest.split(':').take(PREFIX_BYTES).joinToString(":") { b ->
                b.filter { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }.take(2).lowercase()
            }
    }
}

/** The words for a [CarConnection] line (from the app's resources). */
data class CarLabels(
    val androidAuto: String,
    val google: String,
    val carAssistant: String,
    val allowed: String,
    val controls: String,
    val refused: String,
    val refusedCertificate: String,
    /** "%1$s · %2$s · %3$s": caller, outcome, time. */
    val line: String,
    /** "Certificates offered: %1$s". */
    val certificates: String,
)

/** "12:03" in [zone]. */
fun carClock(atMillis: Long, zone: ZoneId = ZoneId.systemDefault()): String =
    DateTimeFormatter.ofPattern("HH:mm").withZone(zone).format(Instant.ofEpochMilli(atMillis))

/**
 * "Android Auto · allowed · 12:03", or for a refused known caller
 * "Android Auto · refused (certificate not recognised) · 12:04" and, on the next line,
 * "Certificates offered: ab:cd:ef:01". Other callers are named by their package.
 */
fun CarConnection.describe(labels: CarLabels, clock: (Long) -> String = { carClock(it) }): String {
    val caller = when (packageName) {
        "com.google.android.projection.gearhead" -> labels.androidAuto
        "com.google.android.googlequicksearchbox" -> labels.google
        "com.google.android.carassistant" -> labels.carAssistant
        else -> packageName
    }
    val outcome = when (outcome) {
        CarOutcome.ALLOWED -> labels.allowed
        CarOutcome.CONTROLS -> labels.controls
        CarOutcome.REFUSED -> labels.refused
        CarOutcome.REFUSED_CERTIFICATE -> labels.refusedCertificate
    }
    val line = labels.line.format(caller, outcome, clock(atMillis))
    return if (certificatePrefixes.isEmpty()) line else line + "\n" + labels.certificates.format(certificatePrefixes.joinToString(", "))
}
