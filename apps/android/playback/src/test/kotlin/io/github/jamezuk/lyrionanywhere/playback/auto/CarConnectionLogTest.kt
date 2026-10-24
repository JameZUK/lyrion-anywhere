package io.github.jamezuk.lyrionanywhere.playback.auto

import java.time.ZoneOffset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

private const val GEARHEAD = "com.google.android.projection.gearhead"
private const val SIDELOADED_DIGEST = "ab:cd:ef:01:23:45:67:89:ab:cd:ef:01:23:45:67:89:ab:cd:ef:01:23:45:67:89:ab:cd:ef:01:23:45:67:89"

private fun caller(pkg: String = GEARHEAD, certs: Set<String> = emptySet(), matches: Boolean = true) =
    CallerFacts(pkg, 10_123, matches, self = false, systemUid = false, platformSigned = false, mediaContentControl = false, certificates = certs, media3Trusted = false)

private val LABELS = CarLabels(
    androidAuto = "Android Auto",
    google = "Google app",
    carAssistant = "Car assistant",
    allowed = "allowed",
    controls = "controls only",
    refused = "refused",
    refusedCertificate = "refused (certificate not recognised)",
    line = "%1\$s · %2\$s · %3\$s",
    certificates = "Certificates offered: %1\$s",
)

/** Settings › About › Car connections: an in-memory ring of the media session's decisions. */
class CarConnectionLogTest {
    private var now = 0L
    private val log = CarConnectionLog(clock = { now })

    private fun utc(at: Long) = carClock(at, ZoneOffset.UTC)

    @Test
    fun startsEmptyAndKeepsTheNewestTen() {
        assertTrue(log.entries.value.isEmpty())
        repeat(25) { i ->
            now = i * 60_000L
            log.record(caller("com.example.app$i"), CallerAccess.REJECT)
        }
        val e = log.entries.value
        assertEquals(CarConnectionLog.CAPACITY, e.size)
        assertEquals("newest first", "com.example.app24", e.first().packageName)
        assertEquals("com.example.app15", e.last().packageName)
    }

    @Test
    fun outcomes() {
        log.record(caller(certs = setOf(CallerPolicy.KNOWN.first().certificates.first())), CallerAccess.BROWSE)
        log.record(caller("com.example.watch"), CallerAccess.CONTROL)
        log.record(caller("com.example.other"), CallerAccess.REJECT)
        log.record(caller(certs = setOf(SIDELOADED_DIGEST)), CallerAccess.REJECT)
        // A known name on someone else's uid is just refused (it isn't that app at all).
        log.record(caller(matches = false, certs = setOf(SIDELOADED_DIGEST)), CallerAccess.REJECT)
        assertEquals(
            listOf(CarOutcome.REFUSED, CarOutcome.REFUSED_CERTIFICATE, CarOutcome.REFUSED, CarOutcome.CONTROLS, CarOutcome.ALLOWED),
            log.entries.value.map { it.outcome },
        )
    }

    @Test
    fun onlyTheFirstBytesOfARefusedCertificateAreKept() {
        log.record(caller(certs = setOf(SIDELOADED_DIGEST, "FD:B0:0C:43:99:99")), CallerAccess.REJECT)
        val e = log.entries.value.single()
        assertEquals(listOf("ab:cd:ef:01", "fd:b0:0c:43"), e.certificatePrefixes)
        for (p in e.certificatePrefixes) assertEquals(CarConnectionLog.PREFIX_BYTES, p.split(':').size)
        // Nothing of an allowed caller's certificates is kept.
        log.record(caller(certs = setOf(SIDELOADED_DIGEST)), CallerAccess.BROWSE)
        assertTrue(log.entries.value.first().certificatePrefixes.isEmpty())
        // Nor of an unknown caller's.
        log.record(caller("com.example.other", certs = setOf(SIDELOADED_DIGEST)), CallerAccess.REJECT)
        assertTrue(log.entries.value.first().certificatePrefixes.isEmpty())
    }

    @Test
    fun hostileValuesAreBounded() {
        val many = (0 until 50).map { "%02x:".repeat(32).format(*Array(32) { _ -> it }).trimEnd(':') }.toSet()
        log.record(caller(certs = many + "zz:<script>:\u0000‮:41"), CallerAccess.REJECT)
        val e = log.entries.value.single()
        assertTrue(e.certificatePrefixes.size <= CarConnectionLog.MAX_CERTIFICATES)
        for (p in e.certificatePrefixes) assertTrue(p, p.all { it == ':' || it in '0'..'9' || it in 'a'..'f' })
        log.record(caller("x".repeat(10_000)), CallerAccess.REJECT)
        assertEquals(CarConnectionLog.MAX_PACKAGE, log.entries.value.first().packageName.length)
        assertEquals("", CarConnectionLog.digestPrefix(""))
    }

    @Test
    fun linesSayWhoWhatAndWhen() {
        now = 12 * 3_600_000L + 3 * 60_000L
        log.record(caller(certs = setOf(CallerPolicy.KNOWN.first().certificates.first())), CallerAccess.BROWSE)
        now += 60_000L
        log.record(caller(certs = setOf(SIDELOADED_DIGEST)), CallerAccess.REJECT)
        now += 60_000L
        log.record(caller("com.example.watch"), CallerAccess.CONTROL)
        val lines = log.entries.value.map { it.describe(LABELS, ::utc) }
        assertEquals(
            listOf(
                "com.example.watch · controls only · 12:05",
                "Android Auto · refused (certificate not recognised) · 12:04\nCertificates offered: ab:cd:ef:01",
                "Android Auto · allowed · 12:03",
            ),
            lines,
        )
        // The full digest never reaches the screen.
        assertFalse(lines.any { SIDELOADED_DIGEST in it })
        assertEquals("Google app · refused · 00:00", CarConnection(0, "com.google.android.googlequicksearchbox", CarOutcome.REFUSED).describe(LABELS, ::utc))
        assertEquals("Car assistant · allowed · 00:00", CarConnection(0, "com.google.android.carassistant", CarOutcome.ALLOWED).describe(LABELS, ::utc))
    }
}
