package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.AlarmInfo
import io.github.jamezuk.lyrionanywhere.core.AlarmListing
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The next alarm for the Rooms view (Wave 2 §4.3), as the Rust layer's `next_alarm_in`. */
class NextAlarmTest {
    private fun alarm(time: Int, days: List<Int>, enabled: Boolean = true) = AlarmInfo(
        id = "a$time", timeSecs = time.toUInt(), days = days.map { it.toByte() }.toByteArray(), enabled = enabled,
        repeat = true, volume = null, shuffle = 0u, soundId = null, soundTitle = "",
    )

    private fun listing(vararg a: AlarmInfo, enabled: Boolean = true) = AlarmListing(enabled, a.toList(), emptyList(), null)

    @Test
    fun theSoonestEnabledAlarm() {
        // Monday 09:00: today's 10:00 is an hour away.
        assertEquals(3_600, nextAlarmIn(listing(alarm(10 * 3600, listOf(1))), 1, 9 * 3600))
        // Today's 07:00 has passed: next Monday.
        assertEquals(7 * 86_400 - 2 * 3600, nextAlarmIn(listing(alarm(7 * 3600, listOf(1))), 1, 9 * 3600))
        // The soonest of several; Saturday to Sunday wraps the week.
        val two = listing(alarm(8 * 3600, listOf(0)), alarm(6 * 3600, listOf(2)))
        assertEquals(2 * 3600 + 8 * 3600, nextAlarmIn(two, 6, 22 * 3600))
    }

    @Test
    fun nothingWhenOffOrEmpty() {
        assertNull(nextAlarmIn(null, 1, 0))
        assertNull(nextAlarmIn(listing(), 1, 0))
        assertNull(nextAlarmIn(listing(alarm(3600, listOf(1), enabled = false)), 1, 0))
        assertNull(nextAlarmIn(listing(alarm(3600, emptyList())), 1, 0))
        // The player's "all alarms" switch is off.
        assertNull(nextAlarmIn(listing(alarm(3600, listOf(1)), enabled = false), 1, 0))
    }

    @Test
    fun junkIsBounded() {
        val odd = listing(alarm(Int.MAX_VALUE, listOf(-3, 9, 1)))
        nextAlarmIn(odd, -100, Int.MIN_VALUE)
        nextAlarmIn(odd, Int.MAX_VALUE, Int.MAX_VALUE)
    }
}

/** The Rooms view's statuses through the repository's default (one by one, for fakes). */
class StatusesDefaultTest {
    private val status = PlayerStatus(
        "Room", true, true, PlaybackMode.PLAYING, 0.0, null, 50u, false, ShuffleMode.OFF, RepeatMode.OFF, null, 0u, false, null,
    )

    @Test
    fun badOrTooManyPlayersAreRefusedBeforeAnythingIsAsked() = kotlinx.coroutines.test.runTest {
        val asked = mutableListOf<String>()
        val repo = object : PlayersRepository by io.github.jamezuk.lyrionanywhere.data.offline.FakePlayersPause({}) {
            override suspend fun status(player: String): ApiResult<PlayerStatus> {
                asked += player
                return if (player.endsWith("2")) ApiResult.Err(AppError.Offline()) else ApiResult.Ok(status)
            }

            override suspend fun statuses(players: List<String>) = super<PlayersRepository>.statuses(players)
        }
        val two = repo.statuses(listOf("02:00:00:00:00:01", "02:00:00:00:00:02", "02:00:00:00:00:01"))
        val rooms = (two as ApiResult.Ok).value
        assertEquals(2, rooms.size)
        assertEquals(status, rooms[0].status)
        assertTrue(rooms[1].status == null && rooms[1].error != null)
        asked.clear()
        assertTrue(repo.statuses(listOf("02:00:00:00:00:01", "../x")) is ApiResult.Err)
        assertTrue(repo.statuses((0..20).map { "02:00:00:00:00:%02x".format(it) }) is ApiResult.Err)
        assertTrue("nothing asked", asked.isEmpty())
    }
}
