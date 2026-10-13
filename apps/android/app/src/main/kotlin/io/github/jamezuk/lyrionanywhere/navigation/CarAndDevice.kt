package io.github.jamezuk.lyrionanywhere.navigation

import android.content.ActivityNotFoundException
import android.content.Intent
import android.speech.RecognizerIntent
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import io.github.jamezuk.lyrionanywhere.AppContainer
import io.github.jamezuk.lyrionanywhere.LaunchIntents
import io.github.jamezuk.lyrionanywhere.LyrionApp
import io.github.jamezuk.lyrionanywhere.ShortcutRequest
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.RadioSeed
import io.github.jamezuk.lyrionanywhere.data.CarModeSetting
import io.github.jamezuk.lyrionanywhere.data.DevicePrefs
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.car.CarAction
import io.github.jamezuk.lyrionanywhere.ui.car.CarAutoOpenEffect
import io.github.jamezuk.lyrionanywhere.ui.car.CarAutoOpener
import io.github.jamezuk.lyrionanywhere.ui.car.CarMessage
import io.github.jamezuk.lyrionanywhere.ui.car.CarModeActions
import io.github.jamezuk.lyrionanywhere.ui.car.CarModeScreen
import io.github.jamezuk.lyrionanywhere.ui.car.CarModeSettingView
import io.github.jamezuk.lyrionanywhere.ui.car.CarModeViewModel
import io.github.jamezuk.lyrionanywhere.ui.car.rememberCarBluetoothAccess
import io.github.jamezuk.lyrionanywhere.ui.components.HapticsSetting
import io.github.jamezuk.lyrionanywhere.ui.components.errorText
import io.github.jamezuk.lyrionanywhere.ui.discovery.DiscoveryEvent
import io.github.jamezuk.lyrionanywhere.ui.discovery.startRadioFrom
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.SleepChoice
import io.github.jamezuk.lyrionanywhere.ui.settings.DynamicColourSetting
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/** Opening and leaving car mode by hand, keeping its automatic opening in step. */
class CarModeHandle internal constructor(
    private val opener: CarAutoOpener,
    private val setting: () -> CarModeSetting,
    private val open: () -> Unit,
    private val leave: () -> Unit,
) {
    fun openByHand() {
        opener.openedByHand()
        open()
    }

    fun leaveByHand() {
        opener.leftByHand(setting())
        leave()
    }
}

/**
 * Car mode's automatic opening (Wave 2 §5.1), while the app is in front: for a matching car
 * it opens ([onOpen]); when the car that opened it disconnects, it leaves ([onLeave]).
 * Without the Bluetooth permission (revoked since), the setting goes back to Off.
 */
@Composable
fun rememberCarMode(
    container: AppContainer,
    scope: CoroutineScope,
    inCar: Boolean,
    onOpen: () -> Unit,
    onLeave: () -> Unit,
): CarModeHandle {
    val prefs = remember(container) { container.settings.settings.map { it.device } }
    val device by prefs.collectAsStateWithLifecycle(DevicePrefs())
    val opener = remember { CarAutoOpener() }
    val showing by rememberUpdatedState(inCar)
    val open by rememberUpdatedState(onOpen)
    val leave by rememberUpdatedState(onLeave)
    val setting by rememberUpdatedState(device.car)
    CarAutoOpenEffect(
        device.car,
        opener,
        onAction = { a ->
            when (a) {
                CarAction.OPEN -> if (!showing) open()
                CarAction.LEAVE -> if (showing) leave()
                CarAction.NONE -> {}
            }
        },
        onPermissionLost = {
            scope.launch {
                val now = container.settings.settings.first().device
                container.settings.setDevicePrefs(now.copy(car = CarModeSetting()))
            }
        },
    )
    // Left some other way (a notification, the system's back): in step with that.
    LaunchedEffect(inCar) { if (!inCar && opener.open) opener.leftByHand(setting) }
    return remember(opener) { CarModeHandle(opener, { setting }, { open() }, { leave() }) }
}

/** The car-mode route: its lists, voice search and messages around [CarModeScreen]. */
@Composable
fun CarModeContent(
    container: AppContainer,
    player: PlayerViewModel,
    playerState: PlayerUiState,
    clock: () -> Long,
    snackbar: SnackbarHostState,
    onExit: () -> Unit,
    discoveryText: (DiscoveryEvent) -> String,
) {
    val vm = viewModel { CarModeViewModel(container.library, container.me, container.discovery, player::target) }
    val lists by vm.state.collectAsStateWithLifecycle()
    val res = LocalResources.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    LaunchedEffect(vm) { vm.load() }
    LaunchedEffect(vm) {
        vm.messages.collect { m ->
            snackbar.showSnackbar(
                when (m) {
                    is CarMessage.StationStarted -> res.getString(R.string.station_started, m.name)
                    is CarMessage.Radio -> discoveryText(m.event)
                    is CarMessage.Failed -> errorText(res, m.error)
                    CarMessage.NoPlayer -> res.getString(R.string.no_player_selected)
                },
            )
        }
    }
    // Voice search: what is said goes through the same search as "Play … on Lyrion Anywhere"
    // (only the text, bounded), and plays on this phone.
    val voice = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { r ->
        val heard = runCatching { r.data?.getStringArrayListExtra(RecognizerIntent.EXTRA_RESULTS)?.firstOrNull() }.getOrNull()
        LaunchIntents.searchQuery(heard)?.let { q -> (context.applicationContext as? LyrionApp)?.playFromSearch(q) }
    }
    CarModeScreen(
        playerState,
        lists,
        clock,
        CarModeActions(
            onExit = onExit,
            onTogglePlay = player::togglePlay,
            onStop = player::stopPlayback,
            onNext = player::next,
            onPrevious = player::previous,
            onStartRadio = { vm.startRadio(playerState.status?.current?.id) },
            onVoice = {
                val ask = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
                    .putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
                    .putExtra(RecognizerIntent.EXTRA_PROMPT, res.getString(R.string.car_voice))
                try {
                    voice.launch(ask)
                } catch (_: ActivityNotFoundException) {
                    scope.launch { snackbar.showSnackbar(res.getString(R.string.voice_unavailable)) }
                }
            },
            onFavourite = { player.playFavourite(it.id, Placement.PLAY_NOW) },
            onStation = vm::startStation,
        ),
    )
}

/** Settings' "This device": haptics, wallpaper colours and car mode (Wave 2 §5). */
@Composable
fun DeviceSettings(container: AppContainer, scope: CoroutineScope) {
    val prefs = remember(container) { container.settings.settings.map { it.device } }
    val device by prefs.collectAsStateWithLifecycle(DevicePrefs())
    fun update(change: (DevicePrefs) -> DevicePrefs) {
        scope.launch { container.settings.setDevicePrefs(change(container.settings.settings.first().device)) }
    }
    Column {
        HapticsSetting(device.haptics, onChange = { on -> update { it.copy(haptics = on) } })
        DynamicColourSetting(device.dynamicColour, onChange = { on -> update { it.copy(dynamicColour = on) } })
        CarModeSettingView(device.car, onChange = { c -> update { it.copy(car = c) } }, access = rememberCarBluetoothAccess())
    }
}

/**
 * The launcher's shortcuts (Wave 2 §5.5), through the player controller: "Start radio" from
 * the current track, and "Sleep 30 min" on the controlled player.
 */
@Composable
fun ShortcutRequests(
    container: AppContainer,
    player: PlayerViewModel,
    snackbar: SnackbarHostState,
    discoveryText: (DiscoveryEvent) -> String,
) {
    val pending by container.shortcuts.pending.collectAsStateWithLifecycle()
    val res = LocalResources.current
    LaunchedEffect(pending) {
        if (pending == null) return@LaunchedEffect
        val request = container.shortcuts.take() ?: return@LaunchedEffect
        // Give the controller a moment to learn the player and what it plays.
        val s = withTimeoutOrNull(SHORTCUT_WAIT_MS) { player.state.first { it.selectedId != null && it.status != null } }
        when (request) {
            ShortcutRequest.START_RADIO -> {
                val seed = s?.status?.current?.id?.takeIf { it > 0 }?.let { RadioSeed.Track(it) }
                snackbar.showSnackbar(discoveryText(startRadioFrom(container.discovery, s?.selectedId, seed)))
            }
            ShortcutRequest.SLEEP_30 ->
                if (s == null) snackbar.showSnackbar(res.getString(R.string.no_player_selected)) else player.setSleep(SleepChoice.Minutes(30))
        }
    }
}

private const val SHORTCUT_WAIT_MS = 8_000L
