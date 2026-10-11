package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.foundation.layout.Box
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.Velocity
import androidx.compose.ui.unit.dp

/** What a swipe asks for. */
enum class SwipeAction { NONE, NEXT, PREVIOUS, OPEN, CLOSE }

/**
 * The gesture rules, apart from Compose so they can be tested on their own. Distances are
 * in pixels; a swipe must travel [minPx] (or a share of the element) to count, so a tap or
 * a wobble does nothing.
 */
object Swipes {
    /** Share of the artwork's width a sideways swipe must travel. */
    const val ARTWORK_SHARE = 0.25f

    /** Artwork: left for the next track, right for the previous one. */
    fun artwork(dx: Float, widthPx: Float, minPx: Float): SwipeAction {
        if (!dx.isFinite()) return SwipeAction.NONE
        val w = if (widthPx.isFinite() && widthPx > 0f) widthPx else 0f
        val needed = maxOf(minPx, w * ARTWORK_SHARE)
        return when {
            dx <= -needed -> SwipeAction.NEXT
            dx >= needed -> SwipeAction.PREVIOUS
            else -> SwipeAction.NONE
        }
    }

    /** The mini player: up (negative) opens Now playing. */
    fun miniPlayer(dy: Float, minPx: Float): SwipeAction =
        if (dy.isFinite() && dy <= -minPx) SwipeAction.OPEN else SwipeAction.NONE

    /** Now playing: pulled down past the top by [pulledPx] closes it. */
    fun pullDown(pulledPx: Float, minPx: Float): SwipeAction =
        if (pulledPx.isFinite() && pulledPx >= minPx) SwipeAction.CLOSE else SwipeAction.NONE
}

/**
 * Tracks how far a scrolling list is pulled down once it is already at its top (the
 * scroll the list can't use), and asks to close when let go far enough down. Any scroll
 * the other way, or one the list uses, starts again.
 */
class PullToClose(private val minPx: Float, private val onClose: () -> Unit) : NestedScrollConnection {
    var pulled = 0f
        private set

    override fun onPreScroll(available: Offset, source: NestedScrollSource): Offset {
        if (available.y < 0f) pulled = 0f
        return Offset.Zero
    }

    override fun onPostScroll(consumed: Offset, available: Offset, source: NestedScrollSource): Offset {
        if (source == NestedScrollSource.UserInput) {
            if (consumed.y != 0f) pulled = 0f
            if (available.y > 0f) pulled += available.y
        }
        return Offset.Zero
    }

    override suspend fun onPreFling(available: Velocity): Velocity {
        val action = Swipes.pullDown(pulled, minPx)
        pulled = 0f
        if (action == SwipeAction.CLOSE) onClose()
        return Velocity.Zero
    }
}

/** How far a swipe must go before it counts. */
val SWIPE_MIN = 72.dp

/** The mini player: swipe up (at least [minPx]) to open Now playing. */
fun Modifier.swipeUpToOpen(minPx: Float, onOpen: () -> Unit): Modifier = pointerInput(minPx, onOpen) {
    var dy = 0f
    detectVerticalDragGestures(
        onDragStart = { dy = 0f },
        onDragEnd = { if (Swipes.miniPlayer(dy, minPx) == SwipeAction.OPEN) onOpen() },
        onDragCancel = { dy = 0f },
    ) { change, amount ->
        dy += amount
        change.consume()
    }
}

/**
 * The artwork: swipe left for the next track and right for the previous one, with a
 * haptic tick. It follows the finger a little while dragging.
 */
@Composable
fun SwipeableArtwork(
    onNext: () -> Unit,
    onPrevious: () -> Unit,
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    val min = with(LocalDensity.current) { SWIPE_MIN.toPx() }
    val haptics = LocalHapticFeedback.current
    var dx by remember { mutableFloatStateOf(0f) }
    Box(
        modifier
            .graphicsLayer { translationX = dx * 0.5f }
            .pointerInput(onNext, onPrevious) {
                detectHorizontalDragGestures(
                    onDragStart = { dx = 0f },
                    onDragEnd = {
                        val action = Swipes.artwork(dx, size.width.toFloat(), min)
                        dx = 0f
                        if (action != SwipeAction.NONE) {
                            haptics.performHapticFeedback(HapticFeedbackType.GestureEnd)
                            if (action == SwipeAction.NEXT) onNext() else onPrevious()
                        }
                    },
                    onDragCancel = { dx = 0f },
                ) { change, amount ->
                    dx += amount
                    change.consume()
                }
            },
        contentAlignment = Alignment.Center,
    ) { content() }
}
