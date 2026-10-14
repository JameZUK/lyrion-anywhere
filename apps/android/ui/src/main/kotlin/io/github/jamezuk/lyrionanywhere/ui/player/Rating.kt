package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import kotlinx.coroutines.launch
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.combinedClickable
import androidx.compose.ui.semantics.toggleableState
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.StarHalf
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.outlined.FavoriteBorder
import androidx.compose.material.icons.outlined.StarOutline
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconToggleButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.RatingStyle
import io.github.jamezuk.lyrionanywhere.core.TrackRating
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.HapticSteps

/**
 * Rate the current track (docs/plans/wave2-design.md §2): five stars or a heart, as the user
 * chose. Shows the blend of the user's own rating and LMS's shared one: the user's own in
 * the accent colour, a shared-only rating muted. Tapping the star that is the user's rating
 * clears it; TalkBack also offers "Half a star more/less".
 */
@Composable
internal fun RatingControl(rating: TrackRating?, style: RatingStyle, failed: Boolean, onRated: (Int) -> Unit) {
    // A light tick as a rating is set (Wave 2 §5.4).
    val haptics = LocalHapticFeedback.current
    val onRate: (Int) -> Unit = {
        haptics.performHapticFeedback(HapticSteps.confirm)
        onRated(it)
    }
    val mine = rating?.mine?.toInt()
    val shown = rating?.rating?.toInt() ?: 0
    if (style == RatingStyle.HEART) {
        val loved = (mine ?: 0) >= 60
        IconToggleButton(
            checked = loved,
            onCheckedChange = { onRate(if (it) 100 else 0) },
            modifier = Modifier.testTag("rating-heart"),
        ) {
            Icon(
                if (loved) Icons.Filled.Favorite else Icons.Outlined.FavoriteBorder,
                contentDescription = stringResource(R.string.rating_love),
                tint = if (loved) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    } else {
        val state = when {
            mine != null -> stringResource(R.string.rating_yours, mine / 20f)
            shown > 0 -> stringResource(R.string.rating_shared, shown / 20f)
            else -> stringResource(R.string.rating_none)
        }
        val more = stringResource(R.string.rating_more)
        val less = stringResource(R.string.rating_less)
        // Each star is its own labelled button ("Rate 3 of 5 stars"), with the rating as its
        // state and half steps as TalkBack actions.
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.testTag("rating-stars")) {
            for (n in 1..5) {
                val fill = (shown - (n - 1) * 20).coerceIn(0, 20)
                val label = stringResource(R.string.rating_star, n)
                IconButton(
                    onClick = { onRate(if (mine == n * 20) 0 else n * 20) },
                    modifier = Modifier
                        .size(36.dp)
                        .semantics {
                            contentDescription = label
                            stateDescription = state
                            customActions = listOf(
                                CustomAccessibilityAction(more) { onRate((mine ?: 0) + 10); true },
                                CustomAccessibilityAction(less) { onRate((mine ?: 0) - 10); true },
                            )
                        }
                        .testTag("rating-star-$n"),
                ) {
                    Icon(
                        when {
                            fill >= 20 -> Icons.Filled.Star
                            fill >= 10 -> Icons.AutoMirrored.Outlined.StarHalf
                            else -> Icons.Outlined.StarOutline
                        },
                        contentDescription = null,
                        tint = if (mine != null) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
    if (failed) {
        Text(
            stringResource(R.string.rating_failed),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.error,
            modifier = Modifier.testTag("rating-failed"),
        )
    }
}

/**
 * The one rating control beside Now playing's title (Wave 3.5): a heart (loved or not) or,
 * with stars, a single star showing whether the track is rated, which opens the full 1–5
 * rating ([onOpenRating], also in ⋮). TalkBack keeps "Half a star more/less" on it, and a
 * long press opens the full rating either way.
 */
@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
@Composable
internal fun RatingToggle(
    rating: TrackRating?,
    style: RatingStyle,
    failed: Boolean,
    onRated: (Int) -> Unit,
    onOpenRating: () -> Unit,
) {
    val haptics = LocalHapticFeedback.current
    val onRate: (Int) -> Unit = {
        haptics.performHapticFeedback(HapticSteps.confirm)
        onRated(it)
    }
    val mine = rating?.mine?.toInt()
    val shown = rating?.rating?.toInt() ?: 0
    val state = when {
        mine != null -> stringResource(R.string.rating_yours, mine / 20f)
        shown > 0 -> stringResource(R.string.rating_shared, shown / 20f)
        else -> stringResource(R.string.rating_none)
    }
    val more = stringResource(R.string.rating_more)
    val less = stringResource(R.string.rating_less)
    val rateLabel = stringResource(R.string.rating_rate)
    val halfSteps = listOf(
        CustomAccessibilityAction(more) { onRate((mine ?: 0) + 10); true },
        CustomAccessibilityAction(less) { onRate((mine ?: 0) - 10); true },
    )
    if (style == RatingStyle.HEART) {
        val loved = (mine ?: 0) >= 60
        val label = stringResource(R.string.rating_love)
        Box(
            Modifier
                .size(48.dp)
                .combinedClickable(
                    role = androidx.compose.ui.semantics.Role.Switch,
                    onLongClickLabel = rateLabel,
                    onLongClick = onOpenRating,
                    onClick = { onRate(if (loved) 0 else 100) },
                )
                .semantics {
                    contentDescription = label
                    toggleableState = androidx.compose.ui.state.ToggleableState(loved)
                    customActions = listOf(CustomAccessibilityAction(rateLabel) { onOpenRating(); true })
                }
                .testTag("rating-heart"),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                if (loved) Icons.Filled.Favorite else Icons.Outlined.FavoriteBorder,
                contentDescription = null,
                tint = if (loved) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    } else {
        val label = stringResource(R.string.rating_rate)
        IconButton(
            onClick = onOpenRating,
            modifier = Modifier
                .semantics {
                    contentDescription = label
                    stateDescription = state
                    customActions = halfSteps
                }
                .testTag("rating-toggle"),
        ) {
            Icon(
                when {
                    shown >= 100 || (shown > 0 && mine != null) -> Icons.Filled.Star
                    shown > 0 -> Icons.AutoMirrored.Outlined.StarHalf
                    else -> Icons.Outlined.StarOutline
                },
                contentDescription = null,
                tint = if (mine != null) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
    if (failed) {
        Text(
            stringResource(R.string.rating_failed),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.error,
            modifier = Modifier.testTag("rating-failed"),
        )
    }
}

/**
 * Settings: stars or a heart, and clearing the user's ratings (after asking: it can't be
 * undone). [onClear] gives the number cleared, or null if it failed.
 */
@Composable
fun RatingsSetting(style: RatingStyle, onStyle: (RatingStyle) -> Unit, onClear: suspend () -> Long?) {
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    var note by androidx.compose.runtime.saveable.rememberSaveable { androidx.compose.runtime.mutableStateOf<String?>(null) }
    var confirm by androidx.compose.runtime.saveable.rememberSaveable { androidx.compose.runtime.mutableStateOf(false) }
    var clearing by androidx.compose.runtime.remember { androidx.compose.runtime.mutableStateOf(false) }
    val res = androidx.compose.ui.platform.LocalResources.current
    if (confirm) {
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { confirm = false },
            title = { Text(stringResource(R.string.clear_ratings_confirm_title)) },
            text = { Text(stringResource(R.string.clear_ratings_confirm_body)) },
            confirmButton = {
                androidx.compose.material3.TextButton(
                    onClick = {
                        confirm = false
                        clearing = true
                        scope.launch {
                            val n = try {
                                onClear()
                            } finally {
                                clearing = false
                            }
                            note = if (n == null) {
                                res.getString(R.string.ratings_clear_failed)
                            } else {
                                val count = n.coerceIn(0, Int.MAX_VALUE.toLong()).toInt()
                                res.getQuantityString(R.plurals.ratings_cleared, count, count)
                            }
                        }
                    },
                    modifier = Modifier.testTag("clear-ratings-confirm"),
                ) { Text(stringResource(R.string.action_clear)) }
            },
            dismissButton = {
                androidx.compose.material3.TextButton(onClick = { confirm = false }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }
    androidx.compose.foundation.layout.Column(Modifier.testTag("ratings-setting")) {
        // A row with a picker (Wave 3.5).
        io.github.jamezuk.lyrionanywhere.ui.components.ChoiceRow(
            title = stringResource(R.string.settings_rating_style),
            choices = listOf(RatingStyle.STARS to R.string.rating_style_stars, RatingStyle.HEART to R.string.rating_style_heart).map { (s, label) ->
                io.github.jamezuk.lyrionanywhere.ui.components.Choice(s, stringResource(label), tag = "rating-style-${s.name.lowercase()}")
            },
            selected = style,
            onSelect = onStyle,
            tag = "rating-style-setting",
            note = stringResource(R.string.settings_ratings_detail),
        )
        androidx.compose.material3.TextButton(
            onClick = {
                note = null
                confirm = true
            },
            enabled = !clearing,
            modifier = Modifier.padding(horizontal = 8.dp).testTag("clear-ratings"),
        ) { Text(stringResource(R.string.clear_ratings)) }
        note?.let { Text(it, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(horizontal = 16.dp)) }
    }
}
