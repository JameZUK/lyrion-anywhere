package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ArrowDropDown
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Pause
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.SkipNext
import androidx.compose.material.icons.outlined.SkipPrevious
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.PhoneAndroid
import androidx.compose.material.icons.outlined.Speaker
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.onClick as clickAction
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.toggleableState
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.min
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
import io.github.jamezuk.lyrionanywhere.data.NowPlayingLayout
import io.github.jamezuk.lyrionanywhere.data.VisualStyle
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.components.HapticSteps
import kotlinx.coroutines.delay

/**
 * Now playing's top row (the owner-approved layout): close at the start, "Playing on …" in
 * the middle (it opens *Play on…*), and ⋮ for the occasional things at the end. Either end
 * keeps its space when absent, so the chip stays centred.
 */
@Composable
internal fun NowPlayingTopRow(
    onClose: (() -> Unit)?,
    onMore: (() -> Unit)?,
    modifier: Modifier = Modifier,
    chip: @Composable () -> Unit,
) {
    Row(modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("now-playing-top"), verticalAlignment = Alignment.CenterVertically) {
        if (onClose != null) NowPlayingCloseButton(onClose) else Spacer(Modifier.size(48.dp))
        Box(Modifier.weight(1f), contentAlignment = Alignment.Center) { chip() }
        if (onMore != null) {
            IconButton(onClick = onMore, modifier = Modifier.testTag("now-playing-more")) {
                Icon(Icons.Outlined.MoreVert, contentDescription = stringResource(R.string.action_more))
            }
        } else {
            Spacer(Modifier.size(48.dp))
        }
    }
}

/**
 * "Playing on <player> ▾": which player this screen controls; a tap opens *Play on…*
 * ([onClick] null: a plain label, as for downloads).
 */
@Composable
internal fun PlayingOnChip(text: String, phone: Boolean, onClick: (() -> Unit)?) {
    val content: @Composable () -> Unit = {
        Row(
            Modifier.padding(start = 12.dp, end = if (onClick != null) 6.dp else 12.dp, top = 6.dp, bottom = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(if (phone) Icons.Outlined.PhoneAndroid else Icons.Outlined.Speaker, contentDescription = null, modifier = Modifier.size(16.dp))
            Text(
                text,
                style = MaterialTheme.typography.labelLarge,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.padding(start = 6.dp).weight(1f, fill = false),
            )
            if (onClick != null) Icon(Icons.Outlined.ArrowDropDown, contentDescription = null, modifier = Modifier.size(20.dp))
        }
    }
    val colour = MaterialTheme.colorScheme.surfaceContainerHighest.copy(alpha = 0.9f)
    if (onClick != null) {
        val label = stringResource(R.string.play_on_title)
        Surface(
            onClick = onClick,
            shape = CircleShape,
            color = colour,
            modifier = Modifier
                .testTag("playing-on-chip")
                .semantics { clickAction(label = label) { onClick(); true } },
            content = content,
        )
    } else {
        Surface(shape = CircleShape, color = colour, modifier = Modifier.testTag("playing-on-chip"), content = content)
    }
}

/** A page's name, for its tab and the actions that show it. */
@Composable
fun layoutName(l: NowPlayingLayout): String = stringResource(
    when (l) {
        NowPlayingLayout.ART -> R.string.page_cover
        NowPlayingLayout.LYRICS -> R.string.page_lyrics
        NowPlayingLayout.INFO -> R.string.page_info
    },
)

/**
 * The page dots under the hero: one tab each ("Cover", "Lyrics", "Track info"), the shown
 * one drawn long. A tap shows that page.
 */
@Composable
internal fun PageDots(pages: List<NowPlayingLayout>, current: NowPlayingLayout, onSelect: (NowPlayingLayout) -> Unit, modifier: Modifier = Modifier) {
    Row(modifier.selectableGroup().testTag("page-dots"), horizontalArrangement = Arrangement.Center) {
        pages.forEach { p ->
            val selected = p == current
            val name = layoutName(p)
            Box(
                Modifier
                    .size(48.dp)
                    .selectable(selected = selected, role = Role.Tab, onClick = { onSelect(p) })
                    .semantics { contentDescription = name }
                    .testTag("page-${p.name}"),
                contentAlignment = Alignment.Center,
            ) {
                val colour = MaterialTheme.colorScheme.onSurface
                Box(
                    Modifier
                        .size(width = if (selected) 18.dp else 7.dp, height = 7.dp)
                        .background(if (selected) colour else colour.copy(alpha = 0.4f), RoundedCornerShape(4.dp)),
                )
            }
        }
    }
}

/**
 * The hero: a horizontal pager of the pages (Cover, Lyrics, Track info) taking all the
 * height it is given, with the page dots under it. [swipe] false leaves sideways swipes to
 * the page (the visualiser uses them to change style); the dots and actions still move.
 */
@Composable
internal fun HeroPager(
    pages: List<NowPlayingLayout>,
    page: NowPlayingLayout,
    onPage: (NowPlayingLayout) -> Unit,
    swipe: Boolean,
    modifier: Modifier = Modifier,
    content: @Composable (NowPlayingLayout) -> Unit,
) {
    val target = pages.indexOf(page).coerceAtLeast(0)
    val pager = rememberPagerState(initialPage = target) { pages.size }
    val latest by rememberUpdatedState(onPage)
    val latestPage by rememberUpdatedState(page)
    LaunchedEffect(target) {
        if (pager.currentPage != target) pager.animateScrollToPage(target)
    }
    LaunchedEffect(pager) {
        androidx.compose.runtime.snapshotFlow { pager.settledPage }.collect { i ->
            pages.getOrNull(i)?.let { if (it != latestPage) latest(it) }
        }
    }
    Column(modifier, horizontalAlignment = Alignment.CenterHorizontally) {
        HorizontalPager(
            state = pager,
            userScrollEnabled = swipe,
            beyondViewportPageCount = 0,
            key = { pages[it].name },
            modifier = Modifier.weight(1f).fillMaxWidth().testTag("now-playing-pager"),
        ) { i -> content(pages[i]) }
        if (pages.size > 1) PageDots(pages, pages[pager.currentPage.coerceIn(0, pages.size - 1)], onPage)
    }
}

/** TalkBack's way to the other pages (the dots are the touch way). */
@Composable
internal fun pageActions(pages: List<NowPlayingLayout>, current: NowPlayingLayout, onPage: (NowPlayingLayout) -> Unit): List<CustomAccessibilityAction> {
    val labels = mapOf(
        NowPlayingLayout.ART to stringResource(R.string.layout_show_art),
        NowPlayingLayout.LYRICS to stringResource(R.string.layout_show_lyrics),
        NowPlayingLayout.INFO to stringResource(R.string.layout_show_info),
    )
    val latest by rememberUpdatedState(onPage)
    return pages.filter { it != current }.map { p -> CustomAccessibilityAction(labels.getValue(p)) { latest(p); true } }
}

/**
 * The cover page: the cover, square, as large as fits (it shrinks rather than push the
 * controls off screen). [overlay] draws over it (the visualiser), [halo] wraps it (Glow).
 * A swipe up shows the queue, a swipe down closes; TalkBack has [actions].
 */
@Composable
internal fun CoverPage(
    artwork: String?,
    actions: List<CustomAccessibilityAction>,
    modifier: Modifier = Modifier,
    overlay: (@Composable () -> Unit)? = null,
    halo: (@Composable (Modifier, @Composable () -> Unit) -> Unit)? = null,
    onSwipeUp: (() -> Unit)? = null,
    onSwipeDown: (() -> Unit)? = null,
) {
    val coverLabel = stringResource(R.string.album_cover)
    val minPx = with(LocalDensity.current) { SWIPE_UP_MIN.toPx() }
    val up by rememberUpdatedState(onSwipeUp)
    val down by rememberUpdatedState(onSwipeDown)
    BoxWithConstraints(modifier.fillMaxSize().padding(horizontal = 8.dp, vertical = 4.dp), contentAlignment = Alignment.Center) {
        val side = min(maxWidth, maxHeight)
        val cover: @Composable (Modifier) -> Unit = { outer ->
            Box(
                outer
                    .size(side)
                    .testTag("artwork")
                    .pointerInput(minPx) {
                        var dy = 0f
                        detectVerticalDragGestures(
                            onDragStart = { dy = 0f },
                            onDragEnd = {
                                when {
                                    dy <= -minPx -> up?.invoke()
                                    dy >= minPx * 2 -> down?.invoke()
                                }
                            },
                            onDragCancel = { dy = 0f },
                        ) { change, amount ->
                            dy += amount
                            change.consume()
                        }
                    }
                    // One node, labelled as the cover, carrying Next, Previous, the queue and
                    // the other pages.
                    .semantics(mergeDescendants = true) {
                        contentDescription = coverLabel
                        customActions = actions
                    },
            ) {
                Crossfade(targetState = artwork, animationSpec = tween(350), label = "artwork") { art ->
                    ArtworkImage(art, 800, contentDescription = null, modifier = Modifier.fillMaxSize(), corner = 16.dp)
                }
                overlay?.let { o ->
                    Box(Modifier.matchParentSize().clip(RoundedCornerShape(16.dp)).background(Color.Black.copy(alpha = 0.55f))) { o() }
                }
            }
        }
        if (halo != null) halo(Modifier.size(side)) { cover(Modifier) } else cover(Modifier)
    }
}

/**
 * A short note over the hero (the visualiser style's name), fading after a moment. Polite
 * for TalkBack.
 */
@Composable
internal fun HeroToast(message: Pair<String, Int>?, onGone: () -> Unit, modifier: Modifier = Modifier) {
    val gone by rememberUpdatedState(onGone)
    LaunchedEffect(message) {
        if (message != null) {
            delay(TOAST_MS)
            gone()
        }
    }
    AnimatedVisibility(visible = message != null, enter = fadeIn(), exit = fadeOut(), modifier = modifier) {
        Surface(shape = CircleShape, color = MaterialTheme.colorScheme.surface, shadowElevation = 6.dp) {
            Text(
                message?.first.orEmpty(),
                style = MaterialTheme.typography.titleSmall,
                modifier = Modifier
                    .padding(horizontal = 16.dp, vertical = 8.dp)
                    .testTag("hero-toast")
                    .semantics { liveRegion = LiveRegionMode.Polite },
            )
        }
    }
}

/** How long the hero's note stays. */
internal const val TOAST_MS = 1_200L

/**
 * The title with the rating at its end, and under them the artist and album with the
 * quality badge (a tap opens the signal path). One line each, ellipsised.
 */
@Composable
internal fun NowPlayingMeta(
    title: String,
    subtitle: String,
    modifier: Modifier = Modifier,
    rating: (@Composable () -> Unit)? = null,
    badge: (@Composable () -> Unit)? = null,
    below: (@Composable () -> Unit)? = null,
) {
    Column(modifier.fillMaxWidth().testTag("now-playing-meta")) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Crossfade(targetState = title, animationSpec = tween(250), label = "title", modifier = Modifier.weight(1f)) { t ->
                Text(
                    t,
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.Bold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.fillMaxWidth().semantics { heading() }.testTag("now-playing-title"),
                )
            }
            rating?.invoke()
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                subtitle,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f, fill = false),
            )
            if (badge != null) {
                Spacer(Modifier.width(8.dp))
                badge()
            }
        }
        below?.invoke()
    }
}

/** The transport: shuffle, previous, the large play/pause, next, repeat. */
@Composable
internal fun TransportRow(
    playing: Boolean,
    onTogglePlay: () -> Unit,
    onPrevious: () -> Unit,
    onNext: () -> Unit,
    modifier: Modifier = Modifier,
    /** False: greyed out (it can't work now, e.g. the server can't be reached). */
    playEnabled: Boolean = true,
    previousEnabled: Boolean = true,
    nextEnabled: Boolean = true,
    shuffle: @Composable () -> Unit,
    repeat: @Composable () -> Unit,
) {
    Row(
        modifier.fillMaxWidth().testTag("transport"),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceEvenly,
    ) {
        shuffle()
        IconButton(onClick = onPrevious, enabled = previousEnabled, modifier = Modifier.testTag("previous")) {
            Icon(Icons.Outlined.SkipPrevious, contentDescription = stringResource(R.string.action_previous))
        }
        PlayPause(playing, onTogglePlay, enabled = playEnabled)
        IconButton(onClick = onNext, enabled = nextEnabled, modifier = Modifier.testTag("next")) {
            Icon(Icons.Outlined.SkipNext, contentDescription = stringResource(R.string.action_next))
        }
        repeat()
    }
}

/** The large play/pause button (72 dp). */
@Composable
internal fun PlayPause(playing: Boolean, onClick: () -> Unit, enabled: Boolean = true) {
    val icon = if (playing) Icons.Outlined.Pause else Icons.Outlined.PlayArrow
    val label = stringResource(if (playing) R.string.action_pause else R.string.action_play)
    androidx.compose.material3.FilledIconButton(onClick = onClick, enabled = enabled, modifier = Modifier.size(72.dp).testTag("play-pause")) {
        Icon(icon, contentDescription = label, modifier = Modifier.size(40.dp))
    }
}

/**
 * One of the four actions along the bottom (Sleep, Queue, Visualiser, Volume): an icon
 * over a short label. With [on] it is a switch (its state said, its icon tinted); a long
 * press does [onLongClick] where given.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun NowPlayingAction(
    icon: ImageVector,
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    on: Boolean? = null,
    enabled: Boolean = true,
    state: String? = null,
    description: String? = null,
    onLongClick: (() -> Unit)? = null,
    onLongClickLabel: String? = null,
) {
    val haptics = LocalHapticFeedback.current
    val tint = when {
        !enabled -> MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f)
        on == true -> MaterialTheme.colorScheme.primary
        else -> MaterialTheme.colorScheme.onSurfaceVariant
    }
    Column(
        modifier
            .heightIn(min = 56.dp)
            .clip(RoundedCornerShape(12.dp))
            .combinedClickable(
                enabled = enabled,
                role = if (on != null) Role.Switch else Role.Button,
                onLongClickLabel = onLongClickLabel,
                onLongClick = onLongClick?.let { l ->
                    {
                        haptics.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.LongPress)
                        l()
                    }
                },
                onClick = onClick,
            )
            .semantics(mergeDescendants = true) {
                description?.let { contentDescription = it }
                state?.let { stateDescription = it }
                if (on != null) toggleableState = ToggleableState(on)
            }
            .padding(vertical = 6.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(icon, contentDescription = null, tint = tint)
        Text(
            label,
            style = MaterialTheme.typography.labelMedium,
            color = tint,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.padding(top = 2.dp, start = 2.dp, end = 2.dp),
        )
    }
}

/**
 * The volume popover: a small vertical slider over the action row, for [name] (a player in
 * LMS, or this phone's media volume). [value] is 0–100; [onChange] is called as it moves
 * (the phone's volume follows at once), [onDone] when let go (an LMS player is set then).
 */
@Composable
internal fun VolumePopover(
    name: String,
    note: String,
    value: Int,
    onChange: (Int) -> Unit,
    onDone: (Int) -> Unit,
    onDismiss: () -> Unit,
    enabled: Boolean = true,
    steps: Int = 0,
) {
    var dragging by remember { mutableStateOf(false) }
    var drag by remember { mutableFloatStateOf(value.toFloat()) }
    val shown = if (dragging) drag else value.toFloat()
    val haptics = LocalHapticFeedback.current
    val label = stringResource(R.string.volume_on, name)
    val density = LocalDensity.current
    val offset = with(density) { androidx.compose.ui.unit.IntOffset(-16.dp.roundToPx(), -76.dp.roundToPx()) }
    Popup(
        alignment = Alignment.BottomEnd,
        offset = offset,
        onDismissRequest = onDismiss,
        properties = PopupProperties(focusable = true),
    ) {
        Surface(
            shape = RoundedCornerShape(18.dp),
            tonalElevation = 3.dp,
            shadowElevation = 8.dp,
            modifier = Modifier.width(88.dp).testTag("volume-popover").semantics { paneTitle = label },
        ) {
            Column(Modifier.padding(vertical = 12.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Text("${shown.toInt()}", style = MaterialTheme.typography.labelLarge, modifier = Modifier.testTag("volume-value"))
                Slider(
                    value = shown,
                    onValueChange = {
                        // A light tick at every volume step (Wave 2 §5.4).
                        if (HapticSteps.crossed(shown, it, HapticSteps.VOLUME)) haptics.performHapticFeedback(HapticSteps.tick)
                        dragging = true
                        drag = it
                        onChange(it.toInt())
                    },
                    onValueChangeFinished = {
                        dragging = false
                        onDone(drag.toInt())
                    },
                    enabled = enabled,
                    valueRange = 0f..100f,
                    steps = steps,
                    modifier = Modifier
                        .padding(vertical = 8.dp)
                        .vertical()
                        .width(180.dp)
                        .testTag("volume-slider")
                        .semantics {
                            contentDescription = label
                            stateDescription = "${shown.toInt()}%"
                        },
                )
                Text(
                    note,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis,
                    textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                    modifier = Modifier.padding(horizontal = 6.dp).testTag("volume-note"),
                )
            }
        }
    }
}

/** Turns a horizontal slider upright: the low end at the bottom. */
private fun Modifier.vertical(): Modifier = this
    .graphicsLayer {
        rotationZ = 270f
        transformOrigin = TransformOrigin(0f, 0f)
    }
    .layout { measurable, constraints ->
        val p = measurable.measure(
            Constraints(
                minWidth = constraints.minHeight,
                maxWidth = constraints.maxHeight,
                minHeight = constraints.minWidth,
                maxHeight = constraints.maxWidth,
            ),
        )
        layout(p.height, p.width) { p.place(-p.width, 0) }
    }

/** One row of a Now playing sheet: an icon, a title and a line under it. */
@Composable
internal fun SheetRow(icon: ImageVector, title: String, detail: String?, onClick: () -> Unit, tag: String, trailing: (@Composable () -> Unit)? = null) {
    ListItem(
        leadingContent = { Icon(icon, contentDescription = null) },
        headlineContent = { Text(title) },
        supportingContent = detail?.let { { Text(it) } },
        trailingContent = trailing,
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .testTag(tag),
    )
}

/** A sheet of Now playing's: a heading and its rows, scrolling if they don't fit. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun NowPlayingSheet(title: String, tag: String, onDismiss: () -> Unit, content: @Composable () -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss, modifier = Modifier.testTag(tag)) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(bottom = 16.dp).navigationBarsPadding()) {
            Text(
                title,
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp).semantics { heading() },
            )
            content()
        }
    }
}

/**
 * The visualiser's styles (a long press on its button, or ⋮ › Visualiser style): choosing one
 * switches it on in that style; "Switch off" turns it off.
 */
@Composable
internal fun VisualiserStyleSheet(on: Boolean, style: VisualStyle, onChoose: (VisualStyle) -> Unit, onOff: () -> Unit, onDismiss: () -> Unit) {
    NowPlayingSheet(stringResource(R.string.visualiser_style_title), "visualiser-style-sheet", onDismiss) {
        Column(Modifier.selectableGroup()) {
            VisualStyle.entries.forEach { s ->
                val chosen = on && s == style
                ListItem(
                    leadingContent = { RadioButton(selected = chosen, onClick = null) },
                    headlineContent = { Text(visualStyleName(s)) },
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    modifier = Modifier
                        .selectable(selected = chosen, role = Role.RadioButton) {
                            onChoose(s)
                            onDismiss()
                        }
                        .testTag("visual-style-${s.name}"),
                )
            }
        }
        if (on) {
            SheetRow(
                Icons.Outlined.Close,
                stringResource(R.string.visualiser_switch_off),
                null,
                onClick = {
                    onOff()
                    onDismiss()
                },
                tag = "visualiser-switch-off",
            )
        }
    }
}
