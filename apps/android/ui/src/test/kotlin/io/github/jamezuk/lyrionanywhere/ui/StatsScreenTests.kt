package io.github.jamezuk.lyrionanywhere.ui

import androidx.activity.ComponentActivity
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.hasScrollToNodeAction
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.captureToImage
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.STATS_NOT_AVAILABLE
import io.github.jamezuk.lyrionanywhere.data.StatsPeriod
import io.github.jamezuk.lyrionanywhere.ui.home.HomeLayouts
import io.github.jamezuk.lyrionanywhere.ui.library.HomeActions
import io.github.jamezuk.lyrionanywhere.ui.library.HomeFeed
import io.github.jamezuk.lyrionanywhere.ui.library.HomeScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.HistorySettings
import io.github.jamezuk.lyrionanywhere.ui.settings.HistoryUiState
import io.github.jamezuk.lyrionanywhere.ui.stats.CardSharer
import io.github.jamezuk.lyrionanywhere.ui.stats.ListeningCard
import io.github.jamezuk.lyrionanywhere.ui.stats.ListeningCardState
import io.github.jamezuk.lyrionanywhere.ui.stats.StatsActions
import io.github.jamezuk.lyrionanywhere.ui.stats.StatsScreen
import io.github.jamezuk.lyrionanywhere.ui.stats.StatsUiState
import io.github.jamezuk.lyrionanywhere.ui.stats.StoryUiState
import io.github.jamezuk.lyrionanywhere.ui.stats.YearStoryScreen
import io.github.jamezuk.lyrionanywhere.ui.stats.hourSummary
import io.github.jamezuk.lyrionanywhere.ui.stats.listenTime
import io.github.jamezuk.lyrionanywhere.ui.stats.monthSummary
import io.github.jamezuk.lyrionanywhere.ui.stats.storyCards
import io.github.jamezuk.lyrionanywhere.ui.stats.weekdaySummary
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import java.time.LocalDate
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

private val TODAY = LocalDate.of(2026, 10, 13)

fun statsState(
    stats: io.github.jamezuk.lyrionanywhere.data.ListeningStats? = sampleStats(),
    period: StatsPeriod = StatsPeriod.Month(2026, 10),
    loading: Boolean = false,
    error: AppError? = null,
) = StatsUiState(
    today = TODAY,
    period = period,
    choices = io.github.jamezuk.lyrionanywhere.data.statsPeriodChoices(TODAY, 396),
    loading = loading,
    stats = stats?.normalised(),
    albums = (7L..30L).associateWith { sampleAlbum(it) },
    tracks = (9L..30L).associateWith { sampleTrack(it) },
    error = error,
)

fun storyState(year: Int = 2026) = StoryUiState(year, loading = false, cards = storyCards(sampleStats(), mapOf(7L to sampleAlbum(7)), mapOf(9L to sampleTrack(9))))

/** Records what would be shared. */
class FakeSharer : CardSharer {
    val shared = mutableListOf<Pair<ImageBitmap, Int>>()

    override suspend fun share(image: ImageBitmap, year: Int) {
        shared += image to year
    }
}

/**
 * The "Your listening" page in each state, its charts' text alternatives, the Home card, the
 * Settings entry, and "Your year in sound" (stepping through, back, sharing an image).
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class StatsScreenTests {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val res get() = ApplicationProvider.getApplicationContext<android.content.Context>().resources

    private fun content(body: @Composable () -> Unit) = compose.setContent { LyrionTheme(dynamicColour = false) { body() } }

    private fun back() = compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }

    private fun scrollTo(tag: String) = compose.onNodeWithTag("stats-screen").performScrollToNode(hasTestTag(tag))

    @Test
    fun showsTotalsAndTheMostPlayed() {
        content { StatsScreen(statsState(), StatsActions()) }
        compose.onNodeWithContentDescription("Listened: 23 h").assertIsDisplayed()
        compose.onNodeWithContentDescription("Plays: 412").assertIsDisplayed()
        compose.onNodeWithContentDescription("Artists: 41").assertIsDisplayed()
        compose.onNodeWithText("1. An Example Band").assertIsDisplayed()
        scrollTo("stats-top-albums")
        compose.onNodeWithText("1. Album 7").assertIsDisplayed()
    }

    @Test
    fun aTopListShowsTenThenTwenty() {
        content { StatsScreen(statsState(), StatsActions()) }
        assertEquals(0, compose.onAllNodes(hasText("11. Example Artist 11")).fetchSemanticsNodes().size)
        compose.onNodeWithTag("stats-top-artists-more").performClick()
        compose.onNodeWithTag("stats-screen").performScrollToNode(hasText("12. Example Artist 12"))
        compose.onNodeWithText("Show 10").assertIsDisplayed()
    }

    @Test
    fun rowsOpenTheirPages() {
        val artists = mutableListOf<Pair<Long, String>>()
        val albums = mutableListOf<Long>()
        content { StatsScreen(statsState(), StatsActions(onArtist = { id, name -> artists += id to name }, onAlbum = { albums += it })) }
        compose.onNodeWithText("1. An Example Band").performClick()
        scrollTo("stats-top-albums")
        compose.onNodeWithText("1. Album 7").performClick()
        // A track opens its album.
        compose.onNodeWithTag("stats-screen").performScrollToNode(hasText("1. Track 9"))
        compose.onNodeWithText("1. Track 9").performClick()
        assertEquals(listOf(100L to "An Example Band"), artists)
        assertEquals(listOf(7L, 7L), albums)
    }

    @Test
    fun choosingAPeriod() {
        val chosen = mutableListOf<StatsPeriod>()
        content { StatsScreen(statsState(), StatsActions(onPeriod = { chosen += it })) }
        compose.onNodeWithText("This month").assertIsDisplayed()
        compose.onNodeWithTag("stats-period-days-30").performClick()
        compose.onNodeWithTag("stats-periods").performScrollToNode(hasTestTag("stats-period-all"))
        compose.onNodeWithTag("stats-period-all").performClick()
        assertEquals(listOf(StatsPeriod.LastDays(30), StatsPeriod.All), chosen)
    }

    @Test
    fun chartsHaveTextAlternativesWithTheKeyFacts() {
        content { StatsScreen(statsState(sampleStats(byMonth = YEAR_MONTHS), period = StatsPeriod.Year(2026)), StatsActions()) }
        scrollTo("stats-chart-hour")
        val hour = compose.onNodeWithTag("stats-chart-hour").fetchSemanticsNode().config[SemanticsProperties.ContentDescription].single()
        assertTrue(hour, hour.contains("Busiest from 21:00 to 22:00, 48 plays"))
        assertTrue(hour, hour.contains("20:00, 30 plays"))
        scrollTo("stats-chart-weekday")
        val week = compose.onNodeWithTag("stats-chart-weekday").fetchSemanticsNode().config[SemanticsProperties.ContentDescription].single()
        assertTrue(week, week.contains("Most on Saturday, fewest on Tuesday"))
        assertTrue(week, week.contains("Monday, 12 plays"))
        scrollTo("stats-chart-month")
        val month = compose.onNodeWithTag("stats-chart-month").fetchSemanticsNode().config[SemanticsProperties.ContentDescription].single()
        assertTrue(month, month.contains("Most in October 2026, 412 plays"))
    }

    @Test
    fun theMonthChartIsOnlyShownWhenThereAreMonths() {
        content { StatsScreen(statsState(), StatsActions()) }
        scrollTo("stats-chart-weekday")
        assertEquals(0, compose.onAllNodes(hasTestTag("stats-chart-month")).fetchSemanticsNodes().size)
    }

    @Test
    fun chartSummariesForEmptyAndSingleHours() {
        assertEquals("By hour of the day: nothing played.", hourSummary(res, List(24) { 0L }))
        assertEquals(
            "Plays by hour of the day. Busiest from 23:00 to 00:00, 1 play.",
            hourSummary(res, List(24) { if (it == 23) 1L else 0L }),
        )
        assertEquals("By day of the week: nothing played.", weekdaySummary(res, List(7) { 0L }))
        assertEquals("By month: nothing played.", monthSummary(res, emptyList()))
        assertEquals("45 min", listenTime(res, 2_700))
        assertEquals("2 h 15 min", listenTime(res, 8_100))
        assertEquals("23 h", listenTime(res, 82_800))
        assertEquals("0 min", listenTime(res, -5))
    }

    @Test
    fun streaksAndNewArtists() {
        content { StatsScreen(statsState(), StatsActions()) }
        scrollTo("stats-new-artists")
        compose.onNodeWithText("Another Example").assertIsDisplayed()
        scrollTo("stats-streaks")
        compose.onNodeWithText("Longest streak").assertIsDisplayed()
        compose.onNodeWithText("Busiest day").assertIsDisplayed()
    }

    @Test
    fun historyOffSaysSoAndLinksToTheSetting() {
        var opened = 0
        content { StatsScreen(statsState(emptyStats(enabled = false)), StatsActions(onHistorySettings = { opened++ })) }
        compose.onNodeWithTag("stats-off").assertIsDisplayed()
        compose.onNodeWithText("Listening history settings").performClick()
        assertEquals(1, opened)
    }

    @Test
    fun tooLittleHistorySaysHowLongUntilThereIsSomething() {
        content { StatsScreen(statsState(emptyStats()), StatsActions()) }
        compose.onNodeWithTag("stats-empty").assertIsDisplayed()
        compose.onNodeWithText("within 5 minutes", substring = true).assertIsDisplayed()
    }

    @Test
    fun loadingErrorAndUnavailable() {
        var state = statsState(stats = null, loading = true)
        var retried = 0
        val holder = androidx.compose.runtime.mutableStateOf(state)
        content { StatsScreen(holder.value, StatsActions(onRetry = { retried++ })) }
        compose.onNodeWithContentDescription("Loading").assertIsDisplayed()
        state = statsState(stats = null, error = AppError.Offline())
        holder.value = state
        compose.onNodeWithText("Try again").performClick()
        assertEquals(1, retried)
        holder.value = statsState(stats = null, error = AppError.Unexpected(STATS_NOT_AVAILABLE))
        compose.onNodeWithTag("stats-unavailable").assertIsDisplayed()
    }

    @Test
    fun theStoryIsOfferedForAYear() {
        val years = mutableListOf<Int>()
        content { StatsScreen(statsState(period = StatsPeriod.Year(2025)), StatsActions(onStory = { years += it })) }
        compose.onNodeWithTag("stats-story").performClick()
        assertEquals(listOf(2025), years)
    }

    /* The Home card and Settings */

    @Test
    fun theHomeCardShowsThisMonth() {
        var opened = 0
        content { ListeningCard(ListeningCardState(true, 82_800, 412), onOpen = { opened++ }, onStory = {}) }
        compose.onNodeWithText("This month: 23 h · 412 plays").assertIsDisplayed()
        compose.onNodeWithTag("home-listening-card").performClick()
        assertEquals(1, opened)
        assertEquals(0, compose.onAllNodes(hasTestTag("home-story")).fetchSemanticsNodes().size)
    }

    @Test
    fun theHomeCardOffersTheStoryInSeason() {
        val years = mutableListOf<Int>()
        content { ListeningCard(ListeningCardState(true, 3_600, 1, storyYear = 2026), onOpen = {}, onStory = { years += it }) }
        compose.onNodeWithText("This month: 1 h · 1 play").assertIsDisplayed()
        compose.onNodeWithTag("home-story").performClick()
        assertEquals(listOf(2026), years)
    }

    @Test
    fun theHomeCardIsHiddenWhenThereIsNothingToShow() {
        content {
            ListeningCard(ListeningCardState(loaded = true), onOpen = {}, onStory = {})
            ListeningCard(ListeningCardState(loaded = false, monthPlays = 3), onOpen = {}, onStory = {})
        }
        assertEquals(0, compose.onAllNodes(hasTestTag("home-listening-card")).fetchSemanticsNodes().size)
    }

    @Test
    fun theHomeCardComesLastOnHome() {
        val feed = HomeFeed(layout = HomeLayouts.DEFAULT, layoutLoaded = true)
        content {
            HomeScreen(
                feed = feed,
                actions = HomeActions({}, {}, {}, {}, {}, {}, {}),
                footer = { ListeningCard(ListeningCardState(true, 82_800, 412), onOpen = {}, onStory = {}) },
            )
        }
        compose.onAllNodes(hasScrollToNodeAction()).onFirst().performScrollToNode(hasTestTag("home-listening-card"))
        compose.onNodeWithTag("home-listening-card").assertIsDisplayed()
    }

    @Test
    fun settingsOpensYourListening() {
        var opened = 0
        content { HistorySettings(HistoryUiState(enabled = true), onEnabled = {}, onView = {}, onClear = {}, onStats = { opened++ }) }
        compose.onNodeWithTag("settings-stats").performClick()
        assertEquals(1, opened)
        compose.onNodeWithText("Your listening").assertIsDisplayed()
    }

    /* Your year in sound */

    private fun story(state: StoryUiState = storyState(), sharer: CardSharer = FakeSharer(), onClose: () -> Unit = {}) =
        content { YearStoryScreen(state, onClose = onClose, onRetry = {}, sharer = sharer) }

    @Test
    fun theStorySteppsThroughWithPreviousAndNext() {
        story()
        compose.onNodeWithTag("story-position").assertTextEquals("Card 1 of 9")
        compose.onNodeWithTag("story-previous").assertIsNotEnabled()
        compose.onNodeWithText("You listened for", substring = true).assertIsDisplayed()
        compose.onNodeWithTag("story-next").performClick()
        compose.waitForIdle()
        compose.onNodeWithTag("story-position").assertTextEquals("Card 2 of 9")
        compose.onNodeWithText("Your top artist").assertIsDisplayed()
        compose.onNodeWithTag("story-previous").assertIsEnabled().performClick()
        compose.waitForIdle()
        compose.onNodeWithTag("story-position").assertTextEquals("Card 1 of 9")
        repeat(8) {
            compose.onNodeWithTag("story-next").performClick()
            compose.waitForIdle()
        }
        compose.onNodeWithTag("story-position").assertTextEquals("Card 9 of 9")
        compose.onNodeWithText("Your 2026 in sound").assertIsDisplayed()
        compose.onNodeWithTag("story-next").assertIsNotEnabled()
    }

    @Test
    fun eachCardOffersTalkBackActionsToMove() {
        story()
        val first = compose.onNodeWithTag("story-card-0").fetchSemanticsNode().config
        val labels = first.getOrNull(SemanticsActions.CustomActions).orEmpty().map { it.label }
        assertEquals(listOf("Next card"), labels)
        val next = first[SemanticsActions.CustomActions].single()
        compose.runOnUiThread { next.action() }
        compose.waitForIdle()
        compose.onNodeWithTag("story-position").assertTextEquals("Card 2 of 9")
        val second = compose.onNodeWithTag("story-card-1").fetchSemanticsNode().config
        assertEquals(listOf("Previous card", "Next card"), second[SemanticsActions.CustomActions].map { it.label })
    }

    @Test
    fun theStoryDoesNotMoveOnItsOwn() {
        story()
        compose.mainClock.advanceTimeBy(60_000)
        compose.waitForIdle()
        compose.onNodeWithTag("story-position").assertTextEquals("Card 1 of 9")
    }

    @Test
    fun backAndCloseCloseTheStory() {
        var closed = 0
        story(onClose = { closed++ })
        back()
        compose.waitForIdle()
        assertEquals(1, closed)
        compose.onNodeWithTag("story-close").performClick()
        assertEquals(2, closed)
    }

    @Test
    fun shareMakesAnImageOfTheCurrentCard() {
        val sharer = FakeSharer()
        story(sharer = sharer)
        compose.onNodeWithTag("story-next").performClick()
        compose.waitForIdle()
        // Robolectric draws only when asked: draw the screen once, as a device would.
        compose.onRoot().captureToImage()
        compose.onNodeWithTag("story-share").performClick()
        compose.waitUntil(5_000) { sharer.shared.isNotEmpty() || compose.onAllNodes(hasTestTag("story-share-failed")).fetchSemanticsNodes().isNotEmpty() }
        assertEquals(0, compose.onAllNodes(hasTestTag("story-share-failed")).fetchSemanticsNodes().size)
        val (image, year) = sharer.shared.single()
        assertEquals(2026, year)
        val card = compose.onNodeWithTag("story-card-1").fetchSemanticsNode().size
        assertTrue("an image of the card: ${image.width}x${image.height}", image.width >= card.width && image.height >= card.height)
    }

    @Test
    fun aStoryWithNothingToTell() {
        story(StoryUiState(2024, loading = false, cards = emptyList()))
        compose.onNodeWithTag("story-note").assertIsDisplayed()
        compose.onNodeWithText("Nothing played in 2024", substring = true).assertIsDisplayed()
        assertEquals(0, compose.onAllNodes(hasTestTag("story-share")).fetchSemanticsNodes().size)
    }

    @Test
    fun aStoryWithHistoryOff() {
        story(StoryUiState(2024, loading = false, enabled = false))
        compose.onNodeWithText("Listening history settings").assertIsDisplayed()
    }

    @Test
    fun aStoryThatFailed() {
        story(StoryUiState(2024, loading = false, error = AppError.Offline()))
        compose.onNodeWithText("Try again").assertIsDisplayed()
        compose.onNodeWithTag("story-close").assertIsDisplayed()
    }
}
