package io.github.jamezuk.lyrionanywhere

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** App shortcuts (Wave 2 §5.5): only their two actions are acted on, once, while fresh. */
class ShortcutsTest {
    @Test
    fun onlyTheTwoActionsAreRecognised() {
        assertEquals(ShortcutRequest.START_RADIO, Shortcuts.parse(Shortcuts.ACTION_START_RADIO))
        assertEquals(ShortcutRequest.SLEEP_30, Shortcuts.parse(Shortcuts.ACTION_SLEEP_30))
        for (bad in listOf(
            null,
            "",
            "android.intent.action.MAIN",
            Shortcuts.ACTION_START_RADIO.lowercase(),
            Shortcuts.ACTION_SLEEP_30 + " ",
            Shortcuts.ACTION_SLEEP_30 + "0",
            "io.github.jamezuk.lyrionanywhere.shortcut.SLEEP_3000",
            "io.github.jamezuk.lyrionanywhere.shortcut.",
            "x".repeat(100_000),
        )) {
            assertNull(bad?.take(40), Shortcuts.parse(bad))
        }
    }

    @Test
    fun aRequestIsTakenOnceAndOnlyWhileFresh() {
        var now = 1_000L
        val s = AppShortcuts { now }
        assertNull(s.take())
        s.offer(ShortcutRequest.SLEEP_30)
        assertEquals(ShortcutRequest.SLEEP_30, s.take())
        assertNull("only once", s.take())
        // The latest wins.
        s.offer(ShortcutRequest.SLEEP_30)
        s.offer(ShortcutRequest.START_RADIO)
        assertEquals(ShortcutRequest.START_RADIO, s.take())
        // Waiting too long (the app wasn't signed in): dropped.
        s.offer(ShortcutRequest.START_RADIO)
        now += Shortcuts.FRESH_MS + 1
        assertNull(s.take())
        assertNull(s.pending.value)
        // A clock that went backwards isn't trusted either.
        s.offer(ShortcutRequest.START_RADIO)
        now -= 10_000
        assertNull(s.take())
    }
}
