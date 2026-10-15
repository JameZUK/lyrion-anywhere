package io.github.jamezuk.lyrionanywhere.data

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

/**
 * How well this phone's player can keep going with the screen off.
 *
 * Android cuts a backgrounded app's network while the phone dozes, except for an app
 * running a foreground service or one the user has let run unrestricted. The playback
 * service is a foreground service while it plays, but Android refuses to make it one
 * when playback starts (or comes back) while the app is in the background, unless
 * battery optimisation is off for the app. Without that, the player's connection to the
 * server comes and goes with the screen off: the music carries on from what was read
 * ahead, but a track changed from another device waits for the phone to wake.
 */
class BackgroundPlayback {
    private val _refusals = MutableStateFlow(0)

    /** How many times Android refused to keep playback in the foreground (this run). */
    val refusals: StateFlow<Int> = _refusals.asStateFlow()

    /** Android refused to start the playback service in the foreground. */
    fun foregroundRefused() {
        _refusals.update { if (it == Int.MAX_VALUE) it else it + 1 }
    }

    companion object {
        /**
         * Whether to offer to let the app run unrestricted: only while this phone is a
         * player and Android still restricts the app. The offer is made once; dismissed, it
         * isn't made again (Settings › Playback keeps the option).
         */
        fun shouldPrompt(playOnPhone: Boolean, unrestricted: Boolean, dismissed: Boolean): Boolean =
            playOnPhone && !unrestricted && !dismissed

        /** Whether Settings › Playback shows the option (it goes once the app is unrestricted). */
        fun shouldOffer(playOnPhone: Boolean, unrestricted: Boolean): Boolean = playOnPhone && !unrestricted
    }
}
