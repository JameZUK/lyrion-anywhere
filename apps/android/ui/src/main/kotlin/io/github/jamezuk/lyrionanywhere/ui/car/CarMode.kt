package io.github.jamezuk.lyrionanywhere.ui.car

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Favorite
import androidx.compose.material.icons.outlined.Mic
import androidx.compose.material.icons.outlined.Pause
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.Radio
import androidx.compose.material.icons.outlined.SkipNext
import androidx.compose.material.icons.outlined.SkipPrevious
import androidx.compose.material.icons.outlined.Stream
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.core.Favourite
import io.github.jamezuk.lyrionanywhere.core.RadioSeed
import io.github.jamezuk.lyrionanywhere.core.Station
import io.github.jamezuk.lyrionanywhere.core.StationBatch
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.DiscoveryRepository
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository
import io.github.jamezuk.lyrionanywhere.data.MeRepository
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.components.formatDuration
import io.github.jamezuk.lyrionanywhere.ui.discovery.DiscoveryEvent
import io.github.jamezuk.lyrionanywhere.ui.discovery.startRadioFrom
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.rememberElapsed
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.launch

/** Car mode's touch targets: at least this big (Wave 2 §5.1). */
val CAR_TARGET = 76.dp

/**
 * Car mode's colours: white and amber on black, for glare and quick glances. Every pair is
 * well over 7:1 (ContrastTests).
 */
val CarColours: ColorScheme = darkColorScheme(
    primary = Color(0xFFFFD54F),
    onPrimary = Color(0xFF000000),
    primaryContainer = Color(0xFFFFD54F),
    onPrimaryContainer = Color(0xFF000000),
    secondaryContainer = Color(0xFF262626),
    onSecondaryContainer = Color(0xFFFFFFFF),
    background = Color(0xFF000000),
    onBackground = Color(0xFFFFFFFF),
    surface = Color(0xFF000000),
    onSurface = Color(0xFFFFFFFF),
    surfaceVariant = Color(0xFF1C1C1C),
    onSurfaceVariant = Color(0xFFE6E6E6),
)

/** Car mode's one-level lists: favourites (not folders) and stations (that still work). */
data class CarListsUi(
    val favourites: List<Favourite> = emptyList(),
    val stations: List<Station> = emptyList(),
    val loading: Boolean = true,
)

/** What car mode tells the user (a snackbar). */
sealed interface CarMessage {
    data class StationStarted(val name: String) : CarMessage

    data class Radio(val event: DiscoveryEvent) : CarMessage

    data class Failed(val error: AppError) : CarMessage

    data object NoPlayer : CarMessage
}

/** Car mode's lists, its stations and "Start radio" (always on the player the app controls). */
class CarModeViewModel(
    private val library: LibraryRepository,
    private val me: MeRepository?,
    private val discovery: DiscoveryRepository?,
    private val player: () -> String?,
) : ViewModel() {
    private val _state = MutableStateFlow(CarListsUi())
    val state: StateFlow<CarListsUi> = _state.asStateFlow()
    private val _messages = Channel<CarMessage>(Channel.BUFFERED)
    val messages: Flow<CarMessage> = _messages.receiveAsFlow()

    fun load() {
        viewModelScope.launch {
            val favourites = (library.favourites(0, LIST_MAX, null) as? ApiResult.Ok)?.value?.items.orEmpty()
                .filter { !it.isFolder }.take(LIST_MAX)
            val stations = (me?.stations() as? ApiResult.Ok)?.value.orEmpty().filter { it.broken == null }.take(LIST_MAX)
            _state.value = CarListsUi(favourites, stations, loading = false)
        }
    }

    fun startStation(station: Station) {
        val m = me ?: return
        val p = player()
        if (p == null) {
            _messages.trySend(CarMessage.NoPlayer)
            return
        }
        viewModelScope.launch {
            _messages.trySend(
                when (val r = m.startStation(station.id, p)) {
                    is ApiResult.Ok -> when (val b = r.value) {
                        is StationBatch.Broken -> CarMessage.Failed(AppError.InvalidInput(b.reason))
                        else -> CarMessage.StationStarted(station.name)
                    }
                    is ApiResult.Err -> CarMessage.Failed(r.error)
                },
            )
        }
    }

    /** "Start radio" from the track playing now. */
    fun startRadio(trackId: Long?) {
        val d = discovery ?: return
        viewModelScope.launch {
            _messages.trySend(CarMessage.Radio(startRadioFrom(d, player(), trackId?.takeIf { it > 0 }?.let { RadioSeed.Track(it) })))
        }
    }

    companion object {
        /** One level only, and not long: these are for a glance. */
        const val LIST_MAX = 30
    }
}

/** What car mode can do. */
data class CarModeActions(
    val onExit: () -> Unit,
    val onTogglePlay: () -> Unit,
    val onStop: () -> Unit,
    val onNext: () -> Unit,
    val onPrevious: () -> Unit,
    /** "Start radio" from the current track (null: not offered). */
    val onStartRadio: (() -> Unit)? = null,
    /** Voice search (null: not offered). */
    val onVoice: (() -> Unit)? = null,
    val onFavourite: (Favourite) -> Unit = {},
    val onStation: (Station) -> Unit = {},
)

private enum class CarView { PLAYING, FAVOURITES, STATIONS }

/**
 * The driving screen (Wave 2 §5.1): huge art, giant previous / play-pause / next (a long
 * press on play/pause stops), "Start radio", a large elapsed time, voice search, and one
 * level of Favourites and Stations. High contrast, every target at least [CAR_TARGET], no
 * visualiser, and the screen stays on while playing. Back leaves car mode.
 */
@Composable
fun CarModeScreen(
    state: PlayerUiState,
    lists: CarListsUi,
    clock: () -> Long,
    actions: CarModeActions,
    modifier: Modifier = Modifier,
) {
    var view by rememberSaveable { mutableStateOf(CarView.PLAYING) }
    BackHandler {
        if (view != CarView.PLAYING) view = CarView.PLAYING else actions.onExit()
    }
    val hostView = LocalView.current
    val playing = state.playing
    DisposableEffect(hostView, playing) {
        hostView.keepScreenOn = playing
        onDispose { hostView.keepScreenOn = false }
    }
    MaterialTheme(colorScheme = CarColours) {
        Surface(modifier.fillMaxSize().testTag("car-mode"), color = MaterialTheme.colorScheme.background) {
            Column(Modifier.fillMaxSize().padding(12.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                    if (view == CarView.PLAYING) {
                        CarIconButton(Icons.Outlined.Close, stringResource(R.string.car_leave), actions.onExit, "car-exit")
                    } else {
                        CarIconButton(
                            Icons.AutoMirrored.Outlined.ArrowBack,
                            stringResource(R.string.car_back_to_playing),
                            { view = CarView.PLAYING },
                            "car-back",
                        )
                    }
                    Text(
                        stringResource(
                            when (view) {
                                CarView.PLAYING -> R.string.car_mode
                                CarView.FAVOURITES -> R.string.car_favourites
                                CarView.STATIONS -> R.string.car_stations
                            },
                        ),
                        style = MaterialTheme.typography.headlineSmall,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f).padding(horizontal = 12.dp).semantics { heading() },
                    )
                    actions.onVoice?.let { voice ->
                        CarIconButton(Icons.Outlined.Mic, stringResource(R.string.car_voice), voice, "car-voice")
                    }
                }
                when (view) {
                    CarView.PLAYING -> Playing(state, clock, actions, onFavourites = { view = CarView.FAVOURITES }, onStations = { view = CarView.STATIONS })
                    CarView.FAVOURITES -> CarList(
                        lists.favourites.map { it.id to it.name },
                        empty = stringResource(if (lists.loading) R.string.car_loading else R.string.car_no_favourites),
                        tag = "car-favourite-",
                    ) { id ->
                        lists.favourites.firstOrNull { it.id == id }?.let(actions.onFavourite)
                        view = CarView.PLAYING
                    }
                    CarView.STATIONS -> CarList(
                        lists.stations.map { it.id.toString() to it.name },
                        empty = stringResource(if (lists.loading) R.string.car_loading else R.string.car_no_stations),
                        tag = "car-station-",
                    ) { id ->
                        lists.stations.firstOrNull { it.id.toString() == id }?.let(actions.onStation)
                        view = CarView.PLAYING
                    }
                }
            }
        }
    }
}

@Composable
private fun ColumnScope.Playing(
    state: PlayerUiState,
    clock: () -> Long,
    actions: CarModeActions,
    onFavourites: () -> Unit,
    onStations: () -> Unit,
) {
    val track = state.status?.current
    val elapsed = rememberElapsed(state, clock)
    val duration = state.status?.durationSecs
    BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
        val wide = maxWidth > maxHeight
        val art: @Composable (Modifier) -> Unit = { m ->
            Box(m, contentAlignment = Alignment.Center) {
                ArtworkImage(track?.artwork, 800, contentDescription = null, modifier = Modifier.fillMaxHeight(), corner = 16.dp)
            }
        }
        val details: @Composable () -> Unit = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    track?.title ?: stringResource(R.string.nothing_playing),
                    style = MaterialTheme.typography.headlineLarge,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
                track?.artist?.let {
                    Text(
                        it,
                        style = MaterialTheme.typography.titleLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                val time = "${formatDuration(elapsed)} / ${formatDuration(duration)}"
                val spoken = stringResource(R.string.car_elapsed, formatDuration(elapsed), formatDuration(duration))
                Text(
                    time,
                    style = MaterialTheme.typography.displaySmall,
                    maxLines = 1,
                    modifier = Modifier.testTag("car-elapsed").semantics { contentDescription = spoken },
                )
            }
        }
        if (wide) {
            Row(Modifier.fillMaxSize(), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                art(Modifier.fillMaxHeight().padding(vertical = 8.dp))
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.Center) { details() }
            }
        } else {
            Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                art(Modifier.weight(1f).fillMaxWidth().padding(top = 8.dp))
                details()
            }
        }
    }
    Transport(state.playing, actions)
    Row(
        Modifier.fillMaxWidth().padding(top = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        actions.onStartRadio?.let { radio ->
            CarTextButton(Icons.Outlined.Radio, stringResource(R.string.action_start_radio), radio, "car-radio", Modifier.weight(1f))
        }
        CarTextButton(Icons.Outlined.Favorite, stringResource(R.string.car_favourites), onFavourites, "car-favourites", Modifier.weight(1f))
        CarTextButton(Icons.Outlined.Stream, stringResource(R.string.car_stations), onStations, "car-stations", Modifier.weight(1f))
    }
}

/** Previous, play/pause (a long press stops) and next, as big as the screen allows. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun Transport(playing: Boolean, actions: CarModeActions) {
    val playLabel = stringResource(if (playing) R.string.action_pause else R.string.action_play)
    val stopLabel = stringResource(R.string.car_stop)
    Row(
        Modifier.fillMaxWidth().padding(vertical = 8.dp),
        horizontalArrangement = Arrangement.SpaceEvenly,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CarIconButton(Icons.Outlined.SkipPrevious, stringResource(R.string.action_previous), actions.onPrevious, "car-previous", size = 96.dp)
        Box(
            Modifier
                .size(120.dp)
                .clip(CircleShape)
                .background(MaterialTheme.colorScheme.primary)
                .combinedClickable(
                    role = Role.Button,
                    onLongClickLabel = stopLabel,
                    onLongClick = actions.onStop,
                    onClick = actions.onTogglePlay,
                )
                .semantics {
                    contentDescription = playLabel
                    customActions = listOf(CustomAccessibilityAction(stopLabel) { actions.onStop(); true })
                }
                .testTag("car-play"),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                if (playing) Icons.Outlined.Pause else Icons.Outlined.PlayArrow,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onPrimary,
                modifier = Modifier.size(72.dp),
            )
        }
        CarIconButton(Icons.Outlined.SkipNext, stringResource(R.string.action_next), actions.onNext, "car-next", size = 96.dp)
    }
}

/** One level of a list: big rows, tapped to play. */
@Composable
private fun CarList(items: List<Pair<String, String>>, empty: String, tag: String, onPick: (String) -> Unit) {
    if (items.isEmpty()) {
        Text(empty, style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(24.dp))
        return
    }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(vertical = 8.dp)) {
        items(items, key = { it.first }) { (id, name) ->
            Box(
                Modifier
                    .fillMaxWidth()
                    .heightIn(min = CAR_TARGET)
                    .clickable(role = Role.Button) { onPick(id) }
                    .padding(horizontal = 16.dp, vertical = 12.dp)
                    .testTag(tag + id),
                contentAlignment = Alignment.CenterStart,
            ) {
                Text(name, style = MaterialTheme.typography.headlineSmall, maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
            HorizontalDivider(color = MaterialTheme.colorScheme.surfaceVariant)
        }
    }
}

@Composable
private fun CarIconButton(icon: ImageVector, label: String, onClick: () -> Unit, tag: String, size: androidx.compose.ui.unit.Dp = CAR_TARGET) {
    Box(
        Modifier
            .size(size)
            .clip(CircleShape)
            .background(MaterialTheme.colorScheme.secondaryContainer)
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label }
            .testTag(tag),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.onSecondaryContainer, modifier = Modifier.size(size * 0.55f))
    }
}

@Composable
private fun CarTextButton(icon: ImageVector, label: String, onClick: () -> Unit, tag: String, modifier: Modifier = Modifier) {
    Button(
        onClick = onClick,
        shape = RoundedCornerShape(16.dp),
        colors = ButtonDefaults.buttonColors(
            containerColor = MaterialTheme.colorScheme.secondaryContainer,
            contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
        ),
        contentPadding = PaddingValues(horizontal = 8.dp, vertical = 8.dp),
        modifier = modifier.heightIn(min = CAR_TARGET).widthIn(min = CAR_TARGET).testTag(tag),
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Icon(icon, contentDescription = null, modifier = Modifier.size(28.dp))
            Spacer(Modifier.size(2.dp))
            Text(label, style = MaterialTheme.typography.titleMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
    }
}
