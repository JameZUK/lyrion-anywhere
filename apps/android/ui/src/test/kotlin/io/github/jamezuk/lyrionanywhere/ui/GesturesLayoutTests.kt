package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.unit.Velocity
import io.github.jamezuk.lyrionanywhere.ui.components.WidthClass
import io.github.jamezuk.lyrionanywhere.ui.components.WindowLayout
import io.github.jamezuk.lyrionanywhere.ui.player.PullToClose
import io.github.jamezuk.lyrionanywhere.ui.player.SwipeAction
import io.github.jamezuk.lyrionanywhere.ui.player.Swipes
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SwipesTest {
    @Test
    fun theArtworkNeedsADeliberateSwipe() {
        // 1000 px wide: a quarter of it (250 px) is needed, more than the minimum.
        assertEquals(SwipeAction.NEXT, Swipes.artwork(-260f, 1000f, 100f))
        assertEquals(SwipeAction.PREVIOUS, Swipes.artwork(260f, 1000f, 100f))
        assertEquals(SwipeAction.NONE, Swipes.artwork(-200f, 1000f, 100f))
        // Narrow: the minimum applies.
        assertEquals(SwipeAction.NONE, Swipes.artwork(-90f, 200f, 100f))
        assertEquals(SwipeAction.NEXT, Swipes.artwork(-100f, 200f, 100f))
        // Nonsense never skips.
        for (dx in listOf(Float.NaN, Float.POSITIVE_INFINITY, Float.NEGATIVE_INFINITY, 0f)) {
            assertEquals(SwipeAction.NONE, Swipes.artwork(dx, 1000f, 100f))
        }
        assertEquals(SwipeAction.NEXT, Swipes.artwork(-150f, Float.NaN, 100f))
    }

    @Test
    fun theMiniPlayerOpensOnlyWhenSwipedUp() {
        assertEquals(SwipeAction.OPEN, Swipes.miniPlayer(-120f, 100f))
        assertEquals(SwipeAction.NONE, Swipes.miniPlayer(-50f, 100f))
        assertEquals(SwipeAction.NONE, Swipes.miniPlayer(300f, 100f))
        assertEquals(SwipeAction.NONE, Swipes.miniPlayer(Float.NaN, 100f))
    }

    @Test
    fun pullingDownPastTheTopCloses() = runTest {
        var closed = 0
        val pull = PullToClose(100f) { closed++ }
        // The list scrolls first (it uses the scroll): nothing pulled.
        pull.onPostScroll(Offset(0f, 40f), Offset.Zero, NestedScrollSource.UserInput)
        assertEquals(0f, pull.pulled)
        // At the top: what the list can't use is pulled.
        pull.onPostScroll(Offset.Zero, Offset(0f, 60f), NestedScrollSource.UserInput)
        pull.onPostScroll(Offset.Zero, Offset(0f, 60f), NestedScrollSource.UserInput)
        pull.onPreFling(Velocity.Zero)
        assertEquals(1, closed)
        assertEquals(0f, pull.pulled)
        // Not far enough, or reversed: stays open.
        pull.onPostScroll(Offset.Zero, Offset(0f, 60f), NestedScrollSource.UserInput)
        pull.onPreFling(Velocity.Zero)
        pull.onPostScroll(Offset.Zero, Offset(0f, 90f), NestedScrollSource.UserInput)
        pull.onPreScroll(Offset(0f, -5f), NestedScrollSource.UserInput)
        pull.onPostScroll(Offset.Zero, Offset(0f, 20f), NestedScrollSource.UserInput)
        pull.onPreFling(Velocity.Zero)
        assertEquals(1, closed)
        // A fling's own scrolling never counts.
        pull.onPostScroll(Offset.Zero, Offset(0f, 500f), NestedScrollSource.SideEffect)
        pull.onPreFling(Velocity.Zero)
        assertEquals(1, closed)
    }
}

class WindowLayoutTest {
    @Test
    fun widthClassesUseMaterialBreakpoints() {
        assertEquals(WidthClass.COMPACT, WindowLayout.widthClass(0f))
        assertEquals(WidthClass.COMPACT, WindowLayout.widthClass(599.9f))
        assertEquals(WidthClass.MEDIUM, WindowLayout.widthClass(600f))
        assertEquals(WidthClass.MEDIUM, WindowLayout.widthClass(839f))
        assertEquals(WidthClass.EXPANDED, WindowLayout.widthClass(840f))
        assertEquals(WidthClass.EXPANDED, WindowLayout.widthClass(2000f))
        assertEquals(WidthClass.COMPACT, WindowLayout.widthClass(Float.NaN))
    }

    @Test
    fun phonesUseOnePaneTabletsTwo() {
        val phone = WindowLayout.of(411f, 891f)
        assertFalse(phone.listDetail)
        assertFalse(phone.nowPlayingTwoPane)
        val tablet = WindowLayout.of(1280f, 800f)
        assertTrue(tablet.listDetail)
        assertTrue(tablet.nowPlayingTwoPane)
        // A small tablet upright, or a foldable open: one list at a time, Now playing in one column.
        val medium = WindowLayout.of(700f, 1000f)
        assertFalse(medium.listDetail)
        assertFalse(medium.nowPlayingTwoPane)
        // A phone on its side: too short for one column, so the queue goes beside.
        val sideways = WindowLayout.of(800f, 380f)
        assertEquals(WidthClass.MEDIUM, sideways.width)
        assertTrue(sideways.nowPlayingTwoPane)
        assertFalse(sideways.listDetail)
        // Height unknown: treated as tall.
        assertFalse(WindowLayout.of(700f, Float.NaN).nowPlayingTwoPane)
    }
}
