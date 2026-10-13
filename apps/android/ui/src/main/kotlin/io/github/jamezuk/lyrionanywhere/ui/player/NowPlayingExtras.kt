package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Bedtime
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.ListItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Surface
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.Quality
import io.github.jamezuk.lyrionanywhere.core.SleepEnd
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.HapticSteps
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import kotlinx.coroutines.launch

/** "45 s", "27 min", "1 h 5 min" (as the web app). */
@Composable
fun sleepLeftText(seconds: Int): String {
    val s = seconds.coerceAtLeast(0)
    if (s < 60) return stringResource(R.string.sleep_left_seconds, s)
    val m = (s + 59) / 60
    if (m < 60) return stringResource(R.string.sleep_left_minutes, m)
    val h = m / 60
    val r = m % 60
    return if (r == 0) stringResource(R.string.sleep_left_whole_hours, h) else stringResource(R.string.sleep_left_hours, h, r)
}

/** The quality badge's text: "FLAC 24/96 · Hi-Res", "MP3 320". */
@Composable
fun qualityText(q: Quality): String = if (q.hiRes) stringResource(R.string.quality_hi_res, q.label) else q.label

/** The quality badge (Wave 1 §5); a tap opens the signal path. [compact]: the small form beside the artist. */
@Composable
fun QualityBadge(quality: Quality, onClick: () -> Unit, modifier: Modifier = Modifier, compact: Boolean = false) {
    val text = qualityText(quality)
    val label = stringResource(R.string.quality_show_path, text)
    Surface(
        onClick = onClick,
        shape = RoundedCornerShape(50),
        color = if (quality.hiRes) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceVariant,
        contentColor = if (quality.hiRes) MaterialTheme.colorScheme.onPrimaryContainer else MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier.testTag("quality-badge").semantics { contentDescription = label },
    ) {
        Text(
            text,
            style = if (compact) MaterialTheme.typography.labelSmall else MaterialTheme.typography.labelLarge,
            maxLines = 1,
            modifier = Modifier.padding(horizontal = if (compact) 8.dp else 12.dp, vertical = if (compact) 3.dp else 6.dp),
        )
    }
}

/**
 * The sleep timer's choices: fixed lengths, a custom length, or the end of this track, album
 * or queue; for this player, its sync group or every player. [canEnd] is false for a stream
 * without a set length.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SleepSheet(
    left: Int?,
    grouped: Boolean,
    others: Boolean,
    canEnd: Boolean,
    fadeSecs: Int,
    onSet: (SleepChoice, SleepScope) -> Unit,
    onCancel: () -> Unit,
    onDismiss: () -> Unit,
) {
    ModalBottomSheet(onDismissRequest = onDismiss, modifier = Modifier.testTag("sleep-sheet")) {
        SleepChoices(left, grouped, others, canEnd, fadeSecs, onSet = { c, s ->
            onSet(c, s)
            onDismiss()
        }, onCancel = onCancel)
    }
}

/** The sheet's content (on its own, for tests and large screens). */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun SleepChoices(
    left: Int?,
    grouped: Boolean,
    others: Boolean,
    canEnd: Boolean,
    fadeSecs: Int,
    onSet: (SleepChoice, SleepScope) -> Unit,
    onCancel: () -> Unit,
) {
    // A light tick as the timer is set (Wave 2 §5.4).
    val haptics = LocalHapticFeedback.current
    val chosen: (SleepChoice, SleepScope) -> Unit = { c, s ->
        haptics.performHapticFeedback(HapticSteps.confirm)
        onSet(c, s)
    }
    var scope by rememberSaveable { mutableStateOf(SleepScope.PLAYER) }
    var custom by rememberSaveable { mutableStateOf("") }
    var invalid by remember { mutableStateOf(false) }
    val submitCustom = {
        val m = SleepTimer.customMinutes(custom)
        if (m == null) invalid = true else chosen(SleepChoice.Minutes(m), scope)
    }
    Column(
        Modifier
            .fillMaxWidth()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 20.dp)
            .padding(bottom = 24.dp)
            .navigationBarsPadding(),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(stringResource(R.string.sleep_timer), style = MaterialTheme.typography.titleLarge, modifier = Modifier.semantics { heading() })
        if (left != null) {
            Card(Modifier.fillMaxWidth()) {
                Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        stringResource(R.string.sleep_stops_in, sleepLeftText(left)),
                        modifier = Modifier.weight(1f).semantics { liveRegion = LiveRegionMode.Polite },
                    )
                    FilledTonalButton(onClick = onCancel, modifier = Modifier.testTag("sleep-off")) { Text(stringResource(R.string.sleep_off)) }
                }
            }
        }
        if (grouped || others) {
            Text(stringResource(R.string.sleep_scope), style = MaterialTheme.typography.titleSmall)
            Column(Modifier.selectableGroup()) {
                val options = buildList {
                    add(SleepScope.PLAYER to R.string.sleep_scope_player)
                    if (grouped) add(SleepScope.GROUP to R.string.sleep_scope_group)
                    if (others) add(SleepScope.ALL to R.string.sleep_scope_all)
                }
                options.forEach { (value, label) ->
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .selectable(selected = scope == value, onClick = { scope = value }, role = Role.RadioButton)
                            .padding(vertical = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(selected = scope == value, onClick = null)
                        Text(stringResource(label), Modifier.padding(start = 8.dp))
                    }
                }
            }
        }
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            SleepTimer.MINUTES.forEach { m ->
                OutlinedButton(onClick = { chosen(SleepChoice.Minutes(m), scope) }, modifier = Modifier.testTag("sleep-$m")) {
                    Text(stringResource(R.string.sleep_minutes, m))
                }
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            OutlinedTextField(
                value = custom,
                onValueChange = {
                    custom = it.filter(Char::isDigit).take(4)
                    invalid = false
                },
                label = { Text(stringResource(R.string.sleep_custom)) },
                isError = invalid,
                supportingText = if (invalid) ({ Text(stringResource(R.string.sleep_custom_invalid)) }) else null,
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number, imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { submitCustom() }),
                modifier = Modifier.weight(1f).testTag("sleep-custom"),
            )
            Spacer(Modifier.width(8.dp))
            TextButton(onClick = submitCustom, modifier = Modifier.testTag("sleep-custom-set")) { Text(stringResource(R.string.sleep_custom_set)) }
        }
        listOf(
            SleepEnd.TRACK to R.string.sleep_end_track,
            SleepEnd.ALBUM to R.string.sleep_end_album,
            SleepEnd.QUEUE to R.string.sleep_end_queue,
        ).forEach { (end, label) ->
            OutlinedButton(
                onClick = { chosen(SleepChoice.End(end), scope) },
                enabled = canEnd,
                modifier = Modifier.fillMaxWidth().testTag("sleep-end-${end.name.lowercase()}"),
            ) { Text(stringResource(label), Modifier.fillMaxWidth()) }
        }
        if (!canEnd) Text(stringResource(R.string.sleep_end_unavailable), style = MaterialTheme.typography.bodySmall)
        Text(
            if (fadeSecs > 0) stringResource(R.string.sleep_note_fade, fadeSecs) else stringResource(R.string.sleep_note_no_fade),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/**
 * "Up next" under the transport (Wave 1 §3): one compact line, the next track with its
 * cover. Tap opens the queue; ✕ (or a swipe, the Remove action or Delete) takes it out of
 * the queue, with Undo. Without [onRemove] it can't be removed (downloads).
 */
@Composable
fun UpNextRow(track: Track, onOpenQueue: () -> Unit, onRemove: (() -> Unit)?, modifier: Modifier = Modifier) {
    UpNextLine(track.title, track.artist, track.artwork, onOpenQueue, onRemove, modifier)
}

/** [UpNextRow] from its parts (a download's cover key, too). */
@Composable
fun UpNextLine(title: String, artist: String?, artwork: String?, onOpenQueue: () -> Unit, onRemove: (() -> Unit)?, modifier: Modifier = Modifier) {
    val dismiss = rememberSwipeToDismissBoxState()
    val scope = rememberCoroutineScope()
    val remove = stringResource(R.string.up_next_remove)
    val line = listOfNotNull(title, artist?.takeIf { it.isNotBlank() }).joinToString(" — ")
    val label = stringResource(R.string.up_next_label, line)
    SwipeToDismissBox(
        state = dismiss,
        enableDismissFromStartToEnd = onRemove != null,
        enableDismissFromEndToStart = onRemove != null,
        onDismiss = {
            if (it != SwipeToDismissBoxValue.Settled) onRemove?.invoke()
            scope.launch { dismiss.snapTo(SwipeToDismissBoxValue.Settled) }
        },
        backgroundContent = {
            // Only while swiping: the line is translucent, so a resting background would show
            // through it (a red wash and a bin behind the close button).
            if (dismiss.dismissDirection != SwipeToDismissBoxValue.Settled) {
                Box(
                    Modifier
                        .fillMaxSize()
                        .background(MaterialTheme.colorScheme.errorContainer, RoundedCornerShape(12.dp))
                        .padding(horizontal = 20.dp)
                        .testTag("up-next-swipe-background"),
                    contentAlignment = Alignment.CenterEnd,
                ) {
                    Icon(Icons.Outlined.Delete, contentDescription = null, tint = MaterialTheme.colorScheme.onErrorContainer)
                }
            }
        },
        modifier = modifier.fillMaxWidth().testTag("up-next"),
    ) {
        Surface(
            shape = RoundedCornerShape(12.dp),
            color = MaterialTheme.colorScheme.surfaceContainer.copy(alpha = 0.85f),
            modifier = Modifier
                .fillMaxWidth()
                .clickable(onClickLabel = stringResource(R.string.up_next_open_queue), onClick = onOpenQueue)
                .onPreviewKeyEvent { e ->
                    if (onRemove != null && e.type == KeyEventType.KeyDown && (e.key == Key.Delete || e.key == Key.Backspace)) {
                        onRemove()
                        true
                    } else {
                        false
                    }
                }
                .semantics(mergeDescendants = true) {
                    contentDescription = label
                    if (onRemove != null) customActions = listOf(CustomAccessibilityAction(remove) { onRemove(); true })
                },
        ) {
            Row(Modifier.padding(start = 8.dp, top = 4.dp, bottom = 4.dp, end = 0.dp), verticalAlignment = Alignment.CenterVertically) {
                ArtworkImage(artwork, 120, contentDescription = null, modifier = Modifier.size(40.dp), corner = 8.dp)
                Column(Modifier.weight(1f).padding(start = 12.dp)) {
                    Text(
                        stringResource(R.string.up_next).uppercase(),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Text(
                        line,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
                if (onRemove != null) {
                    IconButton(onClick = onRemove, modifier = Modifier.testTag("up-next-remove")) {
                        Icon(Icons.Outlined.Close, contentDescription = stringResource(R.string.up_next_remove_named, title), modifier = Modifier.size(20.dp))
                    }
                } else {
                    Spacer(Modifier.width(12.dp))
                }
            }
        }
    }
}

/**
 * The signal path, opened from the quality badge: this phone's full path when it is the
 * player, otherwise the source (all that is known about another player's path).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SignalPathSheet(state: PlayerUiState, onDismiss: () -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss, modifier = Modifier.testTag("signal-path-sheet")) {
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(bottom = 24.dp).navigationBarsPadding(),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (state.localSelected) {
                SignalPathPanel(state.local.signalPath, quality = state.local.streamQuality, metered = state.local.metered, eq = state.local.eqInUse?.name, cushionMs = state.local.cushionMs)
            } else {
                val q = state.status?.current?.quality
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(stringResource(R.string.signal_path), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
                        if (q != null) {
                            Column(Modifier.semantics(mergeDescendants = true) {}) {
                                Text(stringResource(R.string.signal_source), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.primary)
                                Text(
                                    listOf(qualityText(q), stringResource(if (q.lossless) R.string.signal_source_lossless else R.string.signal_source_lossy)).joinToString(" · "),
                                    style = MaterialTheme.typography.bodyMedium,
                                )
                            }
                        }
                        Text(
                            stringResource(R.string.signal_path_other, state.selected?.name.orEmpty()),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
}

/** A fade-out length in words: "Off", "30 seconds", "1 minute", "1.5 minutes". */
@Composable
fun fadeText(seconds: Int): String = when {
    seconds <= 0 -> stringResource(R.string.sleep_fade_off)
    seconds < 60 -> pluralStringResource(R.plurals.sleep_fade_seconds, seconds, seconds)
    seconds == 60 -> stringResource(R.string.sleep_fade_one_minute)
    else -> stringResource(R.string.sleep_fade_minutes, if (seconds % 60 == 0) "${seconds / 60}" else "${seconds / 60}.5")
}

/** Settings › Sleep timer fade-out: off to 2 minutes (30 s by default), kept on this phone. */
@Composable
fun SleepFadeSetting(fadeSecs: Int, onFade: (Int) -> Unit) {
    var open by rememberSaveable { mutableStateOf(false) }
    ListItem(
        leadingContent = { Icon(Icons.Outlined.Bedtime, contentDescription = null) },
        headlineContent = { Text(stringResource(R.string.settings_sleep_fade)) },
        supportingContent = { Text(fadeText(fadeSecs)) },
        modifier = Modifier.testTag("sleep-fade-setting").clickable { open = true },
    )
    if (open) {
        AlertDialog(
            onDismissRequest = { open = false },
            title = { Text(stringResource(R.string.settings_sleep_fade)) },
            text = {
                Column(Modifier.selectableGroup().verticalScroll(rememberScrollState())) {
                    Text(stringResource(R.string.settings_sleep_fade_detail), style = MaterialTheme.typography.bodySmall)
                    SleepTimer.FADE_CHOICES.forEach { f ->
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .selectable(selected = f == fadeSecs, role = Role.RadioButton, onClick = {
                                    onFade(f)
                                    open = false
                                })
                                .padding(vertical = 8.dp)
                                .testTag("sleep-fade-$f"),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(selected = f == fadeSecs, onClick = null)
                            Text(fadeText(f), Modifier.padding(start = 8.dp))
                        }
                    }
                }
            },
            confirmButton = { TextButton(onClick = { open = false }) { Text(stringResource(R.string.action_cancel)) } },
        )
    }
}
