package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.DiagramVerdict
import io.github.jamezuk.lyrionanywhere.core.DownloadFormat
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.core.QualityDiagram
import io.github.jamezuk.lyrionanywhere.core.QualityFile
import io.github.jamezuk.lyrionanywhere.core.QualityNetwork
import io.github.jamezuk.lyrionanywhere.core.QualityOrigin
import io.github.jamezuk.lyrionanywhere.core.SignalPathInfo
import io.github.jamezuk.lyrionanywhere.core.StreamQuality
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.QualityDescriber
import io.github.jamezuk.lyrionanywhere.ui.components.LocalWindowLayout
import io.github.jamezuk.lyrionanywhere.ui.components.WidthClass
import io.github.jamezuk.lyrionanywhere.ui.components.WindowLayout
import io.github.jamezuk.lyrionanywhere.ui.player.Amber
import io.github.jamezuk.lyrionanywhere.ui.player.DiagramKey
import io.github.jamezuk.lyrionanywhere.ui.player.MiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.SignalPathContent
import io.github.jamezuk.lyrionanywhere.ui.player.diagramKey
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsSection
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsUiState
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runCurrent
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
private const val KITCHEN = "aa:00:00:00:00:01"

/** This phone playing a track, with [diagram] as the engine's describer built it. */
private fun phoneState(diagram: QualityDiagram?, path: SignalPathInfo? = signalPath()) = PlayerUiState(
    players = listOf(samplePlayer(PHONE), samplePlayer(KITCHEN)),
    selectedId = PHONE,
    status = sampleStatus(),
    loading = false,
    local = LocalPlayerState(enabled = true, status = localStatus(PHONE), signalPath = path),
    diagram = diagram,
)

/** Another player playing, with [diagram] (the file only). */
private fun remoteState(diagram: QualityDiagram?) = phoneState(diagram).copy(selectedId = KITCHEN)

private val amber = SemanticsMatcher.expectValue(Amber, true)
private val notAmber = SemanticsMatcher.expectValue(Amber, false)

/**
 * The quality badge and the signal-path diagram (docs/plans/signal-path-redesign.md): the
 * badge names what is heard, the sheet draws the shared model's blocks, the switch is this
 * phone's only.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class QualityBadgeAndDiagramTest {
    @get:Rule
    val compose = createComposeRule()

    private fun show(width: WidthClass = WidthClass.COMPACT, content: @Composable () -> Unit) = compose.setContent {
        LyrionTheme(dynamicColour = false) {
            CompositionLocalProvider(LocalWindowLayout provides WindowLayout(width, 1800f)) { content() }
        }
    }

    /** The case reported: FLAC 24/96, Opus 160 decoded, High on mobile data. */
    @Test
    fun theReportedCaseIsTranscodedInAmberOnNowPlaying() {
        show { NowPlayingScreen(phoneState(QualityVectors.expected(QualityVectors.REPORTED)), clock = { 0L }, actions = nowPlayingActions()) }
        compose.onNodeWithTag("quality-badge").assertTextEquals("Transcoded · Opus 160").assert(amber)
        compose.onNodeWithContentDescription("Transcoded · Opus 160. Show the signal path").assertExists()
        // Never the file's format alone.
        compose.onNodeWithText("FLAC 24/96 · Hi-Res").assertDoesNotExist()
    }

    @Test
    fun theReportedCaseIsTranscodedInAmberInTheMiniPlayer() {
        show { MiniPlayer(phoneState(QualityVectors.expected(QualityVectors.REPORTED)), onOpen = {}, onTogglePlay = {}, onNext = {}) }
        compose.onNodeWithTag("mini-quality").assertTextEquals("Transcoded · Opus 160").assert(amber)
    }

    @Test
    fun aPassedThroughFlacIsHiResNotAmber() {
        show { NowPlayingScreen(phoneState(QualityVectors.expected(QualityVectors.PASSED_THROUGH)), clock = { 0L }, actions = nowPlayingActions()) }
        compose.onNodeWithTag("quality-badge").assertTextEquals("Hi-Res · FLAC 24/96").assert(notAmber)
    }

    @Test
    fun noDiagramNoBadgeEvenWithTheFilesFormat() {
        show { NowPlayingScreen(phoneState(null), clock = { 0L }, actions = nowPlayingActions()) }
        compose.onNodeWithTag("quality-badge").assertDoesNotExist()
    }

    /** The sheet draws every vector's blocks, in order, tinting the one that lowers the verdict. */
    @Test
    fun theSheetDrawsTheBlocksOfTheVectors() {
        val names = listOf(
            QualityVectors.REPORTED, QualityVectors.PASSED_THROUGH, QualityVectors.LMS_LOWERED,
            QualityVectors.REMOTE, QualityVectors.OFFLINE_OPUS, "ALAC file served as FLAC", "Server ReplayGain only",
        )
        var current by mutableStateOf(names.first())
        show {
            val d = QualityVectors.expected(current)
            val s = when (QualityVectors.player(current)) {
                "remote" -> remoteState(d)
                "offline" -> phoneState(d).copy(downloads = true)
                else -> phoneState(d)
            }
            SignalPathContent(s, onLevel = {})
        }
        for (name in names) {
            current = name
            compose.waitForIdle()
            val want = QualityVectors.expected(name).blocks
            val nodes = compose.onAllNodesWithTag("diagram-block").fetchSemanticsNodes()
            assertEquals(name, want.size, nodes.size)
            want.zip(nodes).forEach { (b, n) ->
                val said = n.config.getOrNull(SemanticsProperties.ContentDescription)?.joinToString().orEmpty()
                assertTrue("$name: $said", said.startsWith(b.title))
                if (b.format.isNotEmpty()) assertTrue("$name: $said", b.format in said)
                assertEquals("$name: ${b.title}", b.lowers, n.config.getOrNull(Amber))
                assertEquals("$name: ${b.title}", b.lowers, said.endsWith("this step lowers the quality"))
            }
        }
    }

    /** TalkBack reads the blocks in order, top to bottom on a phone and left to right on a wide screen. */
    @Test
    fun talkBackReadsTheBlocksInOrderOnAWideScreen() {
        show(WidthClass.MEDIUM) { SignalPathContent(phoneState(QualityVectors.expected(QualityVectors.REPORTED)), onLevel = {}) }
        compose.onNodeWithTag("quality-diagram-row").assertExists()
        val said = compose.onAllNodesWithTag("diagram-block").fetchSemanticsNodes().map {
            it.config.getOrNull(SemanticsProperties.ContentDescription)?.joinToString().orEmpty()
        }
        assertEquals(
            listOf(
                "File: FLAC 24/96, Hi-Res lossless",
                "Lyrion gateway: Opus 160, Re-encoded to Opus 160 kb/s (High, on mobile data), this step lowers the quality",
                "This device: Opus 48 kHz, Phone speaker",
            ),
            said,
        )
        // Left to right, in that order.
        val lefts = compose.onAllNodesWithTag("diagram-block").fetchSemanticsNodes().map { it.boundsInRoot.left }
        assertEquals(lefts.sorted(), lefts)
    }

    @Test
    fun aPhoneStacksTheBlocks() {
        show { SignalPathContent(phoneState(QualityVectors.expected(QualityVectors.REPORTED)), onLevel = {}) }
        compose.onNodeWithTag("quality-diagram-row").assertDoesNotExist()
        val tops = compose.onAllNodesWithTag("diagram-block").fetchSemanticsNodes().map { it.boundsInRoot.top }
        assertEquals(3, tops.size)
        assertEquals(tops.sorted(), tops)
    }

    /** Details holds today's stage list, cushion, clock and the rest, collapsed until asked for. */
    @Test
    fun detailsHoldTheStageList() {
        show { SignalPathContent(phoneState(QualityVectors.expected(QualityVectors.PASSED_THROUGH)), onLevel = {}) }
        compose.onNodeWithTag("signal-path-details").assertDoesNotExist()
        compose.onNodeWithTag("diagram-details-toggle").performClick()
        compose.onNodeWithTag("signal-path-details").assertExists()
        compose.onNodeWithText("Decode (lossless)").assertExists()
        compose.onNodeWithText("Not bit-perfect end to end:").assertExists()
    }

    /** The switch: this phone's only, the level in force selected, why under it. */
    @Test
    fun theSwitchIsThePhonesOnly() {
        val chosen = mutableListOf<StreamQuality?>()
        var state by mutableStateOf(phoneState(QualityVectors.expected(QualityVectors.REPORTED)))
        show { SignalPathContent(state, onLevel = { chosen += it }) }
        compose.onNodeWithTag("quality-switch").assertExists()
        compose.onNodeWithTag("quality-switch-high").assertIsSelected()
        compose.onNodeWithTag("quality-switch-original").assertIsNotSelected()
        compose.onNodeWithTag("quality-switch-origin").assertTextEquals("High, on mobile data")
        compose.onNodeWithTag("quality-switch-now").assertExists()
        compose.onNodeWithTag("quality-switch-automatic").assertDoesNotExist()
        compose.onNodeWithTag("quality-switch-original").performClick()
        // The level in force again changes nothing.
        compose.onNodeWithTag("quality-switch-high").performClick()
        assertEquals(listOf<StreamQuality?>(StreamQuality.ORIGINAL), chosen)
        // Overridden here: "Back to automatic".
        state = phoneState(QualityVectors.expected(QualityVectors.OVERRIDDEN))
        compose.waitForIdle()
        compose.onNodeWithTag("quality-switch-automatic").performClick()
        assertEquals(listOf<StreamQuality?>(StreamQuality.ORIGINAL, null), chosen)
        // Another player and the downloads: their shapes, no switch.
        state = remoteState(QualityVectors.expected(QualityVectors.REMOTE))
        compose.waitForIdle()
        compose.onNodeWithTag("quality-switch").assertDoesNotExist()
        compose.onNodeWithTag("diagram-details-toggle").assertDoesNotExist()
        state = phoneState(QualityVectors.expected(QualityVectors.OFFLINE_OPUS)).copy(downloads = true)
        compose.waitForIdle()
        compose.onNodeWithTag("quality-switch").assertDoesNotExist()
        compose.onNodeWithTag("diagram-details-toggle").assertExists()
    }

    /** A stream that can't be sought: the switch says the change applies from the next track. */
    @Test
    fun anUnseekableStreamChangesFromTheNextTrack() {
        val d = QualityVectors.expected(QualityVectors.REPORTED).let { it.copy(switch = it.switch!!.copy(seekable = false)) }
        show { SignalPathContent(phoneState(d), onLevel = {}) }
        compose.onNodeWithTag("quality-switch-next-track")
            .assertTextEquals("A change applies from the next track: this stream can't be restarted where it is.")
        compose.onNodeWithTag("quality-switch-now").assertDoesNotExist()
    }

    /** Close 4426, and a plugin too old (protocol below 2): said where the phone's player would be. */
    @Test
    fun protocolMismatchMessages() {
        var local by mutableStateOf(
            LocalPlayerState(enabled = true, status = localStatus(PHONE, PlayerConnection.UpdateApp)),
        )
        show { NowPlayingScreen(PlayerUiState(loading = false, local = local), clock = { 0L }, actions = nowPlayingActions()) }
        compose.onNodeWithTag("phone-blocked").assertTextEquals("This server needs Lyrion Anywhere 1.17.0 or later. Update the app.")
        // The gateway's reason is never shown.
        compose.onNodeWithText("update the app", substring = false).assertDoesNotExist()
        local = LocalPlayerState(enabled = true, protocolChecked = true, serverProtocol = null)
        compose.waitForIdle()
        compose.onNodeWithTag("phone-blocked").assertTextEquals(
            "This phone can't play from this server until its Lyrion Anywhere plugin is updated to 0.7.3 or later. You can still control other players.",
        )
        // Remote control still works: choosing a player is still offered.
        compose.onNodeWithTag("now-playing-choose-player").assertExists()
        local = LocalPlayerState(enabled = true, protocolChecked = true, serverProtocol = 2u)
        compose.waitForIdle()
        compose.onNodeWithTag("phone-blocked").assertDoesNotExist()
    }

    /**
     * The engine itself found the server's protocol below 2 (UpdatePlugin), though the app's
     * own check had passed: the phone is blocked with the same words, and other players can
     * still be chosen.
     */
    @Test
    fun theEngineFindingAnOldPluginBlocksThePhone() {
        val local = LocalPlayerState(
            enabled = true,
            status = localStatus(PHONE, PlayerConnection.UpdatePlugin),
            protocolChecked = true,
            serverProtocol = 2u,
        )
        show { NowPlayingScreen(PlayerUiState(loading = false, local = local), clock = { 0L }, actions = nowPlayingActions()) }
        compose.onNodeWithTag("phone-blocked").assertTextEquals(
            "This phone can't play from this server until its Lyrion Anywhere plugin is updated to 0.7.3 or later. You can still control other players.",
        )
        compose.onNodeWithTag("now-playing-choose-player").assertExists()
    }

    /**
     * The Downloads badge: an Opus download of a lossless original (kept in the record) is
     * Transcoded; an older record without the original keeps its Lossy badge. The rule is
     * the core's ([OriginalDescriber] answers as `offline_quality_diagram` does, held by the
     * mobile crate's tests); this checks the track reaches it and the badge is drawn.
     */
    @Test
    fun aDownloadOfALosslessOriginalIsBadgedTranscoded() {
        val describer = OriginalDescriber()
        val opus = dlTrack(1, format = DownloadFormat.OPUS, codec = "opus", original = QualityFile("flc", 44_100u, 16u, null, null))
        val old = opus.copy(original = null)
        fun downloads(t: OfflineTrack): PlayerUiState {
            val ui = io.github.jamezuk.lyrionanywhere.ui.offline.OfflinePlayerUi(
                io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerState(status = dlStatus(keys = listOf(t.key)), tracks = mapOf(t.key to t), signalPath = signalPath()),
                diagram = describer.offline(signalPath(), t),
            )
            return io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsPlayer.state(ui, "Downloads")!!
        }
        var state by mutableStateOf(downloads(opus))
        show { NowPlayingScreen(state, clock = { 0L }, actions = nowPlayingActions()) }
        compose.onNodeWithTag("quality-badge").assertTextEquals("Transcoded · Opus 160").assert(amber)
        state = downloads(old)
        compose.waitForIdle()
        compose.onNodeWithTag("quality-badge").assertTextEquals("Lossy · Opus 160").assert(amber)
    }

    /** Settings › About shows the server's player protocol and what plugin that means. */
    @Test
    fun aboutShowsTheServersProtocol() {
        var protocol by androidx.compose.runtime.mutableStateOf<UInt?>(2u)
        show {
            SettingsScreen(
                SettingsUiState(),
                appVersion = "1",
                libraryVersion = "1",
                onSignOut = {},
                onLicences = {},
                section = SettingsSection.ABOUT,
                serverProtocol = protocol,
                serverProtocolChecked = true,
            )
        }
        compose.onNodeWithText("Server: player protocol 2 (plugin 0.7.3 or later)").assertExists()
        protocol = 1u
        compose.waitForIdle()
        compose.onNodeWithText("Server: player protocol 1 (plugin 0.7.2 or earlier)").assertExists()
        protocol = null
        compose.waitForIdle()
        compose.onNodeWithText("Server: no player protocol given (plugin 0.7.2 or earlier)").assertExists()
    }

    @Test
    fun theDiagramIsAccessible() {
        show { SignalPathContent(phoneState(QualityVectors.expected(QualityVectors.OVERRIDDEN)), onLevel = {}) }
        compose.onNodeWithTag("diagram-details-toggle").performClick()
        compose.assertAccessible()
        assertTrue(compose.onAllNodes(hasTestTag("diagram-block")).fetchSemanticsNodes().isNotEmpty())
    }
}

/** A describer that answers from the vectors and says what it was asked. */
private class VectorDescriber(var local: String = QualityVectors.REPORTED) : QualityDescriber {
    val asked = mutableListOf<String>()
    var lastOrigin: QualityOrigin? = null
    var lastSeekable: Boolean? = null
    var lastFile: QualityFile? = null

    override fun local(path: SignalPathInfo?, file: QualityFile?, origin: QualityOrigin?, seekable: Boolean): QualityDiagram {
        asked += "local"
        lastOrigin = origin
        lastSeekable = seekable
        lastFile = file
        return QualityVectors.expected(local)
    }

    override fun remote(name: String, file: QualityFile?): QualityDiagram {
        asked += "remote $name"
        lastFile = file
        return QualityVectors.expected(QualityVectors.REMOTE)
    }

    var lastTrack: OfflineTrack? = null

    override fun offline(path: SignalPathInfo?, track: OfflineTrack): QualityDiagram {
        asked += "offline ${track.format} ${track.codec}"
        lastTrack = track
        return QualityVectors.expected(QualityVectors.OFFLINE_OPUS)
    }
}

/**
 * The downloads' describer as the core answers for an Opus download: Transcoded when the
 * record keeps a lossless original, else (a lossy original, or a record from before the
 * original was kept) Lossy.
 */
private class OriginalDescriber : QualityDescriber {
    override fun local(path: SignalPathInfo?, file: QualityFile?, origin: QualityOrigin?, seekable: Boolean): QualityDiagram = error("not here")

    override fun remote(name: String, file: QualityFile?): QualityDiagram = error("not here")

    override fun offline(path: SignalPathInfo?, track: OfflineTrack): QualityDiagram {
        val d = QualityVectors.expected(QualityVectors.OFFLINE_OPUS)
        val badge = d.badge!!
        val lossless = track.original?.kind in setOf("flc", "alc", "wav", "aif", "wvp", "ape")
        return if (lossless) d else d.copy(badge = badge.copy(verdict = DiagramVerdict.LOSSY, text = "Lossy · ${badge.stream}"))
    }
}

/** Under Robolectric: the vectors are read with Android's JSON. */
@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class QualityOverrideViewModelTest : MainDispatcherTest() {
    private val flacFile = QualityFile("flc", 96_000u, 24u, 3021u, 2u)

    private fun setup(canSeek: Boolean = true, mode: PlaybackMode = PlaybackMode.PLAYING): Triple<PlayerViewModel, FakePlayers, FakeLocalPlayer> {
        val players = FakePlayers()
        players.players = players.players + samplePlayer(PHONE)
        players.statusResult = ApiResult.Ok(
            sampleStatus(mode = mode, elapsed = 100.0).copy(canSeek = canSeek, current = sampleTrack(1, 0).copy(fileFormat = flacFile)),
        )
        val local = FakeLocalPlayer()
        local.state.value = LocalPlayerState(
            enabled = true,
            status = localStatus(PHONE),
            metered = true,
            streamQuality = StreamQuality.HIGH,
            streamSetting = StreamQuality.HIGH,
        )
        val vm = PlayerViewModel(players, FakeEvents(), FakeSettings(), local, quality = VectorDescriber()) { 0L }
        return Triple(vm, players, local)
    }

    /** The override reaches the player, then the track is sought to where it is: the new stream starts there. */
    @Test
    fun theOverrideSendsTheLevelAndSeeks() = runTest(dispatcher) {
        val (vm, players, local) = setup()
        vm.start()
        runCurrent()
        assertTrue(vm.state.value.localSelected)
        players.commands.clear()
        vm.overrideQuality(StreamQuality.ORIGINAL)
        runCurrent()
        assertEquals(listOf<StreamQuality?>(StreamQuality.ORIGINAL), local.overrides)
        assertEquals(listOf("seek 100.0 $PHONE"), players.commands.filter { it.startsWith("seek") })
        // Back to automatic: the same, with no level.
        vm.overrideQuality(null)
        runCurrent()
        assertEquals(listOf(StreamQuality.ORIGINAL, null), local.overrides)
        assertEquals(2, players.commands.count { it.startsWith("seek") })
        vm.stop()
    }

    /**
     * The engine's level change blocks until the gateway echoes it (at most 2 s): the seek is
     * issued only once it has returned, so the new stream cannot be asked for at the old level.
     */
    @Test
    fun theSeekWaitsForTheLevelChange() = runTest(dispatcher) {
        val (vm, players, local) = setup()
        vm.start()
        runCurrent()
        players.commands.clear()
        val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
        local.overrideGate = gate
        vm.overrideQuality(StreamQuality.ORIGINAL)
        runCurrent()
        assertEquals(listOf<StreamQuality?>(StreamQuality.ORIGINAL), local.overrides)
        assertTrue("sought before the level was applied", players.commands.none { it.startsWith("seek") })
        gate.complete(Unit)
        runCurrent()
        assertEquals(listOf("seek 100.0 $PHONE"), players.commands.filter { it.startsWith("seek") })
        vm.stop()
    }

    /** Radio (no seeking): the level is set for the next stream, and nothing is sought. */
    @Test
    fun anUnseekableStreamIsNotSought() = runTest(dispatcher) {
        val (vm, players, local) = setup(canSeek = false)
        vm.start()
        runCurrent()
        players.commands.clear()
        vm.overrideQuality(StreamQuality.LOW)
        runCurrent()
        assertEquals(listOf<StreamQuality?>(StreamQuality.LOW), local.overrides)
        assertTrue(players.commands.none { it.startsWith("seek") })
        vm.stop()
    }

    /** Another player has no override (the gateway never transcodes for it). */
    @Test
    fun anotherPlayerHasNoOverride() = runTest(dispatcher) {
        val (vm, players, local) = setup()
        vm.start()
        runCurrent()
        vm.select(KITCHEN)
        runCurrent()
        players.commands.clear()
        vm.overrideQuality(StreamQuality.ORIGINAL)
        runCurrent()
        assertTrue(local.overrides.isEmpty())
        assertTrue(players.commands.none { it.startsWith("seek") })
        vm.stop()
    }

    /** The badge comes from the local diagram, built with why the level is in force; another player's from its file. */
    @Test
    fun theDiagramIsBuiltForThePlayerInUse() = runTest(dispatcher) {
        val players = FakePlayers()
        players.players = players.players + samplePlayer(PHONE)
        players.statusResult = ApiResult.Ok(sampleStatus().copy(current = sampleTrack(1, 0).copy(fileFormat = flacFile)))
        val local = FakeLocalPlayer()
        local.state.value = LocalPlayerState(enabled = true, status = localStatus(PHONE), metered = true, streamQuality = StreamQuality.HIGH, streamSetting = StreamQuality.HIGH)
        val describer = VectorDescriber()
        val vm = PlayerViewModel(players, FakeEvents(), FakeSettings(), local, quality = describer) { 0L }
        vm.start()
        runCurrent()
        assertEquals("Transcoded · Opus 160", vm.state.value.diagram?.badge?.text)
        assertEquals(QualityOrigin.Setting(QualityNetwork.MOBILE, StreamQuality.HIGH), describer.lastOrigin)
        assertEquals(true, describer.lastSeekable)
        assertEquals(flacFile, describer.lastFile)
        // An override changes the origin, and the diagram is built again.
        vm.overrideQuality(StreamQuality.ORIGINAL)
        runCurrent()
        assertEquals(QualityOrigin.Override(StreamQuality.ORIGINAL), describer.lastOrigin)
        // Another player: from its file.
        vm.select(KITCHEN)
        runCurrent()
        assertTrue(describer.asked.last().startsWith("remote"))
        assertEquals("Hi-Res · FLAC 24/96", vm.state.value.diagram?.badge?.text)
        assertTrue(diagramKey(vm.state.value) is DiagramKey.Remote)
        vm.stop()
    }

    /**
     * The downloads player: Download → This device from the download's format and the engine's
     * path, on the main screens through [io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsPlayer].
     */
    @Test
    fun theDownloadsHaveTheirOwnDiagram() = runTest(dispatcher) {
        val tracks = listOf(dlTrack(1), dlTrack(2))
        val player = FakeOfflinePlayer(
            io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerState(status = dlStatus(), tracks = tracks.associateBy { it.key }, signalPath = signalPath()),
        )
        val describer = VectorDescriber()
        val vm = io.github.jamezuk.lyrionanywhere.ui.offline.OfflinePlayerViewModel(player, { 0L }, quality = describer)
        assertEquals("Transcoded · Opus 160", vm.state.value.diagram?.badge?.text)
        assertEquals(listOf("offline ORIGINAL flc"), describer.asked)
        assertEquals("the whole track reaches the core", tracks[0], describer.lastTrack)
        // Built again only when what it comes from changes.
        player.flow.value = player.flow.value.copy(status = dlStatus(elapsedMs = 20_000))
        assertEquals(1, describer.asked.size)
        // The record's original learnt (the same track, rewritten): built again with it.
        val withOriginal = tracks[0].copy(original = QualityFile("flc", 96_000u, 24u, null, null))
        player.flow.value = player.flow.value.copy(tracks = mapOf(withOriginal.key to withOriginal, tracks[1].key to tracks[1]))
        assertEquals(2, describer.asked.size)
        assertEquals(withOriginal, describer.lastTrack)
        val main =io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsPlayer.state(vm.state.value, "Downloads")!!
        assertEquals(vm.state.value.diagram, main.diagram)
        assertEquals(signalPath(), main.downloadsPath)
        // No describer: no badge.
        val bare = io.github.jamezuk.lyrionanywhere.ui.offline.OfflinePlayerViewModel(player, { 0L })
        assertNull(bare.state.value.diagram)
    }

    /** A phone that can't play from this server is not waited for: another player is chosen. */
    @Test
    fun aBlockedPhoneIsNotWaitedFor() = runTest(dispatcher) {
        val players = FakePlayers()
        val local = FakeLocalPlayer()
        local.state.value = LocalPlayerState(enabled = true, protocolChecked = true, serverProtocol = 1u)
        val vm = PlayerViewModel(players, FakeEvents(), FakeSettings(), local) { 0L }
        vm.start()
        runCurrent()
        assertFalse(vm.state.value.waitingForPhone)
        vm.stop()
    }
}
