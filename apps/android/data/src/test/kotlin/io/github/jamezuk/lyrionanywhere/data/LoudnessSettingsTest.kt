package io.github.jamezuk.lyrionanywhere.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Loudness-compensated volume: off by default, a stage of its own, and handed to the engine. */
class LoudnessSettingsTest {
    @Test
    fun offByDefaultAndBitPerfect() {
        val s = SoundSettings()
        assertFalse(s.loudness)
        assertTrue(s.bitPerfect)
        assertEquals(DEFAULT_DSP, dspFor(s))
        assertFalse(DEFAULT_DSP.loudness)
    }

    @Test
    fun onItIsAStageAndSwitchesTheEngineChainOn() {
        val s = SoundSettings(loudness = true)
        assertEquals(listOf(SoundStage.LOUDNESS), s.activeStages)
        assertFalse(s.bitPerfect)
        val dsp = dspFor(s)
        assertTrue(dsp.enabled)
        assertTrue(dsp.loudness)
        // Nothing else is switched on with it.
        assertEquals(0.0, dsp.preampDb, 0.0)
        assertNull(dsp.eqProfile)
        assertEquals(DEFAULT_DSP.crossfeed, dsp.crossfeed)
    }

    @Test
    fun itSitsAfterTheEqAndSurvivesSanitisingAndPerOutputEq() {
        val eq = EqProfile("Main", "Preamp: -2 dB\nFilter 1: ON PK Fc 1000 Hz Gain 2 dB Q 1", 1, -2.0)
        val s = SoundSettings(eqEnabled = true, eq = eq, loudness = true, preampDb = Double.NaN)
        // (A nonsense preamp is reset by sanitising; loudness is kept.)
        assertEquals(listOf(SoundStage.EQ, SoundStage.LOUDNESS), s.sanitised().activeStages)
        assertTrue(s.sanitised().loudness)
        val bound = s.copy(outputEq = listOf(OutputEq(OutputClass.WIRED, "", null)))
        val onWired = bound.forOutput(AudioOutput(OutputClass.WIRED))
        assertFalse("no EQ on that output", onWired.eqEnabled)
        assertTrue("but loudness still applies", onWired.loudness)
        assertTrue(dspFor(onWired).loudness)
    }

    @Test
    fun turningItOffAgainRestoresBitPerfect() {
        val s = SoundSettings(loudness = true).copy(loudness = false)
        assertTrue(s.bitPerfect)
        assertEquals(DEFAULT_DSP, dspFor(s))
    }
}
