package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.NoHandle
import io.github.jamezuk.lyrionanywhere.core.Session
import kotlin.random.Random
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

private const val PROFILE_A = "Preamp: -6.0 dB\nFilter 1: ON PK Fc 1000 Hz Gain -3.0 dB Q 1.41\n"
private const val PROFILE_B = "Preamp: -2.0 dB\nFilter 1: ON LSC Fc 105 Hz Gain 5.5 dB Q 0.70\n"

private fun profile(name: String, text: String = PROFILE_A) = EqProfile(name, text, 1, -6.0)

/** Per-output EQ (Wave 2 §5.3): which profile each output uses, and how bindings are kept. */
class OutputEqTest {
    private val main = profile("Main")
    private val wired = profile("For wired", PROFILE_B)
    private val headphones = profile("Example Headphones")

    private val sound = SoundSettings(
        eqEnabled = true,
        eq = main,
        outputEq = listOf(
            OutputEq(OutputClass.WIRED, "", wired),
            OutputEq(OutputClass.BLUETOOTH, "Example Headphones", headphones),
            OutputEq(OutputClass.BLUETOOTH, "", null),
        ),
    ).sanitised()

    @Test
    fun eachOutputUsesItsOwnProfileElseTheMainOne() {
        assertEquals(wired, sound.forOutput(AudioOutput(OutputClass.WIRED)).eq)
        // A named device's own binding comes before "any of its kind".
        assertEquals(headphones, sound.forOutput(AudioOutput(OutputClass.BLUETOOTH, "Example Headphones")).eq)
        // Any other Bluetooth device: bound to no EQ.
        val other = sound.forOutput(AudioOutput(OutputClass.BLUETOOTH, "Example Speaker"))
        assertNull(other.eq)
        assertFalse(other.eqEnabled)
        assertFalse(SoundStage.EQ in other.activeStages)
        // Unbound outputs (and an unknown one) use the main profile.
        assertEquals(main, sound.forOutput(AudioOutput(OutputClass.SPEAKER)).eq)
        assertEquals(main, sound.forOutput(AudioOutput(OutputClass.USB, "Example DAC")).eq)
        assertEquals(main, sound.forOutput(null).eq)
        // The master switch still rules.
        assertNull(dspOn(sound.copy(eqEnabled = false), AudioOutput(OutputClass.WIRED), bypass = false).eqProfile)
    }

    @Test
    fun theEngineGetsTheBoundProfileUnlessBypassed() {
        assertEquals(PROFILE_B, dspOn(sound, AudioOutput(OutputClass.WIRED), bypass = false).eqProfile)
        assertEquals(PROFILE_A, dspOn(sound, AudioOutput(OutputClass.SPEAKER), bypass = false).eqProfile)
        // A/B: bypassed, the EQ is out and nothing else is touched.
        val bypassed = dspOn(sound.copy(preampDb = 3.0), AudioOutput(OutputClass.WIRED), bypass = true)
        assertNull(bypassed.eqProfile)
        assertEquals(3.0, bypassed.preampDb, 0.0)
        // With only the EQ on, bypassing is bit-perfect.
        assertEquals(DEFAULT_DSP, dspOn(sound, AudioOutput(OutputClass.WIRED), bypass = true))
    }

    @Test
    fun theSwitchStaysOnWithOnlyOutputProfiles() {
        val onlyBound = SoundSettings(eqEnabled = true, outputEq = listOf(OutputEq(OutputClass.WIRED, "", wired))).sanitised()
        assertTrue(onlyBound.eqEnabled)
        assertEquals(PROFILE_B, dspOn(onlyBound, AudioOutput(OutputClass.WIRED), false).eqProfile)
        assertNull(dspOn(onlyBound, AudioOutput(OutputClass.SPEAKER), false).eqProfile)
        // With no profile anywhere it can't be on.
        assertFalse(SoundSettings(eqEnabled = true, outputEq = listOf(OutputEq(OutputClass.WIRED, "", null))).sanitised().eqEnabled)
    }

    @Test
    fun bindingsAreCheckedBeforeTheyAreKept() {
        val messy = OutputEq.sanitise(
            listOf(
                // Names only for USB and Bluetooth; control characters out; at most 64.
                OutputEq(OutputClass.SPEAKER, "Ignored name", main),
                OutputEq(OutputClass.USB, "  Example\u0000 DAC\n ", main),
                OutputEq(OutputClass.BLUETOOTH, "x".repeat(500), main),
                // Over the size limit, or holding the stored form's separators: dropped.
                OutputEq(OutputClass.WIRED, "", profile("Big", "a".repeat(SoundSettings.MAX_EQ_CHARS + 1))),
                OutputEq(OutputClass.WIRED, "", profile("Sneaky", "Filter\u001E1\u001F")),
            ),
        )
        assertEquals(listOf(OutputClass.SPEAKER, OutputClass.USB, OutputClass.BLUETOOTH), messy.map { it.kind })
        assertEquals("", messy[0].deviceName)
        assertEquals("Example DAC", messy[1].deviceName)
        assertEquals(64, messy[2].deviceName.length)
        // One binding per output: the last wins, keeping at most eight.
        val again = OutputEq.sanitise(listOf(OutputEq(OutputClass.WIRED, "", main), OutputEq(OutputClass.WIRED, "", null)))
        assertEquals(listOf(OutputEq(OutputClass.WIRED, "", null)), again)
        val many = OutputEq.sanitise((1..20).map { OutputEq(OutputClass.USB, "DAC $it", main) })
        assertEquals(OutputEq.MAX, many.size)
        assertEquals("DAC 20", many.last().deviceName)
        // Profile names are cleaned too, and never empty.
        assertEquals("EQ", OutputEq.sanitise(listOf(OutputEq(OutputClass.WIRED, "", profile("\u0007")))).single().profile!!.name)
    }

    @Test
    fun bindingsSurviveStorage() {
        val stored = OutputEq.encode(sound.outputEq)
        assertEquals(sound.outputEq, OutputEq.decode(stored))
        assertTrue(OutputEq.decode(OutputEq.encode(emptyList())).isEmpty())
        // Profiles keep their text exactly, newlines and all.
        assertEquals(PROFILE_B, OutputEq.decode(stored).first().profile!!.text)
    }

    /** Stored strings are read defensively: nonsense is skipped, never thrown. */
    @Test
    fun storedNonsenseIsSkipped() {
        val r = '\u001E'
        val f = '\u001F'
        val good = listOf("WIRED", "", "1", "P", "1", "-1.0", PROFILE_A).joinToString(f.toString())
        for (bad in listOf(
            "",
            "WIRED",
            listOf("NOPE", "", "0", "", "0", "0.0", "").joinToString(f.toString()),
            listOf("WIRED", "", "2", "", "0", "0.0", "").joinToString(f.toString()),
            listOf("WIRED", "", "1", "P", "x", "-1.0", PROFILE_A).joinToString(f.toString()),
            listOf("WIRED", "", "1", "P", "1", "NaN", PROFILE_A).joinToString(f.toString()),
            listOf("WIRED", "", "1", "P", "99999", "-1.0", PROFILE_A).joinToString(f.toString()),
            listOf("WIRED", "", "1", "P", "1", "-1.0", "").joinToString(f.toString()),
            "$good${f}extra",
        )) {
            assertTrue(bad.take(30), OutputEq.decode(bad).isEmpty())
        }
        // A good record among bad ones is kept.
        assertEquals(1, OutputEq.decode("junk$r$good${r}more junk").size)
        // Too long to be ours: ignored without reading.
        assertTrue(OutputEq.decode("x".repeat(OutputEq.MAX_STORED + 1)).isEmpty())
    }

    @Test
    fun randomStoredStringsNeverThrow() {
        val rnd = Random(54)
        val alphabet = "WIREDUSBLUETOOTHSPEAKER0123456789.-\u001E\u001F\n PK Fc Hz"
        repeat(3_000) {
            val s = String(CharArray(rnd.nextInt(0, 300)) { alphabet[rnd.nextInt(alphabet.length)] })
            val out = OutputEq.decode(s)
            assertTrue(out.size <= OutputEq.MAX)
        }
    }

    @Test
    fun outputNamesAreKeptOnlyWhereTheyMatter() {
        assertEquals(AudioOutput(OutputClass.WIRED, ""), AudioOutput.of(OutputClass.WIRED, "Wired headset"))
        assertEquals(AudioOutput(OutputClass.USB, "Example DAC"), AudioOutput.of(OutputClass.USB, " Example DAC\u0000"))
        assertEquals("", AudioOutput.cleanName(null))
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class OutputEqRepositoryTest {
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
        val dsp = mutableListOf<io.github.jamezuk.lyrionanywhere.core.DspSettings>()

        override fun start(observer: io.github.jamezuk.lyrionanywhere.core.LocalPlayerObserver) {}

        override fun stop() {}

        override fun setHold(hold: Boolean) {}

        override fun setDuck(duck: Boolean) {}

        override fun setOutputRoute(route: io.github.jamezuk.lyrionanywhere.core.OutputRoute) {}

        override fun setDsp(dsp: io.github.jamezuk.lyrionanywhere.core.DspSettings) {
            this.dsp += dsp
        }

        override fun setOutputNegotiator(negotiator: io.github.jamezuk.lyrionanywhere.core.OutputNegotiator?) {}

        override fun renegotiateOutput() {}

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

    /** The profile follows the output as it changes, once per change, and the bypass is A/B only. */
    @Test
    fun theProfileSwitchesWithTheOutput() = runTest {
        val main = profile("Main")
        val wired = profile("For wired", PROFILE_B)
        settings.setSound(SoundSettings(eqEnabled = true, eq = main, outputEq = listOf(OutputEq(OutputClass.WIRED, "", wired))))
        val r = repo()
        val h = handles.single()
        assertEquals(PROFILE_A, h.dsp.last().eqProfile)
        assertEquals("Main", r.state.value.eqInUse?.name)
        r.setOutput(AudioOutput(OutputClass.WIRED))
        assertEquals(PROFILE_B, h.dsp.last().eqProfile)
        assertEquals("For wired", r.state.value.eqInUse?.name)
        // The same output again changes nothing.
        val sent = h.dsp.size
        r.setOutput(AudioOutput(OutputClass.WIRED))
        assertEquals(sent, h.dsp.size)
        // A/B: out while held, back on release; never remembered in the settings.
        r.setEqBypass(true)
        assertNull(h.dsp.last().eqProfile)
        assertNull(r.state.value.eqInUse)
        assertTrue(r.state.value.eqBypass)
        r.setEqBypass(false)
        assertEquals(PROFILE_B, h.dsp.last().eqProfile)
        assertFalse(settings.state.value.sound.outputEq.isEmpty())
        // Back on the speaker: the main profile.
        r.setOutput(AudioOutput(OutputClass.SPEAKER))
        assertEquals(PROFILE_A, h.dsp.last().eqProfile)
        // A new engine starts with the output's profile.
        r.setOutput(AudioOutput(OutputClass.WIRED))
        r.setEnabled(false)
        r.setEnabled(true)
        assertEquals(PROFILE_B, handles.last().dsp.single().eqProfile)
    }
}
