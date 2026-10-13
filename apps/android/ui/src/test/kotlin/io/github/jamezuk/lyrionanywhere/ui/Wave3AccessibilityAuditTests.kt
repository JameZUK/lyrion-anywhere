package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.STATS_NOT_AVAILABLE
import io.github.jamezuk.lyrionanywhere.data.StatsPeriod
import io.github.jamezuk.lyrionanywhere.ui.settings.HistorySettings
import io.github.jamezuk.lyrionanywhere.ui.settings.HistoryUiState
import io.github.jamezuk.lyrionanywhere.ui.stats.ListeningCard
import io.github.jamezuk.lyrionanywhere.ui.stats.ListeningCardState
import io.github.jamezuk.lyrionanywhere.ui.stats.StatsActions
import io.github.jamezuk.lyrionanywhere.ui.stats.StatsScreen
import io.github.jamezuk.lyrionanywhere.ui.stats.StoryUiState
import io.github.jamezuk.lyrionanywhere.ui.stats.YearStoryScreen
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * Wave 3 part 1, audited as TalkBack sees it ([assertAccessible]): "Your listening" in every
 * state (each section scrolled into view), the Home card with and without the story, the
 * Settings entry, and "Your year in sound" on every card and in its other states.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class Wave3AccessibilityAuditTests {
    @get:Rule
    val compose = createComposeRule()

    private fun audit(content: @Composable () -> Unit) {
        compose.setContent { LyrionTheme(dynamicColour = false) { content() } }
        compose.assertAccessible()
    }

    private fun statsPage(state: io.github.jamezuk.lyrionanywhere.ui.stats.StatsUiState) = audit { StatsScreen(state, StatsActions()) }

    @Test
    fun statsWithEverySection() {
        statsPage(statsState(sampleStats(byMonth = YEAR_MONTHS), period = StatsPeriod.Year(2025)))
        for (tag in listOf("stats-top-albums", "stats-top-tracks", "stats-new-artists", "stats-chart-hour", "stats-chart-month", "stats-streaks")) {
            compose.onNodeWithTag("stats-screen").performScrollToNode(hasTestTag(tag))
            compose.assertAccessible()
        }
    }

    @Test
    fun statsShowingTwenty() {
        statsPage(statsState())
        compose.onNodeWithTag("stats-top-artists-more").performClick()
        compose.assertAccessible()
    }

    @Test
    fun statsLoading() = statsPage(statsState(stats = null, loading = true))

    @Test
    fun statsHistoryOff() = statsPage(statsState(emptyStats(enabled = false)))

    @Test
    fun statsEmpty() = statsPage(statsState(emptyStats()))

    @Test
    fun statsFailed() = statsPage(statsState(stats = null, error = AppError.Offline()))

    @Test
    fun statsUnavailable() = statsPage(statsState(stats = null, error = AppError.Unexpected(STATS_NOT_AVAILABLE)))

    @Test
    fun homeCard() = audit { ListeningCard(ListeningCardState(true, 82_800, 412), onOpen = {}, onStory = {}) }

    @Test
    fun homeCardWithTheStory() = audit { ListeningCard(ListeningCardState(true, 82_800, 412, storyYear = 2026), onOpen = {}, onStory = {}) }

    @Test
    fun settingsEntry() = audit { HistorySettings(HistoryUiState(enabled = true), onEnabled = {}, onView = {}, onClear = {}, onStats = {}) }

    @Test
    fun storyEveryCard() {
        val state = storyState()
        audit { YearStoryScreen(state, onClose = {}, onRetry = {}, sharer = FakeSharer()) }
        repeat(state.cards.size - 1) {
            compose.onNodeWithTag("story-next").performClick()
            compose.waitForIdle()
            compose.assertAccessible()
        }
    }

    private fun story(state: StoryUiState) = audit { YearStoryScreen(state, onClose = {}, onRetry = {}, sharer = FakeSharer()) }

    @Test
    fun storyLoading() = story(StoryUiState(2026))

    @Test
    fun storyEmpty() = story(StoryUiState(2026, loading = false))

    @Test
    fun storyHistoryOff() = story(StoryUiState(2026, loading = false, enabled = false))

    @Test
    fun storyFailed() = story(StoryUiState(2026, loading = false, error = AppError.Offline()))

    @Test
    fun storyUnavailable() = story(StoryUiState(2026, loading = false, error = AppError.Server(404, "")))

    @Test
    @Config(sdk = [35], qualifiers = "w900dp-h420dp")
    fun storyOnAWideShortWindow() = story(storyState())
}
