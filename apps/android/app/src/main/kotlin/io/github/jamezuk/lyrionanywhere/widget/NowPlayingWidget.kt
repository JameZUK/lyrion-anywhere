package io.github.jamezuk.lyrionanywhere.widget

import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.compose.runtime.Composable
import androidx.core.graphics.scale
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.glance.ColorFilter
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.GlanceTheme
import androidx.glance.Image
import androidx.glance.ImageProvider
import androidx.glance.LocalContext
import androidx.glance.action.ActionParameters
import androidx.glance.action.actionParametersOf
import androidx.glance.action.actionStartActivity
import androidx.glance.action.clickable
import androidx.glance.appwidget.ExperimentalGlanceRemoteViewsApi
import androidx.glance.appwidget.GlanceRemoteViews
import androidx.glance.appwidget.action.ActionCallback
import androidx.glance.appwidget.action.actionRunCallback
import androidx.glance.appwidget.cornerRadius
import androidx.glance.background
import androidx.glance.layout.Alignment
import androidx.glance.layout.Box
import androidx.glance.layout.Column
import androidx.glance.layout.Row
import androidx.glance.layout.Spacer
import androidx.glance.layout.fillMaxSize
import androidx.glance.layout.padding
import androidx.glance.layout.size
import androidx.glance.layout.width
import androidx.glance.layout.fillMaxWidth
import androidx.glance.layout.height
import androidx.glance.appwidget.LinearProgressIndicator
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextStyle
import io.github.jamezuk.lyrionanywhere.LyrionApp
import io.github.jamezuk.lyrionanywhere.MainActivity
import io.github.jamezuk.lyrionanywhere.R
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * The home-screen widget: artwork, title and artist, with play/pause and next, for this
 * phone's player (or the player the app controls).
 *
 * It is drawn once per change from what the app already knows
 * ([io.github.jamezuk.lyrionanywhere.data.NowPlayingHub]), composed straight to
 * RemoteViews ([GlanceRemoteViews]) and handed to the launcher. No Glance session, worker,
 * alarm or periodic update is used, so it never wakes the phone or keeps the app running.
 */
object NowPlayingWidget {
    /** Widget artwork is small: decoded no larger than this (pixels). */
    const val ART_PX = 192

    /** Above this font scale the widget's one row fits only the title, so the detail is dropped. */
    const val LARGE_TEXT_SCALE = 1.3f

    /** Whether the artist/player line fits at [fontScale]. */
    fun detailFits(fontScale: Float): Boolean = fontScale <= LARGE_TEXT_SCALE

    /** Redraw every placed widget (cheap when none is placed). */
    @OptIn(ExperimentalGlanceRemoteViewsApi::class)
    suspend fun render(app: LyrionApp) {
        if (!app.started) return
        val manager = AppWidgetManager.getInstance(app) ?: return
        val ids = manager.getAppWidgetIds(ComponentName(app, NowPlayingWidgetReceiver::class.java)) ?: IntArray(0)
        val large = manager.getAppWidgetIds(ComponentName(app, NowPlayingLargeWidgetReceiver::class.java)) ?: IntArray(0)
        if (ids.isEmpty() && large.isEmpty()) return
        val model = WidgetModel.from(app.nowPlaying.shown.first())
        val art = model.artwork?.let { loadArtwork(app, it) }
        if (ids.isNotEmpty()) {
            val views = GlanceRemoteViews().compose(app, DpSize(320.dp, 72.dp)) {
                GlanceTheme { Content(model, art) }
            }.remoteViews
            manager.updateAppWidget(ids, views)
        }
        if (large.isNotEmpty()) {
            val views = GlanceRemoteViews().compose(app, DpSize(320.dp, 220.dp)) {
                GlanceTheme { LargeContent(model, art) }
            }.remoteViews
            manager.updateAppWidget(large, views)
        }
    }

    /**
     * The larger widget (Wave 2 §5.5): art, title and artist, a progress line, and previous,
     * play/pause and next. It fills whatever size the launcher gives it.
     */
    @Composable
    private fun LargeContent(model: WidgetModel, art: Bitmap?) {
        val context = LocalContext.current
        Column(
            GlanceModifier
                .fillMaxSize()
                .background(GlanceTheme.colors.widgetBackground)
                .cornerRadius(16.dp)
                .padding(12.dp)
                .clickable(actionStartActivity<MainActivity>()),
        ) {
            Row(GlanceModifier.fillMaxWidth().defaultWeight(), verticalAlignment = Alignment.Top) {
                Box(GlanceModifier.size(96.dp).cornerRadius(12.dp).background(GlanceTheme.colors.secondaryContainer)) {
                    if (art != null) {
                        Image(ImageProvider(art), contentDescription = null, modifier = GlanceModifier.size(96.dp).cornerRadius(12.dp))
                    }
                }
                Spacer(GlanceModifier.width(12.dp))
                Column(GlanceModifier.defaultWeight()) {
                    val (title, detail) = texts(context, model)
                    Text(
                        title,
                        maxLines = 2,
                        style = TextStyle(color = GlanceTheme.colors.onSurface, fontSize = 17.sp, fontWeight = FontWeight.Medium),
                    )
                    if (detail.isNotEmpty()) {
                        Text(detail, maxLines = 2, style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant, fontSize = 14.sp))
                    }
                }
            }
            val player = model.playerId
            if (model.showsControls && player != null) {
                LinearProgressIndicator(
                    progress = model.progress ?: 0f,
                    modifier = GlanceModifier.fillMaxWidth().height(4.dp),
                    color = GlanceTheme.colors.primary,
                    backgroundColor = GlanceTheme.colors.secondaryContainer,
                )
                Row(GlanceModifier.fillMaxWidth().padding(top = 4.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    LargeButton(R.drawable.ic_widget_previous, context.getString(R.string.widget_previous), player, Control.PREVIOUS)
                    Spacer(GlanceModifier.width(16.dp))
                    LargeButton(
                        if (model.playing) R.drawable.ic_widget_pause else R.drawable.ic_widget_play,
                        context.getString(if (model.playing) R.string.widget_pause else R.string.widget_play),
                        player,
                        Control.TOGGLE,
                    )
                    Spacer(GlanceModifier.width(16.dp))
                    LargeButton(R.drawable.ic_widget_next, context.getString(R.string.widget_next), player, Control.NEXT)
                }
            }
        }
    }

    @Composable
    private fun LargeButton(icon: Int, label: String, player: String, control: Control) {
        Image(
            ImageProvider(icon),
            contentDescription = label,
            colorFilter = ColorFilter.tint(GlanceTheme.colors.onSurface),
            modifier = GlanceModifier
                .size(56.dp)
                .padding(14.dp)
                .clickable(actionRunCallback<WidgetControl>(params(player, control))),
        )
    }

    /** The title and detail lines for [model]. */
    private fun texts(context: Context, model: WidgetModel): Pair<String, String> = when (model.kind) {
        WidgetModel.Kind.NOTHING -> context.getString(R.string.widget_nothing) to ""
        WidgetModel.Kind.IDLE -> context.getString(R.string.widget_idle) to model.playerName
        WidgetModel.Kind.TRACK ->
            model.title to listOfNotNull(model.artist, model.playerName.ifEmpty { null }).joinToString(" · ")
    }

    @Composable
    private fun Content(model: WidgetModel, art: Bitmap?) {
        val context = LocalContext.current
        Row(
            GlanceModifier
                .fillMaxSize()
                .background(GlanceTheme.colors.widgetBackground)
                .cornerRadius(16.dp)
                .padding(8.dp)
                .clickable(actionStartActivity<MainActivity>()),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(GlanceModifier.size(56.dp).cornerRadius(8.dp).background(GlanceTheme.colors.secondaryContainer)) {
                if (art != null) {
                    Image(ImageProvider(art), contentDescription = null, modifier = GlanceModifier.size(56.dp).cornerRadius(8.dp))
                }
            }
            Spacer(GlanceModifier.width(12.dp))
            Column(GlanceModifier.defaultWeight()) {
                val (title, detail) = when (model.kind) {
                    WidgetModel.Kind.NOTHING -> context.getString(R.string.widget_nothing) to ""
                    WidgetModel.Kind.IDLE -> context.getString(R.string.widget_idle) to model.playerName
                    WidgetModel.Kind.TRACK ->
                        model.title to listOfNotNull(model.artist, model.playerName.ifEmpty { null }).joinToString(" · ")
                }
                Text(
                    title,
                    maxLines = 1,
                    style = TextStyle(color = GlanceTheme.colors.onSurface, fontSize = 15.sp, fontWeight = FontWeight.Medium),
                )
                if (detail.isNotEmpty() && detailFits(context.resources.configuration.fontScale)) {
                    Text(detail, maxLines = 1, style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant, fontSize = 13.sp))
                }
            }
            val player = model.playerId
            if (model.showsControls && player != null) {
                Image(
                    ImageProvider(if (model.playing) R.drawable.ic_widget_pause else R.drawable.ic_widget_play),
                    contentDescription = context.getString(if (model.playing) R.string.widget_pause else R.string.widget_play),
                    colorFilter = ColorFilter.tint(GlanceTheme.colors.onSurface),
                    modifier = GlanceModifier
                        .size(48.dp)
                        .padding(12.dp)
                        .clickable(actionRunCallback<WidgetControl>(params(player, Control.TOGGLE))),
                )
                Image(
                    ImageProvider(R.drawable.ic_widget_next),
                    contentDescription = context.getString(R.string.widget_next),
                    colorFilter = ColorFilter.tint(GlanceTheme.colors.onSurface),
                    modifier = GlanceModifier
                        .size(48.dp)
                        .padding(12.dp)
                        .clickable(actionRunCallback<WidgetControl>(params(player, Control.NEXT))),
                )
            }
        }
    }

    private fun params(player: String, control: Control) =
        actionParametersOf(WidgetControl.PLAYER to player, WidgetControl.CONTROL to control.name)

    private suspend fun loadArtwork(app: LyrionApp, key: String): Bitmap? {
        val img = (app.container.art.image(key, ART_PX) as? ApiResult.Ok)?.value ?: return null
        return decodeSampled(img.bytes, ART_PX)
    }

    /** Decode at most about [maxPx] on a side (widgets travel to the launcher by IPC). */
    private fun decodeSampled(bytes: ByteArray, maxPx: Int): Bitmap? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
        val opts = BitmapFactory.Options().apply { inSampleSize = sampleSize(bounds.outWidth, bounds.outHeight, maxPx) }
        val decoded = BitmapFactory.decodeByteArray(bytes, 0, bytes.size, opts) ?: return null
        if (decoded.width <= maxPx && decoded.height <= maxPx) return decoded
        val scale = maxPx.toFloat() / maxOf(decoded.width, decoded.height)
        val w = (decoded.width * scale).toInt().coerceAtLeast(1)
        val h = (decoded.height * scale).toInt().coerceAtLeast(1)
        return decoded.scale(w, h)
    }

    /** The largest power of two that keeps both sides at least [maxPx] (BitmapFactory's inSampleSize). */
    fun sampleSize(width: Int, height: Int, maxPx: Int): Int {
        var sample = 1
        while (width / (sample * 2) >= maxPx && height / (sample * 2) >= maxPx && sample < 64) sample *= 2
        return sample
    }
}

/** Play/pause and next from the widget, for the player it shows. */
class WidgetControl : ActionCallback {
    override suspend fun onAction(context: Context, glanceId: GlanceId, parameters: ActionParameters) {
        val app = context.applicationContext as LyrionApp
        if (!app.started) return
        val player = parameters[PLAYER] ?: return
        val control = Control.entries.firstOrNull { it.name == parameters[CONTROL] } ?: return
        app.controls.run(player, control)
    }

    companion object {
        val PLAYER = ActionParameters.Key<String>("player")
        val CONTROL = ActionParameters.Key<String>("control")
    }
}

/** The larger widget's receiver: the launcher asks, and both widgets are drawn. */
class NowPlayingLargeWidgetReceiver : AppWidgetProvider() {
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        val app = context.applicationContext as LyrionApp
        val pending = goAsync()
        app.appScope.launch {
            try {
                NowPlayingWidget.render(app)
            } finally {
                pending.finish()
            }
        }
    }
}

/** The launcher asks for the widget (placed, resized, or after a reboot): draw it. */
class NowPlayingWidgetReceiver : AppWidgetProvider() {
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        val app = context.applicationContext as LyrionApp
        val pending = goAsync()
        app.appScope.launch {
            try {
                NowPlayingWidget.render(app)
            } finally {
                pending.finish()
            }
        }
    }
}
