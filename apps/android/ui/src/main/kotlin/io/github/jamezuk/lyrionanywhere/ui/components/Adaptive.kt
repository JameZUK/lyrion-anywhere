package io.github.jamezuk.lyrionanywhere.ui.components

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import io.github.jamezuk.lyrionanywhere.ui.R

/** Material 3 window width classes (the breakpoints are Material's: 600 dp and 840 dp). */
enum class WidthClass { COMPACT, MEDIUM, EXPANDED }

/**
 * How screens lay themselves out for the window they have. The rules are here, apart
 * from the screens, so they can be tested by size alone.
 */
data class WindowLayout(val width: WidthClass, val heightDp: Float) {
    /** Browse lists show the chosen item beside them (list and detail) rather than on top. */
    val listDetail: Boolean get() = width == WidthClass.EXPANDED

    /**
     * Now playing puts the queue beside the controls: on wide windows, and on medium ones
     * too short for one column (a phone or small tablet held sideways).
     */
    val nowPlayingTwoPane: Boolean
        get() = width == WidthClass.EXPANDED || (width == WidthClass.MEDIUM && heightDp < SHORT_HEIGHT_DP)

    /**
     * Now playing's wide layout (Wave 2 §4.2): art on the left, lyrics or info on the right,
     * the queue as a side panel. Windows over 840 dp (tablets, and landscape on large phones).
     */
    val nowPlayingWide: Boolean get() = width == WidthClass.EXPANDED

    companion object {
        const val MEDIUM_DP = 600f
        const val EXPANDED_DP = 840f
        /** Below this height a single scrolling column leaves too little of the queue in view. */
        const val SHORT_HEIGHT_DP = 480f

        fun widthClass(widthDp: Float): WidthClass = when {
            !widthDp.isFinite() || widthDp < MEDIUM_DP -> WidthClass.COMPACT
            widthDp < EXPANDED_DP -> WidthClass.MEDIUM
            else -> WidthClass.EXPANDED
        }

        fun of(widthDp: Float, heightDp: Float): WindowLayout =
            WindowLayout(widthClass(widthDp), if (heightDp.isFinite()) heightDp else Float.MAX_VALUE)

        val PHONE = WindowLayout(WidthClass.COMPACT, 800f)
    }
}

/** The window layout for the screen being drawn (set once, near the root). */
val LocalWindowLayout = staticCompositionLocalOf { WindowLayout.PHONE }

/**
 * A list with the chosen item's page beside it on wide windows. [detail] is drawn only
 * when something is chosen; otherwise [placeholder] (e.g. "choose an album").
 */
@Composable
fun ListDetail(
    list: @Composable () -> Unit,
    detail: (@Composable () -> Unit)?,
    placeholder: @Composable () -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(modifier.fillMaxSize()) {
        Box(Modifier.weight(LIST_WEIGHT).fillMaxHeight().testTag("list-pane")) { list() }
        VerticalDivider()
        val pane = stringResource(R.string.detail_pane)
        Box(
            Modifier
                .weight(1f - LIST_WEIGHT)
                .fillMaxHeight()
                .testTag("detail-pane")
                // A pane: TalkBack announces it when its content changes.
                .semantics { paneTitle = pane },
        ) {
            if (detail != null) detail() else placeholder()
        }
    }
}

private const val LIST_WEIGHT = 0.45f
