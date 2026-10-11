package io.github.jamezuk.lyrionanywhere.ui

import android.app.UiModeManager
import android.content.Context
import androidx.compose.material3.ColorScheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.ui.player.UNPLAYED_ALPHA
import io.github.jamezuk.lyrionanywhere.ui.theme.ArtScheme
import io.github.jamezuk.lyrionanywhere.ui.theme.BrandDark
import io.github.jamezuk.lyrionanywhere.ui.theme.BrandLight
import io.github.jamezuk.lyrionanywhere.ui.theme.argb
import io.github.jamezuk.lyrionanywhere.ui.theme.contrast
import io.github.jamezuk.lyrionanywhere.ui.theme.isHighContrast
import io.github.jamezuk.lyrionanywhere.ui.theme.tintedBy
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import kotlin.random.Random

/** [fg] at [alpha], drawn over the opaque [bg] (as the waveform's unplayed bars are). */
private fun over(fg: Color, alpha: Float, bg: Color): Int = Color(
    red = fg.red * alpha + bg.red * (1 - alpha),
    green = fg.green * alpha + bg.green * (1 - alpha),
    blue = fg.blue * alpha + bg.blue * (1 - alpha),
).toArgb()

/**
 * WCAG AA contrast for the app's colours: the brand themes (Android 10 and 11, or dynamic
 * colour off) and the album-art tinted themes built on them, light and dark.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class ContrastTests {
    private fun schemeProblems(name: String, s: ColorScheme): List<String> {
        val problems = mutableListOf<String>()
        fun check(pair: String, fg: Color, bg: Color, min: Double) {
            val c = contrast(fg.toArgb(), bg.toArgb())
            if (c < min) problems += "$name $pair: %.2f < %.1f".format(c, min)
        }
        val text = ArtScheme.TEXT_CONTRAST
        check("onPrimary/primary", s.onPrimary, s.primary, text)
        check("onPrimaryContainer/primaryContainer", s.onPrimaryContainer, s.primaryContainer, text)
        check("onSecondary/secondary", s.onSecondary, s.secondary, text)
        check("onSecondaryContainer/secondaryContainer", s.onSecondaryContainer, s.secondaryContainer, text)
        check("onTertiary/tertiary", s.onTertiary, s.tertiary, text)
        check("onTertiaryContainer/tertiaryContainer", s.onTertiaryContainer, s.tertiaryContainer, text)
        check("onError/error", s.onError, s.error, text)
        check("onErrorContainer/errorContainer", s.onErrorContainer, s.errorContainer, text)
        check("onBackground/background", s.onBackground, s.background, text)
        check("onSurface/surface", s.onSurface, s.surface, text)
        check("onSurfaceVariant/surfaceVariant", s.onSurfaceVariant, s.surfaceVariant, text)
        // Section headings and links are primary text, on the page and on the header's
        // gradient (primaryContainer into the background).
        check("primary/background", s.primary, s.background, text)
        check("primary/primaryContainer", s.primary, s.primaryContainer, text)
        val containers = mapOf(
            "surfaceContainerLowest" to s.surfaceContainerLowest,
            "surfaceContainerLow" to s.surfaceContainerLow,
            "surfaceContainer" to s.surfaceContainer,
            "surfaceContainerHigh" to s.surfaceContainerHigh,
            "surfaceContainerHighest" to s.surfaceContainerHighest,
        )
        for ((n, c) in containers) {
            check("onSurface/$n", s.onSurface, c, text)
            check("onSurfaceVariant/$n", s.onSurfaceVariant, c, text)
        }
        // The waveform's unplayed bars: a graphic, so 3:1, over the page.
        val unplayed = over(s.onSurfaceVariant, UNPLAYED_ALPHA, s.background)
        val wave = contrast(unplayed, s.background.toArgb())
        if (wave < ArtScheme.ACCENT_CONTRAST) problems += "$name waveform unplayed/background: %.2f < 3".format(wave)
        return problems
    }

    @Test
    fun theBrandThemesMeetContrast() {
        val problems = schemeProblems("light", BrandLight) + schemeProblems("dark", BrandDark)
        assertTrue(problems.joinToString("\n"), problems.isEmpty())
    }

    /** The same pairs once a cover tints the page, for the covers ArtColoursTest uses. */
    @Test
    fun tintedThemesMeetContrast() {
        val rnd = Random(20261011)
        val seeds = buildList {
            addAll(
                listOf(
                    argb(0, 0, 0), argb(255, 255, 255), argb(255, 255, 0), argb(0, 0, 255),
                    argb(255, 0, 0), argb(0, 255, 0), argb(0, 255, 255), argb(255, 0, 255),
                    argb(128, 128, 128), argb(250, 240, 200), argb(20, 10, 40),
                ),
            )
            repeat(500) { add(argb(rnd.nextInt(256), rnd.nextInt(256), rnd.nextInt(256))) }
        }
        val problems = mutableListOf<String>()
        for (seed in seeds) {
            problems += schemeProblems("seed %08x light".format(seed), BrandLight.tintedBy(seed, dark = false))
            problems += schemeProblems("seed %08x dark".format(seed), BrandDark.tintedBy(seed, dark = true))
        }
        assertTrue(problems.take(20).joinToString("\n"), problems.isEmpty())
    }

    /** With the system's contrast turned up (Android 14+), pages aren't tinted by covers. */
    @Test
    fun systemHighContrastIsNoticed() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        assertFalse(isHighContrast(context))
        shadowOf(context.getSystemService(UiModeManager::class.java)).setContrast(1f)
        assertTrue(isHighContrast(context))
        shadowOf(context.getSystemService(UiModeManager::class.java)).setContrast(0f)
        assertFalse(isHighContrast(context))
    }
}
