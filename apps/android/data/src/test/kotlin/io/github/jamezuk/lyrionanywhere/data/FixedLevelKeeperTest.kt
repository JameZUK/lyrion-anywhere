package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.data.offline.FakePlayersPause
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** LMS's "Output level is fixed at 100%" for this phone's player follows the Sound setting. */
class FixedLevelKeeperTest {
    private val phone = "02:4c:41:00:00:01"

    /** LMS as the keeper sees it: the status it reports, and the preference calls made. */
    private class Lms(var fixed: Boolean, var fail: Boolean = false, var statusFails: Boolean = false) :
        PlayersRepository by FakePlayersPause({}) {
        val calls = mutableListOf<Pair<String, Boolean>>()
        var statusReads = 0

        override suspend fun status(player: String): ApiResult<PlayerStatus> {
            statusReads++
            if (statusFails) return ApiResult.Err(AppError.Offline())
            return ApiResult.Ok(
                PlayerStatus(
                    "Phone", true, true, PlaybackMode.PLAYING, 0.0, null, 50u, false, ShuffleMode.OFF, RepeatMode.OFF,
                    null, 0u, false, null, fixedLevel = fixed,
                ),
            )
        }

        override suspend fun setFixedOutputLevel(player: String, on: Boolean): ApiResult<Unit> {
            calls += player to on
            if (fail) return ApiResult.Err(AppError.Offline())
            fixed = on
            return ApiResult.Ok(Unit)
        }
    }

    private fun state(id: String?, fixed: Boolean = true, connection: PlayerConnection = PlayerConnection.Connected) = LocalPlayerState(
        enabled = true,
        status = id?.let { status(it, connection) },
        sound = SoundSettings(fixedOutputLevel = fixed),
    )

    private fun TestScope.make(local: MutableStateFlow<LocalPlayerState>, lms: Lms, logs: MutableList<String> = mutableListOf()) =
        FixedLevelKeeper(
            CoroutineScope(backgroundScope.coroutineContext + UnconfinedTestDispatcher(testScheduler)),
            local,
            lms,
            log = { logs += it },
            nowMs = { testScheduler.currentTime },
        )

    private fun TestScope.keeper(local: MutableStateFlow<LocalPlayerState>, lms: Lms, logs: MutableList<String> = mutableListOf()) =
        make(local, lms, logs).start()

    /** LMS's status for this phone's player as the app's own read would see it. */
    private fun lmsStatus(fixed: Boolean) = PlayerStatus(
        "Phone", true, true, PlaybackMode.PLAYING, 0.0, null, 50u, false, ShuffleMode.OFF, RepeatMode.OFF,
        null, 0u, false, null, fixedLevel = fixed,
    )

    @Test
    fun onByDefault() {
        assertTrue(SoundSettings().fixedOutputLevel)
        assertTrue(LocalPlayerState().sound.fixedOutputLevel)
        assertTrue(SoundSettings(fixedOutputLevel = true).sanitised().fixedOutputLevel)
    }

    @Test
    fun settingOnAndLmsVariableSetsItOnce() = runTest {
        val lms = Lms(fixed = false)
        val local = MutableStateFlow(state(null))
        keeper(local, lms)
        runCurrent()
        assertEquals("nothing without a player", 0, lms.statusReads)
        local.value = state(phone)
        runCurrent()
        assertEquals(listOf(phone to true), lms.calls)
        // The same player and setting again (a status update): nothing more.
        local.value = state(phone).copy(eqBypass = true)
        runCurrent()
        assertEquals(1, lms.calls.size)
        assertEquals(1, lms.statusReads)
    }

    @Test
    fun settingOnAndLmsAlreadyFixedSendsNothing() = runTest {
        val lms = Lms(fixed = true)
        keeper(MutableStateFlow(state(phone)), lms)
        runCurrent()
        assertEquals(1, lms.statusReads)
        assertTrue(lms.calls.isEmpty())
    }

    @Test
    fun settingOffAndLmsFixedGivesTheVolumeBack() = runTest {
        val lms = Lms(fixed = true)
        keeper(MutableStateFlow(state(phone, fixed = false)), lms)
        runCurrent()
        assertEquals(listOf(phone to false), lms.calls)
    }

    @Test
    fun settingOffAndLmsVariableSendsNothing() = runTest {
        val lms = Lms(fixed = false)
        keeper(MutableStateFlow(state(phone, fixed = false)), lms)
        runCurrent()
        assertTrue(lms.calls.isEmpty())
    }

    @Test
    fun changingTheSettingWhileThePlayerIsThereFollows() = runTest {
        val lms = Lms(fixed = true)
        val local = MutableStateFlow(state(phone))
        keeper(local, lms)
        runCurrent()
        assertTrue(lms.calls.isEmpty())
        local.value = state(phone, fixed = false)
        runCurrent()
        assertEquals(listOf(phone to false), lms.calls)
        local.value = state(phone, fixed = true)
        runCurrent()
        assertEquals(listOf(phone to false, phone to true), lms.calls)
    }

    @Test
    fun aNewPlayerIdIsCheckedAgain() = runTest {
        val lms = Lms(fixed = true)
        val local = MutableStateFlow(state(phone))
        keeper(local, lms)
        runCurrent()
        // LMS lost the preference (another server, a reset): the next id is put right.
        lms.fixed = false
        local.value = state("02:4c:41:00:00:02")
        runCurrent()
        assertEquals(listOf("02:4c:41:00:00:02" to true), lms.calls)
    }

    /**
     * A failed set is tried once more after [FixedLevelKeeper.RETRY_MS], then left until the
     * next trigger: a reconnect (the id stays the same; the connection state goes and comes).
     */
    @Test
    fun aFailureIsRetriedOnceThenLeftUntilTheNextTrigger() = runTest {
        val lms = Lms(fixed = false, fail = true)
        val logs = mutableListOf<String>()
        val local = MutableStateFlow(state(phone))
        keeper(local, lms, logs)
        runCurrent()
        assertEquals("one attempt at once", 1, lms.calls.size)
        testScheduler.advanceTimeBy(FixedLevelKeeper.RETRY_MS + 1)
        runCurrent()
        assertEquals("one retry", 2, lms.calls.size)
        testScheduler.advanceTimeBy(10 * 60_000)
        runCurrent()
        assertEquals("nothing more", 2, lms.calls.size)
        assertEquals(2, logs.size)
        // Unrelated state changes are not a trigger.
        local.value = state(phone).copy(eqBypass = true)
        runCurrent()
        assertEquals(2, lms.calls.size)
        // A reconnect: the id never goes, the connection does; connected again is a trigger.
        local.value = state(phone, connection = PlayerConnection.Reconnecting(2_000uL))
        runCurrent()
        assertEquals("not while reconnecting", 2, lms.calls.size)
        lms.fail = false
        local.value = state(phone)
        runCurrent()
        assertEquals(listOf(phone to true, phone to true, phone to true), lms.calls)
    }

    /** An id alone (LMS may not have the player yet) is not a trigger; connected is. */
    @Test
    fun theIdAloneIsNotATriggerConnectedIs() = runTest {
        val lms = Lms(fixed = false)
        val local = MutableStateFlow(state(phone, connection = PlayerConnection.Connecting))
        keeper(local, lms)
        runCurrent()
        assertEquals(0, lms.statusReads)
        // Ended for good: nothing either.
        local.value = state(phone, connection = PlayerConnection.Stopped("test"))
        runCurrent()
        assertEquals(0, lms.statusReads)
        // The phone's player switched off, with a connected status left over: not running.
        local.value = state(phone).copy(enabled = false)
        runCurrent()
        assertEquals(0, lms.statusReads)
        local.value = state(phone)
        runCurrent()
        assertEquals(1, lms.statusReads)
        assertEquals(listOf(phone to true), lms.calls)
    }

    /** LMS doesn't know the player yet at the greeting: the read is tried again after 2 s. */
    @Test
    fun aFailedReadIsRetriedOnceAfterAMoment() = runTest {
        val lms = Lms(fixed = false, statusFails = true)
        keeper(MutableStateFlow(state(phone)), lms)
        runCurrent()
        assertEquals(1, lms.statusReads)
        lms.statusFails = false
        testScheduler.advanceTimeBy(FixedLevelKeeper.RETRY_MS - 100)
        runCurrent()
        assertEquals("not before the retry is due", 1, lms.statusReads)
        testScheduler.advanceTimeBy(200)
        runCurrent()
        assertEquals(2, lms.statusReads)
        assertEquals(listOf(phone to true), lms.calls)
    }

    /**
     * A controller flipped the preference back: a status the app read showing it so sets it
     * again, at most every 30 s, and only for this phone's connected player.
     */
    @Test
    fun aControllerFlippingItBackIsNoticedFromTheStatusInHand() = runTest {
        val lms = Lms(fixed = true)
        val local = MutableStateFlow(state(phone))
        val k = make(local, lms)
        k.start()
        runCurrent()
        assertTrue("in step at the start", lms.calls.isEmpty())
        testScheduler.advanceTimeBy(60_000)
        // A status that agrees: nothing.
        k.observeStatus(phone, lmsStatus(fixed = true))
        runCurrent()
        assertTrue(lms.calls.isEmpty())
        // Another player's status: nothing.
        k.observeStatus("02:4c:41:00:00:09", lmsStatus(fixed = false))
        runCurrent()
        assertTrue(lms.calls.isEmpty())
        // This phone's, disagreeing: set back.
        k.observeStatus(phone, lmsStatus(fixed = false))
        runCurrent()
        assertEquals(listOf(phone to true), lms.calls)
        // Flipped again at once (or a stale status): not before 30 s have passed.
        k.observeStatus(phone, lmsStatus(fixed = false))
        runCurrent()
        assertEquals(1, lms.calls.size)
        testScheduler.advanceTimeBy(FixedLevelKeeper.RESEND_EVERY_MS)
        k.observeStatus(phone, lmsStatus(fixed = false))
        runCurrent()
        assertEquals(2, lms.calls.size)
        // Not connected: a status is not acted on.
        local.value = state(phone, connection = PlayerConnection.Reconnecting(2_000uL))
        runCurrent()
        testScheduler.advanceTimeBy(FixedLevelKeeper.RESEND_EVERY_MS)
        k.observeStatus(phone, lmsStatus(fixed = false))
        runCurrent()
        assertEquals(2, lms.calls.size)
    }

    /** The setting off and a controller fixing the level: the volume is given back. */
    @Test
    fun theStatusInHandFollowsTheSettingOffToo() = runTest {
        val lms = Lms(fixed = false)
        val k = make(MutableStateFlow(state(phone, fixed = false)), lms)
        k.start()
        runCurrent()
        testScheduler.advanceTimeBy(60_000)
        k.observeStatus(phone, lmsStatus(fixed = true))
        runCurrent()
        assertEquals(listOf(phone to false), lms.calls)
    }

    @Test
    fun anUnreadableStatusSendsNothing() = runTest {
        val lms = Lms(fixed = false, statusFails = true)
        val logs = mutableListOf<String>()
        keeper(MutableStateFlow(state(phone)), lms, logs)
        runCurrent()
        testScheduler.advanceTimeBy(10 * 60_000)
        runCurrent()
        assertEquals("one read and one retry", 2, lms.statusReads)
        assertTrue(lms.calls.isEmpty())
        assertEquals(2, logs.size)
    }
}
