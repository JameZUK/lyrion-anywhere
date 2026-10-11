package io.github.jamezuk.lyrionanywhere.widget

import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.NowPlaying
import io.github.jamezuk.lyrionanywhere.data.NowPlayingHub
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository

/** What the home-screen widget shows, worked out from [NowPlaying] alone (no Android). */
data class WidgetModel(
    val kind: Kind,
    val title: String,
    val artist: String?,
    val playerName: String,
    val playerId: String?,
    val artwork: String?,
    val playing: Boolean,
) {
    enum class Kind {
        /** Nothing known (signed out, or the app hasn't run yet): tap to open the app. */
        NOTHING,

        /** A player is known but has nothing queued. */
        IDLE,

        /** Something to show and control. */
        TRACK,
    }

    /** Play/pause and next are offered only with something queued on a known player. */
    val showsControls: Boolean get() = kind == Kind.TRACK && playerId != null

    companion object {
        fun from(n: NowPlaying?): WidgetModel {
            if (n == null) return WidgetModel(Kind.NOTHING, "", null, "", null, null, false)
            val kind = if (n.title.isBlank()) Kind.IDLE else Kind.TRACK
            return WidgetModel(
                kind = kind,
                title = n.title.take(MAX_TEXT),
                artist = n.artist?.take(MAX_TEXT),
                playerName = n.playerName.take(MAX_TEXT),
                playerId = n.playerId,
                artwork = n.artwork.takeIf { kind == Kind.TRACK },
                playing = n.playing && kind == Kind.TRACK,
            )
        }

        private const val MAX_TEXT = 120
    }
}

/** What the Quick Settings tile shows for this phone's player. */
data class TileModel(val state: State, val title: String?, val playing: Boolean) {
    enum class State {
        /** This phone isn't playing as a player (switched off, or signed out). */
        UNAVAILABLE,

        /** Ready; paused, stopped or nothing queued. */
        INACTIVE,

        /** Playing. */
        ACTIVE,
    }

    companion object {
        fun from(phone: NowPlaying?, phoneEnabled: Boolean): TileModel = when {
            !phoneEnabled || phone == null -> TileModel(State.UNAVAILABLE, null, false)
            phone.playing -> TileModel(State.ACTIVE, phone.title.ifBlank { null }, true)
            else -> TileModel(State.INACTIVE, phone.title.ifBlank { null }, false)
        }
    }
}

enum class Control { TOGGLE, NEXT }

/**
 * Runs the widget's and tile's buttons through the same repository as the app (so the
 * gateway's policy applies), and shows the change at once.
 */
class Controls(private val players: () -> PlayersRepository, private val hub: NowPlayingHub) {
    suspend fun run(playerId: String, control: Control): ApiResult<Unit> {
        if (!isPlayerId(playerId)) return ApiResult.Err(AppError.InvalidInput("bad player id"))
        val p = players()
        return when (control) {
            Control.NEXT -> p.next(playerId)
            Control.TOGGLE -> {
                val playing = listOf(hub.phone.value, hub.selected.value).firstOrNull { it?.playerId == playerId }?.playing == true
                val r = if (playing) p.setPaused(playerId, true) else p.play(playerId)
                if (r is ApiResult.Ok) hub.setPlaying(playerId, !playing)
                r
            }
        }
    }

    companion object {
        private val MAC = Regex("^[0-9a-fA-F]{2}(:[0-9a-fA-F]{2}){5}$")

        /** Player ids look like MAC addresses; anything else is never sent. */
        fun isPlayerId(v: String): Boolean = MAC.matches(v)
    }
}
