package io.github.jamezuk.lyrionanywhere.widget

import android.content.ComponentName
import android.content.Context
import android.graphics.drawable.Icon
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import io.github.jamezuk.lyrionanywhere.LyrionApp
import io.github.jamezuk.lyrionanywhere.R
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/**
 * Quick Settings: play/pause for this phone's player. It shows what the app already knows
 * and does nothing in the background; the system binds it only while the panel is open.
 */
class PlayPauseTileService : TileService() {
    private var scope: CoroutineScope? = null

    override fun onStartListening() {
        super.onStartListening()
        scope?.cancel()
        val s = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        scope = s
        val app = application as LyrionApp
        s.launch { app.nowPlaying.phone.collect { render() } }
    }

    override fun onStopListening() {
        scope?.cancel()
        scope = null
        super.onStopListening()
    }

    override fun onClick() {
        super.onClick()
        val app = application as LyrionApp
        if (!app.started) return
        val phone = app.nowPlaying.phone.value ?: return
        if (model().state == TileModel.State.UNAVAILABLE) return
        // Shown at once; LMS's answer (through the playback service) follows.
        app.appScope.launch { app.controls.run(phone.playerId, Control.TOGGLE) }
    }

    private fun model(): TileModel {
        val app = application as LyrionApp
        // In safe mode (after a crash) the app hasn't started: nothing to control.
        val enabled = app.started && app.container.localPlayer.state.value.enabled
        return TileModel.from(app.nowPlaying.phone.value, enabled)
    }

    private fun render() {
        val tile = qsTile ?: return
        val m = model()
        tile.state = when (m.state) {
            TileModel.State.UNAVAILABLE -> Tile.STATE_UNAVAILABLE
            TileModel.State.INACTIVE -> Tile.STATE_INACTIVE
            TileModel.State.ACTIVE -> Tile.STATE_ACTIVE
        }
        tile.label = getString(R.string.tile_label)
        tile.subtitle = when (m.state) {
            TileModel.State.UNAVAILABLE -> getString(R.string.tile_unavailable)
            else -> m.title ?: getString(if (m.playing) R.string.tile_playing else R.string.tile_paused)
        }
        tile.icon = Icon.createWithResource(this, if (m.playing) R.drawable.ic_widget_pause else R.drawable.ic_widget_play)
        tile.updateTile()
    }

    companion object {
        /** Ask the system to refresh the tile (it binds briefly if the tile is added). */
        fun refresh(context: Context) {
            try {
                requestListeningState(context, ComponentName(context, PlayPauseTileService::class.java))
            } catch (_: RuntimeException) {
                // Not added, or the system refused: nothing to refresh.
            }
        }
    }
}
