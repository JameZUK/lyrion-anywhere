package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.MenuItemKind
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.Suitability
import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.core.VirtualLibrary
import io.github.jamezuk.lyrionanywhere.data.DefaultPlayer
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumActions
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumDetail
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumScreen
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryDestination
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryScreen
import io.github.jamezuk.lyrionanywhere.ui.library.Loadable
import io.github.jamezuk.lyrionanywhere.ui.menus.AppTile
import io.github.jamezuk.lyrionanywhere.ui.menus.AppsScreen
import io.github.jamezuk.lyrionanywhere.ui.menus.AppsUiState
import io.github.jamezuk.lyrionanywhere.ui.menus.ItemSheetContent
import io.github.jamezuk.lyrionanywhere.ui.menus.MenuNavigator
import io.github.jamezuk.lyrionanywhere.ui.menus.MoreRequest
import io.github.jamezuk.lyrionanywhere.ui.menus.SheetTarget
import io.github.jamezuk.lyrionanywhere.ui.player.DefaultPlayerSetting
import io.github.jamezuk.lyrionanywhere.ui.player.MiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnActions
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnList
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import androidx.compose.ui.test.onNodeWithTag
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val PHONE = "02:4c:41:00:00:01"
private const val KITCHEN = "aa:00:00:00:00:01"

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class BrowseScreenTests {
    @get:Rule
    val compose = createComposeRule()

    private fun state(selected: String?) = PlayerUiState(
        players = listOf(samplePlayer(PHONE), samplePlayer(KITCHEN, playing = true)),
        selectedId = selected,
        status = sampleStatus(),
        loading = false,
        local = LocalPlayerState(enabled = true, status = localStatus(PHONE)),
        playingOn = mapOf(KITCHEN to sampleTrack(9)),
        syncGroups = listOf(SyncGroup(listOf(KITCHEN, PHONE))),
    )

    /**
     * The "Playing on" strip is gone (Wave 3.5): the mini player says which player, and going
     * back to this phone is the This phone row of *Play on…*.
     */
    @Test
    fun theMiniPlayerSaysWhichPlayerAndPlayOnGoesBackToThePhone() {
        var back = 0
        var outputs = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                androidx.compose.foundation.layout.Column {
                    MiniPlayer(state(KITCHEN), onOpen = {}, onTogglePlay = {}, onNext = {}, onOutputs = { outputs++ })
                    PlayOnList(state(KITCHEN), PlayOnActions(onControl = {}, onBackToPhone = { back++ }))
                }
            }
        }
        compose.onNodeWithText("on Player $KITCHEN", substring = true).assertIsDisplayed()
        compose.onNode(hasText("This phone") and hasClickAction()).performClick()
        assertEquals(1, back)
        assertEquals(0, outputs)
    }

    @Test
    fun noPlayerNameWhileThisPhoneIsThePlayer() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) { MiniPlayer(state(PHONE), onOpen = {}, onTogglePlay = {}, onNext = {}) }
        }
        compose.onNodeWithTag("mini-player-on", useUnmergedTree = true).assertDoesNotExist()
        compose.onNodeWithText("Back to this phone").assertDoesNotExist()
    }

    @Test
    fun theMiniPlayerHasTheOutputButtonAndThePlayerName() {
        var outputs = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                MiniPlayer(state(KITCHEN), onOpen = {}, onTogglePlay = {}, onNext = {}, onOutputs = { outputs++ })
            }
        }
        compose.onNodeWithText("on Player $KITCHEN", substring = true).assertIsDisplayed()
        compose.onNodeWithContentDescription("Playing on Player $KITCHEN. Choose where to play").performClick()
        assertEquals(1, outputs)
    }

    @Test
    fun thePlayOnSheetListsThePhonePlayersAndGroups() {
        val controlled = mutableListOf<String>()
        var back = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                PlayOnList(state(KITCHEN), PlayOnActions(onControl = { controlled += it }, onBackToPhone = { back++ }))
            }
        }
        compose.onNodeWithText("Play on…").assertIsDisplayed()
        compose.onNodeWithText("This phone").assertIsDisplayed()
        compose.onNodeWithText("Ready as “Test phone”").assertIsDisplayed()
        // The other player, what it plays, and that it is the one controlled.
        compose.onNode(hasText("Player $KITCHEN") and hasClickAction()).assertIsSelected()
        // A grouped player says who it plays in step with (Wave 3.5: no separate list of groups).
        compose.onNodeWithText("Track 9 · Artist", substring = true).assertIsDisplayed()
        compose.onNodeWithText("with Player $PHONE", substring = true).assertIsDisplayed()
        compose.onNodeWithText("Synchronised groups").assertDoesNotExist()
        compose.onNode(hasText("Player $KITCHEN") and hasClickAction()).performClick()
        compose.onNodeWithText("This phone").performClick()
        assertEquals(listOf(KITCHEN), controlled)
        assertEquals(1, back)
    }

    @Test
    fun theDefaultPlayerSettingOffersThePhoneLastUsedAndPlayers() {
        val chosen = mutableListOf<Pair<DefaultPlayer, String?>>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                DefaultPlayerSetting(state(PHONE), onChoose = { d, id -> chosen += d to id })
            }
        }
        // A row with a picker (Wave 3.5).
        compose.onNodeWithTag("default-player-setting").assertTextContains("This phone").performClick()
        compose.onNode(hasText("This phone") and androidx.compose.ui.test.isSelectable()).assertIsSelected()
        compose.onNodeWithText("Last used").performClick()
        compose.onNodeWithTag("default-player-setting").performClick()
        compose.onNodeWithText("Player $KITCHEN").performClick()
        assertEquals(listOf(DefaultPlayer.LAST_USED to null, DefaultPlayer.SPECIFIC to KITCHEN), chosen)
    }

    @Test
    fun theLibraryPickerShowsOnlyWithLibraries() {
        val opened = mutableListOf<LibraryDestination>()
        var libraries by mutableStateOf(emptyList<VirtualLibrary>())
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                LibraryScreen(libraries, null, {}, onOpen = { opened += it })
            }
        }
        compose.onNodeWithText("Whole library").assertDoesNotExist()
        // One model (Wave 3.5): the sections, then Browse; New music is a sort of Albums.
        compose.onNodeWithText("New Music").assertDoesNotExist()
        compose.onNodeWithText("Albums").assertIsDisplayed()
        compose.onNodeWithText("Apps & Radio").performScrollTo().performClick()
        assertEquals(listOf(LibraryDestination.APPS), opened)
        libraries = listOf(VirtualLibrary("ab12", "Classical"))
        compose.onNodeWithText("Whole library").performScrollTo().assertIsDisplayed()
    }

    @Test
    fun theAlbumPageShowsDetailsDiscsAndActions() {
        val played = mutableListOf<Placement>()
        var shuffled = 0
        val more = mutableListOf<MoreRequest>()
        val detail = AlbumDetail(
            sampleAlbum(7),
            listOf(sampleTrack(1, disc = 1), sampleTrack(2, disc = 2)),
        )
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                AlbumScreen(
                    Loadable.Ready(detail),
                    AlbumActions(
                        onRetry = {},
                        onPlay = { p, _ -> played += p },
                        onShuffle = { shuffled++ },
                        onPlayTrack = {},
                        onArtist = { _, _ -> },
                        onMore = { more += it },
                    ),
                )
            }
        }
        compose.onNodeWithText("Album 7").assertIsDisplayed()
        compose.onNodeWithText("1999 · Jazz · 2 tracks · 2:02").assertIsDisplayed()
        compose.onNodeWithText("FLAC 24/96").assertIsDisplayed()
        compose.onNodeWithText("Disc 1").performScrollTo().assertIsDisplayed()
        compose.onNodeWithText("Disc 2").performScrollTo().assertIsDisplayed()
        compose.onNodeWithText("Play").performClick()
        compose.onNodeWithText("Shuffle").performClick()
        // Add to queue is in the album's sheet, behind ⋮ (Wave 3.5; theMoreSheetOffersPlayActionsThenLmsItems).
        compose.onNodeWithContentDescription("Add to queue").assertDoesNotExist()
        compose.onNodeWithContentDescription("More options").performClick()
        compose.onNodeWithContentDescription("More options for Track 2").performScrollTo().performClick()
        assertEquals(listOf(Placement.PLAY_NOW), played)
        assertEquals(1, shuffled)
        assertEquals(listOf(7L, 2L), more.map { it.id })
    }

    @Test
    fun theMoreSheetOffersPlayActionsThenLmsItems() {
        val menus = FakeMenus().apply {
            pages["trackinfo"] = listOf(
                menuItem("More like this", MenuItemKind.ACTION, go = action("someplugin", "mix", opens = false)),
                menuItem("Duration: 1:01", MenuItemKind.TEXT),
            )
        }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val navigator = MenuNavigator(scope, menus) { KITCHEN }
        navigator.start("Track 1", action("trackinfo", "items"))
        val played = mutableListOf<Placement>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                ItemSheetContent(SheetTarget("Track 1", "Artist"), navigator, onPlay = { played += it }, onClose = {})
            }
        }
        compose.waitForIdle()
        compose.onNodeWithText("Play now").performClick()
        compose.onNodeWithText("Play next").performClick()
        compose.onNodeWithText("Add").performClick()
        assertEquals(listOf(Placement.PLAY_NOW, Placement.PLAY_NEXT, Placement.ADD_TO_QUEUE), played)
        compose.onNodeWithText("Duration: 1:01").assertIsDisplayed()
        compose.onNodeWithText("More like this").performClick()
        compose.waitForIdle()
        assertEquals(listOf(listOf("someplugin", "mix")), menus.ran)
    }

    @Test
    fun appsShowSetupNotesAndHideUnusableOnes() {
        var toggled = 0
        val ui = AppsUiState(
            radios = listOf(AppTile("app:someradio", "Some Radio", null, Suitability.Failed("x"))),
            apps = listOf(AppTile("app:someapp", "Some App", null, Suitability.NeedsSetup("Log in"))),
            loading = false,
        )
        compose.setContent {
            LyrionTheme(dynamicColour = false) { AppsScreen(ui, onOpen = {}, onRetry = {}, onToggleShowAll = { toggled++ }) }
        }
        compose.onNodeWithText("Some App").assertIsDisplayed()
        compose.onNodeWithText("Needs setting up in your music server").assertIsDisplayed()
        compose.onNodeWithText("Some Radio").assertDoesNotExist()
        compose.onNodeWithText("1 app is hidden: it shows nothing here, or the music server couldn't open it.").assertIsDisplayed()
        compose.onNodeWithText("Show all").performClick()
        assertEquals(1, toggled)
    }

    /** Every icon-only button on these screens has a label and a 48 dp target. */
    @Test
    fun iconButtonsAreLabelledAndLargeEnough() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                MiniPlayer(state(KITCHEN), onOpen = {}, onTogglePlay = {}, onNext = {})
            }
        }
        val buttons = compose.onAllNodesWithContentDescription("", substring = true)
            .fetchSemanticsNodes()
            .filter { it.config.contains(SemanticsActions.OnClick) }
        assertTrue(buttons.isNotEmpty())
        for (b in buttons) {
            // The touch target, which Material widens to 48 dp around smaller visuals.
            val touch = b.touchBoundsInRoot
            val density = b.layoutInfo.density.density
            val w = touch.width / density
            val h = touch.height / density
            assertTrue("${b.config} is $w x $h dp", w >= 47.5f && h >= 47.5f)
        }
        compose.onNode(hasContentDescription("Next track")).assertIsDisplayed()
        compose.onNode(SemanticsMatcher.keyIsDefined(SemanticsActions.OnClick) and hasContentDescription("Pause")).assertIsDisplayed()
    }
}
