package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.core.AccountInfo
import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.AlbumPage
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.ArtistPage
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.Favourite
import io.github.jamezuk.lyrionanywhere.core.FavouritePage
import io.github.jamezuk.lyrionanywhere.core.FolderEntry
import io.github.jamezuk.lyrionanywhere.core.FolderKind
import io.github.jamezuk.lyrionanywhere.core.FolderPage
import io.github.jamezuk.lyrionanywhere.core.Genre
import io.github.jamezuk.lyrionanywhere.core.GenrePage
import io.github.jamezuk.lyrionanywhere.core.LibraryFilter
import io.github.jamezuk.lyrionanywhere.core.PlaylistPage
import io.github.jamezuk.lyrionanywhere.core.SavedPlaylist
import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.core.VirtualLibrary
import io.github.jamezuk.lyrionanywhere.core.YearPage
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
import io.github.jamezuk.lyrionanywhere.core.TransferMethod
import io.github.jamezuk.lyrionanywhere.core.TransferOutcome
import io.github.jamezuk.lyrionanywhere.core.TransferStage
import io.github.jamezuk.lyrionanywhere.core.User
import io.github.jamezuk.lyrionanywhere.core.UserRole
import io.github.jamezuk.lyrionanywhere.data.AccountRepository
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AppSettings
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.EqCheck
import io.github.jamezuk.lyrionanywhere.data.EventsRepository
import io.github.jamezuk.lyrionanywhere.data.SoundSettings
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerStatus
import io.github.jamezuk.lyrionanywhere.core.NotBitPerfect
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.core.SignalPathInfo
import io.github.jamezuk.lyrionanywhere.core.SignalStage
import io.github.jamezuk.lyrionanywhere.core.SignalVerdict
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.PasswordChangeResult
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

fun sampleAlbum(id: Long, compilation: Boolean = false) =
    Album(id, "Album $id", "Artist", 3, 1999, "music/$id/cover", compilation)

fun sampleTrack(id: Long, index: Int? = null, disc: Int? = null, artist: String = "Artist") = Track(
    id, "Track $id", artist, "Album", 7, 3, 61.0, id.toUInt(), "music/$id/cover", index?.toUInt(),
    disc?.toUInt(), disc?.let { 2u }, "Jazz", "FLAC 24/96", 1999,
)

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

    /** What checking an address gives (by default [useServerResult], whatever the address). */
    var checkServer: (String) -> ApiResult<ServerInfo> = { useServerResult }

    override fun checkServerAddress(url: String) = checkServer(url)

    override suspend fun useServer(url: String) = useServerResult

    override suspend fun forgetServer() {}

    override suspend fun signIn(username: String, password: String, twoStepCode: String?): ApiResult<Unit> {
        signIns += Triple(username, password, twoStepCode)
        return signInResults.removeFirstOrNull() ?: ApiResult.Ok(Unit)
    }

    /**
     * A simplified stand-in for the Rust check: `lyrion-anywhere://pair?server=https://…&code=…`
     * (server optional, nothing else), or a bare code of 8+ letters and digits.
     */
    override fun readPairingCode(text: String): ApiResult<PairingInvite> {
        val bad = ApiResult.Err(AppError.InvalidInput("that isn't a valid pairing code"))
        fun code(c: String) = c.trim().uppercase().takeIf { it.length >= 8 && it.all { ch -> ch.isLetterOrDigit() && ch.code < 128 } }
        val t = text.trim()
        val prefix = "lyrion-anywhere://pair?"
        if (!t.startsWith(prefix)) return code(t)?.let { ApiResult.Ok(PairingInvite(null, it)) } ?: bad
        val params = t.removePrefix(prefix).split('&').map { it.substringBefore('=') to it.substringAfter('=', "") }
        val keys = params.map { it.first }
        if (keys.any { it != "server" && it != "code" } || keys.size != keys.toSet().size) return bad
        val server = params.firstOrNull { it.first == "server" }?.second
            ?.let { java.net.URLDecoder.decode(it, "UTF-8") }
        if (server != null && !server.matches(Regex("https://[a-z0-9.-]+(:\\d+)?/?"))) return bad
        val c = params.firstOrNull { it.first == "code" }?.second?.let(::code) ?: return bad
        return ApiResult.Ok(PairingInvite(server?.trimEnd('/')?.plus("/"), c))
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

    var accountInfo = AccountInfo("alice", false, true, false, true)

    override suspend fun account(): ApiResult<AccountInfo> = ApiResult.Ok(accountInfo)

    val passwordChanges = mutableListOf<List<Any?>>()
    var passwordResult: PasswordChangeResult = PasswordChangeResult.Changed(0, emptyList())

    override suspend fun changePassword(
        current: String,
        new: String,
        code: String?,
        signOutOthers: Boolean,
    ): PasswordChangeResult {
        passwordChanges += listOf(current, new, code, signOutOthers)
        return passwordResult
    }

    val renames = mutableListOf<String>()
    var renameResult: ApiResult<String>? = null

    override suspend fun renameDevice(name: String): ApiResult<String> {
        renames += name
        return renameResult ?: ApiResult.Ok(name)
    }

    /** Your devices: what the server holds; signing out removes from it. */
    var devices = mutableListOf<io.github.jamezuk.lyrionanywhere.core.MyDevice>()
    var devicesError: AppError? = null
    var signOutError: AppError? = null
    val signedOutDevices = mutableListOf<Long>()
    var othersSignedOut = 0

    override suspend fun myDevices(): ApiResult<List<io.github.jamezuk.lyrionanywhere.core.MyDevice>> =
        devicesError?.let { ApiResult.Err(it) } ?: ApiResult.Ok(devices.toList())

    override suspend fun signOutDevice(id: Long): ApiResult<Boolean> {
        signOutError?.let { return ApiResult.Err(it) }
        signedOutDevices += id
        return ApiResult.Ok(devices.removeAll { it.id == id && !it.current })
    }

    override suspend fun signOutOtherDevices(): ApiResult<Int> {
        signOutError?.let { return ApiResult.Err(it) }
        othersSignedOut++
        val n = devices.count { !it.current }
        devices.removeAll { !it.current }
        return ApiResult.Ok(n)
    }
}

class FakeLibrary(var totalAlbums: Int = 250) : LibraryRepository {
    val albumCalls = mutableListOf<Triple<Int, Int, AlbumSort>>()
    val albumFilters = mutableListOf<LibraryFilter>()
    val artistFilters = mutableListOf<LibraryFilter>()
    var fail: AppError? = null
    val searches = mutableListOf<String>()
    val genreQueries = mutableListOf<String?>()
    /** Roles the library has artists in. */
    var roles = setOf(ArtistRole.COMPOSER)
    var albumsByArtist = listOf(sampleAlbum(1), sampleAlbum(2), sampleAlbum(3, compilation = true))
    var albumArtistOf: List<Album>? = listOf(sampleAlbum(1))
    var tracks = listOf(sampleTrack(1), sampleTrack(2))
    var libs = emptyList<VirtualLibrary>()

    override suspend fun albums(start: Int, count: Int, sort: AlbumSort, search: String?, filter: LibraryFilter): ApiResult<AlbumPage> {
        albumCalls += Triple(start, count, sort)
        albumFilters += filter
        fail?.let { return ApiResult.Err(it) }
        val end = minOf(start + count, totalAlbums)
        val items = (start until end).map { sampleAlbum(it.toLong() + 1) }
        return ApiResult.Ok(AlbumPage(totalAlbums.toUInt(), items))
    }

    override suspend fun album(id: Long): ApiResult<Album?> = ApiResult.Ok(sampleAlbum(id))

    override suspend fun albumTracks(albumId: Long) = ApiResult.Ok(tracks)

    override suspend fun artists(start: Int, count: Int, search: String?, filter: LibraryFilter): ApiResult<ArtistPage> {
        artistFilters += filter
        fail?.let { return ApiResult.Err(it) }
        return ApiResult.Ok(ArtistPage(1u, listOf(Artist(3, "Artist"))))
    }

    override suspend fun hasRole(role: ArtistRole, libraryId: String?) = ApiResult.Ok(role in roles)

    override suspend fun artistAlbums(artistId: Long, role: ArtistRole?): ApiResult<List<Album>> = when (role) {
        null -> ApiResult.Ok(albumsByArtist)
        else -> albumArtistOf?.let { ApiResult.Ok(it) } ?: ApiResult.Err(AppError.Server(500, "x"))
    }

    override suspend fun genres(start: Int, count: Int, libraryId: String?, search: String?): ApiResult<GenrePage> {
        genreQueries += search
        return ApiResult.Ok(GenrePage(1u, listOf(Genre(4, "Jazz"))))
    }

    override suspend fun genre(id: Long) = ApiResult.Ok(Genre(id, "Jazz"))

    override suspend fun years(start: Int, count: Int, libraryId: String?) =
        ApiResult.Ok(YearPage(2u, listOf(2001u, 0u)))

    override suspend fun musicFolder(start: Int, count: Int, folderId: Long?) = ApiResult.Ok(
        FolderPage(2u, listOf(FolderEntry(12, "Jazz", FolderKind.FOLDER, null, null), FolderEntry(13, "One", FolderKind.TRACK, null, "1:01"))),
    )

    override suspend fun libraries(): ApiResult<List<VirtualLibrary>> = ApiResult.Ok(libs)

    override suspend fun favourites(start: Int, count: Int, itemId: String?) = ApiResult.Ok(
        FavouritePage("Favourites", 1u, listOf(Favourite("f0.0", "Radio", null, false))),
    )

    override suspend fun playlists(start: Int, count: Int) = ApiResult.Ok(PlaylistPage(1u, listOf(SavedPlaylist(5, "Road trip"))))

    override suspend fun playlistTracks(playlistId: Long) = ApiResult.Ok(tracks)

    /** What search answers (by default one artist). */
    var searchResult: SearchResults = SearchResults(listOf(Artist(3, "Artist")), emptyList(), emptyList())

    override suspend fun search(term: String, count: Int): ApiResult<SearchResults> {
        searches += term
        fail?.let { return ApiResult.Err(it) }
        return ApiResult.Ok(searchResult)
    }

    var credits = emptyList<io.github.jamezuk.lyrionanywhere.core.Credit>()
    val creditCalls = mutableListOf<Long>()

    override suspend fun trackCredits(trackId: Long): ApiResult<List<io.github.jamezuk.lyrionanywhere.core.Credit>> {
        creditCalls += trackId
        return ApiResult.Ok(credits)
    }

    var roleCounts = emptyList<io.github.jamezuk.lyrionanywhere.core.RoleCount>()
    var roleCountsFail: AppError? = null

    override suspend fun roleCounts(artistId: Long, libraryId: String?) =
        roleCountsFail?.let { ApiResult.Err(it) } ?: ApiResult.Ok(roleCounts)

    /** Biographies, photos and reviews (X5): none by default. */
    var artistInfo = io.github.jamezuk.lyrionanywhere.core.ArtistInfo(null, null, null)
    var albumInfo = io.github.jamezuk.lyrionanywhere.core.AlbumInfo(null)
    var infoFail: AppError? = null
    val infoCalls = mutableListOf<String>()

    override suspend fun artistInfo(artistId: Long): ApiResult<io.github.jamezuk.lyrionanywhere.core.ArtistInfo> {
        infoCalls += "artist $artistId"
        return infoFail?.let { ApiResult.Err(it) } ?: ApiResult.Ok(artistInfo)
    }

    override suspend fun albumInfo(albumId: Long): ApiResult<io.github.jamezuk.lyrionanywhere.core.AlbumInfo> {
        infoCalls += "album $albumId"
        return infoFail?.let { ApiResult.Err(it) } ?: ApiResult.Ok(albumInfo)
    }

    var index = emptyList<io.github.jamezuk.lyrionanywhere.core.IndexEntry>()
    val indexCalls = mutableListOf<String>()

    override suspend fun albumIndex(filter: LibraryFilter): ApiResult<List<io.github.jamezuk.lyrionanywhere.core.IndexEntry>> {
        indexCalls += "albums"
        return ApiResult.Ok(index)
    }

    override suspend fun artistIndex(filter: LibraryFilter): ApiResult<List<io.github.jamezuk.lyrionanywhere.core.IndexEntry>> {
        indexCalls += "artists"
        artistFilters += filter
        return ApiResult.Ok(index)
    }

    var totalTracks = 120
    val trackSearches = mutableListOf<Triple<String, Int, Int>>()

    override suspend fun searchTracks(term: String, start: Int, count: Int): ApiResult<io.github.jamezuk.lyrionanywhere.core.TrackPage> {
        trackSearches += Triple(term, start, count)
        val end = minOf(start + count, totalTracks)
        return ApiResult.Ok(
            io.github.jamezuk.lyrionanywhere.core.TrackPage(totalTracks.toUInt(), (start until end).map { sampleTrack(it.toLong() + 1) }),
        )
    }

    var playlistResults = listOf(SavedPlaylist(5, "Road trip"))
    val playlistSearches = mutableListOf<String>()

    override suspend fun searchPlaylists(term: String, start: Int, count: Int): ApiResult<PlaylistPage> {
        playlistSearches += term
        return ApiResult.Ok(PlaylistPage(playlistResults.size.toUInt(), playlistResults.drop(start).take(count)))
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

    var groups = emptyList<SyncGroup>()

    override suspend fun syncGroups() = ApiResult.Ok(groups)

    /** When set, status reads wait for it (a slow server). */
    var statusGate: kotlinx.coroutines.CompletableDeferred<Unit>? = null

    override suspend fun status(player: String): ApiResult<PlayerStatus> {
        statusCalls += player
        statusGate?.await()
        return statusResult
    }

    /** Rooms (Wave 2 §4.3): each player's own status, if set (else [statusResult]). */
    val roomStatus = mutableMapOf<String, PlayerStatus>()
    val statusesCalls = mutableListOf<List<String>>()

    override suspend fun statuses(players: List<String>): ApiResult<List<io.github.jamezuk.lyrionanywhere.core.RoomStatus>> {
        statusesCalls += players
        return ApiResult.Ok(
            players.map { id ->
                when (val r = roomStatus[id]?.let { ApiResult.Ok(it) } ?: statusResult) {
                    is ApiResult.Ok -> io.github.jamezuk.lyrionanywhere.core.RoomStatus(id, r.value, null)
                    is ApiResult.Err -> io.github.jamezuk.lyrionanywhere.core.RoomStatus(id, null, "unreadable")
                }
            },
        )
    }

    /** The queue, read a page at a time as LMS does. */
    var queueTracks: List<Track> = listOf(sampleTrack(1, 0), sampleTrack(2, 1))
    val queueCalls = mutableListOf<Pair<Int, Int>>()

    /** When set, queue reads fail with it. */
    var queueResult: ApiResult.Err? = null

    override suspend fun queue(player: String, start: Int, count: Int): ApiResult<List<Track>> {
        queueCalls += start to count
        queueResult?.let { return it }
        return ApiResult.Ok(queueTracks.drop(start).take(count))
    }

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

    override suspend fun setSleep(player: String, seconds: Int) = cmd("sleep $seconds $player")

    /** The queue after the current track, for "sleep at the end of …". */
    var upcoming: List<Track> = emptyList()
    val queueTimesCalls = mutableListOf<Pair<String, Int>>()

    override suspend fun queueTimes(player: String, from: Int): ApiResult<List<Track>> {
        queueTimesCalls += player to from
        return ApiResult.Ok(upcoming)
    }

    /** Called as each enqueue lands (to change the queue as LMS would). */
    var onEnqueue: (Selection, Placement) -> Unit = { _, _ -> }

    override suspend fun enqueue(player: String, selection: Selection, placement: Placement, playIndex: Int?): ApiResult<Unit> {
        val r = cmd("enqueue $selection $placement $playIndex $player")
        if (r is ApiResult.Ok) onEnqueue(selection, placement)
        return r
    }

    /** Removals take the entry out of [queueTracks] and shorten the status's queue. */
    var liveQueue = false

    /** Called after each removal (e.g. another controller changing the queue meanwhile). */
    var onRemove: (Int) -> Unit = {}

    override suspend fun playFavourite(player: String, itemId: String, placement: Placement) =
        cmd("favourite $itemId $placement $player")

    override suspend fun removeFromQueue(player: String, index: Int): ApiResult<Unit> {
        val r = cmd("remove $index $player")
        if (r is ApiResult.Ok && liveQueue) {
            queueTracks = queueTracks.filterIndexed { i, _ -> i != index }
            (statusResult as? ApiResult.Ok)?.value?.let { statusResult = ApiResult.Ok(it.copy(queueLength = queueTracks.size.toUInt())) }
        }
        onRemove(index)
        return r
    }

    override suspend fun moveInQueue(player: String, from: Int, to: Int): ApiResult<Unit> {
        val r = cmd("move $from $to $player")
        if (r is ApiResult.Ok && liveQueue) {
            queueTracks = io.github.jamezuk.lyrionanywhere.ui.player.QueueEdit.reorder(queueTracks, from, to)
            (statusResult as? ApiResult.Ok)?.value?.let { s ->
                val cur = s.queueIndex?.toInt()
                val now = cur?.let { io.github.jamezuk.lyrionanywhere.ui.player.QueueEdit.indexAfterMove(it, from, to) }
                statusResult = ApiResult.Ok(s.copy(queueIndex = now?.toUInt()))
            }
        }
        return r
    }

    override suspend fun clearQueue(player: String) = cmd("clear $player")

    var restoreResult: ApiResult<Unit> = ApiResult.Ok(Unit)

    override suspend fun restoreToQueue(player: String, trackId: Long, index: Int): ApiResult<Unit> {
        commands += "restore $trackId $index $player"
        return restoreResult
    }

    /** "Save as playlist": offered or not, and what each save answers. */
    var canSave: ApiResult<Boolean> = ApiResult.Ok(false)
    val saves = mutableListOf<Triple<String, String, Boolean>>()
    var saveAnswer: (name: String, replace: Boolean) -> ApiResult<io.github.jamezuk.lyrionanywhere.core.SaveQueueOutcome> =
        { name, _ -> ApiResult.Ok(io.github.jamezuk.lyrionanywhere.core.SaveQueueOutcome.Saved(9, name)) }

    override suspend fun canSaveQueue() = canSave

    override suspend fun saveQueue(player: String, name: String, replace: Boolean): ApiResult<io.github.jamezuk.lyrionanywhere.core.SaveQueueOutcome> {
        saves += Triple(player, name, replace)
        return saveAnswer(name, replace)
    }

    /** Holds setSynced until completed (to see what is shown meanwhile). */
    var syncGate: kotlinx.coroutines.CompletableDeferred<Unit>? = null

    override suspend fun setSynced(current: String, other: String, on: Boolean): ApiResult<Unit> {
        syncGate?.await()
        return cmd("sync $current $other $on")
    }

    /** Stages reported, then this result. A gate, if set, holds the transfer until completed. */
    var transferStages = listOf(TransferStage.READING, TransferStage.GROUPING, TransferStage.STARTING)
    var transferResult: ApiResult<TransferOutcome> = ApiResult.Ok(TransferOutcome.Moved(TransferMethod.GROUP))
    var transferGate: kotlinx.coroutines.CompletableDeferred<Unit>? = null
    val transfers = mutableListOf<Pair<String, String>>()

    override suspend fun transfer(source: String, target: String, onStage: (TransferStage) -> Unit): ApiResult<TransferOutcome> {
        transfers += source to target
        transferStages.forEach(onStage)
        transferGate?.await()
        return transferResult
    }
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

    override suspend fun setSound(sound: SoundSettings) = state.update { it.copy(sound = sound) }

    override suspend fun setLibrary(id: String?) = state.update { it.copy(libraryId = id) }

    override suspend fun setDefaultPlayer(choice: io.github.jamezuk.lyrionanywhere.data.DefaultPlayer, playerId: String?) =
        state.update { it.copy(defaultPlayer = choice, defaultPlayerId = playerId) }

    override suspend fun setSleepFade(seconds: Int) =
        state.update { it.copy(sleepFadeSecs = io.github.jamezuk.lyrionanywhere.data.sanitiseSleepFade(seconds)) }

    override suspend fun setLyricsPeek(on: Boolean) = state.update { it.copy(lyricsPeek = on) }

    override suspend fun setStreaming(streaming: io.github.jamezuk.lyrionanywhere.data.StreamingSettings) =
        state.update { it.copy(streaming = streaming) }

    override suspend fun setLibraryPrefs(prefs: io.github.jamezuk.lyrionanywhere.data.LibraryPrefs) =
        state.update { it.copy(library = prefs.sanitised()) }

    override suspend fun addRecentSearch(term: String) =
        state.update { it.copy(recentSearches = io.github.jamezuk.lyrionanywhere.data.RecentSearches.add(it.recentSearches, term)) }

    override suspend fun removeRecentSearch(term: String) =
        state.update { it.copy(recentSearches = io.github.jamezuk.lyrionanywhere.data.RecentSearches.remove(it.recentSearches, term)) }

    override suspend fun clearRecentSearches() = state.update { it.copy(recentSearches = emptyList()) }

    override suspend fun setNowPlayingPrefs(prefs: io.github.jamezuk.lyrionanywhere.data.NowPlayingPrefs) =
        state.update { it.copy(nowPlaying = prefs) }
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
    direct: Boolean = bitPerfect,
    reasons: List<NotBitPerfect> = if (bitPerfect) emptyList() else listOf(NotBitPerfect.NO_USB_DAC),
) = SignalPathInfo(
    sourceCodec = "FLAC",
    sourceLossless = true,
    sourceBits = 24u,
    sourceRate = 96_000u,
    sourceChannels = 2u,
    stages = stages,
    dspActive = false,
    outputDevice = if (direct) "Example DAC" else "Headphones",
    outputRate = 96_000u,
    outputFormat = if (direct) "24-bit integer" else "32-bit float",
    outputBits = if (direct) 24u else null,
    direct = direct,
    fixedVolume = direct,
    resampled = verdict == SignalVerdict.RESAMPLED,
    engineBitPerfect = engineBitPerfect,
    bitPerfect = bitPerfect,
    notBitPerfect = reasons,
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

    /** The sleep fades asked of the engine: (seconds to the end, or null to cancel; fade). */
    val fades = mutableListOf<Pair<Double?, Int>>()

    override fun setSleepFade(endsInSecs: Double?, fadeSecs: Int) {
        fades += endsInSecs to fadeSecs
    }

    override fun setOutputRoute(route: OutputRoute) {}

    val sounds = mutableListOf<SoundSettings>()

    override suspend fun setSound(sound: SoundSettings) {
        sounds += sound
        state.update { it.copy(sound = sound) }
    }

    /** The visualiser switches asked of the engine, in order; and the next frame it hands over. */
    val visual = mutableListOf<Boolean>()
    var frame: io.github.jamezuk.lyrionanywhere.data.VisualFrame? = null

    override fun setVisualiser(on: Boolean) {
        visual += on
    }

    override fun visualFrame() = frame

    /** What the engine says about a profile: anything with "Filter" in it is good. */
    override fun checkEq(text: String): EqCheck =
        if ("Filter" in text) EqCheck.Ok(text.lines().count { "Filter" in it }, -3.0, false) else EqCheck.Invalid("line 1: not a profile")
}
