package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.Credit
import io.github.jamezuk.lyrionanywhere.core.LyricLine
import io.github.jamezuk.lyrionanywhere.core.TrackLyrics
import io.github.jamezuk.lyrionanywhere.data.NowPlayingLayout
import io.github.jamezuk.lyrionanywhere.data.NowPlayingPrefs
import io.github.jamezuk.lyrionanywhere.data.VisualStyle
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnActions
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnList
import io.github.jamezuk.lyrionanywhere.ui.player.RoomsActions
import io.github.jamezuk.lyrionanywhere.ui.player.RoomsScreen
import io.github.jamezuk.lyrionanywhere.ui.player.TransferState
import io.github.jamezuk.lyrionanywhere.ui.player.VisualiserSetting
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * Wave 2 part 3, audited as TalkBack sees it ([assertAccessible]): Now playing in each layout,
 * with the visualiser in each style, on a wide window, the Rooms view (its menu and the
 * "Sync with…" dialog), "Play on…" with All rooms, and the visualiser setting.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class Wave2Part3AccessibilityAuditTests {
    @get:Rule
    val compose = createComposeRule()

    private fun audit(content: @Composable () -> Unit) {
        compose.setContent { LyrionTheme(dynamicColour = false) { content() } }
        compose.assertAccessible()
    }

    private val lyrics = TrackLyrics(false, true, listOf(LyricLine(0u, "First line"), LyricLine(5_000u, "Second line")))
    private val actions = nowPlayingActions().copy(onRooms = {}, onCredit = {})

    @Test
    fun nowPlayingArtWithTheLayoutButtons() = audit {
        NowPlayingScreen(auditPlayerState(), clock = { 0L }, actions = actions)
    }

    @Test
    fun nowPlayingLyricsLayout() = audit {
        NowPlayingScreen(
            auditPlayerState().copy(nowPlaying = NowPlayingPrefs(layout = NowPlayingLayout.LYRICS), lyrics = lyrics, lyricsTrackId = 1),
            clock = { 0L },
            actions = actions,
        )
    }

    @Test
    fun nowPlayingLyricsLayoutWithoutLyrics() = audit {
        NowPlayingScreen(auditPlayerState().copy(nowPlaying = NowPlayingPrefs(layout = NowPlayingLayout.LYRICS)), clock = { 0L }, actions = actions)
    }

    @Test
    fun nowPlayingInfoLayout() = audit {
        NowPlayingScreen(
            auditPlayerState().copy(
                nowPlaying = NowPlayingPrefs(layout = NowPlayingLayout.INFO),
                credits = listOf(Credit(ArtistRole.COMPOSER, "Example Composer", 41), Credit(ArtistRole.ARTIST, "Unlinked Name", null)),
                creditsTrackId = 1,
                status = sampleStatus().copy(next = sampleTrack(2)),
            ),
            clock = { 0L },
            actions = actions,
        )
    }

    @Test
    fun nowPlayingOnThisPhoneWithTheVisualiserOff() = audit {
        NowPlayingScreen(phonePlayingState(), clock = { 0L }, actions = actions, visual = FakeVisualSource())
    }

    /** Each style in turn, in one composition. */
    @Test
    fun theVisualiserInEachStyle() {
        val current = androidx.compose.runtime.mutableStateOf(VisualStyle.BARS)
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                NowPlayingScreen(
                    phonePlayingState(NowPlayingPrefs(visualiser = true, visualStyle = current.value)),
                    clock = { 0L },
                    actions = actions,
                    visual = FakeVisualSource(),
                )
            }
        }
        for (s in VisualStyle.entries) {
            compose.runOnUiThread { current.value = s }
            compose.assertAccessible()
        }
    }

    @Test
    @Config(sdk = [35], qualifiers = "w1280dp-h900dp")
    fun nowPlayingWide() = audit {
        NowPlayingScreen(auditPlayerState(), clock = { 0L }, actions = actions, wide = true)
    }

    @Test
    @Config(sdk = [35], qualifiers = "w1280dp-h900dp")
    fun nowPlayingWideWithLyricsAndTheQueueHidden() = audit {
        NowPlayingScreen(
            phonePlayingState(NowPlayingPrefs(layout = NowPlayingLayout.LYRICS, queuePanel = false, visualiser = true)).copy(lyrics = lyrics, lyricsTrackId = 1),
            clock = { 0L },
            actions = actions,
            wide = true,
            visual = FakeVisualSource(),
        )
    }

    private val roomsActions = RoomsActions({}, {}, { _, _ -> }, { _, _ -> }, { _, _ -> }, {}, onTransfer = {}, onAlarms = { _, _ -> })

    @Test
    fun rooms() = audit { RoomsScreen(sampleRooms(), roomsActions, selectedId = "aa:00:00:00:00:02") }

    @Test
    fun roomsWithATransferRunning() = audit {
        RoomsScreen(
            sampleRooms(),
            roomsActions,
            selectedId = "aa:00:00:00:00:02",
            transfer = TransferState.Running("aa:00:00:00:00:01", "Kitchen", io.github.jamezuk.lyrionanywhere.core.TransferStage.GROUPING),
        )
    }

    @Test
    fun aRoomsMenuOpen() {
        audit { RoomsScreen(sampleRooms(), roomsActions, selectedId = "aa:00:00:00:00:02") }
        compose.onNodeWithTag("room-menu-aa:00:00:00:00:01").performClick()
        compose.assertAccessible()
    }

    @Test
    fun theSyncWithDialog() {
        audit { RoomsScreen(sampleRooms(), roomsActions, selectedId = "aa:00:00:00:00:02") }
        compose.onNodeWithTag("room-menu-aa:00:00:00:00:01").performClick()
        compose.onNodeWithTag("room-sync-aa:00:00:00:00:01").performClick()
        compose.assertAccessible()
    }

    @Test
    fun playOnWithAllRooms() = audit {
        PlayOnList(auditPlayerState(), PlayOnActions(onControl = {}, onBackToPhone = {}, onAllRooms = {}))
    }

    @Test
    fun theVisualiserSettingOn() = audit {
        VisualiserSetting(true, VisualStyle.VU, onChange = {}, onStyle = {})
    }
}
