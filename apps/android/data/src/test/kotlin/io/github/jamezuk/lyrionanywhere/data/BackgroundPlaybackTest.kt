package io.github.jamezuk.lyrionanywhere.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.Preferences
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.job
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/** Playing with the screen off: when to offer, the refusals noted, the dismissal kept. */
class BackgroundPlaybackTest {
    @get:Rule val tmp = TemporaryFolder()

    private val scopes = mutableListOf<CoroutineScope>()

    private fun store(): DataStore<Preferences> {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO).also { scopes += it }
        return PreferenceDataStoreFactory.create(scope = scope) { tmp.root.resolve("settings.preferences_pb") }
    }

    private fun restart(): DataStore<Preferences> {
        runBlocking { scopes.forEach { it.coroutineContext.job.cancelAndJoin() } }
        scopes.clear()
        return store()
    }

    @After
    fun tearDown() = scopes.forEach { it.cancel() }

    @Test
    fun offeredOnlyToAPlayerAndroidRestrictsAndOnlyUntilDismissed() {
        for (play in listOf(true, false)) for (free in listOf(true, false)) for (gone in listOf(true, false)) {
            assertEquals("$play $free $gone", play && !free && !gone, BackgroundPlayback.shouldPrompt(play, free, gone))
            // Settings › Playback keeps the option after a dismissal.
            assertEquals("$play $free", play && !free, BackgroundPlayback.shouldOffer(play, free))
        }
    }

    @Test
    fun refusalsAreCountedAndNeverOverflow() {
        val b = BackgroundPlayback()
        assertEquals(0, b.refusals.value)
        b.foregroundRefused()
        b.foregroundRefused()
        assertEquals(2, b.refusals.value)
        repeat(3) { b.foregroundRefused() }
        assertEquals(5, b.refusals.value)
    }

    @Test
    fun theDismissalSurvivesARestart() = runBlocking {
        val before = DataStoreSettings(store())
        assertFalse(before.settings.first().backgroundPromptDismissed)
        before.setBackgroundPromptDismissed(true)
        assertTrue(DataStoreSettings(restart()).settings.first().backgroundPromptDismissed)
    }
}
