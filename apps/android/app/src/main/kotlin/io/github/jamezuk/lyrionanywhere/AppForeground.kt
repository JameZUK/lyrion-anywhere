package io.github.jamezuk.lyrionanywhere

import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.ProcessLifecycleOwner
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Whether the app is in the foreground (one of its activities started), from
 * [ProcessLifecycleOwner]. That library's start-up initialiser is already in the merged
 * manifest; nothing here adds one (a new androidx.startup initialiser can crash the app
 * before any of its code runs). Created on the main thread, in `Application.onCreate`.
 */
class AppForeground {
    private val lifecycle = ProcessLifecycleOwner.get().lifecycle
    private val _foreground = MutableStateFlow(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED))
    val foreground: StateFlow<Boolean> = _foreground.asStateFlow()

    init {
        lifecycle.addObserver(
            LifecycleEventObserver { _, _ ->
                _foreground.value = lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
            },
        )
    }
}
