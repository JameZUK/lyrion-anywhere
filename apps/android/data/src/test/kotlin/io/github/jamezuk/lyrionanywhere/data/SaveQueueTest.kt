package io.github.jamezuk.lyrionanywhere.data

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Saving the queue as a playlist: the name rules (as the gateway's) and the repository's checks. */
class PlaylistNamesTest {
    @Test
    fun goodNamesPassCleanedToNfcAndTrimmed() {
        for (n in listOf("Road trip", "Sigur Rós – live", "日本語", "Mix vol. 2", "AC-DC & friends (2026)", "Console", "é".repeat(100))) {
            assertNull(n, PlaylistNames.problem(PlaylistNames.clean(n)))
        }
        assertEquals("Café", PlaylistNames.clean("  Café \t"))
        assertEquals(4, PlaylistNames.clean("Café").length)
    }

    @Test
    fun badNamesAreRefusedWithAReason() {
        val cases = mapOf(
            "" to PlaylistNameProblem.EMPTY,
            "x".repeat(101) to PlaylistNameProblem.TOO_LONG,
            "new\nline" to PlaylistNameProblem.HIDDEN_CHARACTERS,
            "nul\u0000" to PlaylistNameProblem.HIDDEN_CHARACTERS,
            "esc\u001b[2J" to PlaylistNameProblem.HIDDEN_CHARACTERS,
            "bidi\u202etxt" to PlaylistNameProblem.HIDDEN_CHARACTERS,
            "zero\u200bwidth" to PlaylistNameProblem.HIDDEN_CHARACTERS,
            "../etc/passwd" to PlaylistNameProblem.RESERVED_CHARACTERS,
            "a/b" to PlaylistNameProblem.RESERVED_CHARACTERS,
            "a\\b" to PlaylistNameProblem.RESERVED_CHARACTERS,
            "name:x" to PlaylistNameProblem.RESERVED_CHARACTERS,
            "a*b" to PlaylistNameProblem.RESERVED_CHARACTERS,
            "why?" to PlaylistNameProblem.RESERVED_CHARACTERS,
            "say \"hi\"" to PlaylistNameProblem.RESERVED_CHARACTERS,
            "<b>" to PlaylistNameProblem.RESERVED_CHARACTERS,
            "a|b" to PlaylistNameProblem.RESERVED_CHARACTERS,
            ".." to PlaylistNameProblem.DOTS,
            "a..b" to PlaylistNameProblem.DOTS,
            ".hidden" to PlaylistNameProblem.LEADING_DOT,
            "trailing." to PlaylistNameProblem.TRAILING_DOT,
            "CON" to PlaylistNameProblem.RESERVED_NAME,
            "nul.txt" to PlaylistNameProblem.RESERVED_NAME,
        )
        for ((n, want) in cases) assertEquals(n, want, PlaylistNames.problem(n))
    }

    /** Arbitrary text never throws, and whatever passes keeps every rule. */
    @Test
    fun theCheckNeverThrows() {
        val r = java.util.Random(0x5eed)
        val alphabet = "ab Z.é/\\:*?\"<>|\u0000\n\t\u007f\u202e\u200b日 ..-_~%CON"
        repeat(5000) {
            val sb = StringBuilder()
            repeat(r.nextInt(130)) {
                if (r.nextInt(5) == 0) sb.appendCodePoint(r.nextInt(0x110000).let { c -> if (c in 0xD800..0xDFFF) 0x41 else c })
                else sb.append(alphabet[r.nextInt(alphabet.length)])
            }
            val clean = PlaylistNames.clean(sb.toString())
            if (PlaylistNames.problem(clean) == null) {
                assertTrue(clean.isNotEmpty() && clean.codePointCount(0, clean.length) <= PlaylistNames.MAX)
                assertFalse(clean, Regex("""[/\\:*?"<>|]|\.\.|^\.|\.$""").containsMatchIn(clean))
            }
        }
    }

    @Test
    fun suggestionsAreTheAlbumOrTheDate() {
        assertEquals("Kind of Blue", PlaylistNames.suggest(listOf("Kind of Blue", "Kind of Blue"), "14 October 2026"))
        assertEquals("Queue 14 October 2026", PlaylistNames.suggest(listOf("A", "B"), "14 October 2026"))
        assertEquals("Queue 14 October 2026", PlaylistNames.suggest(emptyList(), "14 October 2026"))
        assertEquals("Live 1990 91 Part 2", PlaylistNames.suggest(listOf("Live 1990/91: Part 2?"), "14 October 2026"))
        assertEquals("Queue 14 October 2026", PlaylistNames.suggest(listOf("..."), "14 October 2026"))
        assertNull(PlaylistNames.problem(PlaylistNames.suggest(listOf("x".repeat(300)), "d")))
    }
}

class SaveQueueRepositoryTest {
    private val noConnection = object : ConnectionSource {
        override val connection: StateFlow<Connection?> = MutableStateFlow(null)
    }
    private val p = "02:4c:41:00:00:01"

    @Test
    fun badNamesAndPlayersAreRefusedBeforeAnythingIsSent() = runTest {
        val repo = RustPlayersRepository(noConnection)
        for (bad in listOf("", "  ", "../x", "a/b", "x:y", ".x", "x.", "a..b", "nul\u0000", "CON", "x".repeat(101))) {
            val r = repo.saveQueue(p, bad, false)
            assertTrue("$bad: $r", r is ApiResult.Err && r.error is AppError.InvalidInput)
        }
        for (bad in listOf("not a player", "../../v1/me", "")) {
            val r = repo.saveQueue(bad, "Fine", false)
            assertTrue("$bad: $r", r is ApiResult.Err && r.error is AppError.InvalidInput)
        }
        // A good name and player reach the connection (here: none, so signed out).
        assertEquals(ApiResult.Err(AppError.SignedOut), repo.saveQueue(p, " Road trip ", true))
    }

    @Test
    fun signedOutOffersNothing() = runTest {
        assertEquals(ApiResult.Err(AppError.SignedOut), RustPlayersRepository(noConnection).canSaveQueue())
    }
}
