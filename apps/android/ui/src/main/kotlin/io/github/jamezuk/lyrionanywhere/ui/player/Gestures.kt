package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.foundation.layout.Box
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.pointer.changedToUpIgnoreConsumed
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

    /**
     * Whose a drag on the artwork is: sideways is the artwork's (next, previous); upwards is
     * the artwork's only when it opens the queue peek ([claimUp]); anything else (down,
     * diagonal) is left to the list it sits in.
     */
    fun artworkAxis(dx: Float, dy: Float, claimUp: Boolean): DragAxis? = when (MiniGestures.axis(dx, dy)) {
        DragAxis.HORIZONTAL -> DragAxis.HORIZONTAL
        DragAxis.VERTICAL -> if (claimUp && dy < 0f) DragAxis.VERTICAL else null
        null -> null
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

/** Which way a drag goes, once decided. */
enum class DragAxis { HORIZONTAL, VERTICAL }

/**
 * The mini player's gestures (Wave 1 §4), the same on web (`gesture.ts`), Android and iOS:
 * swipe left/right for the next/previous track, swipe up to open Now playing, long-press for
 * *Play on…*, tap to open. Taps and long-presses are the click's own (with their
 * accessibility actions); these rules decide the swipes.
 */
object MiniGestures {
    /** How much more along its axis than across it a swipe must move. */
    const val AXIS_RATIO = 1.5f

    /** A sideways swipe must travel [SWIPE_MIN], or this share of the bar's width (the larger). */
    const val SWIPE_SHARE = 0.2f

    /** The axis of a drag of ([dx], [dy]) past the touch slop; null for a diagonal (not ours). */
    fun axis(dx: Float, dy: Float): DragAxis? {
        if (!dx.isFinite() || !dy.isFinite()) return null
        return when {
            kotlin.math.abs(dx) > kotlin.math.abs(dy) * AXIS_RATIO -> DragAxis.HORIZONTAL
            kotlin.math.abs(dy) > kotlin.math.abs(dx) * AXIS_RATIO -> DragAxis.VERTICAL
            else -> null
        }
    }

    /** What a finished drag along [axis] asks for. */
    fun classify(axis: DragAxis?, dx: Float, dy: Float, widthPx: Float, minPx: Float, upPx: Float): SwipeAction {
        if (axis == null || !dx.isFinite() || !dy.isFinite()) return SwipeAction.NONE
        return when (axis) {
            DragAxis.HORIZONTAL -> {
                val w = if (widthPx.isFinite() && widthPx > 0f) widthPx else 0f
                val need = maxOf(minPx, w * SWIPE_SHARE)
                when {
                    dx <= -need -> SwipeAction.NEXT
                    dx >= need -> SwipeAction.PREVIOUS
                    else -> SwipeAction.NONE
                }
            }
            DragAxis.VERTICAL -> if (dy <= -upPx) SwipeAction.OPEN else SwipeAction.NONE
        }
    }
}

/** How far a swipe must go before it counts. */
val SWIPE_MIN = 72.dp

/** How far up the mini player must be swiped to open Now playing. */
val SWIPE_UP_MIN = 40.dp

/**
 * The mini player's swipes ([MiniGestures]): sideways for next/previous, up to open. Once a
 * drag is ours its moves are consumed, so the click (tap, long-press) underneath is
 * cancelled. [onDrag] follows the finger (null when let go).
 */
fun Modifier.miniPlayerSwipes(
    minPx: Float,
    upPx: Float,
    onAction: (SwipeAction) -> Unit,
    onDrag: (Offset?) -> Unit = {},
): Modifier = pointerInput(minPx, upPx, onAction, onDrag) {
    awaitEachGesture {
        val down = awaitFirstDown(requireUnconsumed = false)
        var axis: DragAxis? = null
        var decided = false
        var delta = Offset.Zero
        while (true) {
            val event = awaitPointerEvent()
            val change = event.changes.firstOrNull { it.id == down.id } ?: break
            delta = change.position - down.position
            if (change.changedToUpIgnoreConsumed() || !change.pressed) break
            if (!decided && delta.getDistance() > viewConfiguration.touchSlop) {
                decided = true
                axis = MiniGestures.axis(delta.x, delta.y)
                if (axis == null) break // a diagonal: leave it alone
            }
            if (axis != null) {
                change.consume()
                onDrag(if (axis == DragAxis.HORIZONTAL) Offset(delta.x, 0f) else Offset(0f, minOf(0f, delta.y)))
            }
        }
        onDrag(null)
        val action = MiniGestures.classify(axis, delta.x, delta.y, size.width.toFloat(), minPx, upPx)
        if (action != SwipeAction.NONE) onAction(action)
    }
}

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
 * The artwork's sideways swipes, with a haptic tick: left calls [onNext], right
 * [onPrevious]. On Now playing these move between its layouts (Wave 2 §4.2); the track's
 * Next and Previous stay on the buttons, the mini player's swipes and TalkBack actions. It
 * follows the finger a little while dragging.
 */
@Composable
fun SwipeableArtwork(
    onNext: () -> Unit,
    onPrevious: () -> Unit,
    modifier: Modifier = Modifier,
    /** A swipe up (the queue peek, Wave 2 §3.4); null leaves upward drags to the list. */
    onSwipeUp: (() -> Unit)? = null,
    content: @Composable () -> Unit,
) {
    val density = LocalDensity.current
    val min = with(density) { SWIPE_MIN.toPx() }
    val up = with(density) { SWIPE_UP_MIN.toPx() }
    val haptics = LocalHapticFeedback.current
    var dx by remember { mutableFloatStateOf(0f) }
    val latestNext by rememberUpdatedState(onNext)
    val latestPrevious by rememberUpdatedState(onPrevious)
    val latestUp by rememberUpdatedState(onSwipeUp)
    val claimUp = onSwipeUp != null
    Box(
        modifier
            .graphicsLayer { translationX = dx * 0.5f }
            .pointerInput(min, up, claimUp) {
                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    var axis: DragAxis? = null
                    var decided = false
                    var delta = Offset.Zero
                    while (true) {
                        val event = awaitPointerEvent()
                        val change = event.changes.firstOrNull { it.id == down.id } ?: break
                        delta = change.position - down.position
                        if (change.changedToUpIgnoreConsumed() || !change.pressed) break
                        if (!decided && delta.getDistance() > viewConfiguration.touchSlop) {
                            decided = true
                            axis = Swipes.artworkAxis(delta.x, delta.y, claimUp)
                            if (axis == null) break // the list's (scrolling, pulling down to close)
                        }
                        if (axis != null) {
                            change.consume()
                            if (axis == DragAxis.HORIZONTAL) dx = delta.x
                        }
                    }
                    dx = 0f
                    val action = when (axis) {
                        DragAxis.HORIZONTAL -> Swipes.artwork(delta.x, size.width.toFloat(), min)
                        DragAxis.VERTICAL -> Swipes.miniPlayer(delta.y, up)
                        null -> SwipeAction.NONE
                    }
                    when (action) {
                        SwipeAction.NEXT -> {
                            haptics.performHapticFeedback(HapticFeedbackType.GestureEnd)
                            latestNext()
                        }
                        SwipeAction.PREVIOUS -> {
                            haptics.performHapticFeedback(HapticFeedbackType.GestureEnd)
                            latestPrevious()
                        }
                        SwipeAction.OPEN -> latestUp?.invoke()
                        else -> {}
                    }
                }
            },
        contentAlignment = Alignment.Center,
    ) { content() }
}
