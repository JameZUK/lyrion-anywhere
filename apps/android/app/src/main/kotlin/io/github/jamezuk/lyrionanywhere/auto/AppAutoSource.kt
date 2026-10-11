package io.github.jamezuk.lyrionanywhere.auto

import io.github.jamezuk.lyrionanywhere.AppContainer
import io.github.jamezuk.lyrionanywhere.core.OfflineAlbum
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerRepository
import io.github.jamezuk.lyrionanywhere.data.offline.OfflineState
import io.github.jamezuk.lyrionanywhere.playback.auto.AutoSource
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineAlbumViewModel
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeoutOrNull

/**
 * The car's browse tree's view of the app: the same repositories the screens use, so the
 * gateway's policy, the phone's player (D20) and the downloads behave exactly as in the app.
 */
class AppAutoSource(
    private val auth: StateFlow<AuthState>,
    private val offlineMode: StateFlow<Boolean>,
    override val library: LibraryRepository,
    override val players: PlayersRepository,
    private val local: LocalPlayerRepository,
    private val offline: StateFlow<OfflineState>,
    private val offlinePlayer: OfflinePlayerRepository,
    private val settleMs: Long = SETTLE_MS,
    private val phoneMs: Long = PHONE_MS,
) : AutoSource {
    constructor(c: AppContainer) : this(
        c.account.state,
        c.offlineMode.offline,
        c.library,
        c.players,
        c.localPlayer,
        c.offline.state,
        c.offlinePlayer,
    )

    override fun online(): Boolean = auth.value is AuthState.SignedInState && !offlineMode.value

    override suspend fun settle() {
        withTimeoutOrNull(settleMs) {
            auth.first { it !is AuthState.Starting && it !is AuthState.Restoring }
        }
    }

    override fun downloadedAlbums(): List<OfflineAlbum> =
        offline.value.albums.sortedBy { it.title.lowercase() }

    override fun downloadedTracks(albumRef: String): List<OfflineTrack> =
        OfflineAlbumViewModel.albumOf(offline.value, albumRef).tracks

    override fun allDownloadedTracks(): List<OfflineTrack> = offline.value.tracks

    override fun hasQueue(): Boolean = offlinePlayer.state.value.active || local.state.value.running

    override suspend fun onPhone(command: suspend (playerId: String) -> ApiResult<Unit>): ApiResult<Unit> {
        if (!local.state.value.enabled) {
            // Chosen in the car: this phone plays it (D20), so its player is switched on.
            val r = local.setEnabled(true)
            if (r is ApiResult.Err) return r
        }
        val id = withTimeoutOrNull(phoneMs) { local.state.first { it.playerId != null }.playerId }
            ?: return ApiResult.Err(AppError.Offline("This phone's player isn't connected yet."))
        return command(id)
    }

    override fun playDownloads(tracks: List<OfflineTrack>, start: Int) = offlinePlayer.play(tracks, start)

    override suspend fun resume(): Boolean {
        if (offlinePlayer.state.value.active) {
            offlinePlayer.play()
            return true
        }
        if (!online()) return false
        return onPhone { players.play(it) } is ApiResult.Ok
    }

    companion object {
        /** How long a cold start waits for the saved sign-in. */
        const val SETTLE_MS = 8_000L

        /** How long a play request waits for this phone's player to connect. */
        const val PHONE_MS = 15_000L
    }
}
