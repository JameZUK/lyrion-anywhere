package io.github.jamezuk.lyrionanywhere

import android.content.Intent
import android.provider.MediaStore
import android.view.KeyEvent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** What the app does with the intents it is started with, and the volume keys. */
class LaunchIntentsTest {
    private val search = MediaStore.INTENT_ACTION_MEDIA_PLAY_FROM_SEARCH

    @Test
    fun voiceSearchAndPairingLinksAreActedOnOnce() {
        assertTrue(LaunchIntents.shouldHandle(search, 0))
        assertTrue(LaunchIntents.shouldHandle(Intent.ACTION_VIEW, Intent.FLAG_ACTIVITY_NEW_TASK))
        // Reopened from Recents: Android hands back the first intent; it must not replay.
        assertFalse(LaunchIntents.shouldHandle(search, Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY))
        assertFalse(
            LaunchIntents.shouldHandle(Intent.ACTION_VIEW, Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY or Intent.FLAG_ACTIVITY_NEW_TASK),
        )
        // The launcher (and the plain intent a handled one is replaced with) only opens the app.
        assertFalse(LaunchIntents.shouldHandle(Intent.ACTION_MAIN, 0))
        assertFalse(LaunchIntents.shouldHandle(null, 0))
        assertFalse(LaunchIntents.shouldHandle("android.intent.action.SEND", 0))
    }

    @Test
    fun aSpokenQueryIsBoundedPlainText() {
        assertEquals("Kind of Blue", LaunchIntents.searchQuery("  Kind of Blue \n"))
        // Control characters (a terminal escape, a NUL) are dropped; the rest is plain text.
        assertEquals("a[2Jb", LaunchIntents.searchQuery("a\u0000\u001b[2Jb"))
        assertNull(LaunchIntents.searchQuery(null))
        assertNull(LaunchIntents.searchQuery(" \t\r\n"))
        // Huge input: cut to the limit (and only a bounded prefix is ever looked at).
        val huge = "x".repeat(1_000_000)
        assertEquals(LaunchIntents.MAX_QUERY, LaunchIntents.searchQuery(huge)!!.length)
        val controlsFirst = "\u0001".repeat(LaunchIntents.MAX_QUERY * 4) + "hidden"
        assertNull(LaunchIntents.searchQuery(controlsFirst))
        // Markup and format strings are just text.
        assertEquals("<b>%s %1\$s</b>", LaunchIntents.searchQuery("<b>%s %1\$s</b>"))
    }

    @Test
    fun volumeKeysStepARemotePlayerOrAreLeftAlone() {
        val keys = VolumeKeys()
        // No signed-in screen: the system's.
        assertFalse(keys.onKeyDown(KeyEvent.KEYCODE_VOLUME_UP))
        assertFalse(keys.onKeyUp(KeyEvent.KEYCODE_VOLUME_UP))
        val steps = mutableListOf<Boolean>()
        var remote = true
        keys.handler = { up -> if (remote) steps.add(up) else false }
        assertTrue(keys.onKeyDown(KeyEvent.KEYCODE_VOLUME_UP))
        assertTrue(keys.onKeyUp(KeyEvent.KEYCODE_VOLUME_UP))
        assertTrue(keys.onKeyDown(KeyEvent.KEYCODE_VOLUME_DOWN))
        assertTrue(keys.onKeyUp(KeyEvent.KEYCODE_VOLUME_DOWN))
        assertEquals(listOf(true, false), steps)
        // Other keys are never taken.
        assertFalse(keys.onKeyDown(KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE))
        assertFalse(keys.onKeyUp(KeyEvent.KEYCODE_ENTER))
        // This phone is the player: the handler declines, the system sets the volume.
        remote = false
        assertFalse(keys.onKeyDown(KeyEvent.KEYCODE_VOLUME_UP))
        assertFalse(keys.onKeyUp(KeyEvent.KEYCODE_VOLUME_UP))
        keys.handler = null
        assertFalse(keys.onKeyDown(KeyEvent.KEYCODE_VOLUME_DOWN))
    }
}
