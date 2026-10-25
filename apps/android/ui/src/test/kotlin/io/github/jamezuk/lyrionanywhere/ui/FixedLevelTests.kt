package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.SoundSettings
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnActions
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnList
import io.github.jamezuk.lyrionanywhere.ui.player.Room
import io.github.jamezuk.lyrionanywhere.ui.player.RoomsActions
import io.github.jamezuk.lyrionanywhere.ui.player.RoomsScreen
import io.github.jamezuk.lyrionanywhere.ui.player.RoomsUiState
import io.github.jamezuk.lyrionanywhere.ui.player.RoomsViewModel
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundActions
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundUiState
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundViewModel
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val PHONE_ID = "02:4c:41:00:00:01"
private const val KITCHEN_ID = "aa:00:00:00:00:01"
private const val LOUNGE_ID = "aa:00:00:00:00:02"

/** "Fix output level at 100%" on the Sound screen, and fixed-level players' volume controls. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h3000dp")
class FixedLevelScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private fun sound(state: SoundUiState, actions: SoundActions = object : SoundActions {}) =
        compose.setContent { LyrionTheme(dynamicColour = false) { SoundScreen(state, actions) } }

    @Test
    fun theSwitchIsOnByDefaultAndTheLevelIsFixed() {
        val switched = mutableListOf<Boolean>()
        val levels = mutableListOf<Int>()
        sound(
            SoundUiState(playerLevel = 40),
            object : SoundActions {
                override fun fixedOutputLevel(on: Boolean) {
                    switched += on
                }

                override fun playerLevel(level: Int) {
                    levels += level
                }
            },
        )
        compose.onNodeWithTag("fixed-output-level").assertIsOn()
        compose.onNodeWithText("Fix output level at 100%").assertExists()
        compose.onNode(hasText("no controller can turn it down", substring = true)).assertExists()
        // The level shown is the one LMS plays at (100 %), not the stored volume.
        compose.onNodeWithText("Player level: 100%").assertExists()
        compose.onNodeWithText("Fixed at 100% in Lyrion while Fix output level at 100% is on.").assertExists()
        compose.onNodeWithTag("player-level").assertIsNotEnabled()
        assertTrue(levels.isEmpty())
        compose.onNodeWithTag("fixed-output-level").performClick()
        assertEquals(listOf(false), switched)
    }

    @Test
    fun offTheSliderIsBack() {
        sound(SoundUiState(playerLevel = 40, sound = SoundSettings(fixedOutputLevel = false)))
        compose.onNodeWithTag("fixed-output-level").assertIsOff()
        compose.onNodeWithText("Player level: 40%").assertExists()
        compose.onNode(hasText("normally left at 100%", substring = true)).assertExists()
    }

    @Test
    fun theSwitchIsOfferedBeforeThereIsAPlayer() {
        sound(SoundUiState())
        compose.onNodeWithTag("fixed-output-level").assertIsOn()
        compose.onNodeWithText("Turn on Play on this phone to set its level.").assertExists()
    }

    @Test
    fun aFixedRemotePlayerShowsTheNoteInNowPlaying() {
        val volumes = mutableListOf<Int>()
        val state = busyState().let { it.copy(status = it.status?.copy(fixedLevel = true)) }
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                NowPlayingScreen(state, clock = { 0L }, actions = nowPlayingActions().copy(onVolume = { volumes += it }), phoneVolume = FakePhoneVolume())
            }
        }
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("volume-button").performClick()
        compose.onNodeWithTag("volume-popover").assertExists()
        compose.onNodeWithTag("volume-note").assertTextEquals("Output level is fixed at 100%")
        compose.onNodeWithTag("volume-slider").assertDoesNotExist()
        assertTrue(volumes.isEmpty())
    }

    private val calls = mutableListOf<String>()

    private fun roomActions() = RoomsActions(
        onControl = { calls += "control $it" },
        onTogglePlay = { calls += "play $it" },
        onVolume = { id, v -> calls += "volume $id $v" },
        onPower = { id, on -> calls += "power $id $on" },
        onSync = { a, b -> calls += "sync $a $b" },
        onUnsync = { calls += "unsync $it" },
        onAlarms = { id, _ -> calls += "alarms $id" },
    )

    private fun rooms() = listOf(
        Room(samplePlayer(KITCHEN_ID, playing = true), sampleStatus().copy(fixedLevel = true)),
        Room(samplePlayer(LOUNGE_ID), sampleStatus(mode = PlaybackMode.PAUSED)),
    )

    @Test
    fun aFixedPlayerInPlayOnShowsTheNote() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                PlayOnList(
                    auditPlayerState(LOUNGE_ID),
                    PlayOnActions(onControl = {}, onBackToPhone = {}),
                    rooms = rooms(),
                    roomActions = roomActions(),
                )
            }
        }
        val note = compose.onNodeWithTag("play-on-volume-fixed-$KITCHEN_ID", useUnmergedTree = true)
        note.assertIsDisplayed().assertTextEquals("Output level is fixed at 100%")
        compose.onNodeWithTag("play-on-volume-$KITCHEN_ID").assertDoesNotExist()
        // A player with its volume control keeps its slider.
        compose.onNodeWithTag("play-on-volume-$LOUNGE_ID").assertIsDisplayed()
    }

    @Test
    fun aFixedRoomsCardShowsTheNote() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                RoomsScreen(RoomsUiState(rooms = rooms()), roomActions(), selectedId = LOUNGE_ID)
            }
        }
        compose.onNodeWithTag("room-volume-fixed-$KITCHEN_ID").assertTextEquals("Output level is fixed at 100%")
        compose.onNodeWithTag("room-volume-$KITCHEN_ID").assertDoesNotExist()
        compose.onNodeWithTag("room-volume-$LOUNGE_ID").assertExists()
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class FixedLevelViewModelTest : MainDispatcherTest() {
    @Test
    fun theSettingIsOnByDefaultAndSaved() = runTest(dispatcher) {
        val local = FakeLocalPlayer()
        val players = FakePlayers()
        val vm = SoundViewModel(local, sdk = 34, players = players)
        assertTrue(vm.state.value.sound.fixedOutputLevel)
        vm.setFixedOutputLevel(false)
        assertFalse(local.state.value.sound.fixedOutputLevel)
        assertFalse(vm.state.value.sound.fixedOutputLevel)
        vm.setFixedOutputLevel(true)
        assertTrue(local.state.value.sound.fixedOutputLevel)
        // LMS is put in step by the long-lived keeper (data), not this screen.
        assertTrue(players.commands.isEmpty())
    }

    @Test
    fun thePlayerLevelIsNotSetWhileFixed() = runTest(dispatcher) {
        val local = FakeLocalPlayer()
        val players = FakePlayers()
        val vm = SoundViewModel(local, sdk = 34, players = players)
        local.state.update { it.copy(enabled = true, status = localStatus(PHONE_ID)) }
        vm.setPlayerLevel(50)
        assertTrue(players.commands.isEmpty())
        vm.setFixedOutputLevel(false)
        vm.setPlayerLevel(50)
        assertEquals(listOf("volume 50 $PHONE_ID"), players.commands)
    }

    @Test
    fun aFixedRoomIgnoresTheVolume() = runTest(dispatcher) {
        val players = FakePlayers()
        players.roomStatus[KITCHEN_ID] = sampleStatus().copy(fixedLevel = true)
        players.roomStatus[LOUNGE_ID] = sampleStatus()
        val vm = RoomsViewModel(players)
        try {
            vm.start()
            runCurrent()
            vm.setVolume(KITCHEN_ID, 20)
            runCurrent()
            assertFalse(players.commands.any { it.startsWith("volume") })
            vm.setVolume(LOUNGE_ID, 20)
            runCurrent()
            assertEquals(listOf("volume 20 $LOUNGE_ID"), players.commands.filter { it.startsWith("volume") })
        } finally {
            vm.stop()
        }
    }
}
