package io.github.jamezuk.lyrionanywhere.navigation

import android.app.Application
import android.os.Bundle
import android.os.Parcel
import androidx.activity.ComponentActivity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertContentDescriptionEquals
import androidx.compose.ui.test.assertHasClickAction
import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.data.offline.ConnectionUi
import io.github.jamezuk.lyrionanywhere.ui.components.ConnectionTags
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.annotation.Config

/** Track 120 is well down the album's list: seeing it again means the scroll was kept. */
private const val FAR = 120

/**
 * The signed-in app keeps its place (SPEC §8.4): through a reconnect (the pill, the
 * banner), offline mode taking the screen and handing it back, and the activity being
 * recreated or the process killed (through the "Signing in…" screen).
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], application = Application::class)
class AppPlacesTests {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val h = HostState()

    private fun show() {
        compose.setContent { LyrionTheme(dynamicColour = false) { PlacesHost(h) } }
    }

    private fun openAlbumAndScroll() {
        compose.onNodeWithText("Open album").performClick()
        compose.onNodeWithText("Album 7").assertIsDisplayed()
        compose.onNodeWithTag(ALBUM_LIST).performScrollToIndex(FAR)
        compose.onNodeWithText("Track $FAR").assertIsDisplayed()
    }

    private fun assertOnTheAlbumWhereItWasLeft() {
        compose.onNodeWithText("Album 7").assertIsDisplayed()
        compose.onNodeWithText("Track $FAR").assertIsDisplayed()
    }

    private fun back() {
        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }
        compose.waitForIdle()
    }

    @Test
    fun aReconnectKeepsTheScreenAndShowsThePill() {
        show()
        openAlbumAndScroll()
        h.connection = ConnectionUi(reconnecting = true)
        compose.waitForIdle()
        assertOnTheAlbumWhereItWasLeft()
        compose.onNodeWithTag(ConnectionTags.PILL).assertIsDisplayed()
            .assertContentDescriptionEquals("Reconnecting…")
        compose.onNodeWithTag(ConnectionTags.BANNER).assertDoesNotExist()
        // Taking longer: the banner instead, still on the album.
        h.connection = ConnectionUi(unreachable = true)
        compose.waitForIdle()
        assertOnTheAlbumWhereItWasLeft()
        compose.onNodeWithTag(ConnectionTags.PILL).assertDoesNotExist()
        compose.onNodeWithTag(ConnectionTags.BANNER).assertIsDisplayed()
        // Reconnected: nothing over the screen.
        h.connection = ConnectionUi()
        compose.waitForIdle()
        assertOnTheAlbumWhereItWasLeft()
        compose.onNodeWithTag(ConnectionTags.BANNER).assertDoesNotExist()
        compose.onNodeWithTag(ConnectionTags.PILL).assertDoesNotExist()
    }

    @Test
    fun aBusyMusicServerKeepsTheScreenWithACalmNote() {
        show()
        openAlbumAndScroll()
        h.connection = ConnectionUi(busy = true)
        compose.waitForIdle()
        assertOnTheAlbumWhereItWasLeft()
        compose.onNodeWithTag(ConnectionTags.BUSY).assertIsDisplayed()
            .assertContentDescriptionEquals("Your music server is busy. Waiting for it…")
            .assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.LiveRegion))
        // Not an error and not offline: no banner, no reconnecting pill.
        compose.onNodeWithTag(ConnectionTags.BANNER).assertDoesNotExist()
        compose.onNodeWithTag(ConnectionTags.PILL).assertDoesNotExist()
        // The server answers again: it goes by itself.
        h.connection = ConnectionUi()
        compose.waitForIdle()
        assertOnTheAlbumWhereItWasLeft()
        compose.onNodeWithTag(ConnectionTags.BUSY).assertDoesNotExist()
    }

    @Test
    fun theBannerIsOneLabelledButtonThatOpensTheDownloads() {
        show()
        openAlbumAndScroll()
        h.connection = ConnectionUi(unreachable = true)
        compose.waitForIdle()
        compose.onNodeWithTag(ConnectionTags.BANNER)
            .assertIsDisplayed()
            .assertHasClickAction()
            .assertHeightIsAtLeast(48.dp)
            .assertContentDescriptionEquals("Can’t reach your server. Open your downloads")
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Button))
            .assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.LiveRegion))
        // Its words are read once (as the description), not again as text.
        compose.onNodeWithText("Can’t reach your server · Downloads").assertDoesNotExist()
        compose.onNodeWithTag(ConnectionTags.BANNER).performClick()
        compose.onNodeWithText("Downloads screen").assertIsDisplayed()
        // Back from the downloads: the album, where it was.
        back()
        assertOnTheAlbumWhereItWasLeft()
    }

    @Test
    fun offlineModeTakingOverAndGivingBackReturnsToTheAlbum() {
        show()
        openAlbumAndScroll()
        h.phase = Phase.OFFLINE
        compose.waitForIdle()
        compose.onNodeWithText("Downloaded music").assertIsDisplayed()
        compose.onNodeWithText("Album 7").assertDoesNotExist()
        // The connection is back: the album, scrolled where it was, with its back stack.
        h.phase = Phase.MAIN
        compose.waitForIdle()
        assertOnTheAlbumWhereItWasLeft()
        back()
        compose.onNodeWithText("Home").assertIsDisplayed()
    }

    @Test
    fun aSignInCheckInBetweenKeepsThePlaceToo() {
        show()
        openAlbumAndScroll()
        // Offline mode's "Retry" signs in again: offline, "Signing in…", then the app.
        h.phase = Phase.OFFLINE
        compose.waitForIdle()
        h.phase = Phase.SIGNING_IN
        compose.waitForIdle()
        h.phase = Phase.MAIN
        compose.waitForIdle()
        assertOnTheAlbumWhereItWasLeft()
    }

    @Test
    fun anotherAccountStartsAtHome() {
        show()
        openAlbumAndScroll()
        h.phase = Phase.SIGNING_IN
        compose.waitForIdle()
        h.account = "main|https://music.example.com/|2"
        h.phase = Phase.MAIN
        compose.waitForIdle()
        compose.onNodeWithText("Home").assertIsDisplayed()
        compose.onNodeWithText("Album 7").assertDoesNotExist()
    }

    /** A configuration change, or the process killed, with "Signing in…" shown first on return. */
    @Test
    fun savedAndRestoredThroughTheSigningInScreen() {
        val tester = StateRestorationTester(compose)
        tester.setContent { LyrionTheme(dynamicColour = false) { PlacesHost(h) } }
        openAlbumAndScroll()
        // The new activity (or process) starts with the sign-in being checked.
        h.phase = Phase.SIGNING_IN
        tester.emulateSavedInstanceStateRestore()
        compose.onNodeWithText("Signing in…").assertIsDisplayed()
        h.phase = Phase.MAIN
        compose.waitForIdle()
        assertOnTheAlbumWhereItWasLeft()
    }
}

/** The real activity round trip: recreated, and killed with its state parcelled. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], application = Application::class)
class AppPlacesRecreateTests {
    @get:Rule
    val compose = createEmptyComposeRule()

    @Before
    fun fresh() {
        PlacesActivity.state = HostState()
    }

    private fun openAlbumAndScroll() {
        compose.onNodeWithText("Open album").performClick()
        compose.onNodeWithTag(ALBUM_LIST).performScrollToIndex(FAR)
        compose.onNodeWithText("Track $FAR").assertIsDisplayed()
    }

    @Test
    fun recreatingTheActivityKeepsTheScreen() {
        val controller = Robolectric.buildActivity(PlacesActivity::class.java).setup()
        openAlbumAndScroll()
        controller.recreate()
        compose.waitForIdle()
        compose.onNodeWithText("Album 7").assertIsDisplayed()
        compose.onNodeWithText("Track $FAR").assertIsDisplayed()
    }

    @Test
    fun theProcessKilledReopensOnTheSameScreen() {
        val first = Robolectric.buildActivity(PlacesActivity::class.java).setup()
        openAlbumAndScroll()
        val saved = Bundle()
        first.pause().saveInstanceState(saved).stop().destroy()
        // A new process: only the parcelled state survives, and the app checks its sign-in first.
        val restored = parcelled(saved)
        PlacesActivity.state = HostState().apply { phase = Phase.SIGNING_IN }
        val second = Robolectric.buildActivity(PlacesActivity::class.java).setup(restored)
        compose.onNodeWithText("Signing in…").assertIsDisplayed()
        PlacesActivity.state.phase = Phase.MAIN
        compose.waitForIdle()
        // The screen and its back stack. (Each screen's own saved state, its scroll included,
        // starts afresh after the process is killed: a plain NavHost, without AppPlaces,
        // does the same here; within a process it is all kept, as the tests above show.)
        compose.onNodeWithText("Album 7").assertIsDisplayed()
        second.get().onBackPressedDispatcher.onBackPressed()
        compose.waitForIdle()
        compose.onNodeWithText("Home").assertIsDisplayed()
    }

    private fun parcelled(b: Bundle): Bundle {
        val p = Parcel.obtain()
        try {
            p.writeBundle(b)
            p.setDataPosition(0)
            return requireNotNull(p.readBundle(javaClass.classLoader))
        } finally {
            p.recycle()
        }
    }
}
