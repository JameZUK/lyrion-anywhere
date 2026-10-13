package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.test.assertTextEquals
import androidx.compose.material3.ColorScheme
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeLeft
import androidx.compose.ui.test.swipeRight
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.Quality
import io.github.jamezuk.lyrionanywhere.core.SleepEnd
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.ui.player.MiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.SleepChoice
import io.github.jamezuk.lyrionanywhere.ui.player.SleepChoices
import io.github.jamezuk.lyrionanywhere.ui.player.SleepFadeSetting
import io.github.jamezuk.lyrionanywhere.ui.player.SleepScope
import io.github.jamezuk.lyrionanywhere.ui.player.UpNextRow
import io.github.jamezuk.lyrionanywhere.ui.theme.Ambient
import io.github.jamezuk.lyrionanywhere.ui.theme.ArtScheme
import io.github.jamezuk.lyrionanywhere.ui.theme.BrandDark
import io.github.jamezuk.lyrionanywhere.ui.theme.BrandLight
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import io.github.jamezuk.lyrionanywhere.ui.theme.argb
import io.github.jamezuk.lyrionanywhere.ui.theme.contrast
import io.github.jamezuk.lyrionanywhere.ui.theme.tintedBy
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import kotlin.random.Random

private const val ME = "aa:00:00:00:00:01"
private const val OTHER = "aa:00:00:00:00:02"

private val HI_RES = Quality("FLAC", "FLAC 24/96", true, true, 96_000u, 24u, null)

private fun wave1State(
    sleepSecs: Double? = null,
    next: Boolean = true,
    quality: Quality? = HI_RES,
) = PlayerUiState(
    players = listOf(samplePlayer(ME), samplePlayer(OTHER)),
    selectedId = ME,
    status = sampleStatus().copy(
        current = sampleTrack(1, 0).copy(quality = quality),
        next = if (next) sampleTrack(2, 1).copy(title = "Next song", artist = "Other band") else null,
        sleepSecs = sleepSecs,
    ),
    loading = false,
    local = LocalPlayerState(),
)

/** The mini player's gestures (Wave 1 §4) and their accessibility actions. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class MiniPlayerGestureTest {
    @get:Rule
    val compose = createComposeRule()

    private val calls = mutableListOf<String>()

    private fun mini(state: PlayerUiState = wave1State()) = compose.setContent {
        LyrionTheme(dynamicColour = false) {
            MiniPlayer(
                state,
                onOpen = { calls += "open" },
                onTogglePlay = { calls += "toggle" },
                onNext = { calls += "next" },
                onOutputs = { calls += "play on" },
                onPrevious = { calls += "previous" },
            )
        }
    }

    @Test
    fun swipeLeftForNextAndRightForPrevious() {
        mini()
        compose.onNodeWithTag("mini-player").performTouchInput { swipeLeft() }
        compose.onNodeWithTag("mini-player").performTouchInput { swipeRight() }
        assertEquals(listOf("next", "previous"), calls)
    }

    @Test
    fun longPressOpensPlayOnAndTapOpensNowPlaying() {
        mini()
        compose.onNodeWithTag("mini-player").performTouchInput { longClick() }
        assertEquals(listOf("play on"), calls)
        compose.onNodeWithTag("mini-player").performClick()
        assertEquals(listOf("play on", "open"), calls)
    }

    @Test
    fun everyGestureIsAnAccessibilityAction() {
        mini()
        val config = compose.onNodeWithTag("mini-player").fetchSemanticsNode().config
        assertEquals("Open now playing", config[SemanticsActions.OnClick].label)
        assertEquals("Play on…", config[SemanticsActions.OnLongClick].label)
        val actions = config[SemanticsActions.CustomActions]
        assertEquals(listOf("Next track", "Previous track", "Play on…"), actions.map { it.label })
        actions.forEach { it.action() }
        config[SemanticsActions.OnClick].action?.invoke()
        config[SemanticsActions.OnLongClick].action?.invoke()
        assertEquals(listOf("next", "previous", "play on", "open", "play on"), calls)
    }

    @Test
    fun aSmallMoonShowsWhileASleepTimerRuns() {
        mini(wave1State(sleepSecs = 300.0))
        compose.onNodeWithContentDescription("Sleep timer: stops in 5 min").assertIsDisplayed()
    }

    @Test
    fun noMoonWithoutATimer() {
        mini(wave1State())
        compose.onNodeWithTag("mini-moon").assertDoesNotExist()
    }

    @Test
    fun theMiniPlayerPassesTheAccessibilityAudit() {
        mini(wave1State(sleepSecs = 300.0))
        compose.assertAccessible()
    }
}

/** Now playing's Wave 1 parts: the quality badge, the sleep timer and Up next. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class NowPlayingWave1Test {
    @get:Rule
    val compose = createComposeRule()

    private val calls = mutableListOf<String>()

    private fun nowPlaying(state: PlayerUiState = wave1State()) = compose.setContent {
        LyrionTheme(dynamicColour = false) {
            NowPlayingScreen(
                state,
                clock = { 0L },
                actions = nowPlayingActions().copy(
                    onSleep = { c, s -> calls += "sleep $c $s" },
                    onCancelSleep = { calls += "cancel" },
                    onRemoveUpNext = { calls += "remove next" },
                ),
            )
        }
    }

    @Test
    fun theBadgeShowsFormatAndHiResAndOpensTheSignalPath() {
        nowPlaying()
        compose.onNodeWithText("FLAC 24/96 · Hi-Res").assertIsDisplayed()
        compose.onNodeWithContentDescription("FLAC 24/96 · Hi-Res. Show the signal path").performClick()
        compose.waitForIdle()
        // Another player: only the source is known.
        compose.onNodeWithText("FLAC 24/96 · Hi-Res · lossless").assertIsDisplayed()
    }

    @Test
    fun aLossyTrackShowsCodecAndBitRateOnly() {
        nowPlaying(wave1State(quality = Quality("MP3", "MP3 320", false, false, 44_100u, null, 320u)))
        compose.onNodeWithText("MP3 320").assertIsDisplayed()
        compose.onNodeWithText("MP3 320 · Hi-Res").assertDoesNotExist()
    }

    @Test
    fun theMoonOpensTheSleepTimer() {
        nowPlaying()
        compose.onNodeWithContentDescription("Sleep timer").performClick()
        compose.waitForIdle()
        compose.onNodeWithTag("sleep-30").performClick()
        assertEquals(listOf("sleep ${SleepChoice.Minutes(30)} ${SleepScope.PLAYER}"), calls)
    }

    @Test
    fun aRunningTimerShowsTheTimeLeftAndItsSheetTurnsItOff() {
        nowPlaying(wave1State(sleepSecs = 1620.0))
        // The Sleep action's label is the time left.
        compose.onNodeWithContentDescription("Sleep timer: stops in 27 min").assertIsDisplayed()
        compose.onNodeWithTag("sleep-button").assertTextEquals("27 min")
        compose.onNodeWithTag("sleep-button").performClick()
        compose.onNodeWithTag("sleep-off").performClick()
        assertEquals(listOf("cancel"), calls)
    }

    @Test
    fun upNextShowsTheNextTrack() {
        nowPlaying()
        compose.onNodeWithText("UP NEXT").assertIsDisplayed()
        compose.onNodeWithText("Next song — Other band").assertIsDisplayed()
    }

    @Test
    fun noUpNextWithoutANextTrack() {
        nowPlaying(wave1State(next = false))
        compose.onNodeWithTag("up-next").assertDoesNotExist()
    }
}

/** The Up next row on its own: swipe, the Remove action, and the tap. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class UpNextRowTest {
    @get:Rule
    val compose = createComposeRule()

    private val calls = mutableListOf<String>()

    private fun row() = compose.setContent {
        LyrionTheme(dynamicColour = false) {
            UpNextRow(sampleTrack(2, 1).copy(title = "Next song"), onOpenQueue = { calls += "queue" }, onRemove = { calls += "remove" })
        }
    }

    @Test
    fun swipingEitherWayRemovesIt() {
        row()
        compose.onNodeWithTag("up-next").performTouchInput { swipeLeft() }
        compose.waitForIdle()
        compose.onNodeWithTag("up-next").performTouchInput { swipeRight() }
        compose.waitForIdle()
        assertEquals(listOf("remove", "remove"), calls)
    }

    @Test
    fun tapOpensTheQueueAndScreenReadersCanRemoveIt() {
        row()
        val node = compose.onNodeWithContentDescription("Up next: Next song — Artist")
        node.performClick()
        val config = node.fetchSemanticsNode().config
        assertEquals("Open the queue", config[SemanticsActions.OnClick].label)
        config[SemanticsActions.CustomActions].single { it.label == "Remove from the queue" }.action()
        assertEquals(listOf("queue", "remove"), calls)
    }

    @Test
    fun passesTheAccessibilityAudit() {
        row()
        compose.assertAccessible()
    }
}

/** The sleep timer's choices. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class SleepChoicesTest {
    @get:Rule
    val compose = createComposeRule()

    private val set = mutableListOf<Pair<SleepChoice, SleepScope>>()
    private var cancelled = 0

    private fun choices(left: Int? = null, grouped: Boolean = false, others: Boolean = true, canEnd: Boolean = true, fade: Int = 30) =
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SleepChoices(left, grouped, others, canEnd, fade, onSet = { c, s -> set += c to s }, onCancel = { cancelled++ })
            }
        }

    @Test
    fun lengthsEndsAndScope() {
        choices(grouped = true)
        compose.onNodeWithTag("sleep-15").performClick()
        compose.onNodeWithText("All players").performClick()
        compose.onNodeWithTag("sleep-end-album").performClick()
        compose.onNodeWithText("This sync group").performClick()
        compose.onNodeWithTag("sleep-90").performClick()
        assertEquals(
            listOf(
                SleepChoice.Minutes(15) to SleepScope.PLAYER,
                SleepChoice.End(SleepEnd.ALBUM) to SleepScope.ALL,
                SleepChoice.Minutes(90) to SleepScope.GROUP,
            ),
            set,
        )
    }

    @Test
    fun aCustomLengthIsCheckedFirst() {
        choices()
        compose.onNodeWithTag("sleep-custom").performTextInput("5000")
        compose.onNodeWithTag("sleep-custom-set").performClick()
        compose.onNodeWithText("Enter a whole number of minutes, 1 to 1440.").assertIsDisplayed()
        assertTrue(set.isEmpty())
        // Only digits are taken.
        compose.onNodeWithTag("sleep-custom").performTextInput("x")
        compose.onNodeWithTag("sleep-custom-set").performClick()
        assertTrue(set.isEmpty())
    }

    @Test
    fun aValidCustomLength() {
        choices()
        compose.onNodeWithTag("sleep-custom").performTextInput("20")
        compose.onNodeWithTag("sleep-custom-set").performClick()
        assertEquals(listOf(SleepChoice.Minutes(20) to SleepScope.PLAYER), set)
    }

    @Test
    fun endsAreNotOfferedForAStream() {
        choices(canEnd = false)
        for (end in listOf("track", "album", "queue")) compose.onNodeWithTag("sleep-end-$end").assertIsNotEnabled()
        compose.onNodeWithText("Not for a stream without a set length.").assertIsDisplayed()
    }

    @Test
    fun onlyThisPlayerWhenThereIsNoOther() {
        choices(others = false)
        compose.onNodeWithText("All players").assertDoesNotExist()
        compose.onNodeWithTag("sleep-end-track").assertIsEnabled()
    }

    @Test
    fun aRunningTimerCanBeTurnedOff() {
        choices(left = 90)
        compose.onNodeWithText("Stops in 2 min").assertIsDisplayed()
        compose.onNodeWithTag("sleep-off").performClick()
        assertEquals(1, cancelled)
    }

    @Test
    fun theFadeIsExplained() {
        choices(fade = 0)
        compose.onNodeWithText("No fade-out on this phone (Settings). Other players fade over LMS's last minute, then stop and turn off.")
            .assertIsDisplayed()
    }

    @Test
    fun passesTheAccessibilityAudit() {
        choices(left = 90, grouped = true)
        compose.assertAccessible()
    }
}

/** Settings › Sleep timer fade-out. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class SleepFadeSettingTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun choosesAFadeLength() {
        val chosen = mutableListOf<Int>()
        compose.setContent { LyrionTheme(dynamicColour = false) { SleepFadeSetting(30, onFade = { chosen += it }) } }
        compose.onNodeWithText("30 seconds").assertIsDisplayed()
        compose.onNodeWithTag("sleep-fade-setting").performClick()
        compose.onNodeWithTag("sleep-fade-120").performClick()
        assertEquals(listOf(120), chosen)
    }
}

/**
 * The ambient background (Wave 1 §2) never costs text contrast: for the brand themes and the
 * album-art tinted themes, light and dark, every text colour keeps 4.5:1 (accents 3:1) over
 * any cover once filtered and under the scrim; the mini player's tint keeps its text legible.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class AmbientContrastTest {
    private val covers: List<Int> = listOf(
        argb(0, 0, 0), argb(255, 255, 255), argb(255, 0, 0), argb(0, 255, 0), argb(0, 0, 255),
        argb(255, 255, 0), argb(128, 128, 128), argb(58, 31, 93), argb(245, 230, 200),
    ) + Random(7).let { r -> List(40) { argb(r.nextInt(256), r.nextInt(256), r.nextInt(256)) } }

    private fun drawn(c: Int, dark: Boolean): Int {
        val f = Ambient.filter(dark)
        fun ch(shift: Int) = kotlin.math.round(f.map(((c shr shift) and 0xff) / 255f) * 255f).toInt()
        return argb(ch(16), ch(8), ch(0))
    }

    private fun check(name: String, s: ColorScheme, dark: Boolean) {
        val a = Ambient.scrimAlpha(s, dark)
        assertTrue("$name scrim $a", a in Ambient.MIN_SCRIM..Ambient.MAX_SCRIM)
        val bg = s.background.toArgb()
        for (cover in covers) {
            val under = Ambient.over(bg, a, drawn(cover, dark))
            for ((label, fg, min) in listOf(
                Triple("onBackground", s.onBackground, ArtScheme.TEXT_CONTRAST),
                Triple("onSurfaceVariant", s.onSurfaceVariant, ArtScheme.TEXT_CONTRAST),
                Triple("primary", s.primary, ArtScheme.ACCENT_CONTRAST),
            )) {
                val c = contrast(fg.toArgb(), under)
                assertTrue("$name $label over %06x at $a: %.2f".format(cover and 0xffffff, c), c >= min - 0.01)
            }
        }
    }

    @Test
    fun brandAndArtTintedThemesStayLegible() {
        check("brand light", BrandLight, false)
        check("brand dark", BrandDark, true)
        val r = Random(11)
        repeat(30) {
            val seed = argb(r.nextInt(256), r.nextInt(256), r.nextInt(256))
            check("light seed %06x".format(seed and 0xffffff), BrandLight.tintedBy(seed, false), false)
            check("dark seed %06x".format(seed and 0xffffff), BrandDark.tintedBy(seed, true), true)
        }
    }

    @Test
    fun theDefaultThemesShowTheArt() {
        // Not simply opaque: the art shows through.
        assertTrue(Ambient.scrimAlpha(BrandDark, true) <= 0.75f)
        assertTrue(Ambient.scrimAlpha(BrandLight, false) <= 0.9f)
    }

    @Test
    fun theFilterBoundsTheArt() {
        for (dark in listOf(false, true)) {
            val f = Ambient.filter(dark)
            for (x in 0..20) {
                val v = f.map(x / 20f)
                assertTrue(v >= f.lo - 1e-6f && v <= f.hi + 1e-6f)
            }
            assertEquals(f.lo, f.map(Float.NaN), 1e-6f)
            // The colour matrix does the same map (offsets in 0–255).
            val m = f.matrix().values
            assertEquals(f.contrast * f.brightness, m[0], 1e-6f)
            assertEquals(f.lo * 255f, m[4], 0.01f)
        }
    }

    @Test
    fun theMiniPlayersTintKeepsItsTextLegible() {
        for ((s, name) in listOf(BrandLight to "light", BrandDark to "dark")) {
            for (seed in listOf(argb(0, 0, 0), argb(255, 255, 255), argb(255, 0, 0), argb(0, 0, 255))) {
                val surface = Ambient.miniTint(s.surface.toArgb(), seed)
                assertTrue(name, contrast(s.onSurface.toArgb(), surface) >= ArtScheme.TEXT_CONTRAST)
                assertTrue(name, contrast(s.onSurfaceVariant.toArgb(), surface) >= ArtScheme.TEXT_CONTRAST)
            }
        }
    }
}
