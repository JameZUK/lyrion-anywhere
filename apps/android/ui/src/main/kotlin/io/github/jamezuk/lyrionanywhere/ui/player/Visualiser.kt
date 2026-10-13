package io.github.jamezuk.lyrionanywhere.ui.player

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.database.ContentObserver
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.provider.Settings
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.GraphicEq
import androidx.compose.material.icons.outlined.Style
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconToggleButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.animation.core.withInfiniteAnimationFrameMillis
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.currentStateAsState
import io.github.jamezuk.lyrionanywhere.data.VisualFrame
import io.github.jamezuk.lyrionanywhere.data.VisualStyle
import io.github.jamezuk.lyrionanywhere.ui.R
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.exp
import kotlin.math.sin

/**
 * Where the visualiser's frames come from: this phone's player (Wave 2 §4.1). The engine
 * analyses audio only while a visualiser is [active][setActive].
 */
interface VisualSource {
    fun setActive(on: Boolean)

    /** The newest frame since the last call, or null. */
    fun frame(): VisualFrame?
}

/** Why the visualiser can't run now (it then stays off, whatever the setting). */
enum class VisualBlock { NONE, BATTERY_SAVER, ANIMATIONS_OFF }

object VisualMotion {
    /** At most this often a frame is drawn (30 fps). */
    const val FRAME_MS = 33L

    fun block(powerSave: Boolean, animatorScale: Float): VisualBlock = when {
        powerSave -> VisualBlock.BATTERY_SAVER
        animatorScale == 0f -> VisualBlock.ANIMATIONS_OFF
        else -> VisualBlock.NONE
    }

    /** The style after (or before) [s], for swipes and the style button. */
    fun next(s: VisualStyle, forward: Boolean = true): VisualStyle {
        val all = VisualStyle.entries
        val i = all.indexOf(s)
        return all[Math.floorMod(i + if (forward) 1 else -1, all.size)]
    }
}

/**
 * A VU needle (Wave 2 §4.1): about 300 ms to rise and to fall (to 99 %), and a peak mark that
 * holds for [holdMs] before falling with the needle. Positions are 0–1 over [VuMeter.RANGE_DB]
 * dB below full scale. Plain Kotlin, so its ballistics are tested on the JVM.
 */
class VuMeter(private val riseMs: Float = 300f, private val fallMs: Float = 300f, private val holdMs: Long = 1_500) {
    var needle = 0f
        private set
    var peak = 0f
        private set
    private var held = 0L

    /** Move towards [levelDb] (dBFS) over [dtMs] milliseconds. */
    fun step(levelDb: Float, dtMs: Long) {
        val dt = dtMs.coerceIn(0, 1_000).toFloat()
        val target = position(levelDb)
        val tau = (if (target > needle) riseMs else fallMs) / 4.6f // 99 % in riseMs/fallMs
        needle += (target - needle) * (1f - exp(-dt / tau))
        needle = needle.coerceIn(0f, 1f)
        if (needle >= peak) {
            peak = needle
            held = 0
        } else {
            held += dt.toLong()
            if (held > holdMs) peak = (peak + (needle - peak) * (1f - exp(-dt / (fallMs / 4.6f)))).coerceIn(needle, 1f)
        }
    }

    /** At rest, peak cleared. */
    fun reset() {
        needle = 0f
        peak = 0f
        held = 0
    }

    companion object {
        /** The meter shows this many dB below full scale. */
        const val RANGE_DB = 40f

        fun position(db: Float): Float = if (db.isFinite()) ((db + RANGE_DB) / RANGE_DB).coerceIn(0f, 1f) else 0f
    }
}

/**
 * What the visualiser draws, smoothed between frames: bars fall back gently, the scope fades,
 * the meters follow their ballistics, and the glow follows the bass. [tick] changes whenever
 * something moved, so only the drawing is redone (no recomposition).
 */
@Stable
class VisualState {
    val bars = FloatArray(VisualFrame.BANDS)
    val scope = FloatArray(VisualFrame.SCOPE)
    val vu = arrayOf(VuMeter(), VuMeter())
    var glow by mutableFloatStateOf(0f)
        private set
    var tick by mutableLongStateOf(0L)
        private set

    /** A new frame (or none, [frame] null) after [dtMs]. Returns whether anything moved. */
    fun update(frame: VisualFrame?, dtMs: Long): Boolean {
        val dt = dtMs.coerceIn(0, 1_000) / 1000f
        var moved = false
        val fall = BAR_FALL_PER_S * dt
        for (i in bars.indices) {
            val target = frame?.bands?.getOrNull(i) ?: 0f
            val v = if (target >= bars[i]) target else (bars[i] - fall).coerceAtLeast(target)
            if (abs(v - bars[i]) > 1e-4f) moved = true
            bars[i] = v
        }
        val fade = exp(-dt / SCOPE_FADE_S)
        for (i in scope.indices) {
            val v = frame?.scope?.getOrNull(i) ?: (scope[i] * fade)
            if (abs(v - scope[i]) > 1e-4f) moved = true
            scope[i] = v
        }
        for (side in 0..1) {
            val before = vu[side].needle to vu[side].peak
            vu[side].step(frame?.rmsDb?.getOrNull(side) ?: VisualFrame.FLOOR_DB, dtMs)
            if (abs(vu[side].needle - before.first) > 1e-4f || abs(vu[side].peak - before.second) > 1e-4f) moved = true
        }
        val g = frame?.bass ?: 0f
        val newGlow = if (g >= glow) glow + (g - glow) * 0.6f else glow + (g - glow) * (1f - exp(-dt / GLOW_FALL_S))
        if (abs(newGlow - glow) > 1e-4f) moved = true
        glow = newGlow
        if (moved) tick++
        return moved
    }

    fun reset() {
        bars.fill(0f)
        scope.fill(0f)
        vu.forEach { it.reset() }
        glow = 0f
        tick++
    }

    companion object {
        /** Bars fall at most this much (of full height) a second. */
        const val BAR_FALL_PER_S = 1.8f
        const val SCOPE_FADE_S = 0.15f
        const val GLOW_FALL_S = 0.25f
    }
}

/**
 * Whether the visualiser must stay off now: battery saver, or animations turned off in the
 * system's settings ("Remove animations"). Follows changes to either while shown.
 */
@Composable
fun rememberVisualBlock(): VisualBlock {
    val context = LocalContext.current
    var powerSave by remember(context) { mutableStateOf(readPowerSave(context)) }
    var scale by remember(context) { mutableFloatStateOf(readAnimatorScale(context)) }
    DisposableEffect(context) {
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(c: Context, intent: Intent) {
                powerSave = readPowerSave(context)
            }
        }
        val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) {
                scale = readAnimatorScale(context)
            }
        }
        val app = context.applicationContext
        try {
            app.registerReceiver(receiver, IntentFilter(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED))
        } catch (_: Exception) {
        }
        try {
            app.contentResolver.registerContentObserver(
                Settings.Global.getUriFor(Settings.Global.ANIMATOR_DURATION_SCALE),
                false,
                observer,
            )
        } catch (_: Exception) {
        }
        onDispose {
            try {
                app.unregisterReceiver(receiver)
            } catch (_: Exception) {
            }
            app.contentResolver.unregisterContentObserver(observer)
        }
    }
    return VisualMotion.block(powerSave, scale)
}

private fun readPowerSave(context: Context): Boolean =
    context.getSystemService(PowerManager::class.java)?.isPowerSaveMode == true

private fun readAnimatorScale(context: Context): Float = try {
    Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f)
} catch (_: Exception) {
    1f
}

/**
 * Runs the visualiser while [running] and the screen is in front: tells the [source] to
 * analyse (and to stop when it no longer shows), and pulls frames into the returned state at
 * most every [VisualMotion.FRAME_MS].
 */
@Composable
fun rememberVisualState(source: VisualSource?, running: Boolean): VisualState {
    val state = remember { VisualState() }
    val lifecycle by LocalLifecycleOwner.current.lifecycle.currentStateAsState()
    val active = running && source != null && lifecycle.isAtLeast(Lifecycle.State.STARTED)
    DisposableEffect(source, active) {
        if (active) source?.setActive(true)
        onDispose {
            if (active) source?.setActive(false)
            state.reset()
        }
    }
    LaunchedEffect(source, active) {
        if (!active || source == null) return@LaunchedEffect
        var last = 0L
        while (true) {
            withInfiniteAnimationFrameMillis { now ->
                if (last == 0L || now - last >= VisualMotion.FRAME_MS) {
                    val dt = if (last == 0L) VisualMotion.FRAME_MS else now - last
                    last = now
                    state.update(source.frame(), dt)
                }
            }
        }
    }
    return state
}

/** A style's name, for its button and Settings. */
@Composable
fun visualStyleName(s: VisualStyle): String = stringResource(
    when (s) {
        VisualStyle.BARS -> R.string.visual_bars
        VisualStyle.SCOPE -> R.string.visual_scope
        VisualStyle.GLOW -> R.string.visual_glow
        VisualStyle.VU -> R.string.visual_vu
    },
)

/**
 * The visualiser where the art is (Bars, Scope or VU; Glow lights the background instead).
 * Decorative: hidden from screen readers (its on/off and style controls are labelled).
 * Swiping across it changes the style.
 */
@Composable
fun VisualiserCanvas(
    style: VisualStyle,
    state: VisualState,
    onStyle: (VisualStyle) -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val primary = scheme.primary
    val track = scheme.onSurfaceVariant.copy(alpha = 0.35f)
    val accent = scheme.tertiary
    val error = scheme.error
    val min = with(LocalDensity.current) { SWIPE_MIN.toPx() }
    val latestStyle by rememberUpdatedState(style)
    val latestOnStyle by rememberUpdatedState(onStyle)
    Box(
        modifier
            .fillMaxWidth()
            .aspectRatio(1f)
            .testTag("visualiser")
            .clearAndSetSemantics {}
            .pointerInput(min) {
                var dx = 0f
                detectHorizontalDragGestures(
                    onDragStart = { dx = 0f },
                    onDragEnd = {
                        if (abs(dx) >= maxOf(min, size.width * Swipes.ARTWORK_SHARE)) {
                            latestOnStyle(VisualMotion.next(latestStyle, forward = dx < 0))
                        }
                    },
                ) { change, amount ->
                    dx += amount
                    change.consume()
                }
            },
    ) {
        Canvas(Modifier.fillMaxSize()) {
            state.tick // redraw when the state moves
            when (style) {
                VisualStyle.BARS, VisualStyle.GLOW -> drawBars(state.bars, primary, track)
                VisualStyle.SCOPE -> drawScope(state.scope, primary, track)
                VisualStyle.VU -> drawVu(state.vu, scheme.onSurface, track, accent, error)
            }
        }
    }
}

private fun DrawScope.drawBars(bars: FloatArray, colour: androidx.compose.ui.graphics.Color, base: androidx.compose.ui.graphics.Color) {
    val n = bars.size
    val gap = size.width / n * 0.25f
    val w = size.width / n - gap
    for (i in 0 until n) {
        val h = (bars[i].coerceIn(0f, 1f) * size.height * 0.9f).coerceAtLeast(2f)
        val x = i * (w + gap) + gap / 2
        drawRoundRect(base, Offset(x, size.height - 2f), Size(w, 2f))
        drawRoundRect(colour, Offset(x, size.height - h), Size(w, h), CornerRadius(w / 3, w / 3))
    }
}

private fun DrawScope.drawScope(points: FloatArray, colour: androidx.compose.ui.graphics.Color, base: androidx.compose.ui.graphics.Color) {
    val mid = size.height / 2
    drawLine(base, Offset(0f, mid), Offset(size.width, mid), strokeWidth = 2f)
    val path = Path()
    val step = size.width / (points.size - 1).coerceAtLeast(1)
    points.forEachIndexed { i, v ->
        val y = mid - v.coerceIn(-1f, 1f) * mid * 0.9f
        if (i == 0) path.moveTo(0f, y) else path.lineTo(i * step, y)
    }
    drawPath(path, colour, style = Stroke(width = 4f, cap = StrokeCap.Round))
}

private fun DrawScope.drawVu(
    meters: Array<VuMeter>,
    ink: androidx.compose.ui.graphics.Color,
    scale: androidx.compose.ui.graphics.Color,
    peakColour: androidx.compose.ui.graphics.Color,
    hot: androidx.compose.ui.graphics.Color,
) {
    val each = size.width / 2
    val sweep = 100f
    for (m in 0..1) {
        val cx = each * m + each / 2
        val cy = size.height * 0.78f
        val r = minOf(each * 0.45f, size.height * 0.6f)
        fun at(pos: Float, radius: Float): Offset {
            val a = Math.toRadians((-90f - sweep / 2 + sweep * pos).toDouble())
            return Offset(cx + radius * cos(a).toFloat(), cy + radius * sin(a).toFloat())
        }
        // The scale, red over the last fifth (the loudest 8 dB).
        for (t in 0..20) {
            val pos = t / 20f
            drawLine(if (pos > 0.8f) hot else scale, at(pos, r * 0.86f), at(pos, r), strokeWidth = if (t % 5 == 0) 4f else 2f)
        }
        val meter = meters[m]
        drawLine(peakColour, at(meter.peak, r * 0.8f), at(meter.peak, r), strokeWidth = 6f, cap = StrokeCap.Round)
        drawLine(ink, Offset(cx, cy), at(meter.needle, r * 0.95f), strokeWidth = 5f, cap = StrokeCap.Round)
        drawCircle(ink, radius = 8f, center = Offset(cx, cy))
    }
}

/**
 * The visualiser's controls on Now playing: on/off (a labelled toggle whose state says when
 * battery saver or "Remove animations" keeps it off) and, while on, the style button (each
 * press moves to the next style; its label says which is showing).
 */
@Composable
fun VisualiserControls(
    on: Boolean,
    style: VisualStyle,
    block: VisualBlock,
    onToggle: (Boolean) -> Unit,
    onStyle: (VisualStyle) -> Unit,
) {
    val stateText = when {
        block == VisualBlock.BATTERY_SAVER -> stringResource(R.string.visualiser_off_battery)
        block == VisualBlock.ANIMATIONS_OFF -> stringResource(R.string.visualiser_off_motion)
        on -> stringResource(R.string.toggle_on)
        else -> stringResource(R.string.mode_off)
    }
    Row(horizontalArrangement = Arrangement.spacedBy(0.dp)) {
        IconToggleButton(
            checked = on && block == VisualBlock.NONE,
            onCheckedChange = onToggle,
            enabled = block == VisualBlock.NONE,
            modifier = Modifier.testTag("visualiser-toggle").semantics { stateDescription = stateText },
        ) {
            Icon(Icons.Outlined.GraphicEq, contentDescription = stringResource(R.string.visualiser))
        }
        if (on && block == VisualBlock.NONE) {
            val name = visualStyleName(style)
            IconButton(onClick = { onStyle(VisualMotion.next(style)) }, modifier = Modifier.testTag("visualiser-style")) {
                Icon(Icons.Outlined.Style, contentDescription = stringResource(R.string.visualiser_style, name))
            }
        }
    }
}

/** Settings: the visualiser on Now playing (off by default) and its style. */
@Composable
fun VisualiserSetting(on: Boolean, style: VisualStyle, onChange: (Boolean) -> Unit, onStyle: (VisualStyle) -> Unit) {
    androidx.compose.foundation.layout.Column {
        ListItem(
            leadingContent = { Icon(Icons.Outlined.GraphicEq, contentDescription = null) },
            headlineContent = { Text(stringResource(R.string.settings_visualiser)) },
            supportingContent = { Text(stringResource(R.string.settings_visualiser_detail)) },
            trailingContent = { Switch(checked = on, onCheckedChange = null) },
            modifier = Modifier
                .toggleable(value = on, role = Role.Switch, onValueChange = onChange)
                .testTag("visualiser-setting"),
        )
        if (on) {
            androidx.compose.foundation.layout.Column(Modifier.selectableGroup()) {
                VisualStyle.entries.forEach { s ->
                    ListItem(
                        headlineContent = { Text(visualStyleName(s)) },
                        leadingContent = { RadioButton(selected = s == style, onClick = null) },
                        modifier = Modifier
                            .selectable(selected = s == style, role = Role.RadioButton) { onStyle(s) }
                            .testTag("visual-style-${s.name}"),
                    )
                }
            }
        }
    }
}
