package io.github.jamezuk.lyrionanywhere.ui.offline

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.CloudOff
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material.icons.outlined.DownloadDone
import androidx.compose.material.icons.outlined.GraphicEq
import androidx.compose.material.icons.outlined.Pause
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Shuffle
import androidx.compose.material.icons.outlined.Storage
import androidx.compose.material.icons.outlined.WifiOff
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.remember
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.DownloadFailure
import io.github.jamezuk.lyrionanywhere.core.DownloadItem
import io.github.jamezuk.lyrionanywhere.core.DownloadState
import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.core.OfflineAlbum
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.art.offlineArtKey
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.components.EmptyState
import io.github.jamezuk.lyrionanywhere.ui.components.formatDuration
import io.github.jamezuk.lyrionanywhere.ui.library.SectionHeader
import java.util.Locale

/** "23.4 MB", "1.20 GB". */
fun formatBytes(n: Long): String = when {
    n <= 0 -> "0 MB"
    n >= 1024L * 1024 * 1024 -> String.format(Locale.UK, "%.2f GB", n / (1024.0 * 1024 * 1024))
    else -> String.format(Locale.UK, "%.1f MB", n / (1024.0 * 1024))
}

/** What the Downloads screen can do. */
data class DownloadsActions(
    val onPause: (Long?) -> Unit = {},
    val onResume: (Long?) -> Unit = {},
    val onCancel: (Long?) -> Unit = {},
    val onClearFinished: () -> Unit = {},
    val onAlbum: (String) -> Unit = {},
    val onPlayAlbum: (String) -> Unit = {},
    val onRemoveAlbum: (String) -> Unit = {},
    val onRemoveAll: () -> Unit = {},
    val onStorage: (() -> Unit)? = null,
)

/**
 * A strip saying the app is offline, with what can be done about it. The buttons sit on
 * their own line, so large text never squeezes the message.
 */
@Composable
fun OfflineBannerBar(text: String, onRetry: (() -> Unit)?, modifier: Modifier = Modifier, action: @Composable () -> Unit = {}) {
    Surface(color = MaterialTheme.colorScheme.tertiaryContainer, modifier = modifier.fillMaxWidth().testTag("offline-banner")) {
        Column(Modifier.padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 4.dp)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                modifier = Modifier.padding(end = 8.dp),
            ) {
                Icon(Icons.Outlined.CloudOff, contentDescription = null, tint = MaterialTheme.colorScheme.onTertiaryContainer)
                Text(
                    text,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onTertiaryContainer,
                    modifier = Modifier.weight(1f),
                )
            }
            FlowRow(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(4.dp, Alignment.End),
            ) {
                if (onRetry != null) {
                    TextButton(onClick = onRetry) { Text(stringResource(R.string.action_retry)) }
                }
                action()
            }
        }
    }
}

@Composable
private fun StorageLine(used: Long, cap: Long, tracks: Int, onStorage: (() -> Unit)?) {
    ListItem(
        leadingContent = { Icon(Icons.Outlined.Storage, contentDescription = null) },
        headlineContent = {
            Text(
                if (cap > 0) {
                    stringResource(R.string.storage_used_of, formatBytes(used), formatBytes(cap))
                } else {
                    stringResource(R.string.storage_used, formatBytes(used))
                },
            )
        },
        supportingContent = {
            Column {
                Text(LocalResources.current.getQuantityString(R.plurals.downloaded_tracks, tracks, tracks))
                if (cap > 0) {
                    LinearProgressIndicator(
                        progress = { (used.toDouble() / cap).toFloat().coerceIn(0f, 1f) },
                        modifier = Modifier.fillMaxWidth().padding(top = 6.dp),
                    )
                }
            }
        },
        modifier = if (onStorage != null) Modifier.clickable(onClickLabel = stringResource(R.string.title_storage), onClick = onStorage) else Modifier,
    )
}

@Composable
fun downloadStatusText(item: DownloadItem): String = when (item.state) {
    DownloadState.QUEUED -> item.message.ifEmpty { stringResource(R.string.download_queued) }
    DownloadState.ACTIVE -> if (item.total > 0uL) {
        stringResource(R.string.download_progress, formatBytes(item.received.toLong()), formatBytes(item.total.toLong()))
    } else {
        stringResource(R.string.download_starting)
    }
    DownloadState.WAITING -> item.message.ifEmpty { stringResource(R.string.download_waiting) }
    DownloadState.PAUSED -> stringResource(R.string.download_paused)
    DownloadState.DONE -> stringResource(R.string.download_done)
    DownloadState.FAILED -> item.message.ifEmpty {
        stringResource(
            when (item.failure) {
                DownloadFailure.NOT_ALLOWED -> R.string.download_not_allowed
                DownloadFailure.STORAGE_FULL -> R.string.download_storage_full
                DownloadFailure.LIMIT_REACHED -> R.string.download_limit
                else -> R.string.download_failed
            },
        )
    }
}

@Composable
private fun DownloadRow(item: DownloadItem, actions: DownloadsActions) {
    val title = item.title.ifEmpty { stringResource(R.string.download_track_id, item.trackId) }
    val status = downloadStatusText(item)
    val resume = item.state == DownloadState.PAUSED || item.state == DownloadState.FAILED
    val toggleLabel = stringResource(
        when {
            item.state == DownloadState.FAILED -> R.string.download_retry_for
            resume -> R.string.download_resume_for
            else -> R.string.download_pause_for
        },
        title,
    )
    val cancelLabel = stringResource(R.string.download_cancel_for, title)
    val toggle = { if (resume) actions.onResume(item.trackId) else actions.onPause(item.trackId) }
    ListItem(
        headlineContent = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = {
            Column {
                Text(status, maxLines = 2, overflow = TextOverflow.Ellipsis)
                if (item.state == DownloadState.ACTIVE) {
                    val p = progressOf(item)
                    if (p != null) {
                        LinearProgressIndicator(progress = { p }, modifier = Modifier.fillMaxWidth().padding(top = 4.dp))
                    } else {
                        LinearProgressIndicator(modifier = Modifier.fillMaxWidth().padding(top = 4.dp))
                    }
                }
            }
        },
        // One trailing control (Wave 3.5 rule 3): ⋮ with pause (resume or retry) and cancel,
        // both also TalkBack actions on the row.
        trailingContent = {
            var menu by remember { mutableStateOf(false) }
            Box {
                IconButton(onClick = { menu = true }, modifier = Modifier.testTag("download-more-${item.trackId}")) {
                    Icon(Icons.Outlined.MoreVert, contentDescription = stringResource(R.string.more_for, title))
                }
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    DropdownMenuItem(
                        text = { Text(toggleLabel) },
                        leadingIcon = {
                            Icon(
                                when {
                                    item.state == DownloadState.FAILED -> Icons.Outlined.Refresh
                                    resume -> Icons.Outlined.PlayArrow
                                    else -> Icons.Outlined.Pause
                                },
                                contentDescription = null,
                            )
                        },
                        onClick = {
                            menu = false
                            toggle()
                        },
                    )
                    DropdownMenuItem(
                        text = { Text(cancelLabel) },
                        leadingIcon = { Icon(Icons.Outlined.Delete, contentDescription = null) },
                        onClick = {
                            menu = false
                            actions.onCancel(item.trackId)
                        },
                    )
                }
            }
        },
        modifier = Modifier.semantics {
            customActions = listOf(
                CustomAccessibilityAction(toggleLabel) { toggle(); true },
                CustomAccessibilityAction(cancelLabel) { actions.onCancel(item.trackId); true },
            )
        },
    )
}

@Composable
fun OfflineAlbumRow(album: OfflineAlbum, onOpen: () -> Unit, trailing: @Composable () -> Unit = {}) {
    val title = album.title.ifEmpty { stringResource(R.string.unknown_album) }
    ListItem(
        leadingContent = {
            ArtworkImage(
                if (album.hasArt) offlineArtKey(album.albumRef) else null,
                120,
                contentDescription = null,
                modifier = Modifier.size(56.dp),
                corner = 6.dp,
            )
        },
        headlineContent = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = {
            val n = album.trackCount.toInt()
            Text(
                listOf(
                    album.artist.ifEmpty { stringResource(R.string.various_artists) },
                    LocalResources.current.getQuantityString(R.plurals.track_count, n, n),
                    formatBytes(album.size.toLong()),
                ).joinToString(" · "),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        },
        trailingContent = trailing,
        modifier = Modifier.clickable(onClickLabel = stringResource(R.string.open_album), onClick = onOpen),
    )
}

/** Downloads: in progress, done (by album), storage used, remove and remove all. */
@Composable
fun DownloadsScreen(state: DownloadsUiState, actions: DownloadsActions, modifier: Modifier = Modifier) {
    var confirmAll by rememberSaveable { mutableStateOf(false) }
    var confirmAlbum by rememberSaveable { mutableStateOf<String?>(null) }
    LazyColumn(modifier.fillMaxSize().testTag("downloads"), contentPadding = PaddingValues(bottom = 24.dp)) {
        if (state.forbidden) {
            item {
                OfflineBannerBar(stringResource(R.string.downloads_forbidden), onRetry = { actions.onResume(null) })
            }
        }
        state.waitingFor?.let { w ->
            item {
                ListItem(
                    leadingContent = { Icon(Icons.Outlined.WifiOff, contentDescription = null) },
                    headlineContent = {
                        Text(stringResource(if (w == NetworkKind.OFFLINE) R.string.downloads_wait_connection else R.string.downloads_wait_wifi))
                    },
                )
            }
        }
        item { StorageLine(state.usedBytes, state.capBytes, state.trackCount, actions.onStorage) }
        if (state.empty) {
            item {
                EmptyState(
                    stringResource(if (state.canDownload) R.string.downloads_empty else R.string.downloads_empty_offline),
                    Modifier.padding(24.dp),
                )
            }
        }
        if (state.inProgress.isNotEmpty()) {
            item { SectionHeader(stringResource(R.string.downloads_in_progress)) }
            item {
                FlowRow(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (state.anyRunning) {
                        OutlinedButton(onClick = { actions.onPause(null) }) { Text(stringResource(R.string.downloads_pause_all)) }
                    }
                    if (state.anyPaused) {
                        OutlinedButton(onClick = { actions.onResume(null) }) { Text(stringResource(R.string.downloads_resume_all)) }
                    }
                    OutlinedButton(onClick = { actions.onCancel(null) }) { Text(stringResource(R.string.downloads_cancel_all)) }
                }
            }
            items(state.inProgress, key = { "dl-${it.trackId}" }) { DownloadRow(it, actions) }
        }
        if (state.albums.isNotEmpty()) {
            item { SectionHeader(stringResource(R.string.downloads_done)) }
            items(state.albums, key = { "al-${it.albumRef}" }) { album ->
                // One trailing control (Wave 3.5 rule 3): ⋮ with Play and (destructive, so
                // only here) Remove download.
                OfflineAlbumRow(album, onOpen = { actions.onAlbum(album.albumRef) }) {
                    var menu by remember { mutableStateOf(false) }
                    Box {
                        IconButton(onClick = { menu = true }) {
                            Icon(Icons.Outlined.MoreVert, contentDescription = stringResource(R.string.more_for, album.title))
                        }
                        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                            DropdownMenuItem(
                                text = { Text(stringResource(R.string.play_album_named, album.title)) },
                                leadingIcon = { Icon(Icons.Outlined.PlayArrow, contentDescription = null) },
                                onClick = {
                                    menu = false
                                    actions.onPlayAlbum(album.albumRef)
                                },
                            )
                            DropdownMenuItem(
                                text = { Text(stringResource(R.string.remove_download_named, album.title)) },
                                leadingIcon = { Icon(Icons.Outlined.Delete, contentDescription = null) },
                                onClick = {
                                    menu = false
                                    confirmAlbum = album.albumRef
                                },
                            )
                        }
                    }
                }
            }
            item {
                HorizontalDivider(Modifier.padding(vertical = 8.dp))
                TextButton(onClick = { confirmAll = true }, modifier = Modifier.padding(horizontal = 8.dp)) {
                    Icon(Icons.Outlined.Delete, contentDescription = null)
                    Text(stringResource(R.string.downloads_remove_all), Modifier.padding(start = 8.dp))
                }
            }
        }
    }
    if (confirmAll) {
        AlertDialog(
            onDismissRequest = { confirmAll = false },
            title = { Text(stringResource(R.string.downloads_remove_all_title)) },
            text = {
                Text(
                    stringResource(R.string.downloads_remove_all_body, formatBytes(state.usedBytes)),
                    Modifier.verticalScroll(rememberScrollState()),
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    confirmAll = false
                    actions.onRemoveAll()
                }) { Text(stringResource(R.string.action_remove)) }
            },
            dismissButton = { TextButton(onClick = { confirmAll = false }) { Text(stringResource(R.string.action_cancel)) } },
        )
    }
    confirmAlbum?.let { ref ->
        val title = state.albums.firstOrNull { it.albumRef == ref }?.title.orEmpty()
        AlertDialog(
            onDismissRequest = { confirmAlbum = null },
            title = { Text(stringResource(R.string.remove_download_title)) },
            text = { Text(stringResource(R.string.remove_download_body, title), Modifier.verticalScroll(rememberScrollState())) },
            confirmButton = {
                TextButton(onClick = {
                    confirmAlbum = null
                    actions.onRemoveAlbum(ref)
                }) { Text(stringResource(R.string.action_remove)) }
            },
            dismissButton = { TextButton(onClick = { confirmAlbum = null }) { Text(stringResource(R.string.action_cancel)) } },
        )
    }
}

/** Offline mode's library: the downloaded albums, ready to play. */
@Composable
fun OfflineLibraryScreen(
    albums: List<OfflineAlbum>,
    onAlbum: (String) -> Unit,
    onPlayAlbum: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (albums.isEmpty()) {
        EmptyState(stringResource(R.string.offline_nothing), modifier.padding(24.dp))
        return
    }
    LazyColumn(modifier.fillMaxSize().testTag("offline-library"), contentPadding = PaddingValues(bottom = 24.dp)) {
        item { SectionHeader(stringResource(R.string.offline_library)) }
        items(albums, key = { it.albumRef }) { album ->
            OfflineAlbumRow(album, onOpen = { onAlbum(album.albumRef) }) {
                IconButton(onClick = { onPlayAlbum(album.albumRef) }) {
                    Icon(Icons.Outlined.PlayArrow, contentDescription = stringResource(R.string.play_album_named, album.title))
                }
            }
        }
    }
}

/** A downloaded album: Play, Shuffle, the tracks (tap to play from one), Remove. */
@Composable
fun OfflineAlbumScreen(
    state: OfflineAlbumUi,
    onPlay: (Int) -> Unit,
    onShuffle: () -> Unit,
    onRemove: (() -> Unit)?,
    modifier: Modifier = Modifier,
) {
    if (state.tracks.isEmpty()) {
        EmptyState(stringResource(R.string.offline_album_gone), modifier.padding(24.dp))
        return
    }
    var confirm by rememberSaveable { mutableStateOf(false) }
    val album = state.album
    val title = album?.title?.ifEmpty { null } ?: state.tracks.first().album.ifEmpty { stringResource(R.string.unknown_album) }
    LazyColumn(modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 24.dp)) {
        item {
            Column(
                Modifier.fillMaxWidth().padding(16.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                // Decorative: the title below is the heading.
                ArtworkImage(
                    if (album?.hasArt == true) offlineArtKey(album.albumRef) else null,
                    600,
                    contentDescription = null,
                    modifier = Modifier.fillMaxWidth(0.6f),
                    corner = 12.dp,
                )
                Text(title, style = MaterialTheme.typography.headlineSmall, modifier = Modifier.semantics { heading() })
                Text(
                    (album?.artist ?: state.tracks.first().artist).ifEmpty { stringResource(R.string.various_artists) },
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.primary,
                )
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Outlined.DownloadDone, contentDescription = null, modifier = Modifier.size(16.dp))
                    Text(
                        stringResource(R.string.offline_available),
                        style = MaterialTheme.typography.labelMedium,
                        modifier = Modifier.padding(start = 4.dp),
                    )
                }
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(onClick = { onPlay(0) }) {
                        Icon(Icons.Outlined.PlayArrow, contentDescription = null)
                        Text(stringResource(R.string.action_play), Modifier.padding(start = 4.dp))
                    }
                    FilledTonalButton(onClick = onShuffle) {
                        Icon(Icons.Outlined.Shuffle, contentDescription = null)
                        Text(stringResource(R.string.shuffle), Modifier.padding(start = 4.dp))
                    }
                    if (onRemove != null) {
                        IconButton(onClick = { confirm = true }) {
                            Icon(Icons.Outlined.Delete, contentDescription = stringResource(R.string.remove_download_named, title))
                        }
                    }
                }
            }
            HorizontalDivider()
        }
        items(state.tracks.size, key = { state.tracks[it].key }) { i ->
            OfflineTrackRow(state.tracks[i], onClick = { onPlay(i) })
        }
    }
    if (confirm && onRemove != null) {
        AlertDialog(
            onDismissRequest = { confirm = false },
            title = { Text(stringResource(R.string.remove_download_title)) },
            text = { Text(stringResource(R.string.remove_download_body, title), Modifier.verticalScroll(rememberScrollState())) },
            confirmButton = {
                TextButton(onClick = {
                    confirm = false
                    onRemove()
                }) { Text(stringResource(R.string.action_remove)) }
            },
            dismissButton = { TextButton(onClick = { confirm = false }) { Text(stringResource(R.string.action_cancel)) } },
        )
    }
}

/** A downloaded track; [highlighted] marks the one playing, not by colour alone. */
@Composable
fun OfflineTrackRow(track: OfflineTrack, onClick: () -> Unit, highlighted: Boolean = false) {
    val playLabel = stringResource(R.string.play_track)
    val nowPlaying = stringResource(R.string.now_playing)
    ListItem(
        leadingContent = {
            Text(
                track.trackNumber?.toString() ?: "–",
                style = MaterialTheme.typography.bodyMedium,
                color = if (highlighted) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        },
        headlineContent = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (highlighted) {
                    Icon(
                        Icons.Outlined.GraphicEq,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(end = 6.dp).size(18.dp),
                    )
                }
                Text(
                    track.title.ifEmpty { stringResource(R.string.unknown_track) },
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    color = if (highlighted) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
                    modifier = Modifier.weight(1f, fill = false),
                )
            }
        },
        supportingContent = { Text(track.artist, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        trailingContent = { Text(formatDuration(track.durationSecs.takeIf { it > 0 })) },
        modifier = Modifier.clickable(onClickLabel = playLabel, onClick = onClick).semantics {
            if (highlighted) stateDescription = nowPlaying
        },
    )
}

/** The Download button on album and playlist pages: what it does depends on what's here. */
@Composable
fun DownloadButton(mark: DownloadMark, onDownload: () -> Unit, modifier: Modifier = Modifier) {
    when (mark) {
        DownloadMark.ALL -> IconButton(onClick = onDownload, modifier = modifier) {
            Icon(Icons.Outlined.DownloadDone, contentDescription = stringResource(R.string.downloaded))
        }
        else -> IconButton(onClick = onDownload, modifier = modifier) {
            Icon(
                Icons.Outlined.Download,
                contentDescription = stringResource(if (mark == DownloadMark.SOME) R.string.download_rest else R.string.action_download),
            )
        }
    }
}
