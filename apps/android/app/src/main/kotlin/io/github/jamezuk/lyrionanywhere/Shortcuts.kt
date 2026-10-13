package io.github.jamezuk.lyrionanywhere

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** What an app shortcut asks for (Wave 2 §5.5). */
enum class ShortcutRequest { START_RADIO, SLEEP_30 }

/** The launcher's app shortcuts: their actions, read strictly. Pure, tested on the JVM. */
object Shortcuts {
    const val ACTION_START_RADIO = "io.github.jamezuk.lyrionanywhere.shortcut.START_RADIO"
    const val ACTION_SLEEP_30 = "io.github.jamezuk.lyrionanywhere.shortcut.SLEEP_30"

    /** A request left waiting longer than this (the app wasn't signed in) is dropped. */
    const val FRESH_MS = 60_000L

    /** The request an action names; anything else is nothing. */
    fun parse(action: String?): ShortcutRequest? = when (action) {
        ACTION_START_RADIO -> ShortcutRequest.START_RADIO
        ACTION_SLEEP_30 -> ShortcutRequest.SLEEP_30
        else -> null
    }
}

/**
 * A shortcut's request, held until the signed-in app acts on it through the same player
 * controller as its buttons (so the gateway's policy applies as usual). Only the latest is
 * kept, and only for [Shortcuts.FRESH_MS].
 */
class AppShortcuts(private val clock: () -> Long) {
    private val _pending = MutableStateFlow<Pair<ShortcutRequest, Long>?>(null)

    /** Something waiting (watched by the app's main screen). */
    val pending: StateFlow<Pair<ShortcutRequest, Long>?> = _pending.asStateFlow()

    fun offer(request: ShortcutRequest) {
        _pending.value = request to clock()
    }

    /** The waiting request, once (null: none, or too old). */
    fun take(): ShortcutRequest? {
        val p = _pending.value ?: return null
        _pending.value = null
        return p.first.takeIf { clock() - p.second in 0..Shortcuts.FRESH_MS }
    }
}

/**
 * Where the launcher's shortcuts land. Not exported (the launcher starts shortcuts as this
 * app), no UI: it notes the request and brings the app to the front.
 */
class ShortcutActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val app = application as LyrionApp
        if (app.started && !app.safeMode) {
            Shortcuts.parse(intent?.action)?.let(app.container.shortcuts::offer)
        }
        runCatching {
            startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
        finish()
    }
}
