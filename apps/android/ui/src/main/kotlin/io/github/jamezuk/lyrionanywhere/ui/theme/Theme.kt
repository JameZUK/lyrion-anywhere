package io.github.jamezuk.lyrionanywhere.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.platform.LocalContext

// The brand palette, used where dynamic colour isn't available (Android 10 and 11). A deep
// teal accent, like the web app's Material theme. Album-art colour comes later (A4).
internal val BrandLight = lightColorScheme(
    primary = Color(0xFF006A6A),
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFF9CF1F0),
    onPrimaryContainer = Color(0xFF002020),
    secondary = Color(0xFF4A6363),
    onSecondary = Color(0xFFFFFFFF),
    secondaryContainer = Color(0xFFCCE8E7),
    onSecondaryContainer = Color(0xFF051F1F),
    tertiary = Color(0xFF4B607C),
    onTertiary = Color(0xFFFFFFFF),
    background = Color(0xFFF4FBFA),
    onBackground = Color(0xFF161D1D),
    surface = Color(0xFFF4FBFA),
    onSurface = Color(0xFF161D1D),
    surfaceVariant = Color(0xFFDAE5E4),
    onSurfaceVariant = Color(0xFF3F4948),
    error = Color(0xFFBA1A1A),
    onError = Color(0xFFFFFFFF),
)

internal val BrandDark = darkColorScheme(
    primary = Color(0xFF80D5D4),
    onPrimary = Color(0xFF003737),
    primaryContainer = Color(0xFF004F4F),
    onPrimaryContainer = Color(0xFF9CF1F0),
    secondary = Color(0xFFB0CCCB),
    onSecondary = Color(0xFF1B3534),
    secondaryContainer = Color(0xFF324B4B),
    onSecondaryContainer = Color(0xFFCCE8E7),
    tertiary = Color(0xFFB3C8E8),
    onTertiary = Color(0xFF1C314B),
    background = Color(0xFF0E1514),
    onBackground = Color(0xFFDDE4E3),
    surface = Color(0xFF0E1514),
    onSurface = Color(0xFFDDE4E3),
    surfaceVariant = Color(0xFF3F4948),
    onSurfaceVariant = Color(0xFFBEC9C8),
    error = Color(0xFFFFB4AB),
    onError = Color(0xFF690005),
)

/**
 * The quality badge's and signal-path diagram's amber (docs/plans/signal-path-redesign.md,
 * decision 2): a subtle tint for Transcoded and Lossy, and for the block that lowers the
 * verdict. The word always carries the meaning; the colour only echoes it. The same in
 * every scheme (wallpaper colours have no amber of their own), a light and a dark pair.
 */
data class AmberTint(val container: Color, val onContainer: Color)

internal val AmberLight = AmberTint(container = Color(0xFFFFE2B8), onContainer = Color(0xFF3A2400))
internal val AmberDark = AmberTint(container = Color(0xFF5A3F12), onContainer = Color(0xFFFFDDAF))

/** The amber for the scheme in use: the dark pair on a dark surface. */
@Composable
fun amberTint(): AmberTint = if (MaterialTheme.colorScheme.surface.luminance() < 0.5f) AmberDark else AmberLight

/** The colour scheme: the wallpaper's (Android 12+) when [dynamicColour] is on, else ours. */
@Composable
fun appColorScheme(dark: Boolean, dynamicColour: Boolean): ColorScheme {
    val context = LocalContext.current
    return when {
        dynamicColour && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S ->
            if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        dark -> BrandDark
        else -> BrandLight
    }
}

@Composable
fun LyrionTheme(
    dark: Boolean = isSystemInDarkTheme(),
    dynamicColour: Boolean = true,
    content: @Composable () -> Unit,
) {
    MaterialTheme(colorScheme = appColorScheme(dark, dynamicColour), content = content)
}
