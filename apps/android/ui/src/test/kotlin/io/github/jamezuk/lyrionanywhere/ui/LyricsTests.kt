package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.assertContentDescriptionEquals
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.LyricLine
import io.github.jamezuk.lyrionanywhere.core.TrackLyrics
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.LyricsCache
import io.github.jamezuk.lyrionanywhere.data.LyricsRepository
import io.github.jamezuk.lyrionanywhere.ui.player.LyricsPeekSetting
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val ME = "aa:00:00:00:00:01"

private val SYNCED = TrackLyrics(
    false,
    true,
    listOf(LyricLine(1000u, "First line"), LyricLine(8000u, "Second line"), LyricLine(20000u, "")),
)
private val PLAIN = TrackLyrics(true, false, listOf(LyricLine(0u, "Just words"), LyricLine(0u, "More words")))

/** Track 1 playing at 10 s (sampleStatus), with [lyrics] for it. */
private fun lyricsState(lyrics: TrackLyrics?, peek: Boolean = true, lyricsFor: Long? = 1, canSeek: Boolean = true) = PlayerUiState(
    players = listOf(samplePlayer(ME)),
    selectedId = ME,
    status = sampleStatus().copy(canSeek = canSeek),
    loading = false,
    local = LocalPlayerState(),
    lyrics = lyrics,
    lyricsTrackId = lyricsFor,
    lyricsPeek = peek,
)

/** Lyrics on Now playing (Wave 1 §9): the peek, the button, the full view, tap to seek. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class LyricsScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private val seeks = mutableListOf<Double>()

    private fun nowPlaying(state: PlayerUiState) = compose.setContent {
        LyrionTheme(dynamicColour = false) {
            NowPlayingScreen(state, clock = { 0L }, actions = nowPlayingActions().copy(onSeek = { seeks += it }))
        }
    }

    @Test
    fun thePeekShowsTheCurrentLineAndOpensTheLyrics() {
        nowPlaying(lyricsState(SYNCED))
        compose.onNodeWithTag("lyrics-peek").assertTextEquals("Second line")
        compose.onNodeWithContentDescription("Lyrics: Second line. Show all the lyrics").performClick()
        // The hero turns to its Lyrics page (the peek goes: the lyrics are showing).
        compose.onNodeWithTag("page-panel-LYRICS").assertIsDisplayed()
        compose.onNodeWithTag("lyrics-peek").assertDoesNotExist()
        compose.onNodeWithTag("lyrics-line-1").assertIsSelected()
        compose.onNodeWithTag("lyrics-line-2").assertTextEquals("♪")
        compose.onNodeWithText("From the file’s tags").assertExists()
        // Tap a line: seek there.
        compose.onNodeWithTag("lyrics-line-0").performClick()
        assertEquals(listOf(1.0), seeks)
        // Back to the cover by its tab.
        compose.onNodeWithTag("page-ART").performClick()
        compose.onNodeWithTag("page-panel-LYRICS").assertDoesNotExist()
        compose.onNodeWithTag("artwork").assertIsDisplayed()
    }

    @Test
    fun noPeekWhenSwitchedOffButTheLyricsPageStays() {
        nowPlaying(lyricsState(SYNCED, peek = false))
        compose.onNodeWithTag("lyrics-peek").assertDoesNotExist()
        compose.onNodeWithTag("page-LYRICS").assertContentDescriptionEquals("Lyrics").performClick()
        compose.onNodeWithTag("page-panel-LYRICS").assertIsDisplayed()
    }

    @Test
    fun plainLyricsHaveNoPeekAndNoSeeking() {
        nowPlaying(lyricsState(PLAIN))
        compose.onNodeWithTag("lyrics-peek").assertDoesNotExist()
        compose.onNodeWithTag("page-LYRICS").performClick()
        compose.onNodeWithText("Just words").assertIsDisplayed()
        compose.onNodeWithText("From Music & Artist Information").assertExists()
        compose.onNodeWithTag("lyrics-line-0").performClick()
        assertEquals(emptyList<Double>(), seeks)
    }

    @Test
    fun aPlayerThatCantSeekDoesntSeek() {
        nowPlaying(lyricsState(SYNCED, canSeek = false))
        compose.onNodeWithTag("page-LYRICS").performClick()
        compose.onNodeWithTag("lyrics-line-0").assertIsNotEnabled()
    }

    @Test
    fun nothingWithoutLyricsOrWithAnotherTracksLyrics() {
        nowPlaying(lyricsState(null))
        compose.onNodeWithTag("lyrics-button").assertDoesNotExist()
        compose.onNodeWithTag("lyrics-peek").assertDoesNotExist()
    }

    @Test
    fun lyricsForAnEarlierTrackAreNotShown() {
        nowPlaying(lyricsState(SYNCED, lyricsFor = 99))
        compose.onNodeWithTag("lyrics-button").assertDoesNotExist()
        compose.onNodeWithTag("lyrics-peek").assertDoesNotExist()
    }

    @Test
    fun hostileTextIsShownAsText() {
        val evil = TrackLyrics(false, true, listOf(LyricLine(0u, "<b>bold</b> ‮gpj.exe %s %1\$s")))
        nowPlaying(lyricsState(evil))
        compose.onNodeWithTag("lyrics-peek").assertTextEquals("<b>bold</b> ‮gpj.exe %s %1\$s")
    }

    @Test
    fun theSettingSwitchesThePeek() {
        var on = true
        compose.setContent {
            LyrionTheme(dynamicColour = false) { LyricsPeekSetting(on) { on = it } }
        }
        compose.onNodeWithTag("lyrics-peek-setting").assertIsOn().performClick()
        assertFalse(on)
    }

    @Test
    fun theSettingShowsOff() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) { LyricsPeekSetting(false) {} }
        }
        compose.onNodeWithTag("lyrics-peek-setting").assertIsOff()
    }
}

private class FakeLyrics(private val answers: Map<Long, ApiResult<TrackLyrics?>>) : LyricsRepository {
    val asked = mutableListOf<Long>()

    override suspend fun track(trackId: Long): ApiResult<TrackLyrics?> {
        asked += trackId
        return answers[trackId] ?: ApiResult.Ok(null)
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class LyricsViewModelTest : MainDispatcherTest() {
    @Test
    fun theCurrentTracksLyricsFollowTheTrack() = runTest(dispatcher) {
        val players = FakePlayers()
        val repo = FakeLyrics(mapOf(1L to ApiResult.Ok(SYNCED), 3L to ApiResult.Err(AppError.Offline())))
        val settings = FakeSettings()
        val vm = PlayerViewModel(players, FakeEvents(), settings, lyrics = LyricsCache(repo), clock = { 0L })
        vm.start()
        assertEquals(SYNCED, vm.state.value.lyrics)
        assertEquals(1L, vm.state.value.lyricsTrackId)
        // Another track without lyrics: the old ones go.
        players.statusResult = ApiResult.Ok(sampleStatus().copy(current = sampleTrack(2, 1)))
        vm.refresh()
        assertNull(vm.state.value.lyrics)
        // Failing: nothing shown.
        players.statusResult = ApiResult.Ok(sampleStatus().copy(current = sampleTrack(3, 2)))
        vm.refresh()
        assertNull(vm.state.value.lyrics)
        // A stream (no library id): nothing asked.
        players.statusResult = ApiResult.Ok(sampleStatus().copy(current = sampleTrack(4, 3).copy(id = null)))
        vm.refresh()
        assertNull(vm.state.value.lyrics)
        // Back to the first: remembered, not asked again.
        players.statusResult = ApiResult.Ok(sampleStatus())
        vm.refresh()
        assertEquals(SYNCED, vm.state.value.lyrics)
        assertEquals(listOf(1L, 2L, 3L), repo.asked)
        // The setting.
        vm.setLyricsPeek(false)
        assertFalse(settings.state.value.lyricsPeek)
        assertFalse(vm.state.value.lyricsPeek)
        vm.stop()
    }
}
