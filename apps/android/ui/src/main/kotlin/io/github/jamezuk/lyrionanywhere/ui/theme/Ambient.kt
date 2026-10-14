package io.github.jamezuk.lyrionanywhere.ui.theme

import android.app.ActivityManager
import android.content.Context
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.BlurredEdgeTreatment
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.ColorMatrix
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import coil3.compose.AsyncImage
import io.github.jamezuk.lyrionanywhere.ui.art.Artwork
import kotlin.math.min

/**
 * Now playing's ambient background (Wave 1 §2), as the web app's `theme/ambient.ts`: the
 * cover, small and blurred, drifting slowly behind a scrim of the page colour. Two things
 * keep every text colour at WCAG AA over any cover:
 * - a colour filter bounds the art's own range (linear maps per sRGB channel): at most half
 *   bright in dark mode, washed light in light mode;
 * - the scrim is then as light as it can be while the worst case still passes (mixing in
 *   sRGB, the contrast is worst at one end of the art's range).
 *
 * Plain Kotlin apart from the composables, so the maths is tested on the JVM.
 */
object Ambient {
    /** `contrast` then `brightness`, as linear maps on each channel (0–1), then clamped. */
    data class ArtFilter(val contrast: Float, val brightness: Float) {
        fun map(x: Float): Float {
            val v = if (x.isFinite()) x.coerceIn(0f, 1f) else 0f
            return (((v - 0.5f) * contrast + 0.5f) * brightness).coerceIn(0f, 1f)
        }

        val lo: Float get() = map(0f)
        val hi: Float get() = map(1f)

        /** The same map as a colour matrix (offsets in 0–255), alpha untouched. */
        fun matrix(): ColorMatrix {
            val k = contrast * brightness
            val o = 255f * (1f - contrast) * 0.5f * brightness
            return ColorMatrix(
                floatArrayOf(
                    k, 0f, 0f, 0f, o,
                    0f, k, 0f, 0f, o,
                    0f, 0f, k, 0f, o,
                    0f, 0f, 0f, 1f, 0f,
                ),
            )
        }
    }

    /** Dark: at most half bright. Light: washed out and lifted (as the web app). */
    val DARK = ArtFilter(contrast = 1f, brightness = 0.5f)
    val LIGHT = ArtFilter(contrast = 0.3f, brightness = 1.6f)

    fun filter(dark: Boolean) = if (dark) DARK else LIGHT

    const val MIN_SCRIM = 0.3f
    const val MAX_SCRIM = 0.96f

    /** The mini player's surface over its faint art tint: at least this opaque. */
    const val MINI_TINT_SURFACE = 0.94f

    /** [top] at [alpha] over the opaque [under] (ARGB; sRGB mixing). */
    fun over(top: Int, alpha: Float, under: Int): Int {
        val k = if (alpha.isFinite()) alpha.coerceIn(0f, 1f) else 1f
        fun ch(shift: Int) = (((top shr shift) and 0xff) * k + ((under shr shift) and 0xff) * (1 - k)).let { kotlin.math.round(it).toInt() }
        return argb(ch(16), ch(8), ch(0))
    }

    private fun grey(v: Float): Int = (v.coerceIn(0f, 1f) * 255f).let { kotlin.math.round(it).toInt() }.let { argb(it, it, it) }

    /** The lowest contrast [fg] has on [bg] at [alpha] over art in [lo]..[hi] (per channel). */
    fun worstContrast(fg: Int, bg: Int, alpha: Float, lo: Float = 0f, hi: Float = 1f): Double =
        min(contrast(fg, over(bg, alpha, grey(lo))), contrast(fg, over(bg, alpha, grey(hi))))

    /**
     * The scrim's opacity: the lowest (0.3–0.96, steps of 0.01) at which every [texts]
     * colour keeps 4.5:1 and every [accents] colour 3:1 on [bg] over the filtered art.
     */
    fun scrimAlpha(bg: Int, texts: List<Int>, accents: List<Int>, dark: Boolean): Float {
        val f = filter(dark)
        var a = MIN_SCRIM
        while (a <= MAX_SCRIM + 1e-6f) {
            val r = kotlin.math.round(a * 100f) / 100f
            val ok = texts.all { worstContrast(it, bg, r, f.lo, f.hi) >= ArtScheme.TEXT_CONTRAST } &&
                accents.all { worstContrast(it, bg, r, f.lo, f.hi) >= ArtScheme.ACCENT_CONTRAST }
            if (ok) return r
            a += 0.01f
        }
        return MAX_SCRIM
    }

    /** The lightest a small control's own scrim (Now playing's close button) may be. */
    const val MIN_ICON_SCRIM = 0.55f

    /**
     * The opacity of a small control's own scrim of [bg] (a disc behind an icon drawn over
     * anything: art, the halo, the ambient background): the lowest (from [MIN_ICON_SCRIM],
     * steps of 0.01) at which [fg] keeps 4.5:1 over anything from black to white beneath;
     * fully opaque if nothing less will do.
     */
    fun iconScrimAlpha(fg: Int, bg: Int): Float {
        var a = MIN_ICON_SCRIM
        while (a <= 1f + 1e-6f) {
            val r = kotlin.math.round(a * 100f) / 100f
            if (worstContrast(fg, bg, r) >= ArtScheme.TEXT_CONTRAST) return r
            a += 0.01f
        }
        return 1f
    }

    /** The scrim for a colour scheme: its text and accent colours on its background. */
    fun scrimAlpha(scheme: ColorScheme, dark: Boolean): Float = scrimAlpha(
        scheme.background.toArgb(),
        listOf(scheme.onBackground, scheme.onSurface, scheme.onSurfaceVariant).map { it.toArgb() },
        listOf(scheme.primary).map { it.toArgb() },
        dark,
    )

    /** The mini player's surface with a faint wash of the cover's colour [seed]. */
    fun miniTint(surface: Int, seed: Int): Int = over(surface, MINI_TINT_SURFACE, seed)

    /**
     * Whether the background should hold still: animations turned off (or slowed to nothing)
     * in the system's settings, battery saver, or a low-RAM device.
     */
    fun still(context: Context): Boolean {
        val scale = try {
            Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f)
        } catch (_: Exception) {
            1f
        }
        if (scale == 0f) return true
        val power = context.getSystemService(PowerManager::class.java)
        if (power?.isPowerSaveMode == true) return true
        val activity = context.getSystemService(ActivityManager::class.java)
        return activity?.isLowRamDevice == true
    }
}

/** A light or dark scheme, by its background. */
fun ColorScheme.isDark(): Boolean = background.luminance() < 0.5f

/**
 * The ambient background: the cover [artwork] blurred (RenderEffect on Android 12+; below
 * that a tiny image scaled up, which is already a blur), filtered and drifting, behind the
 * scrim. Holds still when [Ambient.still] says so; a new cover cross-fades in. Nothing under
 * the system's high-contrast setting: the plain page colour.
 */
@Composable
fun AmbientBackground(
    artwork: String?,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val context = LocalContext.current
    val dark = scheme.isDark()
    val still = remember(context) { Ambient.still(context) }
    val highContrast = remember(context) { isHighContrast(context) }
    val alpha = remember(scheme.background, scheme.onBackground, scheme.onSurfaceVariant, scheme.primary, dark) {
        Ambient.scrimAlpha(scheme, dark)
    }
    // Clipped: the drifting cover is scaled and blurred past its bounds, and must not paint
    // over what sits above the page, such as the connection line.
    Box(modifier.fillMaxSize().clipToBounds().background(scheme.background).testTag("ambient")) {
        if (artwork != null && !highContrast) {
            val drift = if (still) {
                null
            } else {
                val t = rememberInfiniteTransition(label = "ambient")
                t.animateFloat(
                    initialValue = 0f,
                    targetValue = 1f,
                    animationSpec = infiniteRepeatable(tween(DRIFT_MS, easing = LinearEasing), RepeatMode.Reverse),
                    label = "drift",
                )
            }
            val filter = remember(dark) { ColorFilter.colorMatrix(Ambient.filter(dark).matrix()) }
            Crossfade(targetState = artwork, animationSpec = tween(if (still) 0 else 900), label = "ambient art") { key ->
                AsyncImage(
                    model = Artwork(key, AMBIENT_PX),
                    contentDescription = null,
                    contentScale = ContentScale.Crop,
                    colorFilter = filter,
                    modifier = Modifier
                        .fillMaxSize()
                        .graphicsLayer {
                            val p = drift?.value ?: 0.5f
                            scaleX = 1.35f + 0.12f * p
                            scaleY = 1.35f + 0.12f * p
                            translationX = (p - 0.5f) * size.width * 0.08f
                            translationY = (0.5f - p) * size.height * 0.05f
                            rotationZ = (p - 0.5f) * 8f
                        }
                        .then(
                            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                                Modifier.blur(48.dp, BlurredEdgeTreatment.Unbounded)
                            } else {
                                Modifier
                            },
                        ),
                )
            }
        }
        Box(Modifier.fillMaxSize().background(scheme.background.copy(alpha = if (artwork != null && !highContrast) alpha else 1f)))
    }
}

/** The cover's size for the background: tiny, so it is cheap and (scaled up) already soft. */
private const val AMBIENT_PX = 48

/** One drift across and back takes twice this. */
private const val DRIFT_MS = 40_000

/** [c] with a faint wash of [seed] (the mini player's art tint), or [c] without a seed. */
fun tinted(c: Color, seed: Int?): Color = if (seed == null) c else Color(Ambient.miniTint(c.toArgb(), seed))
