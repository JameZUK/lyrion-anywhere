package io.github.jamezuk.lyrionanywhere.data

import java.time.LocalDate
import java.util.Locale
import java.util.SimpleTimeZone
import java.util.TimeZone
import kotlin.random.Random
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The stats contract on the app's side: periods, the offset sent, and answers made safe to show. */
class StatsTest {
    @Test
    fun periodsAreWrittenAsTheContractSays() {
        assertEquals("days-30", StatsPeriod.LastDays(30).wire())
        assertEquals("month-2026-03", StatsPeriod.Month(2026, 3).wire())
        assertEquals("year-2025", StatsPeriod.Year(2025).wire())
        assertEquals("all", StatsPeriod.All.wire())
    }

    @Test
    fun periodsOutsideTheContractAreBroughtWithinIt() {
        assertEquals("days-7", StatsPeriod.LastDays(0).wire())
        assertEquals("days-7", StatsPeriod.LastDays(-5).wire())
        assertEquals("days-400", StatsPeriod.LastDays(Int.MAX_VALUE).wire())
        assertEquals("month-2026-12", StatsPeriod.Month(2026, 13).wire())
        assertEquals("month-2026-01", StatsPeriod.Month(2026, 0).wire())
        assertEquals("year-0000", StatsPeriod.Year(-1).wire())
        assertEquals("year-9999", StatsPeriod.Year(123_456).wire())
    }

    @Test
    fun periodsUseAsciiDigitsWhateverTheLanguage() {
        val before = Locale.getDefault()
        try {
            for (tag in listOf("ar-EG", "fa-IR", "hi-IN-u-nu-deva", "th-TH-u-nu-thai")) {
                Locale.setDefault(Locale.forLanguageTag(tag))
                assertEquals(tag, "month-2026-03", StatsPeriod.Month(2026, 3).wire())
                assertEquals(tag, "year-2025", StatsPeriod.Year(2025).wire())
                assertEquals(tag, "days-30", StatsPeriod.LastDays(30).wire())
            }
        } finally {
            Locale.setDefault(before)
        }
    }

    @Test
    fun theOffsetIsMinutesEastOfUtc() {
        val now = 1_790_000_000_000L
        assertEquals(0, deviceTzMinutes(now, TimeZone.getTimeZone("UTC")))
        assertEquals(330, deviceTzMinutes(now, TimeZone.getTimeZone("Asia/Kolkata")))
        assertEquals(345, deviceTzMinutes(now, TimeZone.getTimeZone("Asia/Kathmandu")))
        assertEquals(-210, deviceTzMinutes(now, SimpleTimeZone(-210 * 60_000, "Newfoundland-ish")))
        assertEquals(840, deviceTzMinutes(now, TimeZone.getTimeZone("Pacific/Kiritimati")))
        assertEquals(-600, deviceTzMinutes(now, TimeZone.getTimeZone("Pacific/Honolulu")))
    }

    @Test
    fun theOffsetFollowsSummerTime() {
        val london = TimeZone.getTimeZone("Europe/London")
        val january = java.time.Instant.parse("2026-01-15T12:00:00Z").toEpochMilli()
        val july = java.time.Instant.parse("2026-07-15T12:00:00Z").toEpochMilli()
        assertEquals(0, deviceTzMinutes(january, london))
        assertEquals(60, deviceTzMinutes(july, london))
    }

    @Test
    fun anOffsetBeyondTheContractIsClamped() {
        assertEquals(840, deviceTzMinutes(0, SimpleTimeZone(18 * 3_600_000, "far east")))
        assertEquals(-840, deviceTzMinutes(0, SimpleTimeZone(-18 * 3_600_000, "far west")))
    }

    @Test
    fun choicesReachAsFarAsTheHistory() {
        val today = LocalDate.of(2026, 10, 13)
        assertEquals(
            listOf(StatsPeriod.Month(2026, 10), StatsPeriod.LastDays(30), StatsPeriod.Year(2026), StatsPeriod.Year(2025), StatsPeriod.All),
            statsPeriodChoices(today, 396),
        )
        assertEquals(listOf(2026, 2025, 2024), statsPeriodChoices(today, 700).filterIsInstance<StatsPeriod.Year>().map { it.year })
        assertEquals(listOf(2026), statsPeriodChoices(today, 0).filterIsInstance<StatsPeriod.Year>().map { it.year })
        // On 1 January a day's history already reaches last year.
        assertEquals(listOf(2027, 2026), statsPeriodChoices(LocalDate.of(2027, 1, 1), 1).filterIsInstance<StatsPeriod.Year>().map { it.year })
        // Nonsense retention is bounded.
        assertTrue(statsPeriodChoices(today, Int.MAX_VALUE).size < 20)
        assertEquals(listOf(2026), statsPeriodChoices(today, -50).filterIsInstance<StatsPeriod.Year>().map { it.year })
    }

    @Test
    fun theYearInSoundIsOfferedFromDecemberToJanuary() {
        assertNull(yearInSoundOffer(LocalDate.of(2026, 11, 30)))
        assertEquals(2026, yearInSoundOffer(LocalDate.of(2026, 12, 1)))
        assertEquals(2026, yearInSoundOffer(LocalDate.of(2026, 12, 31)))
        assertEquals(2026, yearInSoundOffer(LocalDate.of(2027, 1, 1)))
        assertEquals(2026, yearInSoundOffer(LocalDate.of(2027, 1, 31)))
        assertNull(yearInSoundOffer(LocalDate.of(2027, 2, 1)))
    }

    private fun stats(
        byHour: List<Long> = List(24) { 0L },
        byWeekday: List<Long> = List(7) { 0L },
        artists: Int = 0,
        name: String = "An Example Band",
    ) = ListeningStats(
        enabled = true,
        retentionDays = 396,
        period = StatsRange(0, 1),
        totals = StatsTotals(1, 60, 1, 1, 1, 1),
        topArtists = List(artists) { TopArtist(it.toLong(), name, 1, 60) },
        topAlbums = List(artists) { TopEntry(it.toLong(), 1, 60) },
        topTracks = List(artists) { TopEntry(it.toLong(), 1, 60) },
        newArtists = List(artists) { NewArtist(it.toLong(), name, 0) },
        byHour = byHour,
        byWeekday = byWeekday,
        byMonth = List(artists) { MonthTotal("2026-01", 1, 60) },
        longestStreak = Streak(1, "2026-01-01", "2026-01-01"),
        busiestDay = BusiestDay("2026-01-01", 60),
    )

    @Test
    fun normalisingFixesListLengthsAndCounts() {
        val n = stats(byHour = listOf(5, -3), byWeekday = List(50) { 2L }, artists = 500, name = "  " + "x".repeat(10_000) + " ").normalised()
        assertEquals(24, n.byHour.size)
        assertEquals(listOf(5L, 0L), n.byHour.take(2))
        assertEquals(7, n.byWeekday.size)
        assertEquals(ListeningStats.MAX_LIST, n.topArtists.size)
        assertEquals(ListeningStats.MAX_LIST, n.topAlbums.size)
        assertEquals(ListeningStats.MAX_LIST, n.topTracks.size)
        assertEquals(ListeningStats.MAX_LIST, n.newArtists.size)
        assertEquals(ListeningStats.MAX_MONTHS, n.byMonth.size)
        assertEquals(ListeningStats.MAX_NAME, n.topArtists.first().name.length)
        assertTrue(n.topArtists.first().name.startsWith("x"))
    }

    @Test
    fun normalisingDropsEmptyStreaksAndNegativeTotals() {
        val n = stats().copy(
            totals = StatsTotals(-1, -1, -1, -1, -1, -1),
            longestStreak = Streak(0, "", ""),
            busiestDay = BusiestDay("2026-01-01", 0),
            retentionDays = -4,
        ).normalised()
        assertEquals(StatsTotals(), n.totals)
        assertTrue(n.isEmpty)
        assertNull(n.longestStreak)
        assertNull(n.busiestDay)
        assertEquals(0, n.retentionDays)
    }

    @Test
    fun normalisingArbitraryAnswersNeverThrows() {
        val r = Random(7)
        repeat(2_000) {
            val s = stats(
                byHour = List(r.nextInt(0, 60)) { r.nextLong() },
                byWeekday = List(r.nextInt(0, 30)) { r.nextLong() },
                artists = r.nextInt(0, 40),
                name = String(CharArray(r.nextInt(0, 300)) { r.nextInt(0, 0xD7FF).toChar() }),
            ).copy(totals = StatsTotals(r.nextLong(), r.nextLong(), r.nextLong(), r.nextLong(), r.nextLong(), r.nextLong()))
            val n = s.normalised()
            assertEquals(24, n.byHour.size)
            assertEquals(7, n.byWeekday.size)
            assertTrue(n.byHour.all { it >= 0 } && n.byWeekday.all { it >= 0 })
            assertTrue(n.topArtists.size <= ListeningStats.MAX_LIST)
            assertTrue(n.totals.plays >= 0)
            assertEquals(n, n.normalised())
        }
    }

    @Test
    fun signedOutTheStatsAreNotAsked() = runBlocking {
        val source = object : ConnectionSource {
            override val connection: StateFlow<Connection?> = MutableStateFlow(null)
        }
        assertEquals(ApiResult.Err(AppError.SignedOut), RustStatsRepository(source).stats(StatsPeriod.All, 0))
    }
}
