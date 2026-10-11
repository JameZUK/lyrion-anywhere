package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.ui.theme.ArtPalette
import io.github.jamezuk.lyrionanywhere.ui.theme.ArtScheme
import io.github.jamezuk.lyrionanywhere.ui.theme.Hsl
import io.github.jamezuk.lyrionanywhere.ui.theme.argb
import io.github.jamezuk.lyrionanywhere.ui.theme.contrast
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.random.Random

/** Album-art colours: the seed picked from a cover, and contrast whatever the cover. */
class ArtColoursTest {
    private fun image(vararg parts: Pair<Int, Int>): IntArray = parts.flatMap { (c, n) -> List(n) { c } }.toIntArray()

    @Test
    fun aColourfulAccentWinsOverGreyBackground() {
        val grey = argb(120, 120, 120)
        val red = argb(200, 30, 30)
        val seed = ArtPalette.seed(image(grey to 7000, red to 3000))!!
        val hsl = Hsl.of(seed)
        assertTrue("hue ${hsl.h}", hsl.h < 15 || hsl.h > 345)
    }

    @Test
    fun aSpeckOfColourDoesNotCount() {
        val grey = argb(90, 90, 90)
        val speck = argb(0, 200, 0)
        val seed = ArtPalette.seed(image(grey to 10_000, speck to 10))!!
        assertTrue(Hsl.of(seed).s < 0.05)
    }

    @Test
    fun emptyOrTransparentImagesHaveNoSeed() {
        assertNull(ArtPalette.seed(IntArray(0)))
        assertNull(ArtPalette.seed(IntArray(100) { 0x00FF0000 }))
    }

    @Test
    fun blackAndWhiteCoversStillGiveAColour() {
        assertEquals(argb(0, 0, 0), ArtPalette.seed(image(argb(0, 0, 0) to 100)))
        assertEquals(argb(255, 255, 255), ArtPalette.seed(image(argb(255, 255, 255) to 100)))
    }

    /** Every tinted pair meets its contrast (WCAG AA), in both themes, for any cover. */
    @Test
    fun tintedColoursAlwaysMeetContrast() {
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
        for (seed in seeds) {
            for (dark in listOf(true, false)) {
                val s = ArtScheme.from(seed, dark)
                val where = "seed %08x dark=%s".format(seed, dark)
                fun check(name: String, fg: Int, bg: Int, min: Double) =
                    assertTrue("$where $name ${contrast(fg, bg)}", contrast(fg, bg) >= min)
                val text = ArtScheme.TEXT_CONTRAST
                check("onPrimary/primary", s.onPrimary, s.primary, text)
                check("onPrimaryContainer/primaryContainer", s.onPrimaryContainer, s.primaryContainer, text)
                check("onBackground/background", s.onBackground, s.background, text)
                for (c in listOf(s.containerLow, s.container, s.containerHigh)) check("onBackground/container", s.onBackground, c, text)
                check("onSurfaceVariant/surfaceVariant", s.onSurfaceVariant, s.surfaceVariant, text)
                check("onSurfaceVariant/background", s.onSurfaceVariant, s.background, text)
                // The accent is used for text (links, headings) on the page.
                check("primary/background", s.primary, s.background, text)
                // ...including the header's gradient, which starts at primaryContainer...
                check("primary/primaryContainer", s.primary, s.primaryContainer, text)
                // ...and on raised surfaces (cards, sheets).
                for (c in listOf(s.containerLow, s.container, s.containerHigh, s.surfaceVariant)) {
                    check("primary/container", s.primary, c, text)
                    check("onSurfaceVariant/container", s.onSurfaceVariant, c, text)
                }
                check("onBackground/surfaceVariant", s.onBackground, s.surfaceVariant, text)
                // Text on the header's gradient, from the container into the background.
                check("onBackground/primaryContainer", s.onBackground, s.primaryContainer, ArtScheme.ACCENT_CONTRAST)
            }
        }
    }

    @Test
    fun ensureMovesAwayFromTheBackground() {
        val dark = argb(10, 10, 10)
        val fg = ArtScheme.ensure(argb(40, 40, 40), dark, 4.5)
        assertTrue(contrast(fg, dark) >= 4.5)
        val light = argb(250, 250, 250)
        val fg2 = ArtScheme.ensure(argb(220, 220, 220), light, 4.5)
        assertTrue(contrast(fg2, light) >= 4.5)
    }

    @Test
    fun hslRoundTrips() {
        val rnd = Random(7)
        repeat(200) {
            val c = argb(rnd.nextInt(256), rnd.nextInt(256), rnd.nextInt(256))
            val back = Hsl.of(c).argb()
            for (shift in listOf(16, 8, 0)) {
                val d = kotlin.math.abs(((c shr shift) and 0xff) - ((back shr shift) and 0xff))
                assertTrue("%08x -> %08x".format(c, back), d <= 1)
            }
        }
    }
}
