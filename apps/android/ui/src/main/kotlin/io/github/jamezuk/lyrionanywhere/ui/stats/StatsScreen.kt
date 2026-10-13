package io.github.jamezuk.lyrionanywhere.ui.stats

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.AutoAwesome
import androidx.compose.material.icons.outlined.Insights
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
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
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.data.ListeningStats
import io.github.jamezuk.lyrionanywhere.data.StatsPeriod
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState

/** What the "Your listening" page can do. */
data class StatsActions(
    val onPeriod: (StatsPeriod) -> Unit = {},
    val onRetry: () -> Unit = {},
    val onArtist: (id: Long, name: String) -> Unit = { _, _ -> },
    val onAlbum: (Long) -> Unit = {},
    /** A story for a year with history. */
    val onStory: (Int) -> Unit = {},
    /** History is off: the setting that switches it on. */
    val onHistorySettings: () -> Unit = {},
)

private const val SHORT_LIST = 10

/** The "Your listening" page (docs/plans/wave3-design.md §1). */
@Composable
fun StatsScreen(state: StatsUiState, actions: StatsActions, modifier: Modifier = Modifier) {
    val res = LocalResources.current
    LazyColumn(modifier.fillMaxSize().testTag("stats-screen"), contentPadding = PaddingValues(bottom = 24.dp)) {
        item(key = "periods") { PeriodPicker(state, actions.onPeriod) }
        val s = state.stats
        when {
            state.loading && s == null -> item(key = "loading") { Loading() }
            state.error != null && state.unavailable -> item(key = "unavailable") { Note(stringResource(R.string.stats_unavailable), "stats-unavailable") }
            state.error != null -> item(key = "error") {
                Box(Modifier.fillMaxWidth().height(320.dp)) { ErrorState(state.error, actions.onRetry) }
            }
            s == null -> Unit
            !s.enabled -> item(key = "off") {
                Column(Modifier.padding(16.dp).testTag("stats-off")) {
                    Text(stringResource(R.string.stats_off), style = MaterialTheme.typography.bodyLarge)
                    FilledTonalButton(onClick = actions.onHistorySettings, modifier = Modifier.padding(top = 12.dp)) {
                        Text(stringResource(R.string.stats_off_action))
                    }
                }
            }
            s.isEmpty -> item(key = "empty") { Note(stringResource(R.string.stats_empty), "stats-empty") }
            else -> content(state, s, actions, res)
        }
    }
}

@Composable
private fun Loading() {
    val label = stringResource(R.string.loading)
    Box(Modifier.fillMaxWidth().height(240.dp), contentAlignment = Alignment.Center) {
        CircularProgressIndicator(Modifier.semantics { contentDescription = label })
    }
}

@Composable
private fun Note(text: String, tag: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodyLarge,
        modifier = Modifier.padding(16.dp).testTag(tag).semantics { liveRegion = LiveRegionMode.Polite },
    )
}

@Composable
private fun PeriodPicker(state: StatsUiState, onPeriod: (StatsPeriod) -> Unit) {
    val res = LocalResources.current
    LazyRow(
        Modifier.fillMaxWidth().testTag("stats-periods"),
        contentPadding = PaddingValues(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        items(state.choices, key = { it.wire() }) { p ->
            FilterChip(
                selected = p == state.period,
                onClick = { onPeriod(p) },
                label = { Text(periodLabel(res, p, state.today)) },
                modifier = Modifier.testTag("stats-period-${p.wire()}"),
            )
        }
    }
}

@Composable
private fun SectionTitle(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.titleMedium,
        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 20.dp, bottom = 4.dp).semantics { heading() },
    )
}

private fun LazyListScope.content(state: StatsUiState, s: ListeningStats, actions: StatsActions, res: android.content.res.Resources) {
    item(key = "totals") { Totals(s) }
    state.storyYear?.let { year ->
        item(key = "story") {
            ListItem(
                leadingContent = { Icon(Icons.Outlined.AutoAwesome, contentDescription = null) },
                headlineContent = { Text(stringResource(R.string.stats_story_open, year)) },
                supportingContent = { Text(stringResource(R.string.stats_story_open_body)) },
                modifier = Modifier.clickable { actions.onStory(year) }.testTag("stats-story"),
            )
        }
    }
    if (s.topArtists.isNotEmpty()) {
        item(key = "artists") {
            TopList(stringResource(R.string.stats_top_artists), s.topArtists.size, "stats-top-artists") { shown ->
                s.topArtists.take(shown).forEachIndexed { i, a ->
                    TopRow(
                        rank = i + 1,
                        title = artistLabel(a.name),
                        plays = a.plays,
                        seconds = a.seconds,
                        art = { Initial(artistLabel(a.name)) },
                        onClick = { actions.onArtist(a.id, a.name) },
                    )
                }
            }
        }
    }
    if (s.topAlbums.isNotEmpty()) {
        item(key = "albums") {
            TopList(stringResource(R.string.stats_top_albums), s.topAlbums.size, "stats-top-albums") { shown ->
                s.topAlbums.take(shown).forEachIndexed { i, e ->
                    val album = state.albums[e.id]
                    TopRow(
                        rank = i + 1,
                        title = album?.title ?: stringResource(R.string.stats_unknown_album),
                        subtitle = album?.artist,
                        plays = e.plays,
                        seconds = e.seconds,
                        art = { ArtworkImage(album?.artwork, 120, contentDescription = null, modifier = Modifier.size(48.dp), corner = 4.dp) },
                        onClick = if (album != null) ({ actions.onAlbum(e.id) }) else null,
                    )
                }
            }
        }
    }
    if (s.topTracks.isNotEmpty()) {
        item(key = "tracks") {
            TopList(stringResource(R.string.stats_top_tracks), s.topTracks.size, "stats-top-tracks") { shown ->
                s.topTracks.take(shown).forEachIndexed { i, e ->
                    val track = state.tracks[e.id]
                    val albumId = track?.albumId
                    TopRow(
                        rank = i + 1,
                        title = track?.title ?: stringResource(R.string.stats_unknown_track),
                        subtitle = track?.artist,
                        plays = e.plays,
                        seconds = e.seconds,
                        art = { ArtworkImage(track?.artwork, 120, contentDescription = null, modifier = Modifier.size(48.dp), corner = 4.dp) },
                        // A track's page is its album.
                        onClick = if (albumId != null) ({ actions.onAlbum(albumId) }) else null,
                    )
                }
            }
        }
    }
    if (s.newArtists.isNotEmpty()) {
        item(key = "new") {
            Column(Modifier.testTag("stats-new-artists")) {
                SectionTitle(stringResource(R.string.stats_new_artists))
                // The history reaches back only so far: old favourites can look new.
                if (longPeriod(state.period)) {
                    Text(
                        stringResource(R.string.stats_new_artists_since, dayText(s.period.from)),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 16.dp),
                    )
                }
                s.newArtists.take(SHORT_LIST).forEach { a ->
                    ListItem(
                        leadingContent = { Initial(artistLabel(a.name)) },
                        headlineContent = { Text(artistLabel(a.name), maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        supportingContent = { Text(stringResource(R.string.stats_first_played, dayText(a.firstPlayed))) },
                        modifier = Modifier.clickable { actions.onArtist(a.id, a.name) },
                    )
                }
            }
        }
    }
    item(key = "when") {
        Column {
            SectionTitle(stringResource(R.string.stats_when))
            HourChart(res, s.byHour)
            WeekdayChart(res, s.byWeekday)
            if (s.byMonth.isNotEmpty()) MonthChart(res, s.byMonth)
        }
    }
    if (s.longestStreak != null || s.busiestDay != null) {
        item(key = "streaks") {
            Column(Modifier.testTag("stats-streaks")) {
                SectionTitle(stringResource(R.string.stats_streaks))
                s.longestStreak?.let { st ->
                    ListItem(
                        headlineContent = { Text(stringResource(R.string.stats_longest_streak)) },
                        supportingContent = {
                            Text(stringResource(R.string.stats_streak_range, daysText(res, st.days.toLong()), dayText(st.from), dayText(st.to)))
                        },
                        modifier = Modifier.semantics(mergeDescendants = true) {},
                    )
                }
                s.busiestDay?.let { d ->
                    ListItem(
                        headlineContent = { Text(stringResource(R.string.stats_busiest_day)) },
                        supportingContent = { Text(stringResource(R.string.stats_busiest_day_value, dayText(d.date), listenTime(res, d.seconds))) },
                        modifier = Modifier.semantics(mergeDescendants = true) {},
                    )
                }
            }
        }
    }
}

/** "Everything kept" and long periods reach the start of the history. */
private fun longPeriod(p: StatsPeriod): Boolean = when (p) {
    StatsPeriod.All, is StatsPeriod.Year -> true
    is StatsPeriod.LastDays -> p.days > 60
    is StatsPeriod.Month -> false
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Totals(s: ListeningStats) {
    val res = LocalResources.current
    val t = s.totals
    Column(Modifier.testTag("stats-totals")) {
        SectionTitle(stringResource(R.string.stats_totals))
        FlowRow(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Tile(stringResource(R.string.stats_total_time), listenTime(res, t.seconds))
            Tile(stringResource(R.string.stats_total_plays), t.plays.toString())
            Tile(stringResource(R.string.stats_total_tracks), t.tracks.toString())
            Tile(stringResource(R.string.stats_total_albums), t.albums.toString())
            Tile(stringResource(R.string.stats_total_artists), t.artists.toString())
        }
    }
}

@Composable
private fun Tile(label: String, value: String) {
    val spoken = stringResource(R.string.stats_tile, label, value)
    Card(
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.secondaryContainer),
        modifier = Modifier.widthIn(min = 96.dp).clearAndSetSemantics { contentDescription = spoken },
    ) {
        Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
            Text(value, style = MaterialTheme.typography.headlineSmall, color = MaterialTheme.colorScheme.onSecondaryContainer)
            Text(label, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSecondaryContainer)
        }
    }
}

/** A top-10 list that can show 20. */
@Composable
private fun TopList(title: String, size: Int, tag: String, rows: @Composable (shown: Int) -> Unit) {
    var all by rememberSaveable(tag) { mutableStateOf(false) }
    Column(Modifier.testTag(tag)) {
        SectionTitle(title)
        rows(if (all) ListeningStats.MAX_LIST else SHORT_LIST)
        if (size > SHORT_LIST) {
            TextButton(onClick = { all = !all }, modifier = Modifier.padding(horizontal = 8.dp).testTag("$tag-more")) {
                Text(stringResource(if (all) R.string.stats_show_fewer else R.string.stats_show_more))
            }
        }
    }
}

@Composable
private fun TopRow(
    rank: Int,
    title: String,
    plays: Long,
    seconds: Long,
    art: @Composable () -> Unit,
    onClick: (() -> Unit)?,
    subtitle: String? = null,
) {
    val res = LocalResources.current
    val detail = stringResource(R.string.stats_plays_and_time, playsText(res, plays), listenTime(res, seconds))
    ListItem(
        leadingContent = art,
        headlineContent = { Text(stringResource(R.string.stats_rank, rank, title), maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = {
            Text(listOfNotNull(subtitle?.takeIf { it.isNotBlank() }, detail).joinToString(" · "), maxLines = 2, overflow = TextOverflow.Ellipsis)
        },
        modifier = if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier.semantics(mergeDescendants = true) {},
    )
}

/** An artist's initial in a circle (artists have no cover of their own). */
@Composable
private fun Initial(name: String) {
    Box(
        Modifier.size(48.dp).background(MaterialTheme.colorScheme.tertiaryContainer, CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            name.trim().firstOrNull()?.uppercase() ?: "?",
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.onTertiaryContainer,
            modifier = Modifier.clearAndSetSemantics {},
        )
    }
}

/** Settings › Listening history: the way in to "Your listening". */
@Composable
fun StatsSettingsItem(onOpen: () -> Unit) {
    ListItem(
        leadingContent = { Icon(Icons.Outlined.Insights, contentDescription = null) },
        headlineContent = { Text(stringResource(R.string.stats_open)) },
        supportingContent = { Text(stringResource(R.string.stats_open_body)) },
        modifier = Modifier.clickable(onClick = onOpen).testTag("settings-stats"),
    )
}

/**
 * The card at the end of Home: "This month: 23 h · 412 plays", and "Your year in sound" from
 * 1 December to 31 January. Hidden while loading, with history off, or with nothing played.
 */
@Composable
fun ListeningCard(state: ListeningCardState, onOpen: () -> Unit, onStory: (Int) -> Unit, modifier: Modifier = Modifier) {
    if (!state.visible) return
    val res = LocalResources.current
    Card(
        onClick = onOpen,
        modifier = modifier.fillMaxWidth().padding(16.dp).testTag("home-listening-card"),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.secondaryContainer),
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(
                stringResource(R.string.stats_card_title),
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
            )
            if (state.monthPlays > 0) {
                Text(
                    stringResource(R.string.stats_card_month, listenTime(res, state.monthSeconds), playsText(res, state.monthPlays)),
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSecondaryContainer,
                )
            }
            state.storyYear?.let { year ->
                OutlinedButton(onClick = { onStory(year) }, modifier = Modifier.padding(top = 8.dp).testTag("home-story")) {
                    Icon(Icons.Outlined.AutoAwesome, contentDescription = null, modifier = Modifier.size(18.dp))
                    Text(stringResource(R.string.stats_card_story, year), Modifier.padding(start = 8.dp))
                }
            }
        }
    }
}

/** An artist's name, or a fallback when the server couldn't name it (it sends ""). */
@Composable
internal fun artistLabel(name: String): String = name.ifBlank { stringResource(R.string.stats_unknown_artist) }
