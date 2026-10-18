package io.github.jamezuk.lyrionanywhere.ui

import androidx.activity.ComponentActivity
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performSemanticsAction
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.AppEntry
import io.github.jamezuk.lyrionanywhere.core.HistoryEntry
import io.github.jamezuk.lyrionanywhere.ui.components.formatDuration
import io.github.jamezuk.lyrionanywhere.ui.menus.AppTile
import io.github.jamezuk.lyrionanywhere.ui.menus.AppsScreen
import io.github.jamezuk.lyrionanywhere.ui.menus.AppsUiState
import io.github.jamezuk.lyrionanywhere.ui.player.SeekBar
import io.github.jamezuk.lyrionanywhere.ui.player.seekableDuration
import io.github.jamezuk.lyrionanywhere.ui.settings.HistoryRow
import io.github.jamezuk.lyrionanywhere.ui.settings.HistoryScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.HistoryUiState
import io.github.jamezuk.lyrionanywhere.ui.settings.historyKeys
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * The 15 October review's lows in the screens: lists whose keys could repeat (a crash in a
 * lazy list), and a seek bar fed an absurd duration.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h900dp")
class ReviewLowsScreenTests {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val key = "a".repeat(64)
    private val entry = HistoryEntry(2, key, 7, null, 1_790_000_100, 200.0, false)

    @Test
    fun historyWithTheSameEntryTwiceStillShows() {
        // The gateway's history has no row id: the same play recorded twice (or a page
        // boundary that repeats an entry) gave two rows the same key, which crashes the list.
        val rows = listOf(HistoryRow(entry, null), HistoryRow(entry, null), HistoryRow(entry.copy(trackId = 3), null))
        compose.setContent {
            LyrionTheme(dynamicColour = false) { HistoryScreen(HistoryUiState(enabled = true, rows = rows, loaded = true), {}, {}) }
        }
        assertEquals(3, compose.onAllNodesWithText("Unknown track").fetchSemanticsNodes().size)
    }

    @Test
    fun historyKeysAreUniqueAndStableAsPagesArrive() {
        val first = historyKeys(listOf(HistoryRow(entry, null), HistoryRow(entry, null)))
        assertEquals(2, first.toSet().size)
        // A page more: the keys already given don't change.
        val more = historyKeys(listOf(HistoryRow(entry, null), HistoryRow(entry, null), HistoryRow(entry, null)))
        assertEquals(first, more.take(2))
        assertEquals(3, more.toSet().size)
    }

    @Test
    fun aTileRepeatedWithinAndAcrossSectionsDoesntCrashTheGrid() {
        val tile = AppTile("app:same", "Same Service", null)
        val state = AppsUiState(radios = listOf(tile), apps = listOf(tile, tile), loading = false)
        compose.setContent { LyrionTheme(dynamicColour = false) { AppsScreen(state, {}, {}, {}) } }
        // Once per section (the view model also drops the repeat across sections: ReviewLowsAppsTest).
        assertEquals(2, compose.onAllNodesWithText("Same Service").fetchSemanticsNodes().size)
    }

    @Test
    fun anAbsurdDurationNeverSeeksToNaN() {
        val seeks = mutableListOf<Double>()
        for (duration in listOf(Double.POSITIVE_INFINITY, Double.NaN, 1e300, -5.0)) {
            assertNull("$duration", seekableDuration(duration))
        }
        compose.setContent {
            LyrionTheme(dynamicColour = false) { SeekBar(elapsed = 10.0, duration = Double.POSITIVE_INFINITY, enabled = true, onSeek = { seeks += it }) }
        }
        val bar = compose.onNodeWithContentDescription("Position in track")
        // Unknown length: nothing to seek in.
        bar.assertIsNotEnabled()
        val range = bar.fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo]
        assertTrue(range.range.endInclusive.isFinite())
        assertTrue(seeks.isEmpty())
    }

    @Test
    fun aLongButRealDurationSeeksWithinIt() {
        val seeks = mutableListOf<Double>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) { SeekBar(elapsed = 10.0, duration = 3_600.0, enabled = true, onSeek = { seeks += it }) }
        }
        val bar = compose.onNodeWithContentDescription("Position in track")
        bar.performSemanticsAction(SemanticsActions.SetProgress) { it(1_800f) }
        compose.waitForIdle()
        assertTrue(seeks.toString(), seeks.isNotEmpty() && seeks.all { it.isFinite() && it in 0.0..3_600.0 })
    }

    @Test
    fun durationsThatArentNumbersAreShownAsUnknown() {
        assertEquals("–:––", formatDuration(Double.POSITIVE_INFINITY))
        assertEquals("–:––", formatDuration(Double.NaN))
        assertEquals("1:00:00", formatDuration(3_600.0))
    }
}

/**
 * Review, 15 October: the volume popover only heard of changes through the settings
 * observer, which Android writes half a second late (and not on every device). It also
 * listens for the system's volume change now, which comes at once.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class PhoneVolumeObserveTest {
    @Test
    fun aVolumeChangeIsHeardAtOnceAndNotAfterStopping() {
        val context = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.content.Context>()
        val volume = io.github.jamezuk.lyrionanywhere.ui.player.SystemPhoneVolume(context)
        var heard = 0
        val stop = volume.observe { heard++ }
        context.sendBroadcast(android.content.Intent(io.github.jamezuk.lyrionanywhere.ui.player.SystemPhoneVolume.VOLUME_CHANGED))
        org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
        assertTrue("heard $heard", heard >= 1)
        stop()
        val before = heard
        context.sendBroadcast(android.content.Intent(io.github.jamezuk.lyrionanywhere.ui.player.SystemPhoneVolume.VOLUME_CHANGED))
        org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
        assertEquals(before, heard)
    }
}

/** Review, 15 October: menu actions with no text of their own are named from the resources. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class MenuLabelTextTest {
    @Test
    fun verbsComeFromTheResourcesAndNamesAsTheyAre() {
        val res = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.content.Context>().resources
        assertEquals("Play next", io.github.jamezuk.lyrionanywhere.ui.menus.menuLabelText(res, io.github.jamezuk.lyrionanywhere.ui.menus.MenuLabel.Verb(io.github.jamezuk.lyrionanywhere.ui.menus.MenuVerb.PLAY_NEXT)))
        assertEquals("Radio Paradise", io.github.jamezuk.lyrionanywhere.ui.menus.menuLabelText(res, io.github.jamezuk.lyrionanywhere.ui.menus.MenuLabel.Named("Radio Paradise")))
    }

    @Test
    fun knownRefusalsAreTheAppsOwnWordsAndOthersKeepTheirDetail() {
        val res = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.content.Context>().resources
        for (reason in io.github.jamezuk.lyrionanywhere.data.AppError.Reason.entries) {
            val text = io.github.jamezuk.lyrionanywhere.ui.components.errorText(res, io.github.jamezuk.lyrionanywhere.data.AppError.InvalidInput("server text 123", reason))
            if (reason == io.github.jamezuk.lyrionanywhere.data.AppError.Reason.OTHER) {
                assertEquals("Server text 123", text)
            } else {
                assertTrue("$reason: $text", !text.contains("server text 123") && text.isNotBlank())
            }
        }
    }
}

class ReviewLowsAppsTest : MainDispatcherTest() {
    @Test
    fun aCommandListedUnderRadioAndAppsGetsOneTile() = kotlinx.coroutines.test.runTest(dispatcher) {
        val menus = FakeMenus().apply {
            val shared = AppEntry("same", "Same Service", null, action("same", "items"))
            radios = listOf(shared)
            apps = listOf(shared, shared, apps.first())
        }
        val vm = io.github.jamezuk.lyrionanywhere.ui.menus.AppsViewModel(menus, { "aa:00:00:00:00:01" }, io.github.jamezuk.lyrionanywhere.data.SuitabilityCache({ 0L }))
        val s = vm.state.value
        val keys = (s.radios + s.apps + s.plugins).map { it.key }
        assertEquals(keys.toString(), keys.size, keys.toSet().size)
        assertEquals(listOf("app:same"), s.radios.map { it.key })
        assertEquals(listOf("app:someapp"), s.apps.map { it.key })
    }
}
