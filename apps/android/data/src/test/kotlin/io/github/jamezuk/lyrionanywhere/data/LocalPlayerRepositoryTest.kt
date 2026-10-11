package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.ApiException
import io.github.jamezuk.lyrionanywhere.core.CrossfeedSetting
import io.github.jamezuk.lyrionanywhere.core.DitherSetting
import io.github.jamezuk.lyrionanywhere.core.DspSettings
import io.github.jamezuk.lyrionanywhere.core.Levelling
import io.github.jamezuk.lyrionanywhere.core.MixerOffer
import io.github.jamezuk.lyrionanywhere.core.OfferBlocker
import io.github.jamezuk.lyrionanywhere.core.OutputNegotiator
import io.github.jamezuk.lyrionanywhere.core.OutputOffers
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerObserver
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerStatus
import io.github.jamezuk.lyrionanywhere.core.NoHandle
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
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
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

fun status(id: String = "02:4c:41:00:00:01", connection: PlayerConnection = PlayerConnection.Connected) =
    LocalPlayerStatus(
        connection = connection,
        playback = LocalPlayback.PLAYING,
        playerId = id,
        name = "Phone",
        elapsedMs = 0u,
        bufferedMs = 0u,
        outputRate = 44_100u,
        underruns = 0u,
        underrunFrames = 0u,
        held = false,
        lastError = null,
    )

private class MemSettings : SettingsStore {
    // Off to start with, so each test turns the phone's player on itself.
    val state = MutableStateFlow(AppSettings(playOnPhone = false))
    override val settings: Flow<AppSettings> = state

    override suspend fun setServerUrl(url: String?) = state.update { it.copy(serverUrl = url) }

    override suspend fun setSelectedPlayer(playerId: String?) = state.update { it.copy(selectedPlayerId = playerId) }

    override suspend fun setPlayOnPhone(on: Boolean) = state.update { it.copy(playOnPhone = on) }

    override suspend fun setSound(sound: SoundSettings) = state.update { it.copy(sound = sound) }

    override suspend fun setLibrary(id: String?) = state.update { it.copy(libraryId = id) }

    override suspend fun setDefaultPlayer(choice: io.github.jamezuk.lyrionanywhere.data.DefaultPlayer, playerId: String?) =
        state.update { it.copy(defaultPlayer = choice, defaultPlayerId = playerId) }
}

private class FakeHandle : LocalPlayerHandle {
    var observer: LocalPlayerObserver? = null
    var stopped = false
    var closed = false
    val calls = mutableListOf<String>()

    override fun start(observer: LocalPlayerObserver) {
        calls += "start"
        this.observer = observer
    }

    override fun stop() {
        stopped = true
    }

    override fun setHold(hold: Boolean) {
        calls += "hold $hold"
    }

    override fun setDuck(duck: Boolean) {
        calls += "duck $duck"
    }

    override fun setOutputRoute(route: OutputRoute) {
        calls += "route ${route.device}"
    }

    val dsp = mutableListOf<DspSettings>()

    override fun setDsp(dsp: DspSettings) {
        this.dsp += dsp
        calls += "dsp ${dsp.analysisLevelling}"
    }

    override fun setOutputNegotiator(negotiator: OutputNegotiator?) {
        calls += "negotiator ${negotiator != null}"
    }

    override fun renegotiateOutput() {
        calls += "renegotiate"
    }

    override fun close() {
        closed = true
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class LocalPlayerRepositoryTest {
    private val sessions = MutableStateFlow<Session?>(null)
    private val settings = MemSettings()
    private val handles = mutableListOf<FakeHandle>()
    private var failWith: Exception? = null

    private fun TestScope.repo(): RustLocalPlayerRepository {
        val d = UnconfinedTestDispatcher(testScheduler)
        val scope = CoroutineScope(backgroundScope.coroutineContext + d)
        return RustLocalPlayerRepository(sessions, settings, scope, d, factory = {
            failWith?.let { throw it }
            FakeHandle().also { handles += it }
        })
    }

    @Test
    fun startsOnlyWhenSignedInAndEnabled() = runTest {
        val r = repo()
        sessions.value = Session(NoHandle)
        assertTrue("disabled: no player", handles.isEmpty())
        assertEquals(ApiResult.Ok(Unit), r.setEnabled(true))
        assertEquals(1, handles.size)
        assertEquals(listOf("route ", "start"), handles[0].calls)
        assertTrue(r.state.value.enabled)

        r.setEnabled(false)
        assertTrue(handles[0].stopped && handles[0].closed)
        assertFalse(r.state.value.enabled)
        assertNull(r.state.value.status)
    }

    @Test
    fun soundIsBitPerfectByDefaultAndAppliesLive() = runTest {
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        // Bit-perfect: the engine keeps its default (no DSP call at all).
        assertEquals(listOf("route ", "start"), handles[0].calls)
        assertTrue(r.state.value.sound.bitPerfect)

        r.setSound(SoundSettings(levelling = LevellingMode.R128))
        assertEquals(LevellingMode.R128, settings.state.value.sound.levelling)
        assertEquals(LevellingMode.R128, r.state.value.sound.levelling)
        assertEquals("dsp true", handles[0].calls.last())

        r.setSound(SoundSettings())
        assertEquals(DEFAULT_DSP, handles[0].dsp.last())

        // A player started while a stage is on gets it before any audio.
        r.setSound(SoundSettings(preampDb = -3.0))
        r.setEnabled(false)
        r.setEnabled(true)
        assertEquals(listOf("route ", "dsp false", "start"), handles.last().calls)
        assertEquals(-3.0, handles.last().dsp.single().preampDb, 0.0)
    }

    @Test
    fun theBitPerfectSwitchRenegotiatesAndTheNegotiatorReachesThePlayer() = runTest {
        val r = repo()
        val neg = object : OutputNegotiator {
            override fun offers() = OutputOffers("", OfferBlocker.NO_USB_DAC, emptyList())

            override fun apply(offer: MixerOffer?) = false
        }
        r.setOutputNegotiator(neg)
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        assertEquals(listOf("route ", "negotiator true", "start"), handles[0].calls)
        r.setSound(SoundSettings(bitPerfectUsb = false))
        assertEquals("renegotiate", handles[0].calls.last())
        assertTrue("no DSP change", handles[0].dsp.isEmpty())
        r.renegotiateOutput()
        r.setOutputNegotiator(null)
        assertEquals(listOf("renegotiate", "negotiator false"), handles[0].calls.takeLast(2))
    }

    @Test
    fun soundSettingsMapToTheEngine() {
        assertEquals(DEFAULT_DSP, dspFor(SoundSettings()))
        val off = DEFAULT_DSP
        assertFalse(off.enabled || off.analysisLevelling)
        assertEquals(Levelling.OFF, off.levelling)
        assertNull(off.eqProfile)
        assertEquals(CrossfeedSetting.OFF, off.crossfeed)

        val r128 = dspFor(SoundSettings(levelling = LevellingMode.R128))
        assertTrue(r128.enabled && r128.analysisLevelling && r128.limiter)
        assertEquals(Levelling.TRACK, r128.levelling)
        assertEquals(-18.0, r128.targetLufs, 0.0)
        val album = dspFor(SoundSettings(levelling = LevellingMode.REPLAYGAIN_ALBUM))
        assertEquals(Levelling.ALBUM, album.levelling)
        assertFalse(album.analysisLevelling)
        val track = dspFor(SoundSettings(levelling = LevellingMode.REPLAYGAIN_TRACK))
        assertEquals(Levelling.TRACK, track.levelling)
        assertFalse(track.analysisLevelling)

        val eq = EqProfile("Headphones", "Filter 1: ON PK Fc 1000 Hz Gain 3 dB Q 1", 1, 0.0)
        val all = dspFor(
            SoundSettings(
                preampDb = -2.5,
                limiter = false,
                limiterCeilingDb = -0.5,
                eqEnabled = true,
                eq = eq,
                crossfeed = CrossfeedMode.NATURAL,
                dither = DitherMode.TPDF_SHAPED,
            ),
        )
        assertTrue(all.enabled)
        assertEquals(-2.5, all.preampDb, 0.0)
        assertFalse(all.limiter)
        assertEquals(-0.5, all.limiterCeilingDb, 0.0)
        assertEquals(eq.text, all.eqProfile)
        assertEquals(CrossfeedSetting.NATURAL, all.crossfeed)
        assertEquals(DitherSetting.TPDF_SHAPED, all.dither)
        // A loaded but switched-off EQ isn't applied (and alone keeps bit-perfect).
        val eqOff = SoundSettings(eq = eq, eqEnabled = false)
        assertTrue(eqOff.bitPerfect)
        assertNull(dspFor(eqOff).eqProfile)
        assertFalse(dspFor(eqOff).enabled)
        // Dither alone changes nothing; it's still passed on for when depth is reduced.
        val shaped = dspFor(SoundSettings(dither = DitherMode.TPDF_SHAPED))
        assertFalse(shaped.enabled)
        assertEquals(DitherSetting.TPDF_SHAPED, shaped.dither)
    }

    @Test
    fun outOfRangeSoundSettingsAreClamped() {
        val s = SoundSettings(preampDb = 99.0, limiterCeilingDb = -99.0).sanitised()
        assertEquals(12.0, s.preampDb, 0.0)
        assertEquals(-6.0, s.limiterCeilingDb, 0.0)
        val n = SoundSettings(preampDb = Double.NaN, limiterCeilingDb = Double.POSITIVE_INFINITY).sanitised()
        assertEquals(0.0, n.preampDb, 0.0)
        assertEquals(-1.0, n.limiterCeilingDb, 0.0)
        assertEquals(-12.0, dspFor(SoundSettings(preampDb = -40.0)).preampDb, 0.0)
        // An oversized profile is dropped, and the EQ is then off.
        val huge = EqProfile("x", "#".repeat(SoundSettings.MAX_EQ_CHARS + 1), 0, 0.0)
        val h = SoundSettings(eq = huge, eqEnabled = true).sanitised()
        assertNull(h.eq)
        assertFalse(h.eqEnabled)
        // An EQ switched on with no profile does nothing.
        assertTrue(SoundSettings(eqEnabled = true).bitPerfect)
    }

    @Test
    fun eqProfilesAreCheckedByTheEngineAndOversizedOnesRefusedFirst() = runTest {
        val seen = mutableListOf<String>()
        val d = UnconfinedTestDispatcher(testScheduler)
        val r = RustLocalPlayerRepository(
            sessions,
            settings,
            CoroutineScope(backgroundScope.coroutineContext + d),
            d,
            eqCheck = { seen += it; EqCheck.Ok(2, -3.0, false) },
        )
        assertEquals(EqCheck.Ok(2, -3.0, false), r.checkEq("Preamp: -3 dB"))
        assertTrue(r.checkEq("x".repeat(SoundSettings.MAX_EQ_CHARS + 1)) is EqCheck.Invalid)
        assertEquals(listOf("Preamp: -3 dB"), seen)
    }

    @Test
    fun cannotEnableWhileSignedOut() = runTest {
        val r = repo()
        assertEquals(ApiResult.Err(AppError.SignedOut), r.setEnabled(true))
        assertFalse(settings.state.value.playOnPhone)
        assertTrue(handles.isEmpty())
    }

    @Test
    fun updatesFlowIntoStateAndStaleOnesAreIgnored() = runTest {
        val r = repo()
        settings.setPlayOnPhone(true)
        sessions.value = Session(NoHandle)
        val first = handles.single()
        first.observer!!.onStatus(status())
        assertEquals("02:4c:41:00:00:01", r.state.value.playerId)
        assertTrue(r.state.value.running)

        // A new sign-in replaces the player; the old one's late updates are ignored.
        sessions.value = Session(NoHandle)
        assertTrue(first.stopped)
        val second = handles[1]
        first.observer!!.onStatus(status(id = "02:4c:41:00:00:99"))
        assertNull(r.state.value.playerId)
        second.observer!!.onStatus(status(connection = PlayerConnection.Stopped("refused")))
        assertFalse(r.state.value.running)
    }

    @Test
    fun signingOutStopsThePlayer() = runTest {
        val r = repo()
        settings.setPlayOnPhone(true)
        sessions.value = Session(NoHandle)
        sessions.value = null
        assertTrue(handles.single().stopped && handles.single().closed)
        assertTrue("the choice is remembered", r.state.value.enabled)
    }

    @Test
    fun holdDuckAndRouteReachThePlayer() = runTest {
        val r = repo()
        r.setOutputRoute(OutputRoute("USB DAC", 96_000u))
        settings.setPlayOnPhone(true)
        sessions.value = Session(NoHandle)
        r.setHold(true)
        r.setDuck(true)
        assertEquals(listOf("route USB DAC", "start", "hold true", "duck true"), handles.single().calls)
    }

    @Test
    fun aFailedStartIsReported() = runTest {
        failWith = ApiException.Unauthorised()
        val r = repo()
        settings.setPlayOnPhone(true)
        sessions.value = Session(NoHandle)
        assertEquals(AppError.SignedOut, r.state.value.error)
        assertFalse(r.state.value.running)
    }
}
