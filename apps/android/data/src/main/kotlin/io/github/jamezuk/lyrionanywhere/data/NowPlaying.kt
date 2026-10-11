package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.update

/** What a player is playing, in brief: for the home-screen widget and the Quick Settings tile. */
data class NowPlaying(
    val playerId: String,
    val playerName: String,
    /** Empty when nothing is queued. */
    val title: String,
    val artist: String?,
    /** An artwork key for the Art API, if the track has one. */
    val artwork: String?,
    val playing: Boolean,
    /** This phone's own player. */
    val isPhone: Boolean,
) {
    companion object {
        /** From a player's status (null: nothing known about it). */
        fun from(playerId: String, playerName: String, status: PlayerStatus?, isPhone: Boolean): NowPlaying? {
            status ?: return null
            val track = status.current
            return NowPlaying(
                playerId = playerId,
                playerName = playerName.ifEmpty { status.name },
                title = track?.title.orEmpty(),
                artist = track?.artist,
                artwork = track?.artwork,
                playing = status.mode == PlaybackMode.PLAYING,
                isPhone = isPhone,
            )
        }
    }
}

/**
 * The latest known playback state, shared by the app, the playback service, the widget and
 * the tile. It is only ever fed by work that runs anyway (the playback service while the
 * phone plays, the UI while it is visible): nothing here polls or keeps the app awake.
 */
class NowPlayingHub {
    private val _phone = MutableStateFlow<NowPlaying?>(null)
    private val _selected = MutableStateFlow<NowPlaying?>(null)

    /** This phone's player, from the playback service. */
    val phone: StateFlow<NowPlaying?> = _phone.asStateFlow()

    /** The player the app controls, from the UI. */
    val selected: StateFlow<NowPlaying?> = _selected.asStateFlow()

    /** What the widget shows. */
    val shown: Flow<NowPlaying?> = combine(_phone, _selected, ::pick).distinctUntilChanged()

    fun publishPhone(n: NowPlaying?) {
        _phone.value = n
    }

    fun publishSelected(n: NowPlaying?) {
        _selected.value = n
    }

    /** After a play/pause sent from the widget or tile, before LMS confirms it. */
    fun setPlaying(playerId: String, playing: Boolean) {
        _phone.update { if (it?.playerId == playerId) it.copy(playing = playing) else it }
        _selected.update { if (it?.playerId == playerId) it.copy(playing = playing) else it }
    }

    /** Signed out: forget everything. */
    fun clear() {
        _phone.value = null
        _selected.value = null
    }

    companion object {
        /**
         * This phone while it plays; otherwise the player the app last controlled (the
         * user's choice); otherwise the phone.
         */
        fun pick(phone: NowPlaying?, selected: NowPlaying?): NowPlaying? = when {
            phone?.playing == true -> phone
            selected != null -> selected
            else -> phone
        }
    }
}
