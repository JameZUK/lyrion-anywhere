package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextClearance
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.SaveQueueOutcome
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.DefaultPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.ExistingPlaylist
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerMessage
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.QueueHeader
import io.github.jamezuk.lyrionanywhere.ui.player.SaveQueueDialog
import io.github.jamezuk.lyrionanywhere.ui.player.SaveQueueForm
import io.github.jamezuk.lyrionanywhere.ui.player.SaveQueueError
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * "Save as playlist" in the player view model: offered only to admins on a gateway that
 * has it; saved, refused, a taken name (replaced only when asked) and an empty queue.
 */
class SaveQueueViewModelTest : MainDispatcherTest() {
    private val me = "aa:00:00:00:00:02"
    private val vms = mutableListOf<PlayerViewModel>()

    private fun setup(offered: Boolean = true): Pair<PlayerViewModel, FakePlayers> {
        val players = FakePlayers()
        players.canSave = ApiResult.Ok(offered)
        val settings = FakeSettings()
        settings.state.update { it.copy(defaultPlayer = DefaultPlayer.LAST_USED, selectedPlayerId = me) }
        val vm = PlayerViewModel(players, FakeEvents(), settings) { 0L }
        vms += vm
        vm.start()
        vm.loadQueue()
        return vm to players
    }

    private fun vmTest(body: suspend TestScope.() -> Unit) = runTest(dispatcher) {
        try {
            body()
        } finally {
            vms.forEach { it.close() }
        }
    }

    @Test
    fun listenersAndOlderGatewaysAreNotOffered() = vmTest {
        val (vm, players) = setup(offered = false)
        assertFalse(vm.state.value.canSaveQueue)
        vm.openSaveQueue("14 October 2026")
        assertNull(vm.state.value.saveQueue)
        vm.saveQueue("Anything")
        assertTrue(players.saves.isEmpty())
    }

    @Test
    fun noAnswerOffersNothing() = vmTest {
        val players = FakePlayers().apply { canSave = ApiResult.Err(AppError.Offline()) }
        val settings = FakeSettings()
        settings.state.update { it.copy(defaultPlayer = DefaultPlayer.LAST_USED, selectedPlayerId = me) }
        val vm = PlayerViewModel(players, FakeEvents(), settings) { 0L }
        vms += vm
        vm.start()
        vm.loadQueue()
        assertFalse(vm.state.value.canSaveQueue)
    }

    @Test
    fun savedWithTheAlbumAsTheSuggestionThenAMessage() = vmTest {
        val (vm, players) = setup()
        assertTrue(vm.state.value.canSaveQueue)
        vm.openSaveQueue("14 October 2026")
        // The sample queue is all one album.
        assertEquals("Album", vm.state.value.saveQueue?.suggestion)
        val messages = mutableListOf<PlayerMessage>()
        val c = backgroundScope.launch { vm.messages.collect { messages += it } }
        vm.saveQueue("Road trip")
        assertEquals(listOf(Triple(me, "Road trip", false)), players.saves)
        assertNull("the dialog closes", vm.state.value.saveQueue)
        assertTrue(messages.contains(PlayerMessage.SavedPlaylist(9, "Road trip")))
        c.cancel()
    }

    @Test
    fun aTakenNameIsReplacedOnlyWhenAsked() = vmTest {
        val (vm, players) = setup()
        players.saveAnswer = { _, replace ->
            if (replace) ApiResult.Ok(SaveQueueOutcome.Saved(5, "Road Trip")) else ApiResult.Ok(SaveQueueOutcome.Exists(5, "Road Trip"))
        }
        vm.openSaveQueue("14 October 2026")
        vm.saveQueue("road trip")
        assertEquals(ExistingPlaylist(5, "Road Trip", "road trip"), vm.state.value.saveQueue?.exists)
        // Change name: back to the name, nothing saved.
        vm.saveQueueChangeName()
        assertNull(vm.state.value.saveQueue?.exists)
        vm.saveQueue("road trip")
        vm.saveQueue("road trip", replace = true)
        assertEquals(Triple(me, "road trip", true), players.saves.last())
        assertNull(vm.state.value.saveQueue)
    }

    @Test
    fun refusalsAndAnEmptyQueueStayInTheDialog() = vmTest {
        val (vm, players) = setup()
        vm.openSaveQueue("14 October 2026")
        players.saveAnswer = { _, _ -> ApiResult.Err(AppError.Forbidden("changing playlists needs the admin role")) }
        vm.saveQueue("Road trip")
        assertEquals(SaveQueueError.Failed(AppError.Forbidden("changing playlists needs the admin role")), vm.state.value.saveQueue?.error)
        players.saveAnswer = { _, _ -> ApiResult.Err(AppError.InvalidInput("names can't contain ..")) }
        vm.saveQueue("a..b")
        assertEquals(SaveQueueError.Failed(AppError.InvalidInput("names can't contain ..")), vm.state.value.saveQueue?.error)
        players.saveAnswer = { _, _ -> ApiResult.Ok(SaveQueueOutcome.Empty) }
        vm.saveQueue("Road trip")
        assertEquals(SaveQueueError.Empty, vm.state.value.saveQueue?.error)
        assertFalse(vm.state.value.saveQueue!!.busy)
        vm.closeSaveQueue()
        assertNull(vm.state.value.saveQueue)
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class SaveQueueScreenTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun theButtonIsShownOnlyWhenOffered() {
        var opened = 0
        var offered by mutableStateOf(false)
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                QueueHeader(3, "Kitchen", onClear = {}, onSaveAsPlaylist = if (offered) ({ opened++ }) else null)
            }
        }
        compose.onNodeWithTag("save-queue").assertDoesNotExist()
        offered = true
        compose.onNodeWithTag("save-queue").performClick()
        assertEquals(1, opened)
    }

    @Test
    fun aBadNameIsExplainedAndNotSaved() {
        val saved = mutableListOf<Pair<String, Boolean>>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SaveQueueForm(SaveQueueDialog("Kind of Blue"), onSave = { n, r -> saved += n to r }, onChangeName = {}, onDismiss = {})
            }
        }
        compose.onNodeWithTag("save-queue-save").assertIsEnabled()
        for (bad in listOf("../secret", "a/b", ".x", "x:y")) {
            compose.onNodeWithTag("save-queue-name").performTextClearance()
            compose.onNodeWithTag("save-queue-name").performTextInput(bad)
            compose.onNodeWithTag("save-queue-problem", useUnmergedTree = true).assertExists()
            compose.onNodeWithTag("save-queue-save").assertIsNotEnabled()
        }
        assertTrue(saved.isEmpty())
        compose.onNodeWithTag("save-queue-name").performTextClearance()
        compose.onNodeWithTag("save-queue-name").performTextInput("Sunday morning")
        compose.onNodeWithTag("save-queue-save").performClick()
        assertEquals(listOf("Sunday morning" to false), saved)
    }

    @Test
    fun aTakenNameOffersReplace() {
        val saved = mutableListOf<Pair<String, Boolean>>()
        var renamed = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SaveQueueForm(
                    SaveQueueDialog("x", exists = ExistingPlaylist(5, "Road Trip", "road trip")),
                    onSave = { n, r -> saved += n to r },
                    onChangeName = { renamed++ },
                    onDismiss = {},
                )
            }
        }
        compose.onNodeWithTag("save-queue-rename").performClick()
        assertEquals(1, renamed)
        compose.onNodeWithTag("save-queue-replace").performClick()
        assertEquals(listOf("road trip" to true), saved)
    }

    @Test
    fun anEmptyQueueIsSaid() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SaveQueueForm(SaveQueueDialog("x", error = SaveQueueError.Empty), onSave = { _, _ -> }, onChangeName = {}, onDismiss = {})
            }
        }
        compose.onNodeWithTag("save-queue-error", useUnmergedTree = true).assertExists()
    }
}
