package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.core.AccountInfo
import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.AlbumPage
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.ArtistPage
import io.github.jamezuk.lyrionanywhere.core.Device
import io.github.jamezuk.lyrionanywhere.core.PairingInvite
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.core.SearchResults
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.core.SignedIn
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.core.User
import io.github.jamezuk.lyrionanywhere.core.UserRole
import io.github.jamezuk.lyrionanywhere.data.AccountRepository
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AppSettings
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.EventsRepository
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerStatus
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.core.SignalPathInfo
import io.github.jamezuk.lyrionanywhere.core.SignalStage
import io.github.jamezuk.lyrionanywhere.core.SignalVerdict
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import io.github.jamezuk.lyrionanywhere.data.ServerInfo
import io.github.jamezuk.lyrionanywhere.data.SettingsStore
import io.github.jamezuk.lyrionanywhere.data.StreamUpdate
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update

val SERVER = ServerInfo("https://music.example.com/", "music.example.com", true)
val WHO = SignedIn(User("alice", UserRole.LISTENER), Device(1, "Phone", "02:4c:41:00:00:01"))

fun sampleAlbum(id: Long) = Album(id, "Album $id", "Artist", 3, 1999, "music/$id/cover")

fun sampleTrack(id: Long, index: Int? = null) =
    Track(id, "Track $id", "Artist", "Album", 7, 3, 61.0, id.toUInt(), "music/$id/cover", index?.toUInt())

fun samplePlayer(id: String, playing: Boolean = false) = Player(id, "Player $id", null, true, true, playing)

fun sampleStatus(mode: PlaybackMode = PlaybackMode.PLAYING, elapsed: Double = 10.0) = PlayerStatus(
    name = "Kitchen",
    connected = true,
    powered = true,
    mode = mode,
    elapsedSecs = elapsed,
    durationSecs = 200.0,
    volume = 40u,
    muted = false,
    shuffle = ShuffleMode.OFF,
    repeat = RepeatMode.OFF,
    queueIndex = 0u,
    queueLength = 3u,
    canSeek = true,
    current = sampleTrack(1, 0),
)

class FakeAccount : AccountRepository {
    val _state = MutableStateFlow<AuthState>(AuthState.SignedOut(SERVER))
    override val state: StateFlow<AuthState> = _state
    val signIns = mutableListOf<Triple<String, String, String?>>()
    var signInResults = ArrayDeque<ApiResult<Unit>>()
    var useServerResult: ApiResult<ServerInfo> = ApiResult.Ok(SERVER)
    val paired = mutableListOf<PairingInvite>()
    var pairResult: ApiResult<Unit> = ApiResult.Ok(Unit)
    var signedOut = 0

    override suspend fun start() {}

    override fun checkServerAddress(url: String) = useServerResult

    override suspend fun useServer(url: String) = useServerResult

    override suspend fun forgetServer() {}

    override suspend fun signIn(username: String, password: String, twoStepCode: String?): ApiResult<Unit> {
        signIns += Triple(username, password, twoStepCode)
        return signInResults.removeFirstOrNull() ?: ApiResult.Ok(Unit)
    }

    /** Accepts `lyrion-anywhere://pair?…` style text or a code of 8+ letters and digits. */
    override fun readPairingCode(text: String): ApiResult<PairingInvite> {
        val code = text.trim().uppercase()
        return if (code.length >= 8 && code.all { it.isLetterOrDigit() }) {
            ApiResult.Ok(PairingInvite(null, code))
        } else {
            ApiResult.Err(AppError.InvalidInput("that isn't a valid pairing code"))
        }
    }

    override suspend fun pair(invite: PairingInvite): ApiResult<Unit> {
        paired += invite
        return pairResult
    }

    override suspend fun retry() {}

    override suspend fun signOut(): Boolean {
        signedOut++
        return true
    }

    override suspend fun account(): ApiResult<AccountInfo> = ApiResult.Ok(AccountInfo("alice", false, true, false))
}

class FakeLibrary(var totalAlbums: Int = 250) : LibraryRepository {
    val albumCalls = mutableListOf<Triple<Int, Int, AlbumSort>>()
    var fail: AppError? = null
    val searches = mutableListOf<String>()

    override suspend fun albums(start: Int, count: Int, sort: AlbumSort, search: String?): ApiResult<AlbumPage> {
        albumCalls += Triple(start, count, sort)
        fail?.let { return ApiResult.Err(it) }
        val end = minOf(start + count, totalAlbums)
        val items = (start until end).map { sampleAlbum(it.toLong() + 1) }
        return ApiResult.Ok(AlbumPage(totalAlbums.toUInt(), items))
    }

    override suspend fun album(id: Long): ApiResult<Album?> = ApiResult.Ok(sampleAlbum(id))

    override suspend fun albumTracks(albumId: Long) = ApiResult.Ok(listOf(sampleTrack(1), sampleTrack(2)))

    override suspend fun artists(start: Int, count: Int, search: String?) =
        ApiResult.Ok(ArtistPage(1u, listOf(Artist(3, "Artist"))))

    override suspend fun artistAlbums(artistId: Long): ApiResult<List<Album>> = ApiResult.Ok(listOf(sampleAlbum(1)))

    override suspend fun search(term: String, count: Int): ApiResult<SearchResults> {
        searches += term
        return ApiResult.Ok(SearchResults(listOf(Artist(3, "Artist")), emptyList(), emptyList()))
    }
}

class FakePlayers : PlayersRepository {
    var players = listOf(samplePlayer("aa:00:00:00:00:01"), samplePlayer("aa:00:00:00:00:02", playing = true))
    var statusResult: ApiResult<PlayerStatus> = ApiResult.Ok(sampleStatus())
    val statusCalls = mutableListOf<String>()
    val commands = mutableListOf<String>()
    var commandResult: ApiResult<Unit> = ApiResult.Ok(Unit)

    private fun cmd(s: String): ApiResult<Unit> {
        commands += s
        return commandResult
    }

    override suspend fun list() = ApiResult.Ok(players)

    override suspend fun status(player: String): ApiResult<PlayerStatus> {
        statusCalls += player
        return statusResult
    }

    override suspend fun queue(player: String, start: Int, count: Int) =
        ApiResult.Ok(listOf(sampleTrack(1, 0), sampleTrack(2, 1)))

    override suspend fun play(player: String) = cmd("play $player")

    override suspend fun setPaused(player: String, paused: Boolean) = cmd("pause $paused $player")

    override suspend fun next(player: String) = cmd("next $player")

    override suspend fun previous(player: String) = cmd("previous $player")

    override suspend fun jumpTo(player: String, index: Int) = cmd("jump $index $player")

    override suspend fun seek(player: String, seconds: Double) = cmd("seek $seconds $player")

    override suspend fun setVolume(player: String, volume: Int) = cmd("volume $volume $player")

    override suspend fun setShuffle(player: String, mode: ShuffleMode) = cmd("shuffle $mode $player")

    override suspend fun setRepeat(player: String, mode: RepeatMode) = cmd("repeat $mode $player")

    override suspend fun setPower(player: String, on: Boolean) = cmd("power $on $player")

    override suspend fun enqueue(player: String, selection: Selection, placement: Placement, playIndex: Int?) =
        cmd("enqueue $selection $placement $playIndex $player")
}

class FakeEvents : EventsRepository {
    val flow = MutableSharedFlow<StreamUpdate>(extraBufferCapacity = 16)
    var collectors = 0

    override fun updates(): Flow<StreamUpdate> = kotlinx.coroutines.flow.flow {
        collectors++
        try {
            flow.collect { emit(it) }
        } finally {
            collectors--
        }
    }
}

class FakeSettings(initial: AppSettings = AppSettings()) : SettingsStore {
    val state = MutableStateFlow(initial)
    override val settings: Flow<AppSettings> = state

    override suspend fun setServerUrl(url: String?) = state.update { it.copy(serverUrl = url) }

    override suspend fun setSelectedPlayer(playerId: String?) = state.update { it.copy(selectedPlayerId = playerId) }

    override suspend fun setPlayOnPhone(on: Boolean) = state.update { it.copy(playOnPhone = on) }
}

fun localStatus(id: String, connection: PlayerConnection = PlayerConnection.Connected) = LocalPlayerStatus(
    connection = connection,
    playback = LocalPlayback.STOPPED,
    playerId = id,
    name = "Test phone",
    elapsedMs = 0u,
    bufferedMs = 0u,
    outputRate = 0u,
    underruns = 0u,
    underrunFrames = 0u,
    held = false,
    lastError = null,
)

fun signalPath(
    verdict: SignalVerdict = SignalVerdict.BIT_PERFECT,
    engineBitPerfect: Boolean = true,
    bitPerfect: Boolean = false,
    stages: List<SignalStage> = listOf(SignalStage("Decode (lossless)", true, false)),
) = SignalPathInfo(
    sourceCodec = "FLAC",
    sourceLossless = true,
    sourceBits = 24u,
    sourceRate = 96_000u,
    sourceChannels = 2u,
    stages = stages,
    dspActive = false,
    outputDevice = "Headphones",
    outputRate = 96_000u,
    outputFormat = "32-bit float",
    resampled = verdict == SignalVerdict.RESAMPLED,
    engineBitPerfect = engineBitPerfect,
    bitPerfect = bitPerfect,
    verdict = verdict,
    summary = "",
)

class FakeLocalPlayer : LocalPlayerRepository {
    override val state = MutableStateFlow(LocalPlayerState())
    var enableResult: ApiResult<Unit> = ApiResult.Ok(Unit)
    val enabled = mutableListOf<Boolean>()

    override suspend fun setEnabled(on: Boolean): ApiResult<Unit> {
        enabled += on
        if (enableResult is ApiResult.Ok) state.update { it.copy(enabled = on) }
        return enableResult
    }

    override fun setHold(hold: Boolean) {}

    override fun setDuck(duck: Boolean) {}

    override fun setOutputRoute(route: OutputRoute) {}
}
