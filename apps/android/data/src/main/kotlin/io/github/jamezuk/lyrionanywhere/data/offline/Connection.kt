package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.data.AuthState

/**
 * What the screen shows about the link to the server (SPEC §8.4).
 *
 * - [takeover]: offline mode has the whole screen (the downloaded library, played here).
 * - [unreachable]: the server can't be reached; a banner on the current screen says so and
 *   leads to the downloads.
 * - [reconnecting]: a reconnect is under way; a small pill over the current screen.
 */
data class ConnectionUi(
    val takeover: Boolean = false,
    val unreachable: Boolean = false,
    val reconnecting: Boolean = false,
)

/** What [ConnectionJudge] decides from, at one moment. */
data class ConnectionInputs(
    val auth: AuthState,
    /** The phone has no network at all (flight mode, no signal). */
    val netDown: Boolean,
    /** This phone's LMS player can't reach the server (it is reconnecting). */
    val trouble: Boolean,
    /** The app is in the foreground (an activity started). */
    val foreground: Boolean,
    /** This phone's LMS player is playing (or buffering): its link matters even in the background. */
    val playing: Boolean,
)

/**
 * Decides the [ConnectionUi] from the inputs over time. Pure (the time is passed in), so
 * it is tested on the JVM.
 *
 * Android cuts a backgrounded app's network when it isn't playing, so the phone's player
 * drops while the app sits in the background. That isn't the server being away:
 * - the player's trouble counts only while the app is in use (in the foreground, after
 *   [FOREGROUND_GRACE_MS] for the reconnect it is kicked into on return) or while it plays;
 * - no network at all counts at once (after [NETWORK_GRACE_MS], which rides out a handover);
 * - counted trouble for [TROUBLE_GRACE_MS], or no network, shows the banner;
 * - offline mode takes the screen only when there's no network as the app is opened, or the
 *   server stays unreachable for [TAKEOVER_MS] while the app is in use; it then stays until
 *   the link is back.
 * A sign-in that couldn't be checked ([AuthState.Unreachable]) is offline mode as before,
 * and a sign-in being checked again keeps whatever was shown.
 */
class ConnectionJudge {
    private var lastForeground = false
    private var foregroundSince: Long? = null
    private var netDownSince: Long? = null
    private var troubleSince: Long? = null
    private var inUseSince: Long? = null
    private var takeover = false

    /** Feed the inputs at [now] (ms, monotonic); returns what to show. */
    fun update(now: Long, i: ConnectionInputs): ConnectionUi {
        val opened = i.foreground && !lastForeground
        lastForeground = i.foreground
        foregroundSince = if (i.foreground) (if (opened) now else foregroundSince ?: now) else null
        netDownSince = if (i.netDown) netDownSince ?: now else null

        val inGrace = foregroundSince?.let { now - it < FOREGROUND_GRACE_MS } ?: false
        val counts = i.trouble && !i.netDown && (i.playing || (i.foreground && !inGrace))
        troubleSince = if (counts) troubleSince ?: now else null
        val unreachableNow = i.netDown || counts
        inUseSince = if (i.foreground && unreachableNow) inUseSince ?: now else null

        val signedIn = i.auth is AuthState.SignedInState
        takeover = when (i.auth) {
            is AuthState.Unreachable -> true
            is AuthState.Restoring -> takeover
            is AuthState.SignedInState -> when {
                opened && i.netDown -> true
                // Once offline mode has the screen, it keeps it until the link is back.
                takeover -> i.netDown || i.trouble
                else -> inUseSince?.let { now - it >= TAKEOVER_MS } ?: false
            }
            else -> false
        }
        val netGone = netDownSince?.let { now - it >= NETWORK_GRACE_MS } ?: false
        val troubleLong = troubleSince?.let { now - it >= TROUBLE_GRACE_MS } ?: false
        val unreachable = signedIn && !takeover && (netGone || troubleLong)
        val reconnecting = signedIn && !takeover && !unreachable && i.foreground && (i.trouble || i.netDown)
        return ConnectionUi(takeover = takeover, unreachable = unreachable, reconnecting = reconnecting)
    }

    /** How long until the decision could change with the same inputs (null: it won't). */
    fun nextCheckIn(now: Long): Long? = listOfNotNull(
        foregroundSince?.plus(FOREGROUND_GRACE_MS),
        netDownSince?.plus(NETWORK_GRACE_MS),
        troubleSince?.plus(TROUBLE_GRACE_MS),
        inUseSince?.plus(TAKEOVER_MS),
    ).filter { it > now }.minOrNull()?.minus(now)

    companion object {
        /** After returning to the foreground: the reconnect's time before trouble counts. */
        const val FOREGROUND_GRACE_MS = 10_000L

        /** No network this long: the server can't be reached (rides out a handover). */
        const val NETWORK_GRACE_MS = OfflineModeController.NETWORK_GRACE_MS

        /** Counted trouble this long: the banner. */
        const val TROUBLE_GRACE_MS = 5_000L

        /** Unreachable this long while in use: offline mode takes the screen. */
        const val TAKEOVER_MS = 15_000L
    }
}
