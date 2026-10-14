package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertContentDescriptionEquals
import androidx.compose.ui.test.assertHasClickAction
import androidx.compose.ui.test.assertHasNoClickAction
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.LocalAvailability
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.OfflineControls
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.DefaultPlayer
import io.github.jamezuk.lyrionanywhere.data.LocalControl
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.offline.ConnectionUi
import io.github.jamezuk.lyrionanywhere.data.offline.Reachability
import io.github.jamezuk.lyrionanywhere.data.offline.reachability
import io.github.jamezuk.lyrionanywhere.ui.components.ConnectionFrame
import io.github.jamezuk.lyrionanywhere.ui.components.ConnectionMessage
import io.github.jamezuk.lyrionanywhere.ui.components.ConnectionTags
import io.github.jamezuk.lyrionanywhere.ui.components.connectionMessage
import io.github.jamezuk.lyrionanywhere.ui.player.ControlGate
import io.github.jamezuk.lyrionanywhere.ui.player.ControlRoute
import io.github.jamezuk.lyrionanywhere.ui.player.MiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerMessage
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.QueueActions
import io.github.jamezuk.lyrionanywhere.ui.player.QueueDrag
import io.github.jamezuk.lyrionanywhere.ui.player.QueueEntry
import io.github.jamezuk.lyrionanywhere.ui.player.QueueMark
import io.github.jamezuk.lyrionanywhere.ui.player.controlGate
import io.github.jamezuk.lyrionanywhere.ui.player.offlineStatus
import io.github.jamezuk.lyrionanywhere.ui.player.onDeviceIds
import io.github.jamezuk.lyrionanywhere.ui.player.queueItems
import io.github.jamezuk.lyrionanywhere.ui.player.queueMark
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val PHONE = "02:4c:41:00:00:01"
private const val KITCHEN = "aa:00:00:00:00:01"

/** What this phone's engine says it can do with the server away. */
internal fun offlineControls(
    index: Int? = 0,
    playing: Boolean = true,
    elapsedMs: Long = 12_000,
    playPause: Boolean = true,
    next: Boolean = false,
    previous: Boolean = false,
    seek: Boolean = false,
    local: Boolean = false,
    waiting: Boolean = false,
) = OfflineControls(
    index = index?.toUInt(),
    trackId = index?.let { it + 1L },
    playing = playing,
    elapsedMs = elapsedMs.toULong(),
    playPause = playPause,
    next = next,
    previous = previous,
    seek = seek,
    local = local,
    waitingForServer = waiting,
)

/** This phone's player, running, with [controls] from its engine (null: connected). */
private fun phone(controls: OfflineControls? = null, playback: LocalPlayback = LocalPlayback.PLAYING) = LocalPlayerState(
    enabled = true,
    status = localStatus(
        PHONE,
        if (controls == null) PlayerConnection.Connected else PlayerConnection.Reconnecting(0uL),
    ).copy(playback = playback, offlineControls = controls),
)

private val UNREACHABLE = ConnectionUi(unreachable = true)

/** The gating rules, the marks, the shown status and the message: pure. */
class OfflineGateLogicTests {
    @Test
    fun onlineEverythingWorksThroughLms() {
        for (local in listOf(true, false)) {
            val g = controlGate(Reachability.ONLINE, local, true, offlineControls(next = false))
            assertEquals(ControlGate.ONLINE, g)
            assertTrue(g.canJump(QueueMark.NONE))
        }
    }

    @Test
    fun anotherPlayerWithTheServerAwayHasNothing() {
        for (r in listOf(Reachability.UNREACHABLE, Reachability.BUSY)) {
            val g = controlGate(r, localSelected = false, localRunning = true, controls = offlineControls(next = true))
            assertEquals(ControlRoute.NONE, g.route)
            assertFalse(g.playPause || g.next || g.previous || g.seek || g.volume || g.other)
            assertFalse(g.canJump(QueueMark.DOWNLOADED))
        }
        // The phone selected but its player not running: nothing either.
        assertEquals(ControlRoute.NONE, controlGate(Reachability.UNREACHABLE, true, false, offlineControls()).route)
    }

    @Test
    fun thisPhoneWithTheServerAwayDoesWhatItsEngineSays() {
        val c = offlineControls(next = true, previous = false, seek = true, playing = false)
        val g = controlGate(Reachability.UNREACHABLE, true, true, c)
        assertEquals(ControlRoute.PHONE, g.route)
        assertTrue(g.playPause && g.next && g.seek)
        assertFalse(g.previous)
        assertFalse("LMS's volume, shuffle, repeat, edits, sleep: off", g.volume || g.other)
        assertEquals(false, g.playing)
        // Only rows on the phone can be played.
        assertTrue(g.canJump(QueueMark.DOWNLOADED))
        assertTrue(g.canJump(QueueMark.ON_DEVICE))
        assertFalse(g.canJump(QueueMark.NONE))
        // Waiting at a track's end: it says so.
        val w = controlGate(Reachability.UNREACHABLE, true, true, offlineControls(playPause = false, waiting = true))
        assertTrue(w.waitingForServer)
        assertFalse(w.playPause)
    }

    @Test
    fun aBusyServerWithTheLinkUpLeavesPlayAndPauseOnThePhone() {
        val g = controlGate(Reachability.BUSY, true, true, null)
        assertEquals(ControlRoute.HOLD, g.route)
        assertTrue(g.playPause)
        assertFalse(g.next || g.previous || g.seek || g.other || g.volume)
        assertFalse(g.canJump(QueueMark.DOWNLOADED))
    }

    @Test
    fun reachabilityFollowsTheConnectionLine() {
        assertEquals(Reachability.ONLINE, ConnectionUi().reachability)
        assertEquals(Reachability.ONLINE, ConnectionUi(reconnecting = true).reachability)
        assertEquals(Reachability.UNREACHABLE, ConnectionUi(unreachable = true).reachability)
        assertEquals(Reachability.UNREACHABLE, ConnectionUi(takeover = true).reachability)
        assertEquals(Reachability.BUSY, ConnectionUi(busy = true).reachability)
    }

    @Test
    fun marksDownloadedForAnyPlayerAndOnDeviceOnlyForThePhone() {
        val downloaded = setOf(1L)
        val onDevice = setOf(1L, 2L)
        assertEquals(QueueMark.DOWNLOADED, queueMark(1, downloaded, onDevice, localSelected = false))
        assertEquals(QueueMark.DOWNLOADED, queueMark(1, downloaded, onDevice, localSelected = true))
        assertEquals(QueueMark.ON_DEVICE, queueMark(2, downloaded, onDevice, localSelected = true))
        assertEquals(QueueMark.NONE, queueMark(2, downloaded, onDevice, localSelected = false))
        assertEquals(QueueMark.NONE, queueMark(3, downloaded, onDevice, true))
        assertEquals(QueueMark.NONE, queueMark(null, downloaded, onDevice, true))
        assertEquals(QueueMark.NONE, queueMark(-1, setOf(-1L), setOf(-1L), true))
        assertEquals(
            setOf(5L, 7L),
            onDeviceIds(listOf(5, 6, 7), listOf(LocalAvailability.DOWNLOADED, LocalAvailability.NONE, LocalAvailability.ON_DEVICE)),
        )
        // A short answer (never trusted to line up beyond itself).
        assertEquals(setOf(5L), onDeviceIds(listOf(5, 6, 7), listOf(LocalAvailability.ON_DEVICE)))
    }

    @Test
    fun theShownStatusIsThePhonesWhileTheServerIsAway() {
        val queue = listOf(QueueEntry(1, sampleTrack(1, 0)), QueueEntry(2, sampleTrack(2, 1)), QueueEntry(3, sampleTrack(3, 2)))
        val lms = sampleStatus(PlaybackMode.PLAYING, elapsed = 50.0)
        assertEquals(lms, offlineStatus(lms, queue, null))
        assertNull(offlineStatus(null, queue, offlineControls()))
        // Paused on the phone, same track.
        val paused = offlineStatus(lms, queue, offlineControls(index = 0, playing = false, elapsedMs = 61_500))!!
        assertEquals(PlaybackMode.PAUSED, paused.mode)
        assertEquals(61.5, paused.elapsedSecs, 1e-9)
        assertEquals(lms.current, paused.current)
        // Skipped on the phone: the entry it plays, from the queue as read.
        val skipped = offlineStatus(lms, queue, offlineControls(index = 2, elapsedMs = 0, seek = true))!!
        assertEquals(sampleTrack(3, 2), skipped.current)
        assertEquals(2u, skipped.queueIndex)
        assertEquals(sampleTrack(3, 2).durationSecs, skipped.durationSecs)
        assertNull("Up next is LMS's", skipped.next)
        assertTrue(skipped.canSeek)
        // An entry beyond what was read: the track stays, the position moves.
        val far = offlineStatus(lms, queue, offlineControls(index = 40))!!
        assertEquals(lms.current, far.current)
        assertEquals(40u, far.queueIndex)
    }

    @Test
    fun oneMessageAtMost() {
        assertNull(connectionMessage(ConnectionUi(), false))
        assertNull("offline mode says it itself", connectionMessage(ConnectionUi(takeover = true, unreachable = true), true))
        assertEquals(ConnectionMessage.UNREACHABLE, connectionMessage(UNREACHABLE, false))
        assertEquals(ConnectionMessage.UNREACHABLE_PLAYING_HERE, connectionMessage(UNREACHABLE, true))
        assertEquals(ConnectionMessage.BUSY, connectionMessage(ConnectionUi(busy = true), true))
        assertEquals(ConnectionMessage.RECONNECTING, connectionMessage(ConnectionUi(reconnecting = true), false))
        // Unreachable wins over the others.
        assertEquals(
            ConnectionMessage.UNREACHABLE,
            connectionMessage(ConnectionUi(unreachable = true, busy = true, reconnecting = true), false),
        )
    }
}

/** A local player whose engine can be told what it can do, and records what it is asked. */
class OfflineLocalPlayer : LocalPlayerRepository {
    override val state = MutableStateFlow(LocalPlayerState())
    val controls = mutableListOf<LocalControl>()
    val holds = mutableListOf<Boolean>()
    val snapshots = mutableListOf<Triple<List<Long?>, Int?, RepeatMode>>()
    var onPhone: Set<Long> = emptySet()

    override suspend fun setEnabled(on: Boolean): ApiResult<Unit> = ApiResult.Ok(Unit)

    override fun setHold(hold: Boolean) {
        holds += hold
    }

    override fun setDuck(duck: Boolean) {}

    override fun setOutputRoute(route: OutputRoute) {}

    override fun setQueueSnapshot(trackIds: List<Long?>, currentIndex: Int?, repeat: RepeatMode) {
        snapshots += Triple(trackIds, currentIndex, repeat)
    }

    override fun localControl(control: LocalControl): Boolean {
        controls += control
        return state.value.offlineControls != null
    }

    override suspend fun localAvailability(trackIds: List<Long>): List<LocalAvailability> =
        trackIds.map { if (it in onPhone) LocalAvailability.ON_DEVICE else LocalAvailability.NONE }
}

/** Where the view model sends each control, with the server there and away. */
@OptIn(ExperimentalCoroutinesApi::class)
class OfflineControlsViewModelTest : MainDispatcherTest() {
    private val vms = mutableListOf<PlayerViewModel>()

    private fun vmTest(body: suspend TestScope.() -> Unit) = runTest(dispatcher) {
        try {
            body()
        } finally {
            vms.forEach { it.close() }
        }
    }

    /** A view model controlling this phone's player (or the kitchen's). */
    private fun vm(players: FakePlayers, local: OfflineLocalPlayer, selected: String = PHONE, downloaded: Set<Long> = emptySet()): PlayerViewModel {
        players.players = listOf(samplePlayer(PHONE), samplePlayer(KITCHEN))
        players.queueTracks = listOf(sampleTrack(1, 0), sampleTrack(2, 1), sampleTrack(3, 2))
        local.state.value = phone(null)
        val settings = FakeSettings()
        settings.state.update { it.copy(defaultPlayer = DefaultPlayer.LAST_USED, selectedPlayerId = selected) }
        val vm = PlayerViewModel(players, FakeEvents(), settings, local, downloaded = flowOf(downloaded)) { 0L }
        vms += vm
        vm.start()
        vm.select(selected)
        vm.loadQueue()
        return vm
    }

    @Test
    fun onlineTheControlsGoToLms() = vmTest {
        val players = FakePlayers()
        val local = OfflineLocalPlayer()
        val v = vm(players, local)
        v.next()
        v.togglePlay()
        assertTrue(players.commands.any { it.startsWith("next") })
        assertTrue(players.commands.any { it.startsWith("pause") || it.startsWith("play") })
        assertTrue(local.controls.isEmpty())
        // The queue as read goes to the phone's engine, for when the server is away.
        assertEquals(listOf<Long?>(1, 2, 3), local.snapshots.last().first)
        assertEquals(0, local.snapshots.last().second)
    }

    @Test
    fun withTheServerAwayThePhonesControlsGoToItsEngineAndNothingToLms() = vmTest {
        val players = FakePlayers()
        val local = OfflineLocalPlayer()
        val v = vm(players, local)
        val before = players.commands.size
        local.state.value = phone(offlineControls(index = 0, next = true, previous = false, seek = true))
        v.setConnection(UNREACHABLE)
        assertEquals(ControlRoute.PHONE, v.state.value.gate.route)
        v.togglePlay()
        v.next()
        v.previous() // greyed out: nothing
        v.seek(30.0)
        v.setVolume(10) // LMS's: nothing
        v.cycleRepeat()
        v.removeFromQueue(1)
        v.enqueue(io.github.jamezuk.lyrionanywhere.core.Selection.Album(1), io.github.jamezuk.lyrionanywhere.core.Placement.PLAY_NOW)
        assertEquals(
            listOf(LocalControl.Pause, LocalControl.Next, LocalControl.Seek(30_000)),
            local.controls,
        )
        assertEquals("nothing sent to LMS", before, players.commands.size)
    }

    @Test
    fun aJumpGoesToTheEngineOnlyForARowOnThePhone() = vmTest {
        val players = FakePlayers()
        val local = OfflineLocalPlayer().apply { onPhone = setOf(3L) }
        val v = vm(players, local, downloaded = setOf(2L))
        local.state.value = phone(offlineControls(index = 0))
        v.setConnection(UNREACHABLE)
        assertEquals(QueueMark.DOWNLOADED, v.state.value.markOf(sampleTrack(2, 1)))
        assertEquals(QueueMark.ON_DEVICE, v.state.value.markOf(sampleTrack(3, 2)))
        assertEquals(QueueMark.NONE, v.state.value.markOf(sampleTrack(1, 0)))
        v.jumpTo(0) // not on the phone: nothing
        v.jumpTo(1)
        v.jumpTo(2)
        assertEquals(listOf(LocalControl.Jump(1), LocalControl.Jump(2)), local.controls)
    }

    @Test
    fun anotherPlayerWithTheServerAwayIsLeftAloneAndSaysNothingPerCommand() = vmTest {
        val players = FakePlayers()
        val local = OfflineLocalPlayer()
        val v = vm(players, local, selected = KITCHEN)
        v.setConnection(UNREACHABLE)
        val before = players.commands.size
        v.togglePlay()
        v.next()
        v.jumpTo(1)
        assertEquals(before, players.commands.size)
        assertTrue(local.controls.isEmpty())
        // A command that fails because the server is away, while the line says so: no message.
        v.setConnection(ConnectionUi(reconnecting = true))
        players.commandResult = ApiResult.Err(AppError.Offline("connect"))
        v.next()
        val message = withTimeoutOrNull(100) { v.messages.first() }
        assertNull(message)
        // Online, a failure is still reported.
        v.setConnection(ConnectionUi())
        v.next()
        assertTrue(withTimeoutOrNull(100) { v.messages.first() } is PlayerMessage.Failed)
    }

    @Test
    fun theShownStatusFollowsThePhoneAndLmsAgainOnReturn() = vmTest {
        val players = FakePlayers()
        val local = OfflineLocalPlayer()
        val v = vm(players, local)
        local.state.value = phone(offlineControls(index = 2, playing = false, elapsedMs = 4_000, local = true))
        v.setConnection(UNREACHABLE)
        val s = v.state.value
        assertEquals(sampleTrack(3, 2), s.status?.current)
        assertFalse(s.playing)
        assertEquals(4.0, s.status!!.elapsedSecs, 1e-9)
        // Back: LMS's view.
        local.state.value = phone(null)
        v.setConnection(ConnectionUi())
        assertEquals(sampleStatus().current, v.state.value.status?.current)
        assertEquals(ControlGate.ONLINE, v.state.value.gate)
    }

    @Test
    fun aBusyServerPausesThePhoneByHoldingItsOutput() = vmTest {
        val players = FakePlayers()
        val local = OfflineLocalPlayer()
        val v = vm(players, local)
        v.setConnection(ConnectionUi(busy = true))
        assertEquals(ControlRoute.HOLD, v.state.value.gate.route)
        val before = players.commands.size
        v.togglePlay()
        assertEquals(listOf(true), local.holds)
        assertEquals(before, players.commands.size)
        // The music server answers again: LMS is told the phone is paused.
        local.state.value = phone(null).let { it.copy(status = it.status!!.copy(held = true)) }
        v.setConnection(ConnectionUi())
        assertTrue(players.commands.drop(before).any { it.startsWith("pause true") })
    }
}

/** What the screen shows with the server away: greyed controls, marks, one line that takes space. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class OfflineControlsScreenTests {
    @get:Rule
    val compose = createComposeRule()

    private fun show(content: @Composable () -> Unit) = compose.setContent { LyrionTheme(dynamicColour = false) { content() } }

    private fun phoneState(controls: OfflineControls?, connection: ConnectionUi = UNREACHABLE) = PlayerUiState(
        players = listOf(samplePlayer(PHONE), samplePlayer(KITCHEN)),
        selectedId = PHONE,
        status = sampleStatus(),
        loading = false,
        local = phone(controls),
        queue = listOf(QueueEntry(1, sampleTrack(1, 0)), QueueEntry(2, sampleTrack(2, 1)), QueueEntry(3, sampleTrack(3, 2))),
        connection = connection,
        downloaded = setOf(2L),
        onDevice = setOf(3L),
    )

    @Test
    fun theLineTakesItsOwnSpaceAboveTheScreen() {
        var shown by mutableStateOf(ConnectionMessage.RECONNECTING)
        show {
            ConnectionFrame(shown, onDownloads = {}, Modifier.fillMaxSize()) {
                Box(Modifier.fillMaxSize().testTag("screen")) { Text("Controls") }
            }
        }
        for (m in ConnectionMessage.entries) {
            shown = m
            compose.waitForIdle()
            val tag = when (m) {
                ConnectionMessage.RECONNECTING -> ConnectionTags.PILL
                ConnectionMessage.BUSY -> ConnectionTags.BUSY
                else -> ConnectionTags.BANNER
            }
            val line = compose.onNodeWithTag(tag).getUnclippedBoundsInRoot()
            val screen = compose.onNodeWithTag("screen").getUnclippedBoundsInRoot()
            assertTrue("$m: the screen starts below the line ($line, $screen)", screen.top >= line.bottom)
            assertTrue("$m: nothing of the screen is under it", line.top.value <= 0.5f)
        }
    }

    @Test
    fun withoutAMessageTheScreenHasAllTheSpace() {
        show {
            ConnectionFrame(null, onDownloads = {}, Modifier.fillMaxSize()) {
                Box(Modifier.fillMaxSize().testTag("screen"))
            }
        }
        assertEquals(0f, compose.onNodeWithTag("screen").getUnclippedBoundsInRoot().top.value, 0.5f)
        compose.onNodeWithTag(ConnectionTags.BANNER).assertDoesNotExist()
    }

    @Test
    fun theLineSaysWhetherThePhonePlaysOnAndLeadsToTheDownloads() {
        var opened = 0
        show {
            ConnectionFrame(connectionMessage(UNREACHABLE, playingHere = true), onDownloads = { opened++ }, Modifier.fillMaxSize()) {
                Box(Modifier.fillMaxSize())
            }
        }
        compose.onNodeWithTag(ConnectionTags.BANNER)
            .assertContentDescriptionEquals("Can’t reach your server. Playing from this device.")
            .assertHasClickAction()
            .assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.LiveRegion))
            .performClick()
        assertEquals(1, opened)
    }

    @Test
    fun aBusyServerKeepsItsCalmWordsAndIsNotAButton() {
        show {
            ConnectionFrame(connectionMessage(ConnectionUi(busy = true), true), onDownloads = {}, Modifier.fillMaxSize()) {
                Box(Modifier.fillMaxSize())
            }
        }
        compose.onNodeWithTag(ConnectionTags.BUSY)
            .assertContentDescriptionEquals("Your music server is busy. Waiting for it…")
            .assertHasNoClickAction()
    }

    @Test
    fun nowPlayingGreysOutWhatCantWork() {
        show {
            NowPlayingScreen(
                phoneState(offlineControls(next = true, previous = false, seek = false)),
                clock = { 0L },
                actions = nowPlayingActions(),
            )
        }
        compose.onNodeWithTag("play-pause").assertIsEnabled()
        compose.onNodeWithTag("next").assertIsEnabled()
        compose.onNodeWithTag("previous").assertIsNotEnabled()
        compose.onNodeWithTag("repeat-button").assertIsNotEnabled()
        compose.onNodeWithTag("needs-server").assertDoesNotExist()
    }

    @Test
    fun nowPlayingSaysWhenTheNextTrackNeedsTheServer() {
        show {
            NowPlayingScreen(
                phoneState(offlineControls(playPause = false, waiting = true, previous = true)),
                clock = { 0L },
                actions = nowPlayingActions(),
            )
        }
        compose.onNodeWithText("Next track needs your server").assertExists()
        compose.onNodeWithTag("play-pause").assertIsNotEnabled()
        compose.onNodeWithTag("next").assertIsNotEnabled()
        compose.onNodeWithTag("previous").assertIsEnabled()
    }

    @Test
    fun onlineNothingIsGreyedOut() {
        show {
            NowPlayingScreen(phoneState(null, ConnectionUi()), clock = { 0L }, actions = nowPlayingActions())
        }
        compose.onNodeWithTag("play-pause").assertIsEnabled()
        compose.onNodeWithTag("next").assertIsEnabled()
        compose.onNodeWithTag("previous").assertIsEnabled()
        compose.onNodeWithTag("repeat-button").assertIsEnabled()
    }

    @Test
    fun theMiniPlayerOffersOnlyWhatWorks() {
        val state = phoneState(offlineControls(next = false))
        show { MiniPlayer(state, onOpen = {}, onTogglePlay = {}, onNext = {}) }
        compose.onNodeWithTag("mini-play-pause").assertIsEnabled()
        compose.onNodeWithTag("mini-next").assertIsNotEnabled()
    }

    @Test
    fun queueRowsShowTheirMarksAndOnlyThoseOnThePhonePlay() {
        val jumped = mutableListOf<Int>()
        val state = phoneState(offlineControls(index = 0))
        show {
            LazyColumn {
                queueItems(
                    state.queue,
                    0,
                    QueueDrag(),
                    QueueActions(onJump = { jumped += it }),
                    marks = state::markOf,
                    gate = state.gate,
                )
            }
        }
        compose.onNodeWithTag("queue-mark-1", useUnmergedTree = true).assertContentDescriptionEquals("Downloaded")
        compose.onNodeWithTag("queue-mark-2", useUnmergedTree = true).assertContentDescriptionEquals("On this device")
        compose.onNodeWithTag("queue-mark-0", useUnmergedTree = true).assertDoesNotExist()
        compose.onNodeWithTag("queue-track-0").assertIsNotEnabled()
        compose.onNodeWithTag("queue-track-1").assertIsEnabled().performClick()
        compose.onNodeWithTag("queue-track-2").performClick()
        compose.onNodeWithTag("queue-track-0").performClick()
        assertEquals(listOf(1, 2), jumped)
        // No edits while the server is away: no drag handles.
        compose.onNodeWithTag("queue-drag-1").assertDoesNotExist()
    }

    @Test
    fun onlineMarksShowAndEveryRowPlays() {
        val jumped = mutableListOf<Int>()
        val state = phoneState(null, ConnectionUi())
        show {
            LazyColumn {
                queueItems(state.queue, 0, QueueDrag(), QueueActions(onJump = { jumped += it }), marks = state::markOf, gate = state.gate)
            }
        }
        compose.onNodeWithContentDescription("Downloaded", useUnmergedTree = true).assertExists()
        compose.onNodeWithTag("queue-track-0").performClick()
        assertEquals(listOf(0), jumped)
        compose.onNodeWithTag("queue-drag-1").assertExists()
    }
}
