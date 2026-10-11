package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.DiscoveryOptions
import io.github.jamezuk.lyrionanywhere.core.DstmProvider
import io.github.jamezuk.lyrionanywhere.core.MenuItemKind
import io.github.jamezuk.lyrionanywhere.core.RadioPlan
import io.github.jamezuk.lyrionanywhere.core.RadioSeed
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.DiscoveryRepository
import io.github.jamezuk.lyrionanywhere.data.ExpiringCache
import io.github.jamezuk.lyrionanywhere.data.key
import io.github.jamezuk.lyrionanywhere.ui.discovery.DiscoveryController
import io.github.jamezuk.lyrionanywhere.ui.discovery.DiscoveryEvent
import io.github.jamezuk.lyrionanywhere.ui.discovery.DiscoveryUiState
import io.github.jamezuk.lyrionanywhere.ui.discovery.MoreLikeThisShelf
import io.github.jamezuk.lyrionanywhere.ui.discovery.StartRadioButton
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val PHONE = "02:4c:41:00:00:01"
private val DSTM = DstmProvider("PLUGIN_DSTM_SIMILAR", "Similar tracks")
private val MIX = action("audiomuseai", "mix", opens = false, next = "nowPlaying")
private val SIMILAR = action("audiomuseai", "similar", opens = true)

class FakeDiscovery : DiscoveryRepository {
    var answer: ApiResult<DiscoveryOptions> = ApiResult.Ok(DiscoveryOptions(null, null, null))
    var startAnswer: ApiResult<Unit> = ApiResult.Ok(Unit)
    val asked = mutableListOf<Pair<String, RadioSeed>>()
    val started = mutableListOf<Triple<String, RadioSeed, RadioPlan>>()

    override suspend fun options(player: String, seed: RadioSeed): ApiResult<DiscoveryOptions> {
        asked += player to seed
        return answer
    }

    override suspend fun startRadio(player: String, seed: RadioSeed, plan: RadioPlan): ApiResult<Unit> {
        started += Triple(player, seed, plan)
        return startAnswer
    }
}

/** "Start radio" appears only when the server offers radio, and starts what it offered. */
@OptIn(ExperimentalCoroutinesApi::class)
class DiscoveryControllerTests {
    private val scope = TestScope(StandardTestDispatcher())
    private val discovery = FakeDiscovery()
    private val menus = FakeMenus()
    private var player: String? = PHONE

    private fun controller(shelf: Boolean = false) = DiscoveryController(scope, discovery, menus, { player }, shelf)

    @Test
    fun nothingOfferedNothingShown() = scope.runTest {
        val c = controller(shelf = true)
        c.load(RadioSeed.Track(7))
        advanceUntilIdle()
        assertNull(c.state.value.radio)
        assertFalse(c.state.value.canStartRadio)
        assertTrue(c.state.value.similar.isEmpty())
        // Starting anyway does nothing.
        c.startRadio()
        advanceUntilIdle()
        assertTrue(discovery.started.isEmpty())
    }

    @Test
    fun anErrorHidesTheActions() = scope.runTest {
        discovery.answer = ApiResult.Err(AppError.Forbidden("no"))
        val c = controller()
        c.load(RadioSeed.Album(3))
        advanceUntilIdle()
        assertEquals(DiscoveryUiState(), c.state.value)
    }

    @Test
    fun radioFromDontStopTheMusicStartsOnThePlayerAndSaysItIsEndless() = scope.runTest {
        val plan = RadioPlan(null, null, DSTM, true)
        discovery.answer = ApiResult.Ok(DiscoveryOptions(plan, null, null))
        val c = controller()
        c.load(RadioSeed.Artist(9))
        advanceUntilIdle()
        assertEquals(plan, c.state.value.radio)
        assertTrue(c.state.value.canStartRadio)
        c.startRadio()
        assertTrue("the button waits", c.state.value.starting)
        c.startRadio() // a second press while starting is ignored
        advanceUntilIdle()
        assertEquals(listOf(Triple(PHONE, RadioSeed.Artist(9) as RadioSeed, plan)), discovery.started)
        assertEquals(DiscoveryEvent.RadioStarted(endless = true), c.events.first())
        assertFalse(c.state.value.starting)
    }

    @Test
    fun aFailedStartIsReported() = scope.runTest {
        discovery.answer = ApiResult.Ok(DiscoveryOptions(RadioPlan(MIX, "Instant mix", null, false), null, null))
        discovery.startAnswer = ApiResult.Err(AppError.Forbidden("not offered"))
        val c = controller()
        c.load(RadioSeed.Track(7))
        advanceUntilIdle()
        c.startRadio()
        advanceUntilIdle()
        assertEquals(DiscoveryEvent.Failed(AppError.Forbidden("not offered")), c.events.first())
    }

    @Test
    fun noPlayerNoRadio() = scope.runTest {
        player = null
        val c = controller()
        c.load(RadioSeed.Track(7))
        advanceUntilIdle()
        assertTrue("nothing is asked without a player", discovery.asked.isEmpty())
        player = PHONE
        c.load(RadioSeed.Track(7))
        discovery.answer = ApiResult.Ok(DiscoveryOptions(RadioPlan(MIX, "Instant mix", null, false), null, null))
        advanceUntilIdle()
        player = null
        c.startRadio()
        advanceUntilIdle()
        assertEquals(DiscoveryEvent.NoPlayer, c.events.first())
    }

    @Test
    fun theShelfShowsPlayableSimilarMusicAndPlaysIt() = scope.runTest {
        menus.pages["audiomuseai"] = listOf(
            menuItem("Two\nArtist", MenuItemKind.PLAYABLE, play = action("playlistcontrol", opens = false)),
            menuItem("Just text", MenuItemKind.TEXT),
            menuItem("A folder", MenuItemKind.FOLDER, go = action("audiomuseai", "items")),
            menuItem("Three\nArtist", MenuItemKind.PLAYABLE, play = action("playlistcontrol", opens = false)),
        )
        discovery.answer = ApiResult.Ok(DiscoveryOptions(null, SIMILAR, "Similar tracks"))
        val c = controller(shelf = true)
        c.load(RadioSeed.Album(3))
        advanceUntilIdle()
        assertEquals(listOf("Two\nArtist", "Three\nArtist"), c.state.value.similar.map { it.text })
        assertEquals("Similar tracks", c.state.value.similarLabel)
        assertNull("no radio here", c.state.value.radio)
        c.play(c.state.value.similar[0])
        advanceUntilIdle()
        assertEquals(listOf(listOf("playlistcontrol")), menus.ran)
        assertEquals(DiscoveryEvent.Played, c.events.first())
        // Without the shelf (a "More…" sheet for an album), nothing more is opened.
        menus.opened.clear()
        val sheet = controller(shelf = false)
        sheet.load(RadioSeed.Album(3))
        advanceUntilIdle()
        assertTrue(menus.opened.isEmpty())
    }

    @Test
    fun itemsThatOpenMenusOrNeedTextAreNotPlayedFromTheShelf() {
        assertFalse(DiscoveryController.playable(menuItem("x", MenuItemKind.FOLDER, go = action("a", "items"))))
        assertFalse(DiscoveryController.playable(menuItem("x", MenuItemKind.SEARCH, go = action("a", "items", input = true))))
        assertFalse(DiscoveryController.playable(menuItem("x", MenuItemKind.TEXT)))
        assertTrue(DiscoveryController.playable(menuItem("x", MenuItemKind.ACTION, go = action("a", "mix", opens = false))))
    }

    @Test
    fun theCacheExpiresAndForgets() = runTest {
        var now = 0L
        val cache = ExpiringCache<String>({ now }, ttlMs = 100, max = 2)
        cache.put("p|track:1", "a")
        assertEquals("a", cache.get("p|track:1"))
        now = 100
        assertNull("expired", cache.get("p|track:1"))
        now = 0
        cache.put("p|track:1", "a")
        cache.put("p|album:2", "b")
        cache.put("q|album:2", "c")
        assertNull("bounded: the oldest went", cache.get("p|track:1"))
        cache.forget("p|")
        assertNull(cache.get("p|album:2"))
        assertEquals("c", cache.get("q|album:2"))
        // A clock going backwards never revives an entry.
        now = -50
        assertNull(cache.get("q|album:2"))
        assertEquals("track:7", RadioSeed.Track(7).key())
        assertEquals("artist:7", RadioSeed.Artist(7).key())
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class DiscoveryViewTests {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun theRadioButtonIsAbsentWithoutRadio() {
        compose.setContent { LyrionTheme(dynamicColour = false) { StartRadioButton(DiscoveryUiState(), {}) } }
        compose.onNodeWithTag("start-radio").assertDoesNotExist()
    }

    @Test
    fun theRadioButtonSaysWhereTheRadioComesFrom() {
        var pressed = 0
        val state = DiscoveryUiState(radio = RadioPlan(MIX, "Instant mix", DSTM, false))
        compose.setContent { LyrionTheme(dynamicColour = false) { StartRadioButton(state, { pressed++ }) } }
        compose.onNodeWithText("Start radio").assertIsDisplayed()
        compose.onNodeWithText("Instant mix, then endless (Don't Stop The Music)").assertIsDisplayed()
        compose.onNodeWithTag("start-radio").performClick()
        assertEquals(1, pressed)
    }

    @Test
    fun theShelfShowsOnlyWhenThereIsSimilarMusic() {
        var played: String? = null
        val items = listOf(menuItem("Two\nArtist", MenuItemKind.PLAYABLE, play = action("playlistcontrol", opens = false)))
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                MoreLikeThisShelf(DiscoveryUiState(similar = items, similarLabel = "Similar tracks"), { played = it.text })
            }
        }
        compose.onNodeWithText("Similar tracks").assertIsDisplayed()
        compose.onNodeWithText("Two").performClick()
        assertEquals("Two\nArtist", played)
    }

    @Test
    fun noShelfWithoutSimilarMusic() {
        compose.setContent { LyrionTheme(dynamicColour = false) { MoreLikeThisShelf(DiscoveryUiState(), {}) } }
        compose.onNodeWithTag("more-like-this").assertDoesNotExist()
    }
}
