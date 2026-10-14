package io.github.jamezuk.lyrionanywhere.navigation

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Which screens hide the tabs (Wave 3.5: Now playing is a sheet over the app). */
class WholeScreenTest {
    @Test
    fun nowPlayingCarModeAndTheStoryHaveTheWholeScreen() {
        assertTrue(wholeScreen(nowPlaying = true, car = false, story = false))
        assertTrue(wholeScreen(nowPlaying = false, car = true, story = false))
        assertTrue(wholeScreen(nowPlaying = false, car = false, story = true))
    }

    @Test
    fun everyOtherScreenKeepsTheTabs() {
        assertFalse(wholeScreen(nowPlaying = false, car = false, story = false))
    }
}
