package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.AccountInfo
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.data.LibraryAlbumOrder
import io.github.jamezuk.lyrionanywhere.data.SoundSettings
import io.github.jamezuk.lyrionanywhere.data.offline.ConnectionUi
import io.github.jamezuk.lyrionanywhere.ui.components.Choice
import io.github.jamezuk.lyrionanywhere.ui.components.ChoiceRow
import io.github.jamezuk.lyrionanywhere.ui.components.ConnectionFrame
import io.github.jamezuk.lyrionanywhere.ui.components.ConnectionMessage
import io.github.jamezuk.lyrionanywhere.ui.components.ConnectionTags
import io.github.jamezuk.lyrionanywhere.ui.components.StatusSlot
import io.github.jamezuk.lyrionanywhere.ui.components.connectionMessage
import io.github.jamezuk.lyrionanywhere.ui.components.statusSlot
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryAlbumsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryDestination
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryScreen
import io.github.jamezuk.lyrionanywhere.ui.player.MiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnActions
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnList
import io.github.jamezuk.lyrionanywhere.ui.player.TransferState
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsSection
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsSummaries
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsUiState
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val KITCHEN = "aa:00:00:00:00:01"
private const val LOUNGE = "aa:00:00:00:00:02"

/** Wave 3.5 phase 2: Settings as an index, one picker row, one status slot, one Library, Play on. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w411dp-h1800dp")
class Wave35Phase2Tests {
    @get:Rule
    val compose = createComposeRule()

    private fun show(content: @Composable () -> Unit) = compose.setContent { LyrionTheme(dynamicColour = false) { content() } }

    private val full = SettingsUiState(
        serverName = "music.example.com",
        username = "alex",
        account = AccountInfo("alex", false, true, false, true),
        sound = SoundSettings(),
        downloadsOffered = true,
        downloadedBytes = 1_271_000_000,
        downloadedTracks = 34,
        deviceName = "Phone",
    )

    /** Settings with every slot filled, each slot a tagged marker. */
    @Composable
    private fun settings(section: SettingsSection?, onSection: (SettingsSection) -> Unit = {}) = SettingsScreen(
        state = full,
        appVersion = "1.13.0",
        libraryVersion = "1.13.0",
        onSignOut = {},
        onLicences = {},
        section = section,
        onSection = onSection,
        summaries = SettingsSummaries(playback = "Starts on This phone", nowPlaying = "Visualiser: Off", history = "On"),
        defaultPlayer = { Text("default player") },
        history = { Text("history settings") },
        sleepFade = { Text("sleep fade") },
        lyrics = { Text("lyric line") },
        streaming = { Text("streaming quality") },
        ratings = { Text("ratings") },
        visualiser = { Text("visualiser") },
        device = { Text("device settings") },
        gaps = { Text("read ahead") },
    )

    // Item 8: Settings is an index.

    @Test
    fun theTopLevelHasTheAgreedRowsEachWithASummary() {
        val opened = mutableListOf<SettingsSection>()
        show { settings(null) { opened += it } }
        val rows = listOf(
            SettingsSection.ACCOUNT to "alex · music.example.com",
            SettingsSection.PLAYBACK to "Starts on This phone",
            SettingsSection.NOW_PLAYING to "Visualiser: Off",
            SettingsSection.DOWNLOADS to "34 tracks",
            SettingsSection.HISTORY to "On",
            SettingsSection.DEVICE to "Phone",
            SettingsSection.ABOUT to "Version 1.13.0",
        )
        rows.forEach { (s, summary) -> compose.onNodeWithTag(s.tag).assertTextContains(summary, substring = true) }
        // Seven rows, nothing else at the top level: no option is laid out inline.
        val index = compose.onNodeWithTag("settings-index").fetchSemanticsNode()
        assertEquals(7, index.children.size)
        compose.onAllNodes(SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.RadioButton)).assertCountEquals(0)
        listOf("default player", "streaming quality", "read ahead").forEach { compose.onNodeWithText(it).assertDoesNotExist() }
        rows.forEach { (s, _) -> compose.onNodeWithTag(s.tag).performClick() }
        assertEquals(rows.map { it.first }, opened)
    }

    @Test
    fun everySettingIsOnItsPage() {
        val pages = mapOf(
            SettingsSection.PLAYBACK to listOf("default player", "streaming quality", "sleep fade"),
            SettingsSection.NOW_PLAYING to listOf("lyric line", "visualiser", "ratings"),
            SettingsSection.HISTORY to listOf("history settings"),
            SettingsSection.DEVICE to listOf("device settings"),
            SettingsSection.ADVANCED to listOf("read ahead"),
        )
        var section by mutableStateOf(SettingsSection.ACCOUNT)
        show { settings(section) }
        compose.onNodeWithText("Two-step verification").assertIsDisplayed()
        compose.onNodeWithText("Sign out").assertIsDisplayed()
        for ((s, texts) in pages) {
            section = s
            compose.waitForIdle()
            texts.forEach { compose.onNodeWithText(it).assertIsDisplayed() }
        }
        section = SettingsSection.PLAYBACK
        compose.onNodeWithTag("settings-sound").assertIsDisplayed()
        section = SettingsSection.DOWNLOADS
        compose.onNodeWithTag("settings-downloads-manager").assertIsDisplayed()
        compose.onNodeWithTag("settings-download-options").assertTextContains("Download options")
        section = SettingsSection.ABOUT
        compose.onNodeWithTag(SettingsSection.ADVANCED.tag).assertIsDisplayed()
        compose.onNodeWithTag("privacy-policy").assertIsDisplayed()
    }

    @Test
    fun aChoiceRowShowsTheValueAndPicksAnother() {
        val chosen = mutableListOf<Int>()
        var value by mutableStateOf(1)
        show {
            ChoiceRow(
                title = "Speed",
                choices = listOf(Choice(1, "Slow", tag = "slow"), Choice(2, "Fast", "Quicker", tag = "fast")),
                selected = value,
                onSelect = {
                    chosen += it
                    value = it
                },
                tag = "speed",
            )
        }
        compose.onNodeWithTag("speed").assertTextContains("Slow")
        compose.onNodeWithTag("slow").assertDoesNotExist()
        compose.onNodeWithTag("speed").performClick()
        compose.onNodeWithTag("slow").assertIsSelected()
        compose.onNodeWithText("Quicker").assertIsDisplayed()
        compose.onNodeWithTag("fast").performClick()
        // The picker closes as one is chosen.
        compose.onNodeWithTag("fast").assertDoesNotExist()
        compose.onNodeWithTag("speed").assertTextContains("Fast")
        // Cancel leaves it as it was.
        compose.onNodeWithTag("speed").performClick()
        compose.onNodeWithText("Cancel").performClick()
        assertEquals(listOf(2), chosen)
    }

    // Item 9: one status slot.

    @Test
    fun theSlotShowsOneThingInPriorityOrder() {
        val all = ConnectionUi(unreachable = true, busy = true, reconnecting = true)
        assertEquals(StatusSlot.Moving("Moving"), statusSlot("Moving", connectionMessage(all, false)))
        assertEquals(StatusSlot.Link(ConnectionMessage.UNREACHABLE), statusSlot(null, connectionMessage(all, false)))
        assertEquals(StatusSlot.Link(ConnectionMessage.BUSY), statusSlot(null, connectionMessage(ConnectionUi(busy = true, reconnecting = true), false)))
        assertEquals(StatusSlot.Link(ConnectionMessage.RECONNECTING), statusSlot(null, connectionMessage(ConnectionUi(reconnecting = true), false)))
        assertNull(statusSlot(null, null))
    }

    @Test
    fun onlyOneStatusLineShowsAtATime() {
        var moving by mutableStateOf<String?>("Moving playback to Study…")
        show {
            ConnectionFrame(ConnectionMessage.UNREACHABLE, onDownloads = {}, moving = moving) {
                MiniPlayer(auditPlayerState(KITCHEN).copy(transfer = TransferState.Running(LOUNGE, "Study", io.github.jamezuk.lyrionanywhere.core.TransferStage.GROUPING)), {}, {}, {})
            }
        }
        val lines = listOf(ConnectionTags.MOVING, ConnectionTags.BANNER, ConnectionTags.BUSY, ConnectionTags.PILL, "transfer-status")
        fun shown() = lines.filter { compose.onAllNodesWithTag(it).fetchSemanticsNodes().isNotEmpty() }
        // Moving outranks out of reach, and the mini player no longer has a band of its own.
        assertEquals(listOf(ConnectionTags.MOVING), shown())
        moving = null
        compose.waitForIdle()
        assertEquals(listOf(ConnectionTags.BANNER), shown())
    }

    @Test
    fun nowPlayingHasNoTransferBandOfItsOwn() {
        show {
            NowPlayingScreen(
                busyState().copy(transfer = TransferState.Running(LOUNGE, "Study", io.github.jamezuk.lyrionanywhere.core.TransferStage.COPYING)),
                clock = { 0L },
                actions = nowPlayingActions(),
            )
        }
        compose.onNodeWithTag("transfer-status").assertDoesNotExist()
    }

    // Item 10: one Library.

    @Test
    fun theLibraryIsOneListOfSections() {
        val opened = mutableListOf<LibraryDestination>()
        show { LibraryScreen(emptyList(), null, {}, onOpen = { opened += it }) }
        val want = listOf("Albums", "Artists", "Playlists", "Favourites", "Downloaded", "Smart playlists", "Genres", "Years", "Compilations", "Folders", "Apps & Radio")
        want.forEach { compose.onNode(hasText(it) and hasClickAction()).performClick() }
        assertEquals(
            listOf(
                LibraryDestination.ALBUMS, LibraryDestination.ARTISTS, LibraryDestination.PLAYLISTS, LibraryDestination.FAVOURITES,
                LibraryDestination.DOWNLOADS, LibraryDestination.FILTERS, LibraryDestination.GENRES, LibraryDestination.YEARS,
                LibraryDestination.COMPILATIONS, LibraryDestination.FOLDER, LibraryDestination.APPS,
            ),
            opened,
        )
        // No chips, no second list, no New music card (it is a sort of Albums).
        compose.onNodeWithTag("library-chips").assertDoesNotExist()
        compose.onNodeWithText("New Music").assertDoesNotExist()
        compose.onNodeWithText("Random Albums").assertDoesNotExist()
    }

    // Item 12: Play on…

    @Test
    fun playOnRowsSwitchMoveIsSecondaryAndGroupingIsOneRow() {
        val controlled = mutableListOf<String>()
        val moved = mutableListOf<String>()
        val synced = mutableListOf<Pair<String, Boolean>>()
        show {
            PlayOnList(
                auditPlayerState(KITCHEN).copy(syncGroups = listOf(SyncGroup(listOf(KITCHEN, LOUNGE)))),
                PlayOnActions(onControl = { controlled += it }, onBackToPhone = {}, onMoveHere = { moved += it }, onSync = { id, on -> synced += id to on }),
            )
        }
        // A tap on a row controls that player.
        compose.onNode(hasText("Player $LOUNGE") and hasClickAction()).performClick()
        assertEquals(listOf(LOUNGE), controlled)
        // Moving playback is in the row's ⋮, named for where it goes.
        compose.onNodeWithTag("play-on-$LOUNGE-more").performClick()
        compose.onNodeWithText("Move playback to Player $LOUNGE").performClick()
        assertEquals(listOf(LOUNGE), moved)
        // Grouping: one row, a dialog of players.
        compose.onNodeWithTag("group-players").assertTextContains("Player $KITCHEN + Player $LOUNGE", substring = true).performClick()
        compose.onNodeWithTag("sync-$LOUNGE").performClick()
        assertEquals(listOf(LOUNGE to false), synced)
        compose.onNodeWithText("Synchronised groups").assertDoesNotExist()
        compose.onNodeWithText("Moving playback replaces the other player's queue.").assertDoesNotExist()
    }
}

/** The random order of Albums (a sort, Wave 3.5): one batch, and Shuffle again reads another. */
@OptIn(ExperimentalCoroutinesApi::class)
class Wave35RandomAlbumsTest : MainDispatcherTest() {
    @Test
    fun randomIsOneBatchAndASortLeavesIt() = runTest(dispatcher) {
        val library = FakeLibrary(totalAlbums = 1000)
        val vm = LibraryAlbumsViewModel(library, LibraryAlbumOrder.TITLE, random = true)
        assertEquals(LibraryAlbumsViewModel.RANDOM_BATCH, vm.state.value.total)
        assertEquals(listOf(Triple(0, LibraryAlbumsViewModel.RANDOM_BATCH, AlbumSort.RANDOM)), library.albumCalls)
        // Nothing past the batch is read.
        vm.ensure(500)
        assertEquals(1, library.albumCalls.size)
        // Shuffle again: another batch.
        vm.shuffle()
        assertEquals(2, library.albumCalls.size)
        assertTrue(vm.random.value)
        // Choosing an order leaves random.
        vm.setOrder(LibraryAlbumOrder.YEAR)
        assertEquals(false, vm.random.value)
        assertEquals(AlbumSort.YEAR, library.albumCalls.last().third)
    }
}
