package io.github.jamezuk.lyrionanywhere.ui.discovery

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.MusicNote
import androidx.compose.material.icons.outlined.Radio
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.MenuItem
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.library.SectionHeader

/**
 * "Start radio", only when the server offers radio for the item (AudioMuse-AI's mix or Don't
 * Stop The Music); nothing at all otherwise.
 */
@Composable
fun StartRadioButton(state: DiscoveryUiState, onStart: () -> Unit, modifier: Modifier = Modifier) {
    val plan = state.radio ?: return
    OutlinedButton(
        onClick = onStart,
        enabled = state.canStartRadio,
        modifier = modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp).testTag("start-radio"),
    ) {
        Icon(Icons.Outlined.Radio, contentDescription = null)
        Column(Modifier.padding(start = 8.dp)) {
            Text(stringResource(R.string.action_start_radio), maxLines = 1, overflow = TextOverflow.Ellipsis)
            val how = when {
                plan.mixLabel != null && plan.dstm != null -> stringResource(R.string.radio_from_mix_endless, plan.mixLabel!!)
                plan.mixLabel != null -> stringResource(R.string.radio_from_mix, plan.mixLabel!!)
                else -> stringResource(R.string.radio_from_dstm, plan.dstm?.name.orEmpty())
            }
            Text(how, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** "More like this": the server's similar music, as a row of cards that play when chosen. */
@Composable
fun MoreLikeThisShelf(state: DiscoveryUiState, onPlay: (MenuItem) -> Unit, modifier: Modifier = Modifier) {
    if (state.similar.isEmpty()) return
    Column(modifier.fillMaxWidth().testTag("more-like-this")) {
        SectionHeader(state.similarLabel?.ifBlank { null } ?: stringResource(R.string.more_like_this))
        LazyRow(contentPadding = PaddingValues(horizontal = 16.dp)) {
            itemsIndexed(state.similar, key = { i, _ -> i }) { _, item ->
                val lines = item.text.lines().map { it.trim() }.filter { it.isNotEmpty() }
                val title = lines.firstOrNull() ?: stringResource(R.string.unknown_track)
                Surface(
                    shape = MaterialTheme.shapes.medium,
                    modifier = Modifier
                        .padding(end = 12.dp)
                        .width(132.dp)
                        .clickable(onClickLabel = stringResource(R.string.action_play)) { onPlay(item) },
                ) {
                    Column(Modifier.padding(bottom = 8.dp)) {
                        val icon = item.icon
                        if (icon != null) {
                            ArtworkImage(icon, 300, contentDescription = null, modifier = Modifier.size(132.dp), corner = 8.dp)
                        } else {
                            Surface(color = MaterialTheme.colorScheme.surfaceVariant, modifier = Modifier.size(132.dp)) {
                                Box(contentAlignment = Alignment.Center) {
                                    Icon(Icons.Outlined.MusicNote, contentDescription = null, modifier = Modifier.size(48.dp))
                                }
                            }
                        }
                        Text(
                            title,
                            style = MaterialTheme.typography.bodyMedium,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.padding(top = 6.dp),
                        )
                        lines.getOrNull(1)?.let {
                            Text(
                                it,
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                }
            }
        }
    }
}
