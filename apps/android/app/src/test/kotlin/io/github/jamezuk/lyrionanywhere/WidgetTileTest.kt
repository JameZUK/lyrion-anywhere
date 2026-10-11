package io.github.jamezuk.lyrionanywhere

import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.core.TransferOutcome
import io.github.jamezuk.lyrionanywhere.core.TransferStage
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.NowPlaying
import io.github.jamezuk.lyrionanywhere.data.NowPlayingHub
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import io.github.jamezuk.lyrionanywhere.widget.Control
import io.github.jamezuk.lyrionanywhere.widget.Controls
import io.github.jamezuk.lyrionanywhere.widget.NowPlayingWidget
import io.github.jamezuk.lyrionanywhere.widget.TileModel
import io.github.jamezuk.lyrionanywhere.widget.WidgetModel
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

private const val PHONE = "02:4c:41:00:00:01"
private const val KITCHEN = "aa:00:00:00:00:01"

private fun np(id: String = PHONE, title: String = "One", playing: Boolean = true) =
    NowPlaying(id, "Kitchen", title, "Artist", "music/1/cover", playing, id == PHONE)

class WidgetModelTest {
    /** With large text, the widget's one row keeps the title and drops the detail line. */
    @Test
    fun theDetailLineIsDroppedWithLargeText() {
        assertTrue(NowPlayingWidget.detailFits(1f))
        assertTrue(NowPlayingWidget.detailFits(1.3f))
        assertFalse(NowPlayingWidget.detailFits(1.5f))
        assertFalse(NowPlayingWidget.detailFits(2f))
    }

    @Test
    fun nothingKnownOffersOnlyOpeningTheApp() {
        val m = WidgetModel.from(null)
        assertEquals(WidgetModel.Kind.NOTHING, m.kind)
        assertFalse(m.showsControls)
        assertNull(m.playerId)
    }

    @Test
    fun aTrackShowsItsDetailsAndControls() {
        val m = WidgetModel.from(np())
        assertEquals(WidgetModel.Kind.TRACK, m.kind)
        assertEquals("One", m.title)
        assertEquals("Artist", m.artist)
        assertEquals("music/1/cover", m.artwork)
        assertTrue(m.playing)
        assertTrue(m.showsControls)
        assertEquals(PHONE, m.playerId)
    }

    @Test
    fun anEmptyQueueShowsNoControlsOrArtwork() {
        val m = WidgetModel.from(np(title = "  ", playing = true))
        assertEquals(WidgetModel.Kind.IDLE, m.kind)
        assertFalse(m.showsControls)
        assertNull(m.artwork)
        assertFalse(m.playing)
    }

    @Test
    fun longTextIsCut() {
        val m = WidgetModel.from(np(title = "x".repeat(5000)))
        assertEquals(120, m.title.length)
    }

    @Test
    fun artworkIsDecodedSmall() {
        assertEquals(1, NowPlayingWidget.sampleSize(192, 192, 192))
        assertEquals(1, NowPlayingWidget.sampleSize(300, 300, 192))
        assertEquals(4, NowPlayingWidget.sampleSize(1000, 800, 192))
        assertEquals(64, NowPlayingWidget.sampleSize(1_000_000, 1_000_000, 192))
        assertEquals(1, NowPlayingWidget.sampleSize(0, 0, 192))
    }
}

class TileModelTest {
    @Test
    fun unavailableUnlessThePhoneIsAPlayer() {
        assertEquals(TileModel.State.UNAVAILABLE, TileModel.from(np(), phoneEnabled = false).state)
        assertEquals(TileModel.State.UNAVAILABLE, TileModel.from(null, phoneEnabled = true).state)
    }

    @Test
    fun activeWhilePlaying() {
        val playing = TileModel.from(np(playing = true), phoneEnabled = true)
        assertEquals(TileModel.State.ACTIVE, playing.state)
        assertEquals("One", playing.title)
        val paused = TileModel.from(np(playing = false), phoneEnabled = true)
        assertEquals(TileModel.State.INACTIVE, paused.state)
        assertFalse(paused.playing)
        assertNull(TileModel.from(np(title = ""), phoneEnabled = true).title)
    }
}

class ControlsTest {
    private class Players : PlayersRepository {
        val sent = mutableListOf<String>()
        var result: ApiResult<Unit> = ApiResult.Ok(Unit)

        private fun cmd(s: String): ApiResult<Unit> {
            sent += s
            return result
        }

        override suspend fun list() = ApiResult.Ok(emptyList<Player>())
        override suspend fun syncGroups() = ApiResult.Ok(emptyList<SyncGroup>())
        override suspend fun status(player: String): ApiResult<PlayerStatus> = ApiResult.Err(AppError.Unexpected("unused"))
        override suspend fun queue(player: String, start: Int, count: Int) = ApiResult.Ok(emptyList<Track>())
        override suspend fun play(player: String) = cmd("play $player")
        override suspend fun setPaused(player: String, paused: Boolean) = cmd("pause $paused $player")
        override suspend fun next(player: String) = cmd("next $player")
        override suspend fun previous(player: String) = cmd("previous $player")
        override suspend fun jumpTo(player: String, index: Int) = cmd("jump")
        override suspend fun seek(player: String, seconds: Double) = cmd("seek")
        override suspend fun setVolume(player: String, volume: Int) = cmd("volume")
        override suspend fun setShuffle(player: String, mode: ShuffleMode) = cmd("shuffle")
        override suspend fun setRepeat(player: String, mode: RepeatMode) = cmd("repeat")
        override suspend fun setPower(player: String, on: Boolean) = cmd("power")
        override suspend fun enqueue(player: String, selection: Selection, placement: Placement, playIndex: Int?) = cmd("enqueue")
        override suspend fun playFavourite(player: String, itemId: String, placement: Placement) = cmd("favourite")
        override suspend fun removeFromQueue(player: String, index: Int) = cmd("remove")
        override suspend fun moveInQueue(player: String, from: Int, to: Int) = cmd("move")
        override suspend fun clearQueue(player: String) = cmd("clear")
        override suspend fun restoreToQueue(player: String, trackId: Long, index: Int) = cmd("restore")
        override suspend fun setSynced(current: String, other: String, on: Boolean) = cmd("sync")
        override suspend fun transfer(source: String, target: String, onStage: (TransferStage) -> Unit): ApiResult<TransferOutcome> =
            ApiResult.Err(AppError.Unexpected("unused"))
    }

    @Test
    fun playPauseFollowsWhatIsShownAndShowsTheChangeAtOnce() = runTest {
        val players = Players()
        val hub = NowPlayingHub().apply { publishPhone(np(playing = true)) }
        val controls = Controls({ players }, hub)
        controls.run(PHONE, Control.TOGGLE)
        assertEquals(listOf("pause true $PHONE"), players.sent)
        assertEquals(false, hub.phone.value?.playing)
        controls.run(PHONE, Control.TOGGLE)
        assertEquals("play $PHONE", players.sent.last())
        assertEquals(true, hub.phone.value?.playing)
        controls.run(PHONE, Control.NEXT)
        assertEquals("next $PHONE", players.sent.last())
    }

    @Test
    fun aRefusedCommandChangesNothing() = runTest {
        val players = Players().apply { result = ApiResult.Err(AppError.Forbidden("player not permitted")) }
        val hub = NowPlayingHub().apply { publishSelected(np(KITCHEN, playing = false)) }
        val r = Controls({ players }, hub).run(KITCHEN, Control.TOGGLE)
        assertTrue(r is ApiResult.Err)
        assertEquals(false, hub.selected.value?.playing)
    }

    @Test
    fun malformedPlayerIdsAreNeverSent() = runTest {
        val players = Players()
        val controls = Controls({ players }, NowPlayingHub())
        for (bad in listOf("", "-", "../x", "aa:bb:cc:dd:ee", "aa:bb:cc:dd:ee:0g", "aa:bb:cc:dd:ee:01;rm", "AA-BB-CC-DD-EE-01")) {
            val r = controls.run(bad, Control.TOGGLE)
            assertTrue(bad, r is ApiResult.Err && (r.error is AppError.InvalidInput))
        }
        assertTrue(players.sent.isEmpty())
        assertTrue(Controls.isPlayerId("AA:bb:CC:dd:EE:01"))
    }
}
