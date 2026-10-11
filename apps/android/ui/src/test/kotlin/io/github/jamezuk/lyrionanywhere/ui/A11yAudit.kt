package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.text.TextLayoutResult
import org.junit.Assert.assertTrue

/** The smallest touch target allowed, dp (48, less rounding). */
private const val MIN_TOUCH_DP = 47.5f

/** What TalkBack would read for [this]: its description, else its text; null if neither. */
internal fun SemanticsNode.spokenLabel(): String? {
    val c = config
    c.getOrNull(SemanticsProperties.ContentDescription)?.joinToString(" ")?.takeIf { it.isNotBlank() }?.let { return it }
    c.getOrNull(SemanticsProperties.Text)?.joinToString(" ") { it.text }?.takeIf { it.isNotBlank() }?.let { return it }
    c.getOrNull(SemanticsProperties.EditableText)?.text?.takeIf { it.isNotBlank() }?.let { return it }
    return null
}

private fun SemanticsNode.describe(): String {
    val c = config
    val tag = c.getOrNull(SemanticsProperties.TestTag)?.let { " tag=$it" } ?: ""
    val role = c.getOrNull(SemanticsProperties.Role)?.let { " role=$it" } ?: ""
    return "node #$id$role$tag '${spokenLabel() ?: ""}' at $boundsInRoot"
}

/** Whether the node is cut off by a scrolling parent (part of it is off screen). */
private fun SemanticsNode.isClipped(): Boolean {
    val shown = boundsInRoot
    return shown.width < size.width - 0.5f || shown.height < size.height - 0.5f
}

/** Every problem the audit finds in [node] (empty when there are none). */
internal fun auditNode(node: SemanticsNode): List<String> {
    val problems = mutableListOf<String>()
    val c = node.config
    val label = node.spokenLabel()
    val enabled = SemanticsProperties.Disabled !in c
    if (SemanticsActions.OnClick in c && enabled) {
        val clickLabel = c[SemanticsActions.OnClick].label
        if (label == null && clickLabel.isNullOrBlank()) problems += "${node.describe()}: clickable, but has no label"
        // Size: only for nodes fully on screen (a row half scrolled away is cut, not small).
        val bounds = node.boundsInRoot
        if (!bounds.isEmpty && !node.isClipped()) {
            val touch = node.touchBoundsInRoot
            val density = node.layoutInfo.density.density
            val w = touch.width / density
            val h = touch.height / density
            if (w < MIN_TOUCH_DP || h < MIN_TOUCH_DP) problems += "${node.describe()}: touch target $w x $h dp"
        }
    }
    if (SemanticsActions.CustomActions in c) {
        // TalkBack offers actions on the node it focuses: one it can reach (clickable or
        // focusable) or one that says something.
        val reachable = SemanticsActions.OnClick in c || SemanticsProperties.Focused in c || label != null
        if (!reachable) problems += "${node.describe()}: actions on a node TalkBack never focuses"
    }
    if (c.getOrNull(SemanticsProperties.Role) == Role.Button && SemanticsProperties.Selected in c) {
        problems += "${node.describe()}: a button announced as selected (use a tab, radio button or plain button)"
    }
    if (SemanticsProperties.ToggleableState in c && label == null) problems += "${node.describe()}: a toggle with no label"
    if (SemanticsActions.SetProgress in c && label == null) problems += "${node.describe()}: a slider with no label"
    return problems
}

/**
 * An accessibility audit of everything on screen, as TalkBack sees it (the merged tree, in
 * every window, so dialogs too):
 * - every enabled clickable has a label and a touch target of at least 48 dp each way;
 * - custom actions sit on a node TalkBack can focus;
 * - nothing is a "selected" button;
 * - every toggle and slider has a label.
 */
fun ComposeContentTestRule.assertAccessible() {
    waitForIdle()
    val nodes = onAllNodes(SemanticsMatcher("any node") { true }).fetchSemanticsNodes(atLeastOneRootRequired = true)
    assertTrue("nothing on screen to audit", nodes.isNotEmpty())
    val problems = nodes.flatMap { auditNode(it) }
    assertTrue("Accessibility problems:\n" + problems.joinToString("\n"), problems.isEmpty())
}

/**
 * Text that fits, as with large font sizes: no text squeezed to nothing, and text that
 * doesn't fit its width or line limit ends in an ellipsis rather than being cut off.
 */
fun ComposeContentTestRule.assertTextFits() {
    waitForIdle()
    val texts = onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsActions.GetTextLayoutResult), useUnmergedTree = true)
        .fetchSemanticsNodes(atLeastOneRootRequired = true)
    assertTrue("no text on screen", texts.isNotEmpty())
    val problems = mutableListOf<String>()
    for (n in texts) {
        // Text fields scroll their text: not cut off.
        if (SemanticsProperties.EditableText in n.config) continue
        val results = mutableListOf<TextLayoutResult>()
        n.config[SemanticsActions.GetTextLayoutResult].action?.invoke(results)
        val r = results.firstOrNull() ?: continue
        val text = r.layoutInput.text.text
        if (text.isBlank()) continue
        if (n.size.width < 1) problems += "'$text' is squeezed to ${n.size.width} px wide"
        // Compose keeps a paragraph laid out for wider constraints when its lines still fit,
        // so judge by the lines themselves against the node's size, not by didOverflowWidth.
        val last = r.lineCount - 1
        val tooWide = (0..last).any { line ->
            r.getLineRight(line) - r.getLineLeft(line) > n.size.width + 1f && !r.isLineEllipsized(line)
        }
        val tooTall = (r.getLineBottom(last) > n.size.height + 1f || r.multiParagraph.didExceedMaxLines) &&
            !r.isLineEllipsized(last)
        if (tooWide || tooTall) problems += "'$text' is cut off without an ellipsis"
    }
    assertTrue("Text that doesn't fit:\n" + problems.joinToString("\n"), problems.isEmpty())
}
