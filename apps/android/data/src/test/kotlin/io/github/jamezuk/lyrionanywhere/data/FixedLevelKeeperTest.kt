package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
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

    private fun state(id: String?, fixed: Boolean = true) = LocalPlayerState(
        enabled = true,
        status = id?.let { status(it) },
        sound = SoundSettings(fixedOutputLevel = fixed),
    )

    private fun TestScope.keeper(local: MutableStateFlow<LocalPlayerState>, lms: Lms, logs: MutableList<String> = mutableListOf()) =
        FixedLevelKeeper(CoroutineScope(backgroundScope.coroutineContext + UnconfinedTestDispatcher(testScheduler)), local, lms) { logs += it }
            .start()

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

    @Test
    fun aFailureIsLoggedAndNotRepeatedUntilTheNextTrigger() = runTest {
        val lms = Lms(fixed = false, fail = true)
        val logs = mutableListOf<String>()
        val local = MutableStateFlow(state(phone))
        keeper(local, lms, logs)
        runCurrent()
        testScheduler.advanceTimeBy(10 * 60_000)
        runCurrent()
        assertEquals("one attempt", 1, lms.calls.size)
        assertEquals(1, logs.size)
        // Unrelated state changes are not a trigger.
        local.value = state(phone).copy(eqBypass = true)
        runCurrent()
        assertEquals(1, lms.calls.size)
        // The player reappears (a reconnect): one more attempt.
        local.value = state(null)
        runCurrent()
        lms.fail = false
        local.value = state(phone)
        runCurrent()
        assertEquals(listOf(phone to true, phone to true), lms.calls)
    }

    @Test
    fun anUnreadableStatusSendsNothing() = runTest {
        val lms = Lms(fixed = false, statusFails = true)
        val logs = mutableListOf<String>()
        keeper(MutableStateFlow(state(phone)), lms, logs)
        runCurrent()
        assertEquals(1, lms.statusReads)
        assertTrue(lms.calls.isEmpty())
        assertEquals(1, logs.size)
    }
}
