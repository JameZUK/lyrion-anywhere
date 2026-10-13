package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.animation.core.withInfiniteAnimationFrameMillis
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.theme.Ambient
import io.github.jamezuk.lyrionanywhere.ui.theme.isHighContrast
import io.github.jamezuk.lyrionanywhere.ui.theme.rememberArtPalette
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.roundToInt
import kotlin.math.sin

/**
 * Now playing's close button: a small chevron at the top left, over the art area, on a disc
 * of its own (the page colour, as opaque as it must be for 4.5:1 over anything beneath, so it
 * reads in light and dark themes over any cover). System back and the swipe down close too.
 */
@Composable
fun NowPlayingCloseButton(onClose: () -> Unit, modifier: Modifier = Modifier) {
    val scheme = MaterialTheme.colorScheme
    val fg = scheme.onSurface
    val bg = scheme.surface
    val alpha = remember(fg, bg) { Ambient.iconScrimAlpha(fg.toArgb(), bg.toArgb()) }
    IconButton(onClick = onClose, modifier = modifier.testTag("now-playing-close")) {
        Box(
            Modifier.size(36.dp).background(bg.copy(alpha = alpha), CircleShape).testTag("now-playing-close-scrim"),
            contentAlignment = Alignment.Center,
        ) {
            Icon(Icons.Outlined.KeyboardArrowDown, contentDescription = stringResource(R.string.now_playing_close), tint = fg)
        }
    }
}

/** The space the close button takes at the start of the row it sits over. */
internal val CLOSE_INSET = 48.dp

/**
 * The visualiser's Glow ([Glow]): a soft halo behind [content] (the cover) in the cover's own
 * colours, as overlapping radial lobes that turn once every [Glow.TURN_MS] while [playing].
 * Each bass onset pulses its radius and opacity (and the cover's scale, very slightly); the
 * smoothed bass level breathes it gently between beats. With [animate] false (battery saver,
 * animations removed) it holds still: no turn, no pulse.
 *
 * The halo spreads sideways but only [fade] above and below the cover, faded out there, so it
 * never sits under text (the title below and the layout row above keep their contrast). None
 * under the system's high-contrast setting.
 */
@Composable
internal fun GlowHalo(
    artwork: String?,
    visual: VisualState,
    playing: Boolean,
    animate: Boolean,
    modifier: Modifier = Modifier,
    fade: Dp = 10.dp,
    content: @Composable () -> Unit,
) {
    val context = LocalContext.current
    val highContrast = remember(context) { isHighContrast(context) }
    val accent = MaterialTheme.colorScheme.primary
    val found = rememberArtPalette(artwork)
    val colours = remember(found, accent) {
        val c = found.ifEmpty { listOf(accent.toArgb()) }.take(Glow.MAX_COLOURS).map { Color(it) }
        // Always three lobes, 120° apart (fewer colours repeat), so the turn shows.
        List(Glow.LOBES) { c[it % c.size] }
    }
    var playingMs by remember { mutableLongStateOf(0L) }
    val turning = animate && playing && !highContrast
    LaunchedEffect(turning) {
        if (!turning) return@LaunchedEffect
        var last = 0L
        while (true) {
            withInfiniteAnimationFrameMillis { now ->
                if (last == 0L) {
                    last = now
                } else if (now - last >= VisualMotion.FRAME_MS) {
                    playingMs = (playingMs + (now - last)) % Glow.TURN_MS
                    last = now
                }
            }
        }
    }
    val fadePx = with(LocalDensity.current) { fade.toPx() }
    Box(modifier.testTag("glow-halo")) {
        if (!highContrast) {
            Box(
                Modifier
                    .matchParentSize()
                    // The halo's own layer: wider than the cover (it spreads sideways), [fade]
                    // taller each way, then faded out at the top and bottom.
                    .layout { measurable, constraints ->
                        val w = constraints.maxWidth
                        val h = constraints.maxHeight
                        val ex = (minOf(w, h) * HALO_REACH).roundToInt()
                        val ey = fadePx.roundToInt()
                        val p = measurable.measure(Constraints.fixed(w + 2 * ex, h + 2 * ey))
                        layout(w, h) { p.place(-ex, -ey) }
                    }
                    .graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }
                    .drawBehind {
                        val ey = fadePx
                        val coverH = size.height - 2 * ey
                        val s = minOf(size.width, coverH).coerceAtLeast(1f)
                        val pulse = if (animate) visual.pulse else 0f
                        val level = if (animate) visual.glow else 0f
                        val drive = Glow.drive(pulse, level)
                        val radius = HALO_RADIUS * s * Glow.radius(drive)
                        val opacity = Glow.opacity(drive)
                        val turn = if (animate) Glow.angle(playingMs) else 0f
                        val centre = Offset(size.width / 2f, size.height / 2f)
                        colours.forEachIndexed { i, colour ->
                            val a = ((turn + i * 360f / colours.size) * PI / 180.0).toFloat()
                            val c = centre + Offset(cos(a), sin(a)) * (LOBE_OFFSET * s)
                            drawCircle(
                                Brush.radialGradient(
                                    0f to colour.copy(alpha = opacity * LOBE_ALPHA),
                                    0.55f to colour.copy(alpha = opacity * LOBE_ALPHA * 0.45f),
                                    1f to Color.Transparent,
                                    center = c,
                                    radius = radius,
                                ),
                                radius = radius,
                                center = c,
                            )
                        }
                        // Fade out above and below the cover.
                        val edge = (ey / size.height).coerceIn(0f, 0.5f)
                        drawRect(
                            Brush.verticalGradient(
                                0f to Color.Transparent,
                                edge to Color.Black,
                                1f - edge to Color.Black,
                                1f to Color.Transparent,
                            ),
                            blendMode = BlendMode.DstIn,
                        )
                    },
            )
        }
        Box(
            Modifier.graphicsLayer {
                val k = if (animate) Glow.coverScale(visual.pulse) else 1f
                scaleX = k
                scaleY = k
            },
        ) { content() }
    }
}

/** The halo's base radius, of the cover's size. */
private const val HALO_RADIUS = 0.55f

/** How far each lobe's centre sits from the cover's centre, of its size. */
private const val LOBE_OFFSET = 0.12f

/** Each lobe's share of the opacity (they overlap). */
private const val LOBE_ALPHA = 0.8f

/** How far the halo can reach beyond the cover's sides, of its size. */
private const val HALO_REACH = (HALO_RADIUS * Glow.RADIUS_MAX + LOBE_OFFSET) - 0.5f + 0.02f
