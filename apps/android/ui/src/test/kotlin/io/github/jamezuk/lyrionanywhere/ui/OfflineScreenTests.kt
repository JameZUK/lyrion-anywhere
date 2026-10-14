package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.DownloadFailure
import io.github.jamezuk.lyrionanywhere.core.DownloadState
import io.github.jamezuk.lyrionanywhere.core.LocalRepeat
import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.data.offline.DownloadSettings
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerState
import io.github.jamezuk.lyrionanywhere.data.offline.OfflineState
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadMark
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsActions
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsUiState
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineAlbumScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineAlbumUi
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineBannerBar
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineLibraryScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineMiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineNowPlayingActions
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineNowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflinePlayerUi
import io.github.jamezuk.lyrionanywhere.ui.offline.StorageSettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.library.PlayButtons
import io.github.jamezuk.lyrionanywhere.ui.settings.SignOutDialog
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class OfflineScreenTests {
    @get:Rule
    val compose = createComposeRule()

    private fun show(content: @androidx.compose.runtime.Composable () -> Unit) {
        compose.setContent { LyrionTheme(dynamicColour = false) { content() } }
    }

    @Test
    fun downloadsShowsProgressAndItsControlsWork() {
        val calls = mutableListOf<String>()
        val state = DownloadsUiState(
            inProgress = listOf(
                dlItem(4, DownloadState.ACTIVE, 1_048_576, 4_194_304),
                dlItem(5, DownloadState.FAILED, failure = DownloadFailure.LIMIT_REACHED),
            ),
            albums = listOf(dlAlbum()),
            trackCount = 2,
            usedBytes = 3 * 1_048_576,
            capBytes = DownloadSettings.GB,
            canDownload = true,
        )
        show {
            DownloadsScreen(
                state,
                DownloadsActions(
                    onPause = { calls += "pause $it" },
                    onResume = { calls += "resume $it" },
                    onCancel = { calls += "cancel $it" },
                    onAlbum = { calls += "album $it" },
                    onPlayAlbum = { calls += "play $it" },
                    onRemoveAlbum = { calls += "remove $it" },
                    onRemoveAll = { calls += "removeAll" },
                ),
            )
        }
        compose.onNodeWithText("3.0 MB used of 1.00 GB").assertIsDisplayed()
        compose.onNodeWithText("1.0 MB of 4.0 MB").assertIsDisplayed()
        compose.onNodeWithText("The storage limit for downloads is reached.").assertIsDisplayed()
        // Each row's controls are in its ⋮ (Wave 3.5 rule 3).
        compose.onNodeWithContentDescription("More options for Song 4").performClick()
        compose.onNodeWithText("Pause Song 4").performClick()
        compose.onNodeWithContentDescription("More options for Song 5").performClick()
        compose.onNodeWithText("Retry Song 5").performClick()
        compose.onNodeWithContentDescription("More options for Song 4").performClick()
        compose.onNodeWithText("Cancel Song 4").performClick()
        compose.onNodeWithText("Pause all").performClick()
        compose.onNodeWithContentDescription("More options for Record").performClick()
        compose.onNodeWithText("Play Record").performClick()
        compose.onNodeWithText("Record").performClick()
        // Removing asks first.
        compose.onNodeWithContentDescription("More options for Record").performClick()
        compose.onNodeWithText("Remove the download of Record").performClick()
        compose.onNodeWithText("Remove download?").assertIsDisplayed()
        compose.onNode(hasText("Remove") and hasClickAction()).performClick()
        compose.onNodeWithTag("downloads").performScrollToNode(hasText("Remove all downloads"))
        compose.onNodeWithText("Remove all downloads").performClick()
        compose.onNodeWithText("Remove all downloads?").assertIsDisplayed()
        compose.onNode(hasText("Remove") and hasClickAction()).performClick()
        assertEquals(
            listOf("pause 4", "resume 5", "cancel 4", "pause null", "play a7", "album a7", "remove a7", "removeAll"),
            calls,
        )
    }

    @Test
    fun aRefusedAccountAndWaitingForWifiAreSaid() {
        var resumed = false
        show {
            DownloadsScreen(
                DownloadsUiState(
                    inProgress = listOf(dlItem(1, DownloadState.QUEUED)),
                    forbidden = true,
                    waitingFor = NetworkKind.WIFI,
                ),
                DownloadsActions(onResume = { resumed = it == null }),
            )
        }
        compose.onNodeWithText("Your account isn’t allowed to download.", substring = true).assertIsDisplayed()
        compose.onNodeWithText("Downloads are waiting for Wi-Fi.").assertIsDisplayed()
        compose.onNodeWithText("Try again").performClick()
        assertTrue(resumed)
    }

    @Test
    fun anEmptyDeviceSaysHowToDownload() {
        show { DownloadsScreen(DownloadsUiState(canDownload = true), DownloadsActions()) }
        compose.onNodeWithText("Nothing downloaded yet.", substring = true).assertIsDisplayed()
    }

    @Test
    fun offlineModeShowsTheBannerAndTheDownloadedLibrary() {
        var retried = false
        var played = ""
        show {
            androidx.compose.foundation.layout.Column {
                OfflineBannerBar("Offline: can’t reach music.example.com. Showing your downloads.", onRetry = { retried = true })
                OfflineLibraryScreen(listOf(dlAlbum(), dlAlbum("a8", "Second")), onAlbum = {}, onPlayAlbum = { played = it })
            }
        }
        compose.onNodeWithTag("offline-banner").assertIsDisplayed()
        compose.onNodeWithText("Downloaded").assertIsDisplayed()
        compose.onNodeWithText("Try again").performClick()
        compose.onNodeWithContentDescription("Play Second").performClick()
        assertTrue(retried)
        assertEquals("a8", played)
    }

    @Test
    fun anOfflineAlbumPlaysFromAnyTrack() {
        val started = mutableListOf<Int>()
        var shuffled = false
        var removed = false
        show {
            OfflineAlbumScreen(
                OfflineAlbumUi(dlAlbum(), listOf(dlTrack(1), dlTrack(2))),
                onPlay = { started += it },
                onShuffle = { shuffled = true },
                onRemove = { removed = true },
            )
        }
        compose.onNodeWithText("Available offline").assertIsDisplayed()
        compose.onNode(hasText("Play") and hasClickAction()).performClick()
        compose.onNodeWithText("Song 2").performClick()
        compose.onNodeWithText("Shuffle").performClick()
        compose.onNodeWithContentDescription("Remove the download of Record").performClick()
        compose.onNode(hasText("Remove") and hasClickAction()).performClick()
        assertEquals(listOf(0, 1), started)
        assertTrue(shuffled && removed)
    }

    @Test
    fun theOfflinePlayerShowsTheQueueAndItsControls() {
        val tracks = listOf(dlTrack(1), dlTrack(2))
        val ui = OfflinePlayerUi(
            OfflinePlayerState(status = dlStatus(index = 1, repeat = LocalRepeat.ALL), tracks = tracks.associateBy { it.key }),
            at = 0,
        )
        val calls = mutableListOf<String>()
        show {
            androidx.compose.foundation.layout.Column {
                OfflineMiniPlayer(ui, onOpen = { calls += "open" }, onToggle = { calls += "toggle" }, onNext = { calls += "next" })
                OfflineNowPlayingScreen(
                    ui,
                    clock = { 0L },
                    actions = OfflineNowPlayingActions(
                        onToggle = { calls += "toggle" },
                        onPrevious = { calls += "previous" },
                        onShuffle = { calls += "shuffle" },
                        onRepeat = { calls += "repeat" },
                        onJump = { calls += "jump $it" },
                        onStop = { calls += "stop" },
                    ),
                )
            }
        }
        compose.onNodeWithTag("offline-mini-player").assertIsDisplayed()
        compose.onNodeWithText("Playing downloads on this phone").assertIsDisplayed()
        // Constant names; the mode is the toggle's state.
        compose.onNodeWithContentDescription("Repeat")
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Repeating the queue"))
            .performClick()
        compose.onNodeWithContentDescription("Shuffle")
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Off"))
            .performClick()
        compose.onNodeWithContentDescription("Previous track").performClick()
        compose.onNodeWithContentDescription("Stop playing downloads").performClick()
        // The queue opens as a sheet from the Queue action.
        compose.onNodeWithTag("queue-button").performClick()
        compose.onNodeWithText("Song 1").performClick()
        assertEquals(listOf("repeat", "shuffle", "previous", "stop", "jump 0"), calls)
    }

    @Test
    fun storageSettingsChangeWhatIsChosen() {
        var settings = DownloadSettings()
        show {
            StorageSettingsScreen(
                OfflineState(settings = settings, usedBytes = 1_048_576),
                refreshing = false,
                error = null,
                onChange = { settings = it(settings) },
                onRefreshSmart = {},
            )
        }
        // The limit is a row with a picker (Wave 3.5).
        compose.onNodeWithTag("storage-cap").assertTextContains("No limit").performClick()
        compose.onNode(hasText("5.00 GB") and androidx.compose.ui.test.isSelectable()).performClick()
        assertEquals(5 * DownloadSettings.GB, settings.capBytes)
        compose.onNodeWithTag("storage-settings").performScrollToNode(hasText("Download on Wi-Fi only"))
        compose.onNodeWithText("Download on Wi-Fi only").performClick()
        assertTrue(settings.wifiOnly)
        compose.onNodeWithTag("storage-settings").performScrollToNode(hasText("Keep favourites offline"))
        compose.onNodeWithText("Keep favourites offline").performClick()
        assertTrue(settings.smartFavourites)
    }

    @Test
    fun signingOutWithDownloadsAsksKeepOrRemove() {
        val chosen = mutableListOf<Boolean>()
        show { SignOutDialog(3, 9 * 1_048_576, onSignOut = { chosen += it }, onDismiss = {}) }
        compose.onNodeWithText("This phone has 9.0 MB of downloaded music.", substring = true).assertIsDisplayed()
        compose.onNodeWithText("Sign out, keep downloads").performClick()
        compose.onNodeWithText("Sign out and remove downloads").performClick()
        assertEquals(listOf(false, true), chosen)
    }

    @Test
    fun signingOutWithoutDownloadsAsksPlainly() {
        val chosen = mutableListOf<Boolean>()
        show { SignOutDialog(0, 0, onSignOut = { chosen += it }, onDismiss = {}) }
        compose.onNodeWithText("Sign out, keep downloads").assertDoesNotExist()
        compose.onNode(hasText("Sign out") and hasClickAction()).performClick()
        assertEquals(listOf(false), chosen)
    }

    @Test
    fun theDownloadButtonSaysWhatItWillDo() {
        var downloads = 0
        show {
            androidx.compose.foundation.layout.Column {
                PlayButtons(onPlay = {}, onShuffle = null, onMore = null, onDownload = { downloads++ })
                PlayButtons(onPlay = {}, onShuffle = null, onMore = null, onDownload = {}, downloadMark = DownloadMark.SOME)
                PlayButtons(onPlay = {}, onShuffle = null, onMore = null, onDownload = {}, downloadMark = DownloadMark.ALL)
            }
        }
        compose.onNodeWithContentDescription("Download").performClick()
        compose.onNodeWithContentDescription("Download the rest").assertIsDisplayed()
        compose.onNodeWithContentDescription("Downloaded").assertIsDisplayed()
        assertEquals(1, downloads)
    }
}
