package io.github.jamezuk.lyrionanywhere.navigation

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.SaveableStateHolder
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import io.github.jamezuk.lyrionanywhere.data.AuthState

/**
 * Where the user was in the signed-in app, kept above the switch between it, offline mode
 * and the sign-in screens ([AppRoot]).
 *
 * The signed-in app ([MainScreen]) keeps its place in saveable state: its navigation back
 * stack (`rememberNavController`), each screen's own saved state and scroll positions (the
 * NavHost's), and its sheets. Offline mode taking the screen, or the "Signing in…" screen
 * after the process was killed, used to leave that state behind, so the app came back at
 * Home. Here it is hosted by a [SaveableStateHolder] that [AppRoot] always composes: when
 * MainScreen leaves the screen its state is saved under the account's key; when it comes
 * back it is restored; and the holder is itself saved with the activity, so the same holds
 * across a configuration change or the process being killed (the restored state waits in
 * the holder until MainScreen is shown again). Another account (or signing out) starts
 * afresh.
 */
class AppPlaces internal constructor(
    private val holder: SaveableStateHolder,
    private val current: () -> String?,
    private val setCurrent: (String?) -> Unit,
) {
    /** The signed-in app for [account], at the place it was left. */
    @Composable
    fun Keep(account: String, content: @Composable () -> Unit) {
        LaunchedEffect(account) {
            val before = current()
            if (before != null && before != account) holder.removeState(before)
            setCurrent(account)
        }
        holder.SaveableStateProvider(account, content)
    }

    /** Signed out (or another server): the place is forgotten. */
    fun forget() {
        current()?.let(holder::removeState)
        setCurrent(null)
    }

    companion object {
        /** The key for a sign-in: its server and this device's registration there. */
        fun keyOf(s: AuthState.SignedInState): String = "main|${s.server.url}|${s.who.device.id}"
    }
}

/** The [AppPlaces] for this activity (saved with it). */
@Composable
fun rememberAppPlaces(): AppPlaces {
    val holder = rememberSaveableStateHolder()
    var current by rememberSaveable { mutableStateOf<String?>(null) }
    return remember(holder) { AppPlaces(holder, { current }, { current = it }) }
}
