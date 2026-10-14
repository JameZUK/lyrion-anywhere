package io.github.jamezuk.lyrionanywhere.ui

import android.content.ActivityNotFoundException
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertHasClickAction
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.DownloadState
import io.github.jamezuk.lyrionanywhere.core.Favourite
import io.github.jamezuk.lyrionanywhere.core.MenuItemKind
import io.github.jamezuk.lyrionanywhere.core.SavedPlaylist
import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.core.VirtualLibrary
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.SoundSettings
import io.github.jamezuk.lyrionanywhere.data.offline.DownloadSettings
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerState
import io.github.jamezuk.lyrionanywhere.data.offline.OfflineState
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumActions
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumDetail
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumScreen
import io.github.jamezuk.lyrionanywhere.ui.library.HomeActions
import io.github.jamezuk.lyrionanywhere.ui.library.HomeFeed
import io.github.jamezuk.lyrionanywhere.ui.library.HomeScreen
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryScreen
import io.github.jamezuk.lyrionanywhere.ui.library.Loadable
import io.github.jamezuk.lyrionanywhere.ui.menus.ItemSheetContent
import io.github.jamezuk.lyrionanywhere.ui.menus.MenuNavigator
import io.github.jamezuk.lyrionanywhere.ui.menus.SheetTarget
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsActions
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsUiState
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineNowPlayingActions
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineNowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflinePlayerUi
import io.github.jamezuk.lyrionanywhere.ui.offline.StorageSettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairConfirm
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairScreen
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairUiState
import io.github.jamezuk.lyrionanywhere.ui.onboarding.ServerScreen
import io.github.jamezuk.lyrionanywhere.ui.onboarding.ServerUiState
import io.github.jamezuk.lyrionanywhere.ui.onboarding.SignInScreen
import io.github.jamezuk.lyrionanywhere.ui.onboarding.SignInUiState
import io.github.jamezuk.lyrionanywhere.core.RatingStyle
import io.github.jamezuk.lyrionanywhere.core.TrackRating
import io.github.jamezuk.lyrionanywhere.ui.alarms.AlarmsScreen
import io.github.jamezuk.lyrionanywhere.ui.alarms.AlarmsViewModel
import io.github.jamezuk.lyrionanywhere.ui.onboarding.UnreachableScreen
import io.github.jamezuk.lyrionanywhere.ui.player.MiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.RatingControl
import io.github.jamezuk.lyrionanywhere.ui.player.RatingsSetting
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnActions
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnList
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.QueueEntry
import io.github.jamezuk.lyrionanywhere.ui.player.isPlayPauseKey
import io.github.jamezuk.lyrionanywhere.ui.library.isContextMenuKey
import io.github.jamezuk.lyrionanywhere.ui.settings.PRIVACY_POLICY_URL
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsUiState
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundActions
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundUiState
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val PHONE = "02:4c:41:00:00:01"
private const val KITCHEN = "aa:00:00:00:00:01"
private const val LOUNGE = "aa:00:00:00:00:02"

/** A player state with another player controlled, a queue and a sync group. */
internal fun auditPlayerState(selected: String = KITCHEN) = PlayerUiState(
    players = listOf(samplePlayer(PHONE), samplePlayer(KITCHEN, playing = true), samplePlayer(LOUNGE)),
    selectedId = selected,
    status = sampleStatus(),
    loading = false,
    local = LocalPlayerState(enabled = true, status = localStatus(PHONE)),
    playingOn = mapOf(KITCHEN to sampleTrack(9)),
    syncGroups = listOf(SyncGroup(listOf(KITCHEN, LOUNGE))),
    queue = listOf(QueueEntry(1, sampleTrack(1, 0)), QueueEntry(2, sampleTrack(2, 1)), QueueEntry(3, sampleTrack(3, 2))),
)

/** A "More…" sheet with LMS items under the quick play buttons. */
internal fun auditNavigator(): MenuNavigator {
    val menus = FakeMenus().apply {
        pages["trackinfo"] = listOf(
            menuItem("More like this", MenuItemKind.ACTION, go = action("someplugin", "mix", opens = false)),
            menuItem("Duration: 1:01", MenuItemKind.TEXT),
        )
    }
    val navigator = MenuNavigator(CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate), menus) { KITCHEN }
    navigator.start("Track 1", action("trackinfo", "items"))
    return navigator
}

/**
 * Every main screen, audited as TalkBack sees it ([assertAccessible]): labels, touch
 * targets, reachable actions, no "selected" buttons, labelled toggles and sliders.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class AccessibilityAuditTests {
    @get:Rule
    val compose = createComposeRule()

    private fun audit(content: @Composable () -> Unit) {
        compose.setContent { LyrionTheme(dynamicColour = false) { content() } }
        compose.assertAccessible()
    }

    @Test
    fun miniPlayer() = audit {
        MiniPlayer(auditPlayerState(), onOpen = {}, onTogglePlay = {}, onNext = {})
    }

    @Test
    fun playOnSheet() = audit {
        PlayOnList(
            auditPlayerState(),
            PlayOnActions(onControl = {}, onBackToPhone = {}, onMoveHere = {}, onBringToPhone = {}, onSync = { _, _ -> }),
        )
    }

    @Test
    fun nowPlayingWithAQueue() = audit {
        NowPlayingScreen(auditPlayerState(), clock = { 0L }, actions = nowPlayingActions().copy(onMore = {}))
    }

    @Test
    fun library() = audit {
        LibraryScreen(listOf(VirtualLibrary("ab12", "Classical")), null, {}, onOpen = {})
    }

    @Test
    fun home() = audit {
        HomeScreen(
            HomeFeed(
                recent = Loadable.Ready(listOf(sampleAlbum(1), sampleAlbum(2))),
                random = Loadable.Failed(AppError.Offline("connection refused")),
                favourites = Loadable.Ready(listOf(Favourite("f0.0", "Radio", null, false))),
                playlists = Loadable.Ready(listOf(SavedPlaylist(5, "Road trip"))),
            ),
            HomeActions(
                onAlbum = {},
                onMore = {},
                onSeeAll = {},
                onFavourite = {},
                onPlaylist = {},
                onReshuffle = {},
                onRetry = {},
            ),
        )
    }

    @Test
    fun album() = audit {
        AlbumScreen(
            Loadable.Ready(AlbumDetail(sampleAlbum(7), listOf(sampleTrack(1, disc = 1), sampleTrack(2, disc = 2)))),
            AlbumActions(
                onRetry = {},
                onPlay = { _, _ -> },
                onShuffle = {},
                onPlayTrack = {},
                onArtist = { _, _ -> },
                onMore = {},
                onDownload = {},
            ),
        )
    }

    @Test
    fun moreSheet() {
        val navigator = auditNavigator()
        audit {
            ItemSheetContent(SheetTarget("Track 1", "Artist"), navigator, onPlay = {}, onClose = {}, onDownload = {})
        }
    }

    @Test
    fun settings() = audit {
        SettingsScreen(
            SettingsUiState(
                serverName = "music.example.com",
                username = "alice",
                sound = SoundSettings(),
                downloadsOffered = true,
                deviceName = "Test phone",
            ),
            appVersion = "1",
            libraryVersion = "1",
            onSignOut = {},
            onLicences = {},
        )
    }

    @Test
    fun sound() = audit { SoundScreen(SoundUiState(SoundSettings(limiter = true)), object : SoundActions {}) }

    @Test
    fun downloads() = audit {
        DownloadsScreen(
            DownloadsUiState(
                inProgress = listOf(dlItem(4, DownloadState.ACTIVE, 1_048_576, 4_194_304), dlItem(5, DownloadState.PAUSED)),
                albums = listOf(dlAlbum()),
                trackCount = 2,
                usedBytes = 3 * 1_048_576,
                capBytes = DownloadSettings.GB,
                canDownload = true,
            ),
            DownloadsActions(onStorage = {}),
        )
    }

    @Test
    fun offlineNowPlaying() {
        val tracks = listOf(dlTrack(1), dlTrack(2))
        val ui = OfflinePlayerUi(OfflinePlayerState(status = dlStatus(index = 1), tracks = tracks.associateBy { it.key }), at = 0)
        audit { OfflineNowPlayingScreen(ui, clock = { 0L }, actions = OfflineNowPlayingActions()) }
    }

    @Test
    fun storageSettings() = audit {
        StorageSettingsScreen(
            OfflineState(settings = DownloadSettings(), usedBytes = 1_048_576),
            refreshing = false,
            error = null,
            onChange = {},
            onRefreshSmart = {},
        )
    }

    @Test
    fun signIn() = audit {
        SignInScreen(
            serverName = "music.example.com",
            state = SignInUiState(error = AppError.SignedOut),
            onUsernameChange = {},
            onPasswordChange = {},
            onCodeChange = {},
            onSubmit = {},
            onPair = {},
            onChangeServer = {},
        )
    }

    @Test
    fun server() = audit { ServerScreen(ServerUiState(address = "music.example.com"), onAddressChange = {}, onSubmit = {}) }

    @Test
    fun pair() = audit {
        PairScreen(
            state = PairUiState(code = "ABCDE"),
            cameraPermitted = false,
            onRequestCamera = {},
            onScanned = {},
            onCodeChange = {},
            onSubmit = {},
            onBack = {},
        )
    }

    @Test
    fun pairConfirmation() = audit {
        PairScreen(
            state = PairUiState(
                scanning = false,
                confirm = PairConfirm("ABCDE-FGHJK", "music.example.com", "https://music.example.com/", replaces = "lms.example.lan"),
            ),
            cameraPermitted = false,
            onRequestCamera = {},
            onScanned = {},
            onCodeChange = {},
            onSubmit = {},
            onBack = {},
        )
    }

    @Test
    fun alarms() {
        setClock24(true)
        val vm = AlarmsViewModel(FakeAlarms(), KITCHEN)
        audit { AlarmsScreen(vm, "Kitchen") }
    }

    /** The alarm editor (in its dialog): days, repeat, sound, shuffle and the labelled volume. */
    @Test
    fun alarmEditor() {
        setClock24(true)
        val vm = AlarmsViewModel(FakeAlarms(), KITCHEN)
        audit { AlarmsScreen(vm, "Kitchen") }
        compose.onNodeWithTag("alarm-1a2b3c4d").performClick()
        compose.assertAccessible()
    }

    @Test
    fun ratingsSetting() {
        audit { RatingsSetting(RatingStyle.STARS, onStyle = {}) { 3 } }
        // Its confirmation too.
        compose.onNodeWithTag("clear-ratings").performClick()
        compose.assertAccessible()
    }

    @Test
    fun ratingControls() = audit {
        androidx.compose.foundation.layout.Column {
            RatingControl(TrackRating(9, 80u, 60u, 80u), RatingStyle.STARS, failed = false) {}
            RatingControl(TrackRating(9, null, 60u, 60u), RatingStyle.STARS, failed = true) {}
            RatingControl(TrackRating(9, 100u, null, 100u), RatingStyle.HEART, failed = false) {}
        }
    }

    @Test
    fun nothingPlayingOffersAPlayer() = audit {
        androidx.compose.foundation.layout.Column {
            MiniPlayer(PlayerUiState(players = listOf(samplePlayer(KITCHEN)), loading = false), {}, {}, {})
            UnreachableScreen("music.example.com", "", 2, 1_000, onRetry = {}, onSignOut = {})
        }
    }

    /** The audit itself: a check that passes everything would prove nothing. */
    @Test
    fun theAuditCatchesAnUnlabelledButton() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                IconButton(onClick = {}) { Icon(Icons.Outlined.Close, contentDescription = null) }
            }
        }
        assertTrue("an unlabelled icon button must fail the audit", runCatching { compose.assertAccessible() }.isFailure)
    }

    @Test
    fun theAuditCatchesASelectedButton() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                // The pattern the Home cards used to have: a "button" that says "not selected".
                Text("Choice", Modifier.selectable(selected = false, role = Role.Button, onClick = {}).padding(24.dp))
            }
        }
        assertTrue("a button announced as selected must fail the audit", runCatching { compose.assertAccessible() }.isFailure)
    }
}

/** The privacy policy link in Settings › About. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class PrivacyLinkTests {
    @get:Rule
    val compose = createComposeRule()

    private fun settings(handler: UriHandler) = compose.setContent {
        CompositionLocalProvider(LocalUriHandler provides handler) {
            LyrionTheme(dynamicColour = false) {
                SettingsScreen(SettingsUiState(username = "alice"), "1", "1", {}, {}, section = io.github.jamezuk.lyrionanywhere.ui.settings.SettingsSection.ABOUT)
            }
        }
    }

    @Test
    fun thePrivacyPolicyOpensTheDocument() {
        val opened = mutableListOf<String>()
        settings(object : UriHandler {
            override fun openUri(uri: String) {
                opened += uri
            }
        })
        val row = compose.onNodeWithTag("privacy-policy").performScrollTo()
        row.assertHasClickAction()
        row.assert(hasText("Privacy policy"))
        row.assert(hasText("What the app and your music server keep, and what they don't"))
        row.performClick()
        assertEquals(listOf("https://github.com/JameZUK/lyrion-anywhere/blob/main/docs/PRIVACY.md"), opened)
        assertEquals(PRIVACY_POLICY_URL, opened.single())
    }

    /** No browser on the phone: the tap does nothing rather than crash the app. */
    @Test
    fun noBrowserDoesNotCrash() {
        var tried = 0
        settings(object : UriHandler {
            override fun openUri(uri: String) {
                tried++
                throw ActivityNotFoundException("no browser")
            }
        })
        compose.onNodeWithTag("privacy-policy").performScrollTo().performClick()
        assertEquals(1, tried)
    }
}

/** Keyboard and D-pad shortcuts. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class KeyboardShortcutTests {
    @Test
    fun spaceAndTheMediaKeyPlayAndPauseOnKeyDownOnly() {
        assertTrue(isPlayPauseKey(keyDown(android.view.KeyEvent.KEYCODE_SPACE)))
        assertTrue(isPlayPauseKey(keyDown(android.view.KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE)))
        val up = androidx.compose.ui.input.key.KeyEvent(
            android.view.KeyEvent(0L, 0L, android.view.KeyEvent.ACTION_UP, android.view.KeyEvent.KEYCODE_SPACE, 0),
        )
        assertFalse(isPlayPauseKey(up))
        assertFalse(isPlayPauseKey(keyDown(android.view.KeyEvent.KEYCODE_ENTER)))
    }

    @Test
    fun menuOrShiftF10OpensMore() {
        assertTrue(isContextMenuKey(keyDown(android.view.KeyEvent.KEYCODE_MENU)))
        assertTrue(isContextMenuKey(keyDown(android.view.KeyEvent.KEYCODE_F10, android.view.KeyEvent.META_SHIFT_ON)))
        assertFalse(isContextMenuKey(keyDown(android.view.KeyEvent.KEYCODE_F10)))
        assertFalse(isContextMenuKey(keyDown(android.view.KeyEvent.KEYCODE_ENTER)))
    }
}

/** The current track in a list is marked for TalkBack, not by colour alone. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class NowPlayingMarkTests {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun theOfflineQueueMarksThePlayingTrackByState() {
        val tracks = listOf(dlTrack(1), dlTrack(2))
        val ui = OfflinePlayerUi(OfflinePlayerState(status = dlStatus(index = 1), tracks = tracks.associateBy { it.key }), at = 0)
        compose.setContent {
            LyrionTheme(dynamicColour = false) { OfflineNowPlayingScreen(ui, clock = { 0L }, actions = OfflineNowPlayingActions()) }
        }
        // The queue is a sheet, from the Queue action.
        compose.onNodeWithTag("queue-button").performClick()
        val playing = compose.onNode(hasText("Song 2") and SemanticsMatcher.keyIsDefined(SemanticsActions.OnClick))
        playing.performScrollTo()
        playing.assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Now playing"))
        // Its title, not replaced by a description.
        playing.assert(SemanticsMatcher.keyNotDefined(SemanticsProperties.ContentDescription))
    }
}

/** Names and states that TalkBack reads, from the audit's findings. */
@RunWith(AndroidJUnit4::class)
// Tall, so the whole Sound page is composed.
@Config(sdk = [35], qualifiers = "w420dp-h3000dp")
class SpokenLabelTests {
    @get:Rule
    val compose = createComposeRule()

    private fun show(content: @Composable () -> Unit) = compose.setContent { LyrionTheme(dynamicColour = false) { content() } }

    private fun signIn(state: SignInUiState) = show {
        SignInScreen(
            serverName = "music.example.com",
            state = state,
            onUsernameChange = {},
            onPasswordChange = {},
            onCodeChange = {},
            onSubmit = {},
            onPair = {},
            onChangeServer = {},
        )
    }

    /** While signing in the button shows a spinner, but keeps its name. */
    @Test
    fun aBusyButtonKeepsItsName() {
        signIn(SignInUiState(username = "alice", password = "pw", busy = true))
        // The button (disabled while busy) is still called "Sign in", not just a spinner.
        compose.onNode(hasText("Sign in") and SemanticsMatcher.keyIsDefined(SemanticsProperties.Role))
            .assertIsNotEnabled()
    }

    /** A refused sign-in is announced, and the fields say they are in error. */
    @Test
    fun aSignInErrorIsAnnouncedAndMarksTheFields() {
        signIn(SignInUiState(username = "alice", password = "pw", error = AppError.SignedOut))
        compose.onNodeWithText("Wrong user name, password or code.")
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Polite))
        compose.onNode(hasText("User name") and SemanticsMatcher.keyIsDefined(SemanticsProperties.EditableText))
            .assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.Error))
    }

    /** Toggles keep one name; on or off is their state. */
    @Test
    fun thePowerItemNamesThePlayerAndWhatItDoes() {
        // In the player's row of Play on… (Wave 3.5: Rooms folded in), not Now playing's ⋮.
        val kitchen = samplePlayer(KITCHEN, playing = true)
        show {
            PlayOnList(
                auditPlayerState(),
                PlayOnActions(onControl = {}, onBackToPhone = {}),
                rooms = listOf(io.github.jamezuk.lyrionanywhere.ui.player.Room(kitchen, sampleStatus())),
                roomActions = io.github.jamezuk.lyrionanywhere.ui.player.RoomsActions({}, {}, { _, _ -> }, { _, _ -> }, { _, _ -> }, {}),
            )
        }
        compose.onNodeWithTag("play-on-$KITCHEN-more").performClick()
        compose.onNodeWithTag("play-on-$KITCHEN-power").assert(hasText("Switch Player $KITCHEN off"))
    }

    @Test
    fun theSyncToggleNamesBothPlayers() {
        show { PlayOnList(auditPlayerState(), PlayOnActions(onControl = {}, onBackToPhone = {}, onSync = { _, _ -> })) }
        // In Group players… (Wave 3.5).
        compose.onNodeWithTag("group-players").performClick()
        compose.onNodeWithContentDescription("Play Player $LOUNGE in sync with Player $KITCHEN").assertExists()
    }

    @Test
    fun seeAllSaysWhatItShows() {
        show {
            HomeScreen(
                HomeFeed(recent = Loadable.Ready(listOf(sampleAlbum(1)))),
                HomeActions({}, {}, {}, {}, {}, {}, {}),
            )
        }
        compose.onNodeWithContentDescription("See all: Recently added").assertHasClickAction()
    }

    @Test
    fun theLimiterCeilingSliderHasItsOwnName() {
        show { SoundScreen(SoundUiState(SoundSettings(limiter = true)), object : SoundActions {}) }
        compose.onNodeWithTag("ceiling").performScrollTo()
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription, listOf("Limiter ceiling")))
    }
}
