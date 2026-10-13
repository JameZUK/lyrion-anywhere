package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.core.DownloadState
import io.github.jamezuk.lyrionanywhere.core.DownloadsSnapshot
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalRepeat
import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.offline.DownloadSettings
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerState
import io.github.jamezuk.lyrionanywhere.data.offline.OfflineState
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadActions
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadMark
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadMessage
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsViewModel
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineAlbumViewModel
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflinePlayerUi
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflinePlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.offline.StorageSettingsViewModel
import io.github.jamezuk.lyrionanywhere.ui.offline.downloadMark
import io.github.jamezuk.lyrionanywhere.ui.offline.downloadsUi
import io.github.jamezuk.lyrionanywhere.ui.offline.formatBytes
import io.github.jamezuk.lyrionanywhere.ui.offline.progressOf
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsViewModel
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

private fun snapshot(vararg items: io.github.jamezuk.lyrionanywhere.core.DownloadItem, forbidden: Boolean = false, waiting: Boolean = false) =
    DownloadsSnapshot(items.toList(), forbidden, waiting, NetworkKind.WIFI, 0uL, 0uL, 0u)

class DownloadsViewModelTest : MainDispatcherTest() {
    @Test
    fun inProgressIsOrderedAndFinishedOnesMoveToTheAlbums() {
        val s = OfflineState(
            albums = listOf(dlAlbum()),
            tracks = listOf(dlTrack(1), dlTrack(2)),
            usedBytes = 5_000_000,
            settings = DownloadSettings(capBytes = 10 * DownloadSettings.GB),
            downloads = snapshot(
                dlItem(1, DownloadState.FAILED),
                dlItem(2, DownloadState.DONE),
                dlItem(3, DownloadState.QUEUED),
                dlItem(4, DownloadState.ACTIVE, 50, 100),
                dlItem(5, DownloadState.PAUSED),
                dlItem(6, DownloadState.WAITING),
            ),
            canDownload = true,
        )
        val ui = downloadsUi(s)
        assertEquals(listOf(4L, 6L, 3L, 5L, 1L), ui.inProgress.map { it.trackId })
        assertEquals(2, ui.trackCount)
        assertEquals(10 * DownloadSettings.GB, ui.capBytes)
        assertTrue(ui.anyRunning && ui.anyPaused)
        assertFalse(ui.empty)
        assertNull(ui.waitingFor)
        assertEquals(0.5f, progressOf(ui.inProgress[0])!!, 0f)
        assertNull("size unknown", progressOf(dlItem(9, DownloadState.ACTIVE)))
    }

    @Test
    fun refusalsAndWaitingAreShown() {
        val forbidden = downloadsUi(OfflineState(downloads = snapshot(dlItem(1, DownloadState.FAILED), forbidden = true)))
        assertTrue(forbidden.forbidden)
        val wifi = downloadsUi(OfflineState(downloads = snapshot(dlItem(1, DownloadState.QUEUED), waiting = true), network = NetworkKind.MOBILE))
        assertEquals(NetworkKind.WIFI, wifi.waitingFor)
        val none = downloadsUi(OfflineState(downloads = snapshot(dlItem(1, DownloadState.QUEUED), waiting = true), network = NetworkKind.OFFLINE))
        assertEquals(NetworkKind.OFFLINE, none.waitingFor)
        assertTrue("offline: no manager, nothing in progress", downloadsUi(OfflineState()).empty)
    }

    @Test
    fun actionsReachTheRepository() = runTest(dispatcher) {
        val repo = FakeOffline()
        val vm = DownloadsViewModel(repo)
        vm.pause(4)
        vm.resume(null)
        vm.cancel(5)
        vm.clearFinished()
        vm.removeAlbum("a7")
        vm.removeAll()
        assertEquals(listOf("pause 4", "resume null", "cancel 5", "clear", "removeAlbum a7", "removeAll"), repo.calls)
        repo.flow.update { it.copy(downloads = snapshot(dlItem(1, DownloadState.ACTIVE))) }
        assertEquals(1, vm.state.value.inProgress.size)
    }

    @Test
    fun bytesAreFormatted() {
        assertEquals("0 MB", formatBytes(0))
        assertEquals("1.5 MB", formatBytes(1_572_864))
        assertEquals("2.00 GB", formatBytes(2 * DownloadSettings.GB))
    }
}

class DownloadActionsTest : MainDispatcherTest() {
    @Test
    fun downloadsReportWhatWasQueuedOrWhyNot() = runTest(dispatcher) {
        val repo = FakeOffline(OfflineState(canDownload = true))
        val actions = DownloadActions(repo, CoroutineScope(dispatcher))
        val seen = mutableListOf<DownloadMessage>()
        val job = launch { actions.messages.collect { seen += it } }
        actions.album(3)
        actions.playlist(9)
        actions.track(42)
        repo.albumResult = ApiResult.Err(AppError.Forbidden("no"))
        actions.album(4)
        repo.flow.update { it.copy(canDownload = false) }
        actions.track(43)
        assertEquals(listOf("album 3", "playlist 9", "tracks [42]", "album 4"), repo.calls)
        assertEquals(
            listOf(
                DownloadMessage.Queued(2),
                DownloadMessage.Queued(2),
                DownloadMessage.Queued(1),
                DownloadMessage.Failed(AppError.Forbidden("no")),
                DownloadMessage.Failed(AppError.SignedOut),
            ),
            seen,
        )
        job.cancel()
    }

    @Test
    fun marksSayHowMuchIsHere() {
        assertEquals(DownloadMark.NONE, downloadMark(emptyList(), setOf(1)))
        assertEquals(DownloadMark.NONE, downloadMark(listOf(1, 2), emptySet()))
        assertEquals(DownloadMark.SOME, downloadMark(listOf(1, 2), setOf(2)))
        assertEquals(DownloadMark.ALL, downloadMark(listOf(1, 2), setOf(1, 2, 3)))
    }
}

class OfflineAlbumViewModelTest : MainDispatcherTest() {
    @Test
    fun theAlbumsTracksInOrderPlayFromTheChosenOne() = runTest(dispatcher) {
        val tracks = listOf(dlTrack(3), dlTrack(1), dlTrack(2, trackNumber = null), dlTrack(9, albumId = 8))
        val repo = FakeOffline(OfflineState(albums = listOf(dlAlbum()), tracks = tracks))
        val player = FakeOfflinePlayer()
        val vm = OfflineAlbumViewModel(repo, player, "a7")
        assertEquals(listOf(101L, 103L, 102L), vm.state.value.tracks.map { it.trackId })
        assertEquals("Record", vm.state.value.album?.title)
        vm.play(1)
        vm.play(0, shuffle = true)
        vm.play(99)
        assertEquals(
            listOf("queue [101, 103, 102] 1 false", "queue [101, 103, 102] 0 true", "queue [101, 103, 102] 2 false"),
            player.calls,
        )
        vm.remove()
        assertEquals(listOf("removeAlbum a7"), repo.calls)
        // A track without an album id is its own "album".
        val loose = dlTrack(5, albumId = null)
        assertEquals("t${loose.key.take(16)}", OfflineAlbumViewModel.refOf(loose))
        // Gone from the device: nothing to play.
        repo.flow.update { OfflineState() }
        assertTrue(vm.state.value.tracks.isEmpty())
        vm.play()
        assertEquals(3, player.calls.size)
    }
}

class OfflinePlayerViewModelTest : MainDispatcherTest() {
    @Test
    fun controlsAndThePositionBetweenReports() = runTest(dispatcher) {
        var now = 1_000L
        val tracks = listOf(dlTrack(1), dlTrack(2))
        val player = FakeOfflinePlayer(OfflinePlayerState(status = dlStatus(), tracks = tracks.associateBy { it.key }))
        val vm = OfflinePlayerViewModel(player, { now })
        assertEquals(10_000L, vm.state.value.positionAt(1_000))
        assertEquals(12_500L, vm.state.value.positionAt(3_500))
        assertEquals("never past the end", 125_000L, vm.state.value.positionAt(1_000_000))
        vm.cycleRepeat()
        vm.toggleShuffle()
        vm.seek(30.5)
        vm.jumpTo(1)
        vm.next()
        vm.previous()
        vm.toggle()
        vm.stop()
        assertEquals(listOf("repeat ALL", "shuffle true", "seek 30500", "jump 1", "next", "previous", "pause", "stop"), player.calls)
        now = 5_000
        player.flow.value = OfflinePlayerState(status = dlStatus(playback = LocalPlayback.PAUSED, repeat = LocalRepeat.ONE))
        assertEquals(5_000L, vm.state.value.at)
        assertEquals("paused: it holds", 10_000L, vm.state.value.positionAt(99_000))
        vm.cycleRepeat()
        assertEquals("repeat OFF", player.calls.last())
        assertEquals(0L, OfflinePlayerUi().positionAt(5))
    }
}

class StorageSettingsViewModelTest : MainDispatcherTest() {
    @Test
    fun changesAreSanitisedAndSmartRefreshReportsErrors() = runTest(dispatcher) {
        val repo = FakeOffline()
        val vm = StorageSettingsViewModel(repo)
        vm.update { it.copy(capBytes = 5 * DownloadSettings.GB, wifiOnly = true) }
        assertEquals(5 * DownloadSettings.GB, repo.flow.value.settings.capBytes)
        assertTrue(repo.flow.value.settings.wifiOnly)
        vm.update { it.copy(smartRecentAlbums = 10_000) }
        assertEquals(DownloadSettings.MAX_RECENT, repo.flow.value.settings.smartRecentAlbums)
        repo.smartResult = ApiResult.Err(AppError.InvalidInput("smart downloads wait for Wi-Fi"))
        vm.refreshSmart()
        assertEquals(AppError.InvalidInput("smart downloads wait for Wi-Fi"), vm.error.value)
        assertFalse(vm.refreshing.value)
        repo.smartResult = ApiResult.Ok(Unit)
        vm.refreshSmart()
        assertNull(vm.error.value)
    }
}

class SignOutWithDownloadsTest : MainDispatcherTest() {
    @Test
    fun signingOutKeepsOrRemovesDownloadsFirst() = runTest(dispatcher) {
        val account = FakeAccount()
        val offline = FakeOffline(OfflineState(tracks = listOf(dlTrack(1)), usedBytes = 2_000_000))
        val vm = SettingsViewModel(account, null, offline)
        assertTrue(vm.state.first().downloadsOffered)
        assertEquals(1, vm.state.value.downloadedTracks)
        vm.signOut(removeDownloads = true)
        assertEquals(listOf("signOut true"), offline.calls)
        assertEquals(1, account.signedOut)
        val vm2 = SettingsViewModel(account, null, offline)
        vm2.signOut()
        assertEquals("signOut false", offline.calls.last())
        assertEquals(2, account.signedOut)
        // Without downloads offered, nothing extra.
        assertFalse(SettingsViewModel(account).state.value.downloadsOffered)
    }
}
