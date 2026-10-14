package io.github.jamezuk.lyrionanywhere.ui.components

import androidx.compose.foundation.layout.Box
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.SubcomposeLayout
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * One action of an [OverflowRow]: shown as [inline] (its own button) while it fits, otherwise
 * as an item labelled [label] in the row's ⋮ menu. [tag] marks the menu item, so either way
 * the action can be found.
 */
class RowAction(
    val label: String,
    val icon: ImageVector?,
    val onClick: () -> Unit,
    val enabled: Boolean = true,
    val tag: String? = null,
    val inline: @Composable () -> Unit,
)

/**
 * A row that never wraps: [lead] (a heading, one line, ellipsised) takes the width the rest
 * leaves; [pinned] (Play, Shuffle) always shows; [actions] show as their own buttons, in
 * order, while they fit, and the rest move into a ⋮ menu with their labels. [menuItems] are
 * always in the menu. The ⋮ is there whenever something is in it, or always when [onMore] is
 * given: then, with nothing in the menu, it does [onMore] directly; otherwise [onMore] is the
 * menu's last item. With [dropMoved], what [onMore] opens already offers the actions, so
 * those that don't fit are left out rather than put in a menu that opens another.
 */
@Composable
fun OverflowRow(
    actions: List<RowAction>,
    menuLabel: String,
    modifier: Modifier = Modifier,
    menuTag: String = "row-overflow",
    spacing: Dp = 0.dp,
    lead: (@Composable () -> Unit)? = null,
    pinned: (@Composable () -> Unit)? = null,
    onMore: (() -> Unit)? = null,
    moreLabel: String = menuLabel,
    menuItems: List<RowAction> = emptyList(),
    dropMoved: Boolean = false,
) {
    SubcomposeLayout(modifier) { constraints ->
        val loose = constraints.copy(minWidth = 0, minHeight = 0)
        val gap = spacing.roundToPx()
        val pinnedP = pinned?.let { subcompose("pinned", it).map { m -> m.measure(loose) } }.orEmpty()
        val leadM = lead?.let { subcompose("lead", it) }.orEmpty()
        // The heading keeps its natural width while the buttons are chosen (then gets the rest).
        val leadWant = leadM.maxOfOrNull { it.maxIntrinsicWidth(constraints.maxHeight) } ?: 0
        // An IconButton: its 48 dp touch target.
        val menuW = 48.dp.roundToPx()
        // Sized out of sight (no semantics, never placed); only those that fit are shown.
        val actionW = actions.mapIndexed { i, a ->
            subcompose("size-$i") { Box(Modifier.clearAndSetSemantics {}) { a.inline() } }
                .maxOfOrNull { it.maxIntrinsicWidth(constraints.maxHeight) } ?: 0
        }

        fun width(shown: Int, menu: Boolean): Int {
            val parts = buildList {
                if (lead != null) add(leadWant)
                pinnedP.forEach { add(it.width) }
                addAll(actionW.take(shown))
                if (menu) add(menuW)
            }
            return parts.sum() + gap * (parts.size - 1).coerceAtLeast(0)
        }
        val max = constraints.maxWidth
        val alwaysMenu = onMore != null || menuItems.isNotEmpty()
        val shown = if (width(actions.size, alwaysMenu) <= max) {
            actions.size
        } else {
            (actions.size - 1 downTo 0).firstOrNull { width(it, alwaysMenu || !dropMoved) <= max } ?: 0
        }
        val moved = actions.drop(shown)
        val inMenu = (if (dropMoved && onMore != null) emptyList() else moved) + menuItems
        val actionP = actions.take(shown).mapIndexed { i, a -> subcompose("action-$i", a.inline).map { it.measure(loose) } }
        val menuP = if (inMenu.isNotEmpty() || onMore != null) {
            subcompose("menu") { Menu(inMenu, menuLabel, menuTag, onMore, moreLabel) }.map { it.measure(loose) }
        } else {
            emptyList()
        }
        val fixed = buildList {
            addAll(pinnedP)
            actionP.forEach { addAll(it) }
            addAll(menuP)
        }
        val fixedW = fixed.sumOf { it.width } + gap * (fixed.size - 1).coerceAtLeast(0)
        val leadP = if (lead != null) {
            val w = (max - fixedW - if (fixed.isEmpty()) 0 else gap).coerceAtLeast(0)
            val c = if (max == Constraints.Infinity) loose else Constraints(minWidth = w, maxWidth = w, maxHeight = constraints.maxHeight)
            leadM.map { it.measure(c) }
        } else {
            emptyList()
        }
        val items = leadP + fixed
        val totalW = (items.sumOf { it.width } + gap * (items.size - 1).coerceAtLeast(0))
            .coerceIn(constraints.minWidth, max)
        val height = (items.maxOfOrNull { it.height } ?: 0).coerceIn(constraints.minHeight, constraints.maxHeight)
        layout(totalW, height) {
            var x = 0
            items.forEach {
                it.placeRelative(x, (height - it.height) / 2)
                x += it.width + gap
            }
        }
    }
}

@Composable
private fun MenuButton(label: String, tag: String, onClick: () -> Unit) {
    IconButton(onClick = onClick, modifier = Modifier.testTag(tag)) {
        Icon(Icons.Outlined.MoreVert, contentDescription = label)
    }
}

@Composable
private fun Menu(moved: List<RowAction>, label: String, tag: String, onMore: (() -> Unit)?, moreLabel: String) {
    var open by remember { mutableStateOf(false) }
    Box {
        MenuButton(label, tag) { if (moved.isEmpty()) onMore?.invoke() else open = true }
        DropdownMenu(expanded = open && moved.isNotEmpty(), onDismissRequest = { open = false }) {
            moved.forEach { a ->
                DropdownMenuItem(
                    text = { Text(a.label) },
                    leadingIcon = a.icon?.let { i -> { Icon(i, contentDescription = null) } },
                    enabled = a.enabled,
                    onClick = {
                        open = false
                        a.onClick()
                    },
                    modifier = if (a.tag != null) Modifier.testTag(a.tag) else Modifier,
                )
            }
            if (onMore != null) {
                DropdownMenuItem(
                    text = { Text(moreLabel) },
                    leadingIcon = { Icon(Icons.Outlined.MoreVert, contentDescription = null) },
                    onClick = {
                        open = false
                        onMore()
                    },
                    modifier = Modifier.testTag("$tag-more"),
                )
            }
        }
    }
}
