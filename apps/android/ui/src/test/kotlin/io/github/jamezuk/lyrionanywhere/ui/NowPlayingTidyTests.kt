package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.Quality
import io.github.jamezuk.lyrionanywhere.data.NowPlayingLayout
import io.github.jamezuk.lyrionanywhere.data.NowPlayingPrefs
import io.github.jamezuk.lyrionanywhere.data.VisualFrame
import io.github.jamezuk.lyrionanywhere.data.VisualStyle
import io.github.jamezuk.lyrionanywhere.ui.components.HomeSettingsButton
import io.github.jamezuk.lyrionanywhere.ui.components.MainTab
import io.github.jamezuk.lyrionanywhere.ui.components.MainTabBar
import io.github.jamezuk.lyrionanywhere.ui.player.Glow
import io.github.jamezuk.lyrionanywhere.ui.player.GlowDriver
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.OnsetDetector
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.PulseEnvelope
import io.github.jamezuk.lyrionanywhere.ui.player.QueueActions
import io.github.jamezuk.lyrionanywhere.ui.player.QueueScreen
import io.github.jamezuk.lyrionanywhere.ui.player.QueueScreenActions
import io.github.jamezuk.lyrionanywhere.ui.player.UpNext
import io.github.jamezuk.lyrionanywhere.ui.player.VisualState
import io.github.jamezuk.lyrionanywhere.ui.theme.Ambient
import io.github.jamezuk.lyrionanywhere.ui.theme.ArtPalette
import io.github.jamezuk.lyrionanywhere.ui.theme.ArtScheme
import io.github.jamezuk.lyrionanywhere.ui.theme.Hsl
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import io.github.jamezuk.lyrionanywhere.ui.theme.argb
import io.github.jamezuk.lyrionanywhere.ui.theme.contrast
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import kotlin.math.abs
import kotlin.math.exp
import kotlin.random.Random

/** 33 ms frames, as the visualiser's. */
private const val FRAME = 33L

private fun bands(bass: Float, rest: Float = 0.1f) = FloatArray(VisualFrame.BANDS) { if (it < Glow.BASS_BANDS) bass else rest }

/**
 * Glow's beat detection and envelope (shared design with the web app): onsets, not levels;
 * robust on garbage; the exact timing of the pulse.
 */
class GlowLogicTests {
    /** Feeds [energy] per frame for [ms]; returns the onset times. */
    private fun run(d: OnsetDetector, from: Long, ms: Long, energy: (Long) -> Float): List<Long> {
        val onsets = mutableListOf<Long>()
        var t = from
        while (t < from + ms) {
            if (d.step(energy(t), t)) onsets += t
            t += FRAME
        }
        return onsets
    }

    @Test
    fun aSteadyToneFiresNoRepeatedOnsets() {
        val d = OnsetDetector()
        val onsets = run(d, 0, 10_000) { 0.6f }
        assertTrue("onsets $onsets", onsets.size <= 1)
        // Nothing at all once it has settled.
        assertEquals(emptyList<Long>(), run(d, 10_000, 10_000) { 0.6f })
    }

    @Test
    fun aPeriodicKickFiresOncePerKick() {
        val d = OnsetDetector()
        // 120 BPM: a kick every 500 ms, ringing over three frames, over a quiet bed.
        val period = 495L // a multiple of the frame
        val onsets = run(d, 0, 10_000) { t ->
            when ((t % period) / FRAME) {
                0L -> 0.9f
                1L -> 0.6f
                2L -> 0.3f
                else -> 0.08f
            }
        }
        val kicks = (0 until 10_000L step period).count { it >= 3 * FRAME } // the first needs history
        assertEquals("onsets $onsets", kicks, onsets.size)
        onsets.forEach { assertEquals(0L, it % period) }
    }

    @Test
    fun theRefractoryPeriodIsRespected() {
        val d = OnsetDetector()
        // A kick every 66 ms: far faster than 150 ms allows.
        val onsets = run(d, 0, 3_000) { t -> if ((t / FRAME) % 2 == 0L) 0.9f else 0.05f }
        assertTrue(onsets.size >= 2)
        onsets.zipWithNext().forEach { (a, b) -> assertTrue("$a then $b", b - a >= Glow.REFRACTORY_MS) }
    }

    @Test
    fun silenceFiresNothing() {
        val d = OnsetDetector()
        assertEquals(emptyList<Long>(), run(d, 0, 5_000) { 0f })
        // Nor does a whisper, however it compares with its (tiny) mean.
        assertEquals(emptyList<Long>(), run(d, 5_000, 5_000) { t -> if ((t / FRAME) % 10 == 0L) 0.019f else 0f })
    }

    @Test
    fun theFirstFramesNeverFire() {
        val d = OnsetDetector()
        assertFalse(d.step(0.9f, 0))
        assertFalse(d.step(0.0f, 33))
        assertFalse(d.step(0.95f, 66))
        assertFalse(d.step(0.0f, 99))
        // Three earlier frames: now it can.
        assertTrue(d.step(1f, 132))
    }

    @Test
    fun garbageFramesAreSafe() {
        val driver = GlowDriver()
        val rnd = Random(7)
        val weird = listOf(Float.NaN, Float.POSITIVE_INFINITY, Float.NEGATIVE_INFINITY, -5f, 1e30f, -0f)
        var t = 0L
        repeat(5_000) {
            val b = when (rnd.nextInt(5)) {
                0 -> null
                1 -> FloatArray(0)
                2 -> FloatArray(rnd.nextInt(1, 40)) { weird[rnd.nextInt(weird.size)] }
                3 -> FloatArray(3) { rnd.nextFloat() }
                else -> FloatArray(VisualFrame.BANDS) { rnd.nextFloat() * 4 - 2 }
            }
            val p = driver.step(b, t)
            assertTrue("pulse $p", p.isFinite() && p in 0f..1f)
            t += if (rnd.nextInt(50) == 0) -rnd.nextLong(0, 10_000) else rnd.nextLong(0, 200)
        }
        listOf(Float.NaN, Float.POSITIVE_INFINITY, -1f, 2f).forEach { assertTrue(Glow.bassEnergy(floatArrayOf(it)) in 0f..1f) }
        assertEquals(0f, Glow.bassEnergy(null))
        listOf(Float.NaN, Float.NEGATIVE_INFINITY, 5f).forEach {
            assertTrue(Glow.drive(it, it) in 0f..1f)
            assertTrue(Glow.radius(it) in Glow.RADIUS_MIN..Glow.RADIUS_MAX)
            assertTrue(Glow.opacity(it) in Glow.OPACITY_MIN..Glow.OPACITY_MAX)
            assertTrue(Glow.coverScale(it) in 1f..Glow.COVER_SCALE_MAX)
        }
    }

    @Test
    fun timeGoingBackwardsResetsTheDetector() {
        val d = OnsetDetector()
        run(d, 10_000, 1_000) { 0.1f }
        // Back in time: history gone, so the next frames can't fire straight away.
        assertFalse(d.step(0.9f, 0))
    }

    @Test
    fun theEnvelopeRisesIn30MsThenDecaysWithA350MsTimeConstant() {
        val e = PulseEnvelope()
        assertEquals(0f, e.value(0))
        e.kick(1_000)
        assertEquals(0f, e.value(1_000), 1e-6f)
        assertEquals(0.5f, e.value(1_015), 1e-3f)
        assertEquals(1f, e.value(1_030), 1e-3f)
        assertEquals(exp(-1f), e.value(1_030 + 350), 1e-3f)
        assertEquals(exp(-2f), e.value(1_030 + 700), 1e-3f)
        assertEquals(0f, e.value(10_000))
        // A second kick rises from wherever the pulse was.
        val before = e.value(1_200)
        e.kick(1_200)
        assertEquals(before, e.value(1_200), 1e-6f)
        assertEquals(1f, e.value(1_230), 1e-3f)
    }

    @Test
    fun theHaloFollowsThePulseWithinItsRanges() {
        assertEquals(1.0f, Glow.radius(0f))
        assertEquals(1.25f, Glow.radius(1f))
        assertEquals(0.35f, Glow.opacity(0f))
        assertEquals(0.85f, Glow.opacity(1f))
        assertEquals(1.02f, Glow.coverScale(1f), 1e-6f)
        // Between beats: a gentle breath (0.15 of the level), never a full pulse.
        assertEquals(0.15f, Glow.drive(0f, 1f), 1e-6f)
        assertEquals(1f, Glow.coverScale(0f))
        assertEquals(0f, Glow.angle(0))
        assertEquals(180f, Glow.angle(12_000), 1e-3f)
        assertEquals(0f, Glow.angle(Glow.TURN_MS), 1e-3f)
    }

    @Test
    fun visualStatePulsesOnTheBeatAndSettles() {
        val s = VisualState()
        repeat(10) { s.update(frame(0.08f), FRAME) }
        assertEquals(0f, s.pulse)
        s.update(frame(0.9f), FRAME)
        // The onset's frame starts the 30 ms attack; by the next frame it has peaked.
        s.update(frame(0.9f), FRAME)
        assertTrue("pulse ${s.pulse}", s.pulse > 0.9f)
        // Silence: it decays to rest, and then nothing more is redrawn.
        repeat(200) { s.update(null, FRAME) }
        assertEquals(0f, s.pulse)
        assertFalse(s.update(null, FRAME))
    }

    private fun frame(bass: Float) = VisualFrame(bands(bass), FloatArray(VisualFrame.SCOPE), floatArrayOf(-20f, -20f), floatArrayOf(-10f, -10f), 0f)
}

/** The halo's colours, from the cover. */
class HaloPaletteTests {
    @Test
    fun twoColoursComeOutMostProminentFirst() {
        val red = argb(200, 30, 30)
        val blue = argb(30, 60, 200)
        val pixels = IntArray(100 * 100) { if (it < 7_000) red else blue }
        val found = ArtPalette.dominant(pixels, 3)
        assertEquals(2, found.size)
        assertTrue(close(found[0], red))
        assertTrue(close(found[1], blue))
    }

    @Test
    fun greyOrEmptyCoversGiveNoneSoTheAccentIsUsed() {
        assertEquals(emptyList<Int>(), ArtPalette.dominant(IntArray(0)))
        assertEquals(emptyList<Int>(), ArtPalette.dominant(IntArray(4_096) { argb(it % 256, it % 256, it % 256) }))
        assertEquals(emptyList<Int>(), ArtPalette.dominant(IntArray(4_096) { 0x00FF0000 })) // transparent
        // A colourful speck under 5% of the image doesn't count.
        val speck = IntArray(10_000) { if (it < 300) argb(220, 20, 20) else argb(128, 128, 128) }
        assertEquals(emptyList<Int>(), ArtPalette.dominant(speck))
    }

    @Test
    fun everyColourIsSaturatedAndTheResultIsStable() {
        val rnd = Random(3)
        repeat(30) {
            val pixels = IntArray(64 * 64) { rnd.nextInt() or (0xFF shl 24) }
            val a = ArtPalette.dominant(pixels)
            assertTrue(a.size <= 3)
            a.forEach { assertTrue(Hsl.of(it).s >= ArtPalette.HALO_MIN_SATURATION) }
            assertEquals(a, ArtPalette.dominant(pixels))
        }
    }

    private fun close(a: Int, b: Int): Boolean =
        listOf(16, 8, 0).all { abs(((a shr it) and 0xff) - ((b shr it) and 0xff)) <= 6 }
}

/** The close button's own scrim keeps its chevron at 4.5:1 over anything. */
class CloseButtonContrastTests {
    @Test
    fun theChevronReadsOverAnyArtInEveryTheme() {
        val rnd = Random(11)
        val schemes = buildList {
            add(lightColorScheme().let { it.onSurface.toArgb() to it.surface.toArgb() })
            add(darkColorScheme().let { it.onSurface.toArgb() to it.surface.toArgb() })
            repeat(40) {
                val seed = rnd.nextInt() or (0xFF shl 24)
                listOf(true, false).forEach { dark ->
                    val a = ArtScheme.from(seed, dark)
                    add(a.onBackground to a.background)
                }
            }
        }
        for ((fg, bg) in schemes) {
            val alpha = Ambient.iconScrimAlpha(fg, bg)
            assertTrue(alpha in Ambient.MIN_ICON_SCRIM..1f)
            // Black, white, and random art beneath.
            val unders = listOf(0xFF000000.toInt(), 0xFFFFFFFF.toInt()) + List(50) { rnd.nextInt() or (0xFF shl 24) }
            for (u in unders) {
                val c = contrast(fg, Ambient.over(bg, alpha, u))
                assertTrue("contrast $c for ${fg.toUInt().toString(16)} on ${bg.toUInt().toString(16)}", c >= 4.5 - 0.02)
            }
        }
    }
}

/** Now playing without its top bar, the signal path on demand, Glow's halo, and the Queue tab. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class NowPlayingTidyTests {
    @get:Rule
    val compose = createComposeRule()

    private fun show(content: @Composable () -> Unit) = compose.setContent { LyrionTheme(dynamicColour = false) { content() } }

    private val flac = Quality("FLAC", "FLAC 16/44.1", true, false, 44_100u, 16u, null)

    private fun withQuality(s: PlayerUiState): PlayerUiState {
        val st = s.status!!
        return s.copy(status = st.copy(current = st.current!!.copy(quality = flac)))
    }

    @Test
    fun theChevronClosesNowPlaying() {
        var closed = 0
        show { NowPlayingScreen(auditPlayerState(), clock = { 0L }, actions = nowPlayingActions(onClose = { closed++ })) }
        compose.onNodeWithContentDescription("Close Now playing").performClick()
        assertEquals(1, closed)
        compose.onNodeWithTag("now-playing-close-scrim", useUnmergedTree = true).assertExists()
        // The old top bar's title is gone.
        compose.onAllNodesWithTag("now-playing-close").assertCountEquals(1)
    }

    @Test
    fun theChevronIsThereBeforeAPlayerIsChosen() {
        var closed = 0
        show {
            NowPlayingScreen(PlayerUiState(loading = false), clock = { 0L }, actions = nowPlayingActions(onClose = { closed++ }))
        }
        compose.onNodeWithTag("now-playing-choose-player").assertExists()
        compose.onNodeWithContentDescription("Close Now playing").performClick()
        assertEquals(1, closed)
    }

    @Test
    fun theSignalPathIsNotShownUntilAskedFor() {
        val base = withQuality(phonePlayingState())
        val state = base.copy(local = base.local.copy(signalPath = signalPath()))
        show { NowPlayingScreen(state, clock = { 0L }, actions = nowPlayingActions()) }
        compose.onNodeWithText("Signal path").assertDoesNotExist()
        compose.onNodeWithTag("signal-path-sheet").assertDoesNotExist()
        // The quality badge opens it.
        compose.onNodeWithTag("quality-badge").performClick()
        compose.onNodeWithTag("signal-path-sheet").assertExists()
        compose.onNodeWithText("Signal path").assertExists()
    }

    @Test
    fun theInfoLayoutOpensTheSignalPath() {
        show {
            NowPlayingScreen(phonePlayingState(NowPlayingPrefs(layout = NowPlayingLayout.INFO)), clock = { 0L }, actions = nowPlayingActions())
        }
        compose.onNodeWithTag("signal-path-sheet").assertDoesNotExist()
        compose.onNodeWithTag("signal-path-button").performClick()
        compose.onNodeWithTag("signal-path-sheet").assertExists()
    }

    @Test
    fun glowPutsAHaloBehindTheCover() {
        val source = FakeVisualSource()
        show {
            NowPlayingScreen(
                phonePlayingState(NowPlayingPrefs(visualiser = true, visualStyle = VisualStyle.GLOW)),
                clock = { 0L },
                actions = nowPlayingActions(),
                visual = source,
            )
        }
        compose.onNodeWithTag("glow-halo").assertExists()
        compose.onNodeWithTag("artwork").assertExists()
        compose.onNodeWithTag("visualiser").assertDoesNotExist()
        assertEquals(listOf(true), source.active)
    }

    @Test
    fun noHaloForOtherStylesOrWithTheVisualiserOff() {
        show {
            NowPlayingScreen(phonePlayingState(NowPlayingPrefs(visualiser = false, visualStyle = VisualStyle.GLOW)), clock = { 0L }, actions = nowPlayingActions())
        }
        compose.onNodeWithTag("glow-halo").assertDoesNotExist()
    }

    private fun queueState() = auditPlayerState().copy(upNext = UpNext(0, listOf(2L)))

    @Test
    fun theQueueTabShowsTheQueueWithItsSections() {
        val jumped = mutableListOf<Int>()
        show { QueueScreen(queueState(), QueueScreenActions(QueueActions(onJump = { jumped += it }))) }
        compose.onNodeWithTag("queue-screen").assertExists()
        compose.onNodeWithTag("section-now").assertExists()
        compose.onNodeWithTag("section-up-next").assertExists()
        compose.onNodeWithTag("section-then").assertExists()
        compose.onNodeWithText("Track 3").performClick()
        assertEquals(listOf(2), jumped)
    }

    @Test
    fun theQueueTabAsksForAPlayerFirst() {
        var chose = 0
        show { QueueScreen(PlayerUiState(loading = false), QueueScreenActions(QueueActions(onJump = {}), onChoosePlayer = { chose++ })) }
        compose.onNodeWithTag("queue-no-player").assertExists()
        compose.onNodeWithText("Choose a player").performClick()
        assertEquals(1, chose)
    }

    @Test
    fun theFourTabsInOrder() {
        assertEquals(listOf(MainTab.HOME, MainTab.LIBRARY, MainTab.SEARCH, MainTab.QUEUE), MainTab.entries)
        val chosen = mutableListOf<MainTab>()
        show { MainTabBar(MainTab.HOME, onSelect = { chosen += it }) }
        listOf("Home", "Library", "Search", "Queue").forEach { compose.onNodeWithText(it).assertExists() }
        compose.onNodeWithText("Settings").assertDoesNotExist()
        compose.onNodeWithTag("tab-queue").performClick()
        assertEquals(listOf(MainTab.QUEUE), chosen)
    }

    @Test
    fun settingsIsTheGearOnHome() {
        var opened = 0
        show { HomeSettingsButton(onClick = { opened++ }) }
        compose.onNodeWithContentDescription("Settings").performClick()
        assertEquals(1, opened)
    }
}

/** The changed screens, audited as TalkBack sees them ([assertAccessible]). */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class NowPlayingTidyAccessibilityAuditTests {
    @get:Rule
    val compose = createComposeRule()

    private fun audit(content: @Composable () -> Unit) {
        compose.setContent { LyrionTheme(dynamicColour = false) { content() } }
        compose.assertAccessible()
    }

    @Test
    fun nowPlayingWithTheCloseChevron() = audit {
        NowPlayingScreen(auditPlayerState(), clock = { 0L }, actions = nowPlayingActions(onClose = {}))
    }

    @Test
    fun nowPlayingWithGlow() = audit {
        NowPlayingScreen(
            phonePlayingState(NowPlayingPrefs(visualiser = true, visualStyle = VisualStyle.GLOW)),
            clock = { 0L },
            actions = nowPlayingActions(onClose = {}),
            visual = FakeVisualSource(),
        )
    }

    @Test
    fun nowPlayingInfoLayout() = audit {
        NowPlayingScreen(phonePlayingState(NowPlayingPrefs(layout = NowPlayingLayout.INFO)), clock = { 0L }, actions = nowPlayingActions(onClose = {}))
    }

    @Test
    fun nowPlayingWithoutAPlayer() = audit {
        NowPlayingScreen(PlayerUiState(loading = false), clock = { 0L }, actions = nowPlayingActions(onClose = {}))
    }

    @Test
    fun theQueueTab() = audit {
        QueueScreen(auditPlayerState().copy(upNext = UpNext(0, listOf(2L))), QueueScreenActions(QueueActions(onJump = {})))
    }

    @Test
    fun theFourTabs() = audit { MainTabBar(MainTab.QUEUE, onSelect = {}) }

    @OptIn(ExperimentalMaterial3Api::class)
    @Test
    fun homeWithItsSettingsGear() = audit {
        TopAppBar(title = { Text("Home") }, actions = { HomeSettingsButton(onClick = {}) })
    }
}

/** Now playing's close chevron on a wide window, beside the panes. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w1280dp-h800dp")
class WideNowPlayingCloseTests {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun wideAndTwoPaneKeepTheChevron() {
        var closed = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                NowPlayingScreen(auditPlayerState(), clock = { 0L }, actions = nowPlayingActions(onClose = { closed++ }), wide = true)
            }
        }
        compose.onNodeWithTag("now-playing-wide").assertExists()
        compose.onNodeWithContentDescription("Close Now playing").performClick()
        assertEquals(1, closed)
        compose.assertAccessible()
    }
}
