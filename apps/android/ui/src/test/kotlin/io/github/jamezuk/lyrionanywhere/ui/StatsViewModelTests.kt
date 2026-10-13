package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.ListeningStats
import io.github.jamezuk.lyrionanywhere.data.STATS_NOT_AVAILABLE
import io.github.jamezuk.lyrionanywhere.data.StatsPeriod
import io.github.jamezuk.lyrionanywhere.ui.stats.ListeningCardViewModel
import io.github.jamezuk.lyrionanywhere.ui.stats.StatsViewModel
import io.github.jamezuk.lyrionanywhere.ui.stats.StoryCard
import io.github.jamezuk.lyrionanywhere.ui.stats.YearStoryViewModel
import io.github.jamezuk.lyrionanywhere.ui.stats.busiest
import io.github.jamezuk.lyrionanywhere.ui.stats.storyCards
import java.time.LocalDate
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** "Your listening", the Home card and "Your year in sound": periods, states and the time zone sent. */
class StatsViewModelTests : MainDispatcherTest() {
    private val october = LocalDate.of(2026, 10, 13)

    @Test
    fun opensOnThisMonthAndSendsTheDevicesOffset() = runTest(dispatcher) {
        val stats = FakeStats()
        val vm = StatsViewModel(stats, FakeMe(), today = { october }, tz = { 330 })
        assertEquals(listOf(StatsPeriod.Month(2026, 10) to 330), stats.calls)
        val s = vm.state.value
        assertFalse(s.loading)
        assertEquals(412L, s.stats?.totals?.plays)
        assertNull(s.error)
    }

    @Test
    fun negativeAndExtremeOffsetsAreSentAsGiven() = runTest(dispatcher) {
        for (tz in listOf(-210, -840, 840, 0)) {
            val stats = FakeStats()
            StatsViewModel(stats, FakeMe(), today = { october }, tz = { tz })
            assertEquals(tz, stats.calls.single().second)
        }
    }

    @Test
    fun theOffsetIsReadAgainForEachLoad() = runTest(dispatcher) {
        // Summer time can begin while the page is open.
        var tz = 0
        val stats = FakeStats()
        val vm = StatsViewModel(stats, FakeMe(), today = { october }, tz = { tz })
        tz = 60
        vm.select(StatsPeriod.LastDays(30))
        assertEquals(listOf(0, 60), stats.calls.map { it.second })
    }

    @Test
    fun offersTheMonthThirtyDaysThisYearPastYearsAndEverything() = runTest(dispatcher) {
        val vm = StatsViewModel(FakeStats(), FakeMe(), today = { october }, tz = { 0 })
        assertEquals(
            listOf(
                StatsPeriod.Month(2026, 10),
                StatsPeriod.LastDays(30),
                StatsPeriod.Year(2026),
                StatsPeriod.Year(2025),
                StatsPeriod.All,
            ),
            vm.state.value.choices,
        )
    }

    @Test
    fun pastYearsFollowHowFarTheHistoryReaches() = runTest(dispatcher) {
        val stats = FakeStats { ApiResult.Ok(sampleStats(retentionDays = 800)) }
        val vm = StatsViewModel(stats, FakeMe(), today = { october }, tz = { 0 })
        val years = vm.state.value.choices.filterIsInstance<StatsPeriod.Year>().map { it.year }
        assertEquals(listOf(2026, 2025, 2024), years)
        // A short history reaches no past year.
        val short = StatsViewModel(FakeStats { ApiResult.Ok(sampleStats(retentionDays = 30)) }, FakeMe(), today = { october }, tz = { 0 })
        assertEquals(listOf(2026), short.state.value.choices.filterIsInstance<StatsPeriod.Year>().map { it.year })
    }

    @Test
    fun choosingAPeriodLoadsIt() = runTest(dispatcher) {
        val stats = FakeStats()
        val vm = StatsViewModel(stats, FakeMe(), today = { october }, tz = { 0 })
        vm.select(StatsPeriod.Year(2025))
        vm.select(StatsPeriod.All)
        assertEquals(listOf(StatsPeriod.Month(2026, 10), StatsPeriod.Year(2025), StatsPeriod.All), stats.calls.map { it.first })
        assertEquals(StatsPeriod.All, vm.state.value.period)
        // The same period again is not asked twice.
        vm.select(StatsPeriod.All)
        assertEquals(3, stats.calls.size)
    }

    @Test
    fun aLateAnswerForAnEarlierPeriodNeverReplacesTheChosenOne() = runTest(dispatcher) {
        val gate = CompletableDeferred<Unit>()
        val stats = FakeStats { p -> ApiResult.Ok(sampleStats(plays = if (p == StatsPeriod.All) 9_999 else 1)) }
        stats.gate = gate
        val vm = StatsViewModel(stats, FakeMe(), today = { october }, tz = { 0 })
        vm.select(StatsPeriod.All)
        stats.gate = null
        gate.complete(Unit)
        assertEquals(StatsPeriod.All, vm.state.value.period)
        assertEquals(9_999L, vm.state.value.stats?.totals?.plays)
    }

    @Test
    fun albumsAndTracksAreLoadedByIdAndArtistsKeepTheirNames() = runTest(dispatcher) {
        val me = FakeMe()
        val vm = StatsViewModel(FakeStats(), me, today = { october }, tz = { 0 })
        val s = vm.state.value
        assertEquals((7L..18L).toList(), me.albumIdCalls.single())
        assertEquals("Album 7", s.albums[7]?.title)
        assertEquals("Track 9", s.tracks[9]?.title)
        assertEquals("An Example Band", s.stats?.topArtists?.first()?.name)
    }

    @Test
    fun albumsThatFailToLoadLeaveThePageUp() = runTest(dispatcher) {
        val me = FakeMe().apply { albumsById = { ApiResult.Err(AppError.Offline()) } }
        val vm = StatsViewModel(FakeStats(), me, today = { october }, tz = { 0 })
        assertTrue(vm.state.value.albums.isEmpty())
        assertEquals(412L, vm.state.value.stats?.totals?.plays)
    }

    @Test
    fun anErrorCanBeRetried() = runTest(dispatcher) {
        var fail = true
        val stats = FakeStats { if (fail) ApiResult.Err(AppError.Offline("down")) else ApiResult.Ok(sampleStats()) }
        val vm = StatsViewModel(stats, FakeMe(), today = { october }, tz = { 0 })
        assertEquals(AppError.Offline("down"), vm.state.value.error)
        assertFalse(vm.state.value.unavailable)
        fail = false
        vm.retry()
        assertNull(vm.state.value.error)
        assertEquals(412L, vm.state.value.stats?.totals?.plays)
    }

    @Test
    fun anOlderGatewayOrTheUnwiredCallReadsAsUnavailable() = runTest(dispatcher) {
        for (e in listOf(AppError.Server(404, "not found"), AppError.Unexpected(STATS_NOT_AVAILABLE))) {
            val vm = StatsViewModel(FakeStats { ApiResult.Err(e) }, FakeMe(), today = { october }, tz = { 0 })
            assertTrue(vm.state.value.unavailable)
        }
        val other = StatsViewModel(FakeStats { ApiResult.Err(AppError.Server(500, "x")) }, FakeMe(), today = { october }, tz = { 0 })
        assertFalse(other.state.value.unavailable)
    }

    @Test
    fun answersAreNormalisedBeforeUse() = runTest(dispatcher) {
        val wild = sampleStats(artists = 50).copy(byHour = listOf(-5, 3), byWeekday = List(30) { 1L })
        val vm = StatsViewModel(FakeStats { ApiResult.Ok(wild) }, FakeMe(), today = { october }, tz = { 0 })
        val s = vm.state.value.stats!!
        assertEquals(ListeningStats.MAX_LIST, s.topArtists.size)
        assertEquals(24, s.byHour.size)
        assertEquals(0L, s.byHour[0])
        assertEquals(7, s.byWeekday.size)
    }

    @Test
    fun theStoryIsOfferedForAYearWithPlaysOnly() = runTest(dispatcher) {
        val vm = StatsViewModel(FakeStats(), FakeMe(), today = { october }, tz = { 0 }, initial = StatsPeriod.Year(2025))
        assertEquals(2025, vm.state.value.storyYear)
        val month = StatsViewModel(FakeStats(), FakeMe(), today = { october }, tz = { 0 })
        assertNull(month.state.value.storyYear)
        val empty = StatsViewModel(FakeStats { ApiResult.Ok(emptyStats()) }, FakeMe(), today = { october }, tz = { 0 }, initial = StatsPeriod.Year(2025))
        assertNull(empty.state.value.storyYear)
        val off = StatsViewModel(FakeStats { ApiResult.Ok(emptyStats(enabled = false)) }, FakeMe(), today = { october }, tz = { 0 }, initial = StatsPeriod.Year(2025))
        assertNull(off.state.value.storyYear)
    }

    /* The Home card */

    @Test
    fun homeCardShowsThisMonth() = runTest(dispatcher) {
        val stats = FakeStats()
        val vm = ListeningCardViewModel(stats, today = { october }, tz = { -300 })
        assertFalse(vm.state.value.visible)
        vm.refresh()
        val s = vm.state.value
        assertTrue(s.visible)
        assertEquals(82_800L, s.monthSeconds)
        assertEquals(412L, s.monthPlays)
        assertNull(s.storyYear)
        assertEquals(listOf(StatsPeriod.Month(2026, 10) to -300), stats.calls)
    }

    @Test
    fun homeCardIsHiddenWithHistoryOffEmptyOrFailing() = runTest(dispatcher) {
        for (answer in listOf<ApiResult<ListeningStats>>(
            ApiResult.Ok(emptyStats(enabled = false)),
            ApiResult.Ok(emptyStats()),
            ApiResult.Err(AppError.Offline()),
            ApiResult.Err(AppError.Unexpected(STATS_NOT_AVAILABLE)),
        )) {
            val vm = ListeningCardViewModel(FakeStats { answer }, today = { october }, tz = { 0 })
            vm.refresh()
            assertTrue(vm.state.value.loaded)
            assertFalse("$answer", vm.state.value.visible)
        }
    }

    @Test
    fun homeCardStaysThroughAReconnect() = runTest(dispatcher) {
        var answer: ApiResult<ListeningStats> = ApiResult.Ok(sampleStats())
        val vm = ListeningCardViewModel(FakeStats { answer }, today = { october }, tz = { 0 })
        vm.refresh()
        assertTrue(vm.state.value.visible)
        // Home shown again while the server can't be reached: the card is kept.
        answer = ApiResult.Err(AppError.Offline())
        vm.refresh()
        assertTrue(vm.state.value.visible)
        assertEquals(412L, vm.state.value.monthPlays)
        // The server saying history is off is an answer, though: the card goes.
        answer = ApiResult.Ok(emptyStats(enabled = false))
        vm.refresh()
        assertFalse(vm.state.value.visible)
    }

    @Test
    fun homeCardOffersThisYearInDecember() = runTest(dispatcher) {
        val stats = FakeStats()
        val vm = ListeningCardViewModel(stats, today = { LocalDate.of(2026, 12, 1) }, tz = { 0 })
        vm.refresh()
        assertEquals(2026, vm.state.value.storyYear)
        // This month's plays prove the year has some: no second call.
        assertEquals(1, stats.calls.size)
    }

    @Test
    fun homeCardOffersLastYearInJanuaryEvenBeforeAPlayThisMonth() = runTest(dispatcher) {
        val stats = FakeStats { p -> if (p is StatsPeriod.Month) ApiResult.Ok(emptyStats()) else ApiResult.Ok(sampleStats()) }
        val vm = ListeningCardViewModel(stats, today = { LocalDate.of(2027, 1, 31) }, tz = { 0 })
        vm.refresh()
        assertEquals(2026, vm.state.value.storyYear)
        assertTrue(vm.state.value.visible)
        assertEquals(0L, vm.state.value.monthPlays)
        assertEquals(StatsPeriod.Year(2026), stats.calls.last().first)
    }

    @Test
    fun homeCardOffersNoStoryForAYearWithNothingPlayed() = runTest(dispatcher) {
        val stats = FakeStats { p -> if (p is StatsPeriod.Year) ApiResult.Ok(emptyStats()) else ApiResult.Ok(sampleStats()) }
        val vm = ListeningCardViewModel(stats, today = { LocalDate.of(2027, 1, 5) }, tz = { 0 })
        vm.refresh()
        assertNull(vm.state.value.storyYear)
        assertTrue(vm.state.value.visible)
    }

    @Test
    fun homeCardOffersNoStoryOutsideTheSeason() = runTest(dispatcher) {
        for (day in listOf(LocalDate.of(2026, 11, 30), LocalDate.of(2027, 2, 1))) {
            val vm = ListeningCardViewModel(FakeStats(), today = { day }, tz = { 0 })
            vm.refresh()
            assertNull("$day", vm.state.value.storyYear)
        }
    }

    /* Your year in sound */

    @Test
    fun theStoryTellsTheYearInOrder() = runTest(dispatcher) {
        val stats = FakeStats()
        val vm = YearStoryViewModel(stats, FakeMe(), 2025, tz = { 60 })
        assertEquals(listOf(StatsPeriod.Year(2025) to 60), stats.calls)
        val cards = vm.state.value.cards
        assertEquals(
            listOf(
                StoryCard.Hours::class,
                StoryCard.TopArtistCard::class,
                StoryCard.TopAlbum::class,
                StoryCard.TopTrack::class,
                StoryCard.TopFive::class,
                StoryCard.Discovery::class,
                StoryCard.BusiestHour::class,
                StoryCard.LongestStreak::class,
                StoryCard.Summary::class,
            ),
            cards.map { it::class },
        )
        assertEquals("Album 7", (cards[2] as StoryCard.TopAlbum).album?.title)
        assertEquals(5, (cards[4] as StoryCard.TopFive).artists.size)
        assertEquals(StoryCard.BusiestHour(21, 48), cards[6])
    }

    @Test
    fun cardsWithNothingToShowAreLeftOut() {
        val sparse = sampleStats(artists = 1, albums = 0, tracks = 0).copy(newArtists = emptyList(), longestStreak = null, byHour = List(24) { 0L })
        val cards = storyCards(sparse, emptyMap(), emptyMap())
        assertEquals(listOf(StoryCard.Hours::class, StoryCard.TopArtistCard::class, StoryCard.Summary::class), cards.map { it::class })
        assertTrue(storyCards(emptyStats(), emptyMap(), emptyMap()).isEmpty())
        assertTrue(storyCards(sampleStats(enabled = false), emptyMap(), emptyMap()).isEmpty())
    }

    @Test
    fun theStorysStates() = runTest(dispatcher) {
        val empty = YearStoryViewModel(FakeStats { ApiResult.Ok(emptyStats()) }, FakeMe(), 2024, tz = { 0 })
        assertTrue(empty.state.value.cards.isEmpty())
        assertTrue(empty.state.value.enabled)
        val off = YearStoryViewModel(FakeStats { ApiResult.Ok(emptyStats(enabled = false)) }, FakeMe(), 2024, tz = { 0 })
        assertFalse(off.state.value.enabled)
        var fail = true
        val failing = YearStoryViewModel(FakeStats { if (fail) ApiResult.Err(AppError.Offline()) else ApiResult.Ok(sampleStats()) }, FakeMe(), 2024, tz = { 0 })
        assertEquals(AppError.Offline(), failing.state.value.error)
        fail = false
        failing.retry()
        assertEquals(9, failing.state.value.cards.size)
    }

    @Test
    fun theBusiestHourIsTheFirstOfTheLargest() {
        assertEquals(3 to 5L, busiest(listOf(0, 1, 2, 5, 5)))
        assertNull(busiest(List(24) { 0L }))
        assertNull(busiest(emptyList()))
    }
}
