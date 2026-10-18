package io.github.jamezuk.lyrionanywhere

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Crash reports are shown with Copy and Share, so they must never carry secrets (pentest
 * 2026-10, A7: Android's record of a native crash includes memory dumps, which can hold the
 * tokens; before the fix its readable strings went into the report as they were).
 */
class CrashLogTest {
    private val access = "0123456789abcdef".repeat(4)
    private val refresh = "fedcba9876543210".repeat(4)
    private val sessionSecret = "00112233445566778899aabbccddeeff"

    @Test
    fun tokensAndSecretsAreRemoved() {
        val report = CrashLog.report(
            listOf(
                "java.lang.IllegalStateException: Authorization: Bearer $access failed",
                """{"access_token":"$access","refresh_token":"$refresh","password":"hunter2 hunter2"}""",
                "greeting {\"secret\": \"$sessionSecret\"} token=abc.def&x=1",
                "stray $refresh in memory",
            ),
        )!!
        for (secret in listOf(access, refresh, sessionSecret, "hunter2", "abc.def")) {
            assertFalse("$secret survived:\n$report", report.contains(secret))
        }
        assertTrue(report.contains("Bearer <redacted>"))
        assertTrue(report.contains("\"refresh_token\": \"<redacted>\""))
    }

    @Test
    fun theServerAndAddressesAreRemoved() {
        val text = CrashLog.redact(
            "error sending request for url (https://music.example.com:8443/v1/me) from 10.0.0.10 " +
                "via 2001:db8:0:0:0:0:0:1, wss://music.example.com/v1/player",
        )
        assertFalse(text, text.contains("music.example.com"))
        assertFalse(text, text.contains("10.0.0.10"))
        assertFalse(text, text.contains("2001:db8"))
        assertTrue(text, text.contains("https://<server>/v1/me"))
    }

    @Test
    fun homeFoldersInPanicLocationsAreRemoved() {
        val text = CrashLog.redact("panicked at /home/someone/.cargo/registry/src/x-1.0/src/lib.rs:10:5")
        assertEquals("panicked at /home/~/.cargo/registry/src/x-1.0/src/lib.rs:10:5", text)
    }

    @Test
    fun aStackTraceSurvivesRedaction() {
        val trace = "java.lang.NullPointerException\n" +
            "\tat io.github.jamezuk.lyrionanywhere.ui.Foo.bar(Foo.kt:42)\n" +
            "\tat kotlinx.coroutines.DispatchedTask.run(DispatchedTask.kt:108)"
        assertEquals(trace, CrashLog.redact(trace))
    }

    @Test
    fun aNativeTombstoneKeepsOnlyItsBacktraceNames() {
        // Readable runs as they come out of a tombstone: the backtrace's libraries and
        // symbols, and memory near the registers, where a token or password may sit.
        val readable = listOf(
            "/data/app/~~x/io.github.jamezuk.lyrionanywhere-1/lib/arm64/libanywhere_mobile.so",
            "_ZN15anywhere_mobile6player6engine3run17h0123456789abcdefE",
            "_RNvCs1234_15anywhere_mobile4main",
            "/apex/com.android.runtime/lib64/bionic/libc.so",
            "Abort message: 'panic'",
            "Authorization: Bearer $access",
            "my password is correct horse",
            "music.example.com",
            refresh,
        ).joinToString("\n")
        val kept = CrashLog.nativeSummary(readable)
        assertTrue(kept, kept.contains("libanywhere_mobile.so"))
        assertTrue(kept, kept.contains("_ZN15anywhere_mobile6player6engine3run"))
        assertTrue(kept, kept.contains("libc.so"))
        for (gone in listOf(access, refresh, "correct horse", "music.example.com", "Bearer")) {
            assertFalse("$gone survived:\n$kept", kept.contains(gone))
        }
    }

    @Test
    fun nothingToReportIsNull() {
        assertNull(CrashLog.report(emptyList()))
    }

    // Safe mode (review, 15 October): only repeated crashes stop the app starting; a single
    // crash or an ANR shows its report once and the app carries on.

    private val minute = 60_000L

    @Test
    fun nothingPendingStartsNormally() {
        assertEquals(CrashLog.StartMode.NORMAL, CrashLog.startMode(emptyList(), seenUpTo = 0, hasReport = false))
    }

    @Test
    fun aSingleCrashShowsItsReportOnceAndCarriesOn() {
        assertEquals(CrashLog.StartMode.REPORT_ONCE, CrashLog.startMode(listOf(10 * minute), seenUpTo = 0, hasReport = true))
    }

    @Test
    fun twoCrashesInQuickSuccessionAreSafeMode() {
        val times = listOf(10 * minute, 10 * minute + 20_000)
        assertEquals(CrashLog.StartMode.SAFE, CrashLog.startMode(times, seenUpTo = 0, hasReport = true))
        // Even when the first one's report was already shown (a crash at every start).
        assertEquals(CrashLog.StartMode.SAFE, CrashLog.startMode(times, seenUpTo = 10 * minute, hasReport = true))
    }

    @Test
    fun crashesFarApartAreNotALoop() {
        val times = listOf(10 * minute, 10 * minute + CrashLog.REPEAT_WINDOW_MS + 1)
        assertEquals(CrashLog.StartMode.REPORT_ONCE, CrashLog.startMode(times, seenUpTo = 10 * minute, hasReport = true))
    }

    @Test
    fun crashesAlreadySeenDontComeBack() {
        val times = listOf(10 * minute, 10 * minute + 20_000)
        assertEquals(CrashLog.StartMode.NORMAL, CrashLog.startMode(times, seenUpTo = 10 * minute + 20_000, hasReport = false))
    }

    @Test
    fun onlyCrashesCountNotAnrsOrOtherExits() {
        assertTrue(CrashLog.isCrash(android.app.ApplicationExitInfo.REASON_CRASH))
        assertTrue(CrashLog.isCrash(android.app.ApplicationExitInfo.REASON_CRASH_NATIVE))
        assertTrue(CrashLog.isCrash(android.app.ApplicationExitInfo.REASON_INITIALIZATION_FAILURE))
        assertFalse(CrashLog.isCrash(android.app.ApplicationExitInfo.REASON_ANR))
        assertFalse(CrashLog.isCrash(android.app.ApplicationExitInfo.REASON_USER_REQUESTED))
        assertFalse(CrashLog.isCrash(android.app.ApplicationExitInfo.REASON_LOW_MEMORY))
    }
}
