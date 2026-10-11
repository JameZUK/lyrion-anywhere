package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isToggleable
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.AnalysisOutcome
import io.github.jamezuk.lyrionanywhere.core.SignalStage
import io.github.jamezuk.lyrionanywhere.core.SignalVerdict
import io.github.jamezuk.lyrionanywhere.core.TrackAnalysisInfo
import io.github.jamezuk.lyrionanywhere.data.AnalysisRepository
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.Waveform
import io.github.jamezuk.lyrionanywhere.data.WaveformCache
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.SeekBar
import io.github.jamezuk.lyrionanywhere.ui.player.SignalPathPanel
import io.github.jamezuk.lyrionanywhere.ui.player.WaveformMath
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsUiState
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsViewModel
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** The drawing maths (plain JVM). */
class WaveformMathTest {
    @Test
    fun barsFitTheWidth() {
        assertEquals(100, WaveformMath.barCount(300f, 3f))
        assertEquals(WaveformMath.MIN_BARS, WaveformMath.barCount(1f, 3f))
        assertEquals(WaveformMath.MAX_BARS, WaveformMath.barCount(1e6f, 3f))
        for (w in listOf(Float.NaN, -1f, 0f, Float.POSITIVE_INFINITY)) {
            assertEquals(WaveformMath.MIN_BARS, WaveformMath.barCount(w, 3f))
        }
        assertEquals(WaveformMath.MIN_BARS, WaveformMath.barCount(300f, 0f))
    }

    @Test
    fun eachBarKeepsItsLoudestBucket() {
        val values = IntArray(1000)
        values[537] = 255
        val b = WaveformMath.bars(values.size, 10) { values[it] }
        assertEquals(10, b.size)
        assertEquals(1f, b[5], 0f)
        assertEquals(1, b.count { it > 0f })
        // More bars than buckets.
        assertArrayEquals(floatArrayOf(0f, 0f, 1f, 1f), WaveformMath.bars(2, 4) { if (it == 1) 255 else 0 }, 0f)
        assertEquals(0, WaveformMath.bars(0, 10) { 0 }.size)
        assertEquals(0, WaveformMath.bars(10, 0) { 0 }.size)
        assertEquals(0, WaveformMath.bars(10, -3) { 0 }.size)
    }

    @Test
    fun levelsAreMonotonicAndBounded() {
        assertEquals(0f, WaveformMath.level(0), 0f)
        assertEquals(1f, WaveformMath.level(255), 0f)
        assertEquals(1f, WaveformMath.level(1000), 0f)
        assertEquals(0f, WaveformMath.level(-5), 0f)
        assertTrue("quiet parts lifted", WaveformMath.level(64) > 64 / 255f)
        var last = -1f
        for (v in 0..255) {
            val l = WaveformMath.level(v)
            assertTrue(l >= last)
            last = l
        }
    }

    @Test
    fun playedFraction() {
        assertEquals(0.25f, WaveformMath.fraction(30.0, 120.0), 1e-6f)
        assertEquals(1f, WaveformMath.fraction(500.0, 120.0), 0f)
        assertEquals(0f, WaveformMath.fraction(-1.0, 120.0), 0f)
        assertEquals(0f, WaveformMath.fraction(10.0, 0.0), 0f)
        assertEquals(0f, WaveformMath.fraction(Double.NaN, 100.0), 0f)
    }

    @Test
    fun waveformsReadUnsignedBucketsWhateverTheBytes() {
        val w = Waveform(1, ByteArray(256) { it.toByte() }, 10.0)
        assertEquals((0..255).toList(), (0 until w.size).map { w[it] })
        val b = WaveformMath.bars(w, 16)
        assertTrue(b.all { it in 0f..1f })
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class WaveformScreenTests {
    @get:Rule
    val compose = createComposeRule()

    private fun seekBar(waveform: Waveform?) {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SeekBar(elapsed = 30.0, duration = 120.0, enabled = true, onSeek = {}, waveform = waveform)
            }
        }
    }

    @Test
    fun theWaveformIsTheSeekBarsTrack() {
        seekBar(Waveform(1, ByteArray(1000) { (it % 256).toByte() }, 120.0))
        compose.onNodeWithTag("waveform", useUnmergedTree = true).assertExists()
        // Still the accessible slider.
        compose.onNodeWithContentDescription("Position in track").assertIsDisplayed()
    }

    @Test
    fun withoutAnAnalysisItIsThePlainSeekBar() {
        seekBar(null)
        compose.onNodeWithTag("waveform", useUnmergedTree = true).assertDoesNotExist()
        compose.onNodeWithContentDescription("Position in track").assertIsDisplayed()
    }

    @Test
    fun levellingIsASwitchInSettings() {
        var changed: Boolean? = null
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SettingsScreen(
                    state = SettingsUiState(username = "alice", levelling = false),
                    appVersion = "1",
                    libraryVersion = "1",
                    onSignOut = {},
                    onLicences = {},
                    onLevelling = { changed = it },
                )
            }
        }
        compose.onNodeWithText("Off: tracks play bit-perfect, as mastered.").assertIsDisplayed()
        compose.onNode(hasText("Loudness levelling") and isToggleable()).assertIsOff().performClick()
        assertEquals(true, changed)
    }

    @Test
    fun noLevellingSettingWithoutALocalPlayer() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SettingsScreen(SettingsUiState(username = "alice"), "1", "1", {}, {})
            }
        }
        compose.onNodeWithText("Loudness levelling").assertDoesNotExist()
    }

    @Test
    fun theSignalPathNamesTheLevellingSource() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SignalPathPanel(
                    signalPath(
                        verdict = SignalVerdict.LOSSLESS_PROCESSED,
                        engineBitPerfect = false,
                        stages = listOf(
                            SignalStage("Decode (lossless)", true, false),
                            SignalStage("Levelling −8.0 dB (R128 analysis)", false, false),
                            SignalStage("True-peak limiter −1.0 dBTP", false, false),
                        ),
                    ),
                )
            }
        }
        compose.onNodeWithContentDescription("Levelling −8.0 dB (R128 analysis), changes samples").assertIsDisplayed()
    }
}

private class FakeAnalysis(private val peaks: ByteArray) : AnalysisRepository {
    val asked = mutableListOf<Long>()

    override suspend fun track(trackId: Long): ApiResult<AnalysisOutcome> {
        asked += trackId
        return ApiResult.Ok(
            AnalysisOutcome.Ready(TrackAnalysisInfo(61.0, 44_100u, -14.0, 5.0, -0.4, peaks, peaks)),
        )
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class WaveformViewModelTest : MainDispatcherTest() {
    @Test
    fun theCurrentTracksWaveformIsLoadedAndFollowsTheTrack() = runTest(dispatcher) {
        val players = FakePlayers()
        val analysis = FakeAnalysis(byteArrayOf(1, 2, 3))
        val vm = PlayerViewModel(
            players,
            FakeEvents(),
            FakeSettings(),
            waveforms = WaveformCache(analysis),
            clock = { 0L },
        )
        vm.start()
        val w = vm.state.value.waveform!!
        assertEquals(1L, w.trackId)
        assertEquals(listOf(1L), analysis.asked)
        // The track changes: the old waveform goes at once, the new one is fetched.
        players.statusResult = ApiResult.Ok(sampleStatus().copy(current = sampleTrack(2, 1)))
        vm.refresh()
        assertEquals(2L, vm.state.value.waveform?.trackId)
        // A stream (no library id): no waveform.
        players.statusResult = ApiResult.Ok(sampleStatus().copy(current = sampleTrack(2, 1).copy(id = null)))
        vm.refresh()
        assertNull(vm.state.value.waveform)
        assertEquals(listOf(1L, 2L), analysis.asked)
        vm.stop()
    }

    @Test
    fun settingsToggleLevellingThroughThePlayer() = runTest(dispatcher) {
        val local = FakeLocalPlayer()
        val vm = SettingsViewModel(FakeAccount(), local)
        assertEquals(false, vm.state.value.levelling)
        vm.setLevelling(true)
        assertEquals(listOf(true), local.levelling)
        assertEquals(true, vm.state.value.levelling)
        assertNull(SettingsViewModel(FakeAccount()).state.value.levelling)
    }
}
