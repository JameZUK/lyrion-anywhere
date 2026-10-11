package io.github.jamezuk.lyrionanywhere.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Album
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import coil3.compose.SubcomposeAsyncImage
import io.github.jamezuk.lyrionanywhere.ui.art.Artwork

/**
 * Artwork by key, loaded through the Rust Art API (with the bearer token, never in a URL).
 * Shows a neutral placeholder while loading, on failure, and when there is no artwork.
 */
@Composable
fun ArtworkImage(
    key: String?,
    sizePx: Int,
    contentDescription: String?,
    modifier: Modifier = Modifier,
    corner: Dp = 8.dp,
) {
    val shape = RoundedCornerShape(corner)
    val box = modifier
        .aspectRatio(1f)
        .clip(shape)
        .background(MaterialTheme.colorScheme.surfaceVariant)
    if (key == null) {
        Placeholder(box)
        return
    }
    SubcomposeAsyncImage(
        model = Artwork(key, sizePx),
        contentDescription = contentDescription,
        contentScale = ContentScale.Crop,
        modifier = box,
        loading = { Placeholder(Modifier.fillMaxSize()) },
        error = { Placeholder(Modifier.fillMaxSize()) },
    )
}

@Composable
private fun Placeholder(modifier: Modifier) {
    Box(modifier, contentAlignment = Alignment.Center) {
        Icon(
            Icons.Outlined.Album,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.fillMaxWidth(0.4f).aspectRatio(1f),
        )
    }
}
