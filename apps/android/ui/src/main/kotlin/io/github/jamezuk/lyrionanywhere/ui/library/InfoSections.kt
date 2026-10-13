package io.github.jamezuk.lyrionanywhere.ui.library

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import coil3.compose.AsyncImage
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.art.Artwork
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage

/** Lines shown before "Read more". */
const val INFO_COLLAPSED_LINES = 3

/** Text longer than this always offers "Read more" (about three lines on a phone). */
private const val LONG_INFO_CHARS = 240

/**
 * A biography or review (X5): a heading, then the text (plain text, as the core checked it),
 * its first [INFO_COLLAPSED_LINES] lines until "Read more" is pressed. The button says whether
 * the text is collapsed or expanded, for TalkBack; the text itself is read out whole.
 */
@Composable
fun ExpandableInfo(heading: String, text: String, modifier: Modifier = Modifier) {
    var expanded by rememberSaveable(text) { mutableStateOf(false) }
    // Whether the collapsed text was cut short (so the button is worth showing): as laid
    // out, or plainly longer than three lines (several paragraphs, or a long one).
    var overflows by remember(text) { mutableStateOf(false) }
    val long = remember(text) { text.length > LONG_INFO_CHARS || text.count { it == '\n' } >= INFO_COLLAPSED_LINES }
    val stateText = stringResource(if (expanded) R.string.info_expanded else R.string.info_collapsed)
    val toggleLabel = stringResource(if (expanded) R.string.info_show_less else R.string.info_read_more)
    Column(modifier.fillMaxWidth().padding(vertical = 8.dp).testTag("info-$heading")) {
        Text(
            heading,
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(bottom = 4.dp).semantics { heading() },
        )
        Text(
            text,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurface,
            maxLines = if (expanded) Int.MAX_VALUE else INFO_COLLAPSED_LINES,
            overflow = TextOverflow.Ellipsis,
            onTextLayout = { if (!expanded) overflows = it.hasVisualOverflow },
        )
        if (long || overflows || expanded) {
            TextButton(
                onClick = { expanded = !expanded },
                modifier = Modifier.semantics {
                    stateDescription = stateText
                    onClick(label = toggleLabel) {
                        expanded = !expanded
                        true
                    }
                },
            ) {
                Text(toggleLabel)
            }
        }
    }
}

/**
 * An artist's photo (an image-proxy artwork key, fetched through the gateway), round, with
 * its credit. If it can't be loaded, the [fallback] artwork is shown instead (decoratively),
 * and the credit is hidden.
 */
@Composable
fun ArtistPhoto(key: String, name: String, credits: String?, fallback: String?) {
    var failed by remember(key) { mutableStateOf(false) }
    val size = Modifier.widthIn(max = 240.dp).fillMaxWidth(0.56f)
    if (failed) {
        ArtworkImage(fallback, 600, contentDescription = null, modifier = Modifier.widthIn(max = 320.dp).fillMaxWidth(0.72f), corner = 12.dp)
        return
    }
    Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        AsyncImage(
            model = Artwork(key, 600),
            contentDescription = stringResource(R.string.info_photo_of, name),
            contentScale = ContentScale.Crop,
            onError = { failed = true },
            modifier = size.aspectRatio(1f).clip(CircleShape).testTag("artist-photo"),
        )
        if (!credits.isNullOrBlank()) {
            Text(
                stringResource(R.string.info_photo_credit, credits),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
