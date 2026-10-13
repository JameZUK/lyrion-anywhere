package io.github.jamezuk.lyrionanywhere.ui.discovery

import io.github.jamezuk.lyrionanywhere.core.MenuItem
import io.github.jamezuk.lyrionanywhere.core.MenuItemKind
import io.github.jamezuk.lyrionanywhere.core.RadioPlan
import io.github.jamezuk.lyrionanywhere.core.RadioSeed
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.DiscoveryRepository
import io.github.jamezuk.lyrionanywhere.data.MenusRepository
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** "Start radio" and "More like this" for one item, as the screens show them. */
data class DiscoveryUiState(
    /** The radio there is for the item (null: none here, so the action is hidden). */
    val radio: RadioPlan? = null,
    /** "More like this": similar music (empty: none, so the shelf is hidden). */
    val similar: List<MenuItem> = emptyList(),
    /** The shelf's own title from the server, e.g. "Similar tracks". */
    val similarLabel: String? = null,
    /** Starting radio right now (the button waits). */
    val starting: Boolean = false,
) {
    val canStartRadio: Boolean get() = radio != null && !starting
}

/** What happened, for a snackbar. */
sealed interface DiscoveryEvent {
    /** Radio started; `endless`: Don't Stop The Music keeps it going. */
    data class RadioStarted(val endless: Boolean) : DiscoveryEvent

    data object Played : DiscoveryEvent

    data class Failed(val error: AppError) : DiscoveryEvent

    /** Nothing to play on: no player is chosen. */
    data object NoPlayer : DiscoveryEvent

    /** The server offers no radio for it (or nothing is playing to start from). */
    data object NoRadio : DiscoveryEvent
}

/**
 * Start radio from [seed] straight away, on [player] (car mode and the app shortcut, Wave 2
 * §5.1 and §5.5): asks what the server offers, then starts it.
 */
suspend fun startRadioFrom(discovery: DiscoveryRepository, player: String?, seed: RadioSeed?): DiscoveryEvent {
    if (player == null) return DiscoveryEvent.NoPlayer
    if (seed == null) return DiscoveryEvent.NoRadio
    val plan = when (val o = discovery.options(player, seed)) {
        is ApiResult.Ok -> o.value.radio
        is ApiResult.Err -> return DiscoveryEvent.Failed(o.error)
    } ?: return DiscoveryEvent.NoRadio
    return when (val r = discovery.startRadio(player, seed, plan)) {
        is ApiResult.Ok -> DiscoveryEvent.RadioStarted(plan.dstm != null)
        is ApiResult.Err -> DiscoveryEvent.Failed(r.error)
    }
}

/**
 * Finds what the server offers for an item (AudioMuse-AI's mixes and similar lists, Don't
 * Stop The Music) and starts it on whichever player commands go to (D20: the phone by
 * default). Asking costs one or two requests, remembered for a while by the repository.
 */
class DiscoveryController(
    private val scope: CoroutineScope,
    private val discovery: DiscoveryRepository,
    private val menus: MenusRepository,
    private val player: () -> String?,
    /** Also open "More like this" for a shelf (album and track pages). */
    private val withShelf: Boolean = false,
) {
    private val _state = MutableStateFlow(DiscoveryUiState())
    val state: StateFlow<DiscoveryUiState> = _state.asStateFlow()
    private val _events = Channel<DiscoveryEvent>(Channel.BUFFERED)
    val events: Flow<DiscoveryEvent> = _events.receiveAsFlow()
    private var seed: RadioSeed? = null
    private var loading: Job? = null

    /** Find out what there is for [item] (nothing is shown until the server has said). */
    fun load(item: RadioSeed) {
        seed = item
        loading?.cancel()
        _state.value = DiscoveryUiState()
        val p = player() ?: return
        loading = scope.launch {
            val o = (discovery.options(p, item) as? ApiResult.Ok)?.value ?: return@launch
            _state.update { it.copy(radio = o.radio, similarLabel = o.moreLikeThisLabel) }
            val similar = o.moreLikeThis
            if (withShelf && similar != null) {
                val page = (menus.open(p, similar, 0, SHELF_SIZE, null) as? ApiResult.Ok)?.value
                val shelf = page?.items.orEmpty().filter(::playable).take(SHELF_SIZE)
                _state.update { it.copy(similar = shelf) }
            }
        }
    }

    /** Start radio from the item (only if the server offered it). */
    fun startRadio() {
        val s = _state.value
        val plan = s.radio ?: return
        val item = seed ?: return
        if (s.starting) return
        val p = player()
        if (p == null) {
            _events.trySend(DiscoveryEvent.NoPlayer)
            return
        }
        _state.update { it.copy(starting = true) }
        scope.launch {
            val r = discovery.startRadio(p, item, plan)
            _state.update { it.copy(starting = false) }
            _events.trySend(
                when (r) {
                    is ApiResult.Ok -> DiscoveryEvent.RadioStarted(plan.dstm != null)
                    is ApiResult.Err -> DiscoveryEvent.Failed(r.error)
                },
            )
        }
    }

    /** Play an entry of the "More like this" shelf, as the server offered it. */
    fun play(item: MenuItem) {
        val action = item.play ?: item.go?.takeIf { !it.opensMenu && !it.needsInput } ?: return
        val p = player()
        if (p == null) {
            _events.trySend(DiscoveryEvent.NoPlayer)
            return
        }
        scope.launch {
            _events.trySend(
                when (val r = menus.run(p, action, null)) {
                    is ApiResult.Ok -> DiscoveryEvent.Played
                    is ApiResult.Err -> DiscoveryEvent.Failed(r.error)
                },
            )
        }
    }

    companion object {
        const val SHELF_SIZE = 12

        /** Shelf entries must play when chosen. */
        fun playable(item: MenuItem): Boolean =
            item.kind != MenuItemKind.TEXT && item.kind != MenuItemKind.SEARCH &&
                (item.play != null || item.go?.let { !it.opensMenu && !it.needsInput } == true)
    }
}
