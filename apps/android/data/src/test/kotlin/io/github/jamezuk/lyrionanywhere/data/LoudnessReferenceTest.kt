package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.NoHandle
import io.github.jamezuk.lyrionanywhere.core.Session
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * Loudness compensation follows the phone's media volume while this phone plays: the
 * repository passes the attenuation on (cleaned, once per change) and gives it to every new
 * engine before it starts.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class LoudnessReferenceTest {
    private class Settings : SettingsStore {
        val state = MutableStateFlow(AppSettings(playOnPhone = true))
        override val settings: Flow<AppSettings> = state

        override suspend fun setServerUrl(url: String?) {}

        override suspend fun setSelectedPlayer(playerId: String?) {}

        override suspend fun setPlayOnPhone(on: Boolean) = state.update { it.copy(playOnPhone = on) }

        override suspend fun setSound(sound: SoundSettings) = state.update { it.copy(sound = sound) }

        override suspend fun setLibrary(id: String?) {}

        override suspend fun setDefaultPlayer(choice: DefaultPlayer, playerId: String?) {}
    }

    private class Handle : LocalPlayerHandle {
        /** Every reference given, in order; "start" marks when the engine started. */
        val calls = mutableListOf<Any?>()

        override fun start(observer: io.github.jamezuk.lyrionanywhere.core.LocalPlayerObserver) {
            calls += "start"
        }

        override fun stop() {}

        override fun setHold(hold: Boolean) {}

        override fun setDuck(duck: Boolean) {}

        override fun setOutputRoute(route: io.github.jamezuk.lyrionanywhere.core.OutputRoute) {}

        override fun setDsp(dsp: io.github.jamezuk.lyrionanywhere.core.DspSettings) {}

        override fun setOutputNegotiator(negotiator: io.github.jamezuk.lyrionanywhere.core.OutputNegotiator?) {}

        override fun renegotiateOutput() {}

        override fun setLoudnessReferenceDb(db: Double?) {
            calls += db
        }

        override fun close() {}
    }

    private val settings = Settings()
    private val handles = mutableListOf<Handle>()

    private fun TestScope.repo(): RustLocalPlayerRepository {
        val d = UnconfinedTestDispatcher(testScheduler)
        val scope = CoroutineScope(backgroundScope.coroutineContext + d)
        return RustLocalPlayerRepository(
            MutableStateFlow(Session(NoHandle)),
            settings,
            scope,
            d,
            factory = { Handle().also { handles += it } },
            serverProtocol = { 2u },
        )
    }

    @Test
    fun theAttenuationReachesTheEngineOncePerChangeAndSurvivesARestart() = runTest {
        val r = repo()
        val h = handles.single()
        // A new engine is told (nothing known yet) before it starts.
        assertEquals(listOf<Any?>(null, "start"), h.calls)
        r.setLoudnessReferenceDb(18.5)
        r.setLoudnessReferenceDb(18.5)
        assertEquals(listOf<Any?>(null, "start", 18.5), h.calls)
        // A Bluetooth headset with its own volume: not known.
        r.setLoudnessReferenceDb(null)
        assertNull(h.calls.last())
        r.setLoudnessReferenceDb(30.0)
        // The next engine starts with what is known now.
        r.setEnabled(false)
        r.setEnabled(true)
        assertEquals(listOf<Any?>(30.0, "start"), handles.last().calls)
    }

    @Test
    fun oddValuesAreCleanedBeforeTheyLeave() {
        assertNull(cleanLoudnessReference(null))
        assertNull(cleanLoudnessReference(Double.NaN))
        assertEquals(MAX_LOUDNESS_REFERENCE_DB, cleanLoudnessReference(Double.POSITIVE_INFINITY)!!, 0.0)
        assertEquals(MAX_LOUDNESS_REFERENCE_DB, cleanLoudnessReference(1e9)!!, 0.0)
        assertEquals(0.0, cleanLoudnessReference(Double.NEGATIVE_INFINITY)!!, 0.0)
        assertEquals(0.0, cleanLoudnessReference(-4.0)!!, 0.0)
        assertEquals(12.25, cleanLoudnessReference(12.25)!!, 0.0)
    }
}
