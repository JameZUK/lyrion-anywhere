package io.github.jamezuk.lyrionanywhere.ui.theme

import android.util.LruCache
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import coil3.SingletonImageLoader
import coil3.compose.LocalPlatformContext
import coil3.request.ImageRequest
import coil3.request.SuccessResult
import coil3.request.allowHardware
import coil3.toBitmap
import io.github.jamezuk.lyrionanywhere.ui.art.Artwork
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Seeds already worked out, by artwork key (a cover's colour doesn't change). */
private val seeds = LruCache<String, Int>(200)

/** Pixels of the small image the seed is taken from. */
private const val SEED_PX = 96

/** The colour scheme tinted by [seed], keeping the base's other colours. */
fun ColorScheme.tintedBy(seed: Int, dark: Boolean): ColorScheme {
    val a = ArtScheme.from(seed, dark)
    return copy(
        primary = Color(a.primary),
        onPrimary = Color(a.onPrimary),
        primaryContainer = Color(a.primaryContainer),
        onPrimaryContainer = Color(a.onPrimaryContainer),
        secondaryContainer = Color(a.primaryContainer),
        onSecondaryContainer = Color(a.onPrimaryContainer),
        background = Color(a.background),
        onBackground = Color(a.onBackground),
        surface = Color(a.background),
        onSurface = Color(a.onBackground),
        surfaceVariant = Color(a.surfaceVariant),
        onSurfaceVariant = Color(a.onSurfaceVariant),
        surfaceContainerLowest = Color(a.background),
        surfaceContainerLow = Color(a.containerLow),
        surfaceContainer = Color(a.container),
        surfaceContainerHigh = Color(a.containerHigh),
        surfaceContainerHighest = Color(a.surfaceVariant),
        surfaceTint = Color(a.primary),
    )
}

/** The seed colour of the artwork [key], once its small image has loaded (null until then). */
@Composable
fun rememberArtSeed(key: String?): Int? {
    val context = LocalPlatformContext.current
    var seed by remember(key) { mutableStateOf(key?.let { seeds.get(it) }) }
    LaunchedEffect(key) {
        if (key == null || seed != null) return@LaunchedEffect
        val request = ImageRequest.Builder(context)
            .data(Artwork(key, SEED_PX))
            .allowHardware(false)
            .build()
        val result = SingletonImageLoader.get(context).execute(request) as? SuccessResult ?: return@LaunchedEffect
        val found = withContext(Dispatchers.Default) {
            val bmp = result.image.toBitmap()
            val pixels = IntArray(bmp.width * bmp.height)
            bmp.getPixels(pixels, 0, bmp.width, 0, 0, bmp.width, bmp.height)
            ArtPalette.seed(pixels)
        }
        if (found != null) {
            seeds.put(key, found)
            seed = found
        }
    }
    return seed
}

/**
 * [content] in the current theme tinted by the artwork's colours (animated in once the
 * image is in), or untinted while there is none.
 */
@Composable
fun ArtTinted(key: String?, content: @Composable () -> Unit) {
    val base = MaterialTheme.colorScheme
    val dark = base.background.luminance() < 0.5f || isSystemInDarkTheme() && base.background.luminance() < 0.5f
    val seed = rememberArtSeed(key)
    val target = if (seed == null) base else base.tintedBy(seed, dark)
    val spec = tween<Color>(durationMillis = 450)
    val primary by animateColorAsState(target.primary, spec, label = "primary")
    val background by animateColorAsState(target.background, spec, label = "background")
    val container by animateColorAsState(target.primaryContainer, spec, label = "container")
    MaterialTheme(
        colorScheme = target.copy(
            primary = primary,
            background = background,
            surface = background,
            primaryContainer = container,
        ),
        typography = MaterialTheme.typography,
        shapes = MaterialTheme.shapes,
        content = content,
    )
}
