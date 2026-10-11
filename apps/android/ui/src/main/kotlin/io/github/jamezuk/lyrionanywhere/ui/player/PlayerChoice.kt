package io.github.jamezuk.lyrionanywhere.ui.player

import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.data.AppSettings
import io.github.jamezuk.lyrionanywhere.data.DefaultPlayer

/**
 * Which player the app controls (SPEC D20, "player-first"): this phone unless the user
 * chooses otherwise. The rules live here, apart from the view model, so they can change
 * (and be tested) on their own.
 */
object PlayerChoice {
    /** What to control when the app starts. */
    sealed interface Start {
        /** Control this player now. */
        data class Control(val playerId: String) : Start

        /** Wait for this phone's player to appear in LMS (it is starting). */
        data object WaitForPhone : Start

        /** Nothing to control. */
        data object Nothing : Start
    }

    /**
     * The player to control on start, from the default-player setting:
     * - this phone: the phone, once LMS lists it;
     * - last used / a specific player: that player if it is listed, else the phone.
     *
     * If the phone can't play here at all ([phoneAvailable] false), the last-used player,
     * a playing one, or the first, is controlled instead.
     */
    fun start(players: List<Player>, phoneId: String?, phoneAvailable: Boolean, settings: AppSettings): Start {
        fun listed(id: String?) = id?.takeIf { i -> players.any { it.id == i } }
        val preferred = when (settings.defaultPlayer) {
            DefaultPlayer.THIS_PHONE -> null
            DefaultPlayer.LAST_USED -> listed(settings.selectedPlayerId)
            DefaultPlayer.SPECIFIC -> listed(settings.defaultPlayerId)
        }
        if (preferred != null) return Start.Control(preferred)
        listed(phoneId)?.let { return Start.Control(it) }
        if (phoneAvailable) return Start.WaitForPhone
        val fallback = listed(settings.selectedPlayerId)
            ?: players.firstOrNull { it.playing }?.id
            ?: players.firstOrNull()?.id
        return fallback?.let { Start.Control(it) } ?: Start.Nothing
    }

    /** Whether the phone's player should be switched on when the app starts. */
    fun wantsPhone(settings: AppSettings): Boolean = settings.defaultPlayer == DefaultPlayer.THIS_PHONE
}
