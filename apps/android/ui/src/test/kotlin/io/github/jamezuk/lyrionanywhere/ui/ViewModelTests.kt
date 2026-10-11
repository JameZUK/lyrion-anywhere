package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.EventsState
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.ServerEvent
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AppSettings
import io.github.jamezuk.lyrionanywhere.data.StreamUpdate
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.PagedViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.SearchViewModel
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairViewModel
import io.github.jamezuk.lyrionanywhere.ui.onboarding.ServerViewModel
import io.github.jamezuk.lyrionanywhere.ui.onboarding.SignInViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerMessage
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
abstract class MainDispatcherTest {
    protected val dispatcher = UnconfinedTestDispatcher()

    @Before
    fun setMainDispatcher() = Dispatchers.setMain(dispatcher)

    @After
    fun resetMainDispatcher() = Dispatchers.resetMain()
}

class SignInViewModelTest : MainDispatcherTest() {
    @Test
    fun asksForATwoStepCodeThenSendsIt() = runTest(dispatcher) {
        val account = FakeAccount().apply { signInResults.add(ApiResult.Err(AppError.TwoStepRequired)) }
        val vm = SignInViewModel(account)
        vm.onUsernameChange(" alice ")
        vm.onPasswordChange("pw")
        assertTrue(vm.state.value.canSubmit)
        vm.submit()
        assertTrue(vm.state.value.needsCode)
        assertNull(vm.state.value.error)
        // The code field accepts digits only, six at most; until then, no submit.
        vm.onCodeChange("12a3")
        assertFalse(vm.state.value.canSubmit)
        vm.onCodeChange("123 456 7")
        assertEquals("123456", vm.state.value.code)
        vm.submit()
        assertEquals(Triple("alice", "pw", "123456"), account.signIns.last())
        assertEquals(null, account.signIns.first().third)
        // Signed in: the password is gone from memory.
        assertEquals("", vm.state.value.password)
    }

    @Test
    fun wrongDetailsAndRateLimitsAreShown() = runTest(dispatcher) {
        val account = FakeAccount().apply {
            signInResults.add(ApiResult.Err(AppError.SignedOut))
            signInResults.add(ApiResult.Err(AppError.RateLimited(900)))
        }
        val vm = SignInViewModel(account)
        vm.onUsernameChange("alice")
        vm.onPasswordChange("wrong")
        vm.submit()
        assertEquals(AppError.SignedOut, vm.state.value.error)
        assertFalse(vm.state.value.busy)
        vm.submit()
        assertEquals(AppError.RateLimited(900), vm.state.value.error)
        // Typing clears the error.
        vm.onPasswordChange("x")
        assertNull(vm.state.value.error)
    }

    @Test
    fun nothingIsSentWithoutDetails() = runTest(dispatcher) {
        val account = FakeAccount()
        val vm = SignInViewModel(account)
        vm.submit()
        vm.onUsernameChange("   ")
        vm.onPasswordChange("pw")
        vm.submit()
        assertTrue(account.signIns.isEmpty())
    }
}

class ServerViewModelTest : MainDispatcherTest() {
    @Test
    fun refusedAddressesShowWhy() = runTest(dispatcher) {
        val account = FakeAccount().apply {
            useServerResult = ApiResult.Err(AppError.InvalidInput("use https://"))
        }
        val vm = ServerViewModel(account)
        assertEquals(SERVER.url, vm.state.value.address)
        vm.onAddressChange("http://music.example.com")
        var done = false
        vm.submit { done = true }
        assertFalse(done)
        assertEquals(AppError.InvalidInput("use https://"), vm.state.value.error)
        account.useServerResult = ApiResult.Ok(SERVER)
        vm.submit { done = true }
        assertTrue(done)
    }
}

class PairViewModelTest : MainDispatcherTest() {
    @Test
    fun garbageIsRefusedWithoutContactingTheServer() = runTest(dispatcher) {
        val account = FakeAccount()
        val vm = PairViewModel(account)
        vm.onScanned("https://evil.example/phish")
        assertTrue(account.paired.isEmpty())
        assertTrue(vm.state.value.error is AppError.InvalidInput)
        assertTrue(vm.state.value.scanning)
    }

    @Test
    fun aScannedCodePairsOnce() = runTest(dispatcher) {
        val account = FakeAccount().apply { pairResult = ApiResult.Err(AppError.SignedOut) }
        val vm = PairViewModel(account)
        vm.onScanned("abcd2345")
        assertEquals("ABCD2345", account.paired.single().code)
        // Refused: scanning resumes so another code can be tried.
        assertEquals(AppError.SignedOut, vm.state.value.error)
        assertTrue(vm.state.value.scanning)
        account.pairResult = ApiResult.Ok(Unit)
        vm.onCodeChange("efgh2345")
        vm.submit()
        assertEquals(2, account.paired.size)
        // Paired: the scanner stops, so the same QR code isn't redeemed again.
        assertFalse(vm.state.value.scanning)
        vm.onScanned("abcd2345")
        assertEquals(2, account.paired.size)
    }
}

class AlbumsViewModelTest : MainDispatcherTest() {
    @Test
    fun pagesUntilTheEnd() = runTest(dispatcher) {
        val library = FakeLibrary(totalAlbums = 250)
        val vm = AlbumsViewModel(library)
        assertEquals(100, vm.state.value.items.size)
        vm.loadMore()
        vm.loadMore()
        assertEquals(250, vm.state.value.items.size)
        assertTrue(vm.state.value.endReached)
        vm.loadMore()
        assertEquals(3, library.albumCalls.size)
        assertEquals(listOf(0, 100, 200), library.albumCalls.map { it.first })
        assertTrue(library.albumCalls.all { it.second == PagedViewModel.PAGE_SIZE })
    }

    @Test
    fun aShortPageEndsTheListWhateverTheCountSays() = runTest(dispatcher) {
        val library = FakeLibrary(totalAlbums = 30)
        val vm = AlbumsViewModel(library)
        assertTrue(vm.state.value.endReached)
        assertEquals(30, vm.state.value.items.size)
    }

    @Test
    fun errorsCanBeRetriedAndSortingStartsOver() = runTest(dispatcher) {
        val library = FakeLibrary().apply { fail = AppError.Offline() }
        val vm = AlbumsViewModel(library)
        assertTrue(vm.state.value.failedEmpty)
        library.fail = null
        vm.retry()
        assertEquals(100, vm.state.value.items.size)
        vm.setSort(AlbumSort.NEWEST)
        assertEquals(AlbumSort.NEWEST, library.albumCalls.last().third)
        assertEquals(0, library.albumCalls.last().first)
        assertEquals(100, vm.state.value.items.size)
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class SearchViewModelTest : MainDispatcherTest() {
    @Test
    fun searchesAfterAPauseAndIgnoresTinyQueries() = runTest(dispatcher) {
        val library = FakeLibrary()
        val vm = SearchViewModel(library)
        vm.onQueryChange("a")
        advanceTimeBy(1000)
        assertTrue(library.searches.isEmpty())
        vm.onQueryChange("ab")
        vm.onQueryChange("abb")
        vm.onQueryChange("abba")
        advanceTimeBy(SearchViewModel.DEBOUNCE_MS + 1)
        assertEquals(listOf("abba"), library.searches)
        assertEquals("Artist", vm.state.value.results!!.artists.single().name)
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class PlayerViewModelTest : MainDispatcherTest() {
    private var now = 1_000_000L

    private fun vm(players: FakePlayers, events: FakeEvents, settings: FakeSettings) =
        PlayerViewModel(players, events, settings) { now }

    @Test
    fun choosesTheSavedPlayerOrAPlayingOne() = runTest(dispatcher) {
        val players = FakePlayers()
        val settings = FakeSettings()
        val vm = vm(players, FakeEvents(), settings)
        vm.start()
        // Nothing saved: the playing one, which is then remembered.
        assertEquals("aa:00:00:00:00:02", vm.state.value.selectedId)
        assertEquals("aa:00:00:00:00:02", settings.state.value.selectedPlayerId)
        vm.select("aa:00:00:00:00:01")
        assertEquals("aa:00:00:00:00:01", settings.state.value.selectedPlayerId)
        vm.stop()

        val saved = vm(players, FakeEvents(), FakeSettings(AppSettings(selectedPlayerId = "aa:00:00:00:00:01")))
        saved.start()
        assertEquals("aa:00:00:00:00:01", saved.state.value.selectedId)
        saved.stop()
    }

    @Test
    fun eventsForTheSelectedPlayerRefreshItsStatus() = runTest(dispatcher) {
        val players = FakePlayers()
        val events = FakeEvents()
        val vm = vm(players, events, FakeSettings())
        vm.start()
        val selected = vm.state.value.selectedId!!
        events.flow.emit(StreamUpdate.State(EventsState.Live))
        assertTrue(vm.state.value.live)
        val before = players.statusCalls.size
        // A burst of events is coalesced into one refresh.
        repeat(3) { events.flow.emit(StreamUpdate.Event(ServerEvent.PlayerChanged(selected, "mixer", listOf("volume")))) }
        advanceTimeBy(PlayerViewModel.EVENT_COALESCE_MS + 1)
        assertEquals(before + 1, players.statusCalls.size)
        // Another player's events don't.
        events.flow.emit(StreamUpdate.Event(ServerEvent.PlayerChanged("bb:00:00:00:00:09", "play", emptyList())))
        advanceTimeBy(PlayerViewModel.EVENT_COALESCE_MS + 1)
        assertEquals(before + 1, players.statusCalls.size)
        // Stopping closes the stream.
        vm.stop()
        runCurrent()
        assertEquals(0, events.collectors)
        assertFalse(vm.state.value.live)
    }

    @Test
    fun pollsWhileTheStreamIsDown() = runTest(dispatcher) {
        val players = FakePlayers()
        val vm = vm(players, FakeEvents(), FakeSettings())
        vm.start()
        val before = players.statusCalls.size
        advanceTimeBy(PlayerViewModel.FAST_POLL_MS * 2 + 1)
        assertEquals(before + 2, players.statusCalls.size)
        vm.stop()
    }

    @Test
    fun transportIsOptimisticAndReportsFailures() = runTest(dispatcher) {
        val players = FakePlayers()
        val vm = vm(players, FakeEvents(), FakeSettings())
        vm.start()
        val id = vm.state.value.selectedId!!
        assertTrue(vm.state.value.playing)
        // Pause shows at once, even before the server's status comes back.
        players.statusResult = ApiResult.Ok(sampleStatus(mode = PlaybackMode.PAUSED))
        vm.togglePlay()
        assertEquals("pause true $id", players.commands.last())
        assertEquals(PlaybackMode.PAUSED, vm.state.value.status!!.mode)

        players.commandResult = ApiResult.Err(AppError.Forbidden("no"))
        val messages = mutableListOf<PlayerMessage>()
        val collector = backgroundScope.launch { vm.messages.collect { messages += it } }
        vm.next()
        runCurrent()
        assertEquals(PlayerMessage.Failed(AppError.Forbidden("no")), messages.last())
        collector.cancel()
        vm.stop()
    }

    @Test
    fun seekingStartsPlaybackWhenPausedOrStopped() = runTest(dispatcher) {
        for ((mode, expected) in listOf(
            PlaybackMode.PLAYING to listOf("seek 42.0"),
            PlaybackMode.PAUSED to listOf("seek 42.0", "pause false"),
            PlaybackMode.STOPPED to listOf("play", "seek 42.0"),
        )) {
            val players = FakePlayers()
            players.statusResult = ApiResult.Ok(sampleStatus(mode = mode))
            val vm = vm(players, FakeEvents(), FakeSettings())
            vm.start()
            runCurrent()
            val id = vm.state.value.selectedId!!
            val before = players.commands.size
            vm.seek(42.0)
            runCurrent()
            vm.stop()
            assertEquals("from $mode", expected.map { "$it $id" }, players.commands.drop(before))
        }
    }

    @Test
    fun aFailedSeekDoesNotResume() = runTest(dispatcher) {
        val players = FakePlayers()
        players.statusResult = ApiResult.Ok(sampleStatus(mode = PlaybackMode.PAUSED))
        val vm = vm(players, FakeEvents(), FakeSettings())
        vm.start()
        runCurrent()
        players.commandResult = ApiResult.Err(AppError.Forbidden("no"))
        val before = players.commands.size
        vm.seek(10.0)
        runCurrent()
        vm.stop()
        assertEquals("only the seek was tried", 1, players.commands.size - before)
    }

    @Test
    fun enqueueNeedsAPlayer() = runTest(dispatcher) {
        val players = FakePlayers().apply { players = emptyList() }
        val vm = vm(players, FakeEvents(), FakeSettings())
        vm.start()
        vm.enqueue(Selection.Album(7), Placement.PLAY_NOW)
        assertEquals(PlayerMessage.NoPlayer, vm.messages.first())
        assertTrue(players.commands.isEmpty())
        vm.stop()
    }

    @Test
    fun aRefusedPlayOnPhoneIsReported() = runTest(dispatcher) {
        val local = FakeLocalPlayer().apply { enableResult = ApiResult.Err(AppError.SignedOut) }
        val vm = PlayerViewModel(FakePlayers(), FakeEvents(), FakeSettings(), local) { now }
        val messages = mutableListOf<PlayerMessage>()
        val collector = backgroundScope.launch { vm.messages.collect { messages += it } }
        vm.setPlayOnPhone(true)
        runCurrent()
        assertEquals(PlayerMessage.Failed(AppError.SignedOut), messages.single())
        assertFalse(vm.state.value.local.enabled)
        collector.cancel()
    }

    @Test
    fun thePositionTicksOnlyWhilePlaying() {
        val playing = PlayerUiState(status = sampleStatus(PlaybackMode.PLAYING, elapsed = 10.0), statusAt = 1000)
        assertEquals(15.0, PlayerViewModel.elapsedAt(playing, 6000), 0.001)
        // Never past the end of the track.
        assertEquals(200.0, PlayerViewModel.elapsedAt(playing, 10_000_000), 0.001)
        val paused = playing.copy(status = sampleStatus(PlaybackMode.PAUSED, elapsed = 10.0))
        assertEquals(10.0, PlayerViewModel.elapsedAt(paused, 6000), 0.001)
        assertEquals(0.0, PlayerViewModel.elapsedAt(PlayerUiState(), 6000), 0.001)
    }
}
