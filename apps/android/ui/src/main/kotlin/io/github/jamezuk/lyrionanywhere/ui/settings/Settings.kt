package io.github.jamezuk.lyrionanywhere.ui.settings

import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Logout
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material.icons.outlined.DirectionsCar
import androidx.compose.material.icons.outlined.LockOpen
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material.icons.outlined.Storage
import androidx.compose.material.icons.outlined.Gavel
import androidx.compose.material.icons.outlined.GraphicEq
import androidx.compose.material.icons.outlined.Password
import androidx.compose.material.icons.outlined.Person
import androidx.compose.material.icons.outlined.PrivacyTip
import androidx.compose.material.icons.outlined.Security
import androidx.compose.material.icons.outlined.Smartphone
import androidx.compose.material.icons.outlined.PlayCircle
import androidx.compose.material.icons.outlined.Album
import androidx.compose.material.icons.outlined.History
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.Tune
import io.github.jamezuk.lyrionanywhere.ui.components.NavRow
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.core.AccountInfo
import io.github.jamezuk.lyrionanywhere.core.MyDevice
import io.github.jamezuk.lyrionanywhere.data.AccountRepository
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.DevicesMessage
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.PasswordChangeError
import io.github.jamezuk.lyrionanywhere.data.PasswordChangeResult
import io.github.jamezuk.lyrionanywhere.data.SoundSettings
import io.github.jamezuk.lyrionanywhere.data.offline.OfflineRepository
import io.github.jamezuk.lyrionanywhere.ui.offline.formatBytes
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.errorMessage
import io.github.jamezuk.lyrionanywhere.ui.library.SectionHeader
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** The privacy policy: what the app and the server keep, and what they don't. */
const val PRIVACY_POLICY_URL = "https://github.com/JameZUK/lyrion-anywhere/blob/main/docs/PRIVACY.md"

data class SettingsUiState(
    val serverName: String = "",
    /** The server is reached over https (false: plain http:// to a private address, or loopback). */
    val serverSecure: Boolean = true,
    val username: String = "",
    val account: AccountInfo? = null,
    val signingOut: Boolean = false,
    /** This phone's player's Sound settings (null: no local player here). */
    val sound: SoundSettings? = null,
    /** Downloads are offered here. */
    val downloadsOffered: Boolean = false,
    /** Bytes of downloads on this device (for the sign-out question). */
    val downloadedBytes: Long = 0,
    val downloadedTracks: Int = 0,
    /** This phone's name: its player's name in LMS. */
    val deviceName: String = "",
    val renaming: Boolean = false,
    /** Why the last rename failed, if it did. */
    val renameError: AppError? = null,
    /** A password change is being sent. */
    val passwordChanging: Boolean = false,
    /** Why the last password change failed, if it did. */
    val passwordError: PasswordChangeError? = null,
    /** The password was changed (shown until the form is closed). */
    val passwordChanged: PasswordChangeResult.Changed? = null,
    /** Your devices: the account's signed-in devices (null until loaded). */
    val devices: List<MyDevice>? = null,
    /** A device is being signed out. */
    val devicesBusy: Boolean = false,
    /** What the last Your devices action came to (one message at a time). */
    val devicesMessage: DevicesMessage? = null,
)

/** A name the server takes for this phone (it cleans it further), trimmed; or null. */
fun validDeviceName(name: String): String? {
    val n = name.trim()
    val count = n.codePointCount(0, n.length)
    return n.takeIf { count in 1..64 && n.none { it.isISOControl() } }
}

class SettingsViewModel(
    private val account: AccountRepository,
    private val localPlayer: LocalPlayerRepository? = null,
    private val offline: OfflineRepository? = null,
) : ViewModel() {
    private val _state = MutableStateFlow(SettingsUiState())
    val state: StateFlow<SettingsUiState> = _state.asStateFlow()

    init {
        (account.state.value as? AuthState.SignedInState)?.let { s ->
            _state.update {
                it.copy(
                    serverName = s.server.name,
                    serverSecure = s.server.secure,
                    username = s.who.user.username,
                    deviceName = s.who.device.name,
                )
            }
        }
        viewModelScope.launch {
            (account.account() as? ApiResult.Ok)?.let { r -> _state.update { it.copy(account = r.value) } }
        }
        localPlayer?.let { lp ->
            viewModelScope.launch {
                lp.state.collect { local -> _state.update { it.copy(sound = local.sound) } }
            }
        }
        offline?.let { o ->
            _state.update { it.copy(downloadsOffered = true) }
            viewModelScope.launch {
                o.state.collect { s ->
                    _state.update { it.copy(downloadedBytes = s.usedBytes, downloadedTracks = s.tracks.size) }
                }
            }
        }
    }

    /** Rename this phone; its LMS player follows. */
    fun rename(name: String) {
        val n = validDeviceName(name) ?: return
        if (_state.value.renaming) return
        _state.update { it.copy(renaming = true, renameError = null) }
        viewModelScope.launch {
            when (val r = account.renameDevice(n)) {
                is ApiResult.Ok -> _state.update { it.copy(renaming = false, deviceName = r.value) }
                is ApiResult.Err -> _state.update { it.copy(renaming = false, renameError = r.error) }
            }
        }
    }

    /** Change the password; the outcome shows in the form ([SettingsUiState.passwordChanged]). */
    fun changePassword(current: String, new: String, code: String?, signOutOthers: Boolean) {
        if (_state.value.passwordChanging) return
        _state.update { it.copy(passwordChanging = true, passwordError = null, passwordChanged = null) }
        viewModelScope.launch {
            when (val r = account.changePassword(current, new, code, signOutOthers)) {
                is PasswordChangeResult.Changed -> _state.update { it.copy(passwordChanging = false, passwordChanged = r) }
                is PasswordChangeResult.Failed -> _state.update { it.copy(passwordChanging = false, passwordError = r.error) }
            }
        }
    }

    /**
     * Your devices: (re)load the list. A list that loads clears an earlier failure (an
     * action's outcome stays); one that doesn't is said, with Try again on an empty page.
     */
    fun loadDevices() {
        viewModelScope.launch {
            when (val r = account.myDevices()) {
                is ApiResult.Ok -> _state.update {
                    it.copy(devices = r.value, devicesMessage = it.devicesMessage.takeUnless { m -> m is DevicesMessage.Failed })
                }
                is ApiResult.Err -> _state.update { it.copy(devicesMessage = DevicesMessage.Failed(r.error)) }
            }
        }
    }

    /**
     * After an action: read the list again, still busy until it is in (so a second action
     * can't start from a stale list and the two lists can't land out of order). The action's
     * message stays whatever the reload comes to; if the reload fails, what the action
     * removed ([gone]) is taken off the list here instead, so no revoked device keeps its
     * Sign out button.
     */
    private suspend fun reloadAfter(message: DevicesMessage, gone: (MyDevice) -> Boolean) {
        val fresh = (account.myDevices() as? ApiResult.Ok)?.value
        _state.update {
            it.copy(
                devices = fresh ?: if (message is DevicesMessage.Failed) it.devices else it.devices?.filterNot(gone),
                devicesMessage = message,
                devicesBusy = false,
            )
        }
    }

    /** Sign out another device (the page has asked first); the list is read again after. */
    fun signOutDevice(device: MyDevice) {
        if (device.current || _state.value.devicesBusy) return
        _state.update { it.copy(devicesBusy = true, devicesMessage = null) }
        viewModelScope.launch {
            val message = when (val r = account.signOutDevice(device.id)) {
                is ApiResult.Ok -> if (r.value) DevicesMessage.SignedOut(device.name) else DevicesMessage.AlreadyGone
                is ApiResult.Err -> DevicesMessage.Failed(r.error)
            }
            reloadAfter(message) { it.id == device.id }
        }
    }

    /** Sign out every other device (the page has asked first). */
    fun signOutOtherDevices() {
        if (_state.value.devicesBusy) return
        _state.update { it.copy(devicesBusy = true, devicesMessage = null) }
        viewModelScope.launch {
            val message = when (val r = account.signOutOtherDevices()) {
                is ApiResult.Ok -> DevicesMessage.SignedOutOthers(r.value)
                is ApiResult.Err -> DevicesMessage.Failed(r.error)
            }
            reloadAfter(message) { !it.current }
        }
    }

    /** The password form was closed: forget its outcome. */
    fun closePasswordForm() {
        _state.update { it.copy(passwordError = null, passwordChanged = null) }
    }

    /**
     * Sign out. Downloads are kept unless [removeDownloads] (as on the web, keeping is the
     * default); either way this account's downloads are only shown again after signing in.
     */
    fun signOut(removeDownloads: Boolean = false) {
        if (_state.value.signingOut) return
        _state.update { it.copy(signingOut = true) }
        viewModelScope.launch {
            offline?.signOut(removeDownloads)
            account.signOut()
            _state.update { it.copy(signingOut = false) }
        }
    }
}

/** An open-source component and its licence, for the licences page. */
data class Licence(val name: String, val licence: String)

/** The main components this app ships (the Rust crates and Android libraries it bundles). */
val LICENCES = listOf(
    Licence("Lyrion Anywhere", "GPL-2.0-or-later"),
    Licence("UniFFI", "MPL-2.0"),
    Licence("rustls, rustls-platform-verifier", "Apache-2.0 / MIT / ISC"),
    Licence("ring", "Apache-2.0 AND ISC"),
    Licence("tokio, hyper, tungstenite, serde", "MIT / Apache-2.0"),
    Licence("Symphonia (audio decoding)", "MPL-2.0"),
    Licence("opus-decoder", "MIT / Apache-2.0"),
    Licence("rubato (resampling)", "MIT / Apache-2.0"),
    Licence("ndk (AAudio bindings)", "MIT / Apache-2.0"),
    Licence("Jetpack Compose, AndroidX, CameraX, Media3", "Apache-2.0"),
    Licence("Kotlin, kotlinx.coroutines", "Apache-2.0"),
    Licence("Coil", "Apache-2.0"),
    Licence("ZXing", "Apache-2.0"),
    Licence("JNA", "Apache-2.0 / LGPL-2.1-or-later"),
    Licence("Material Symbols", "Apache-2.0"),
)

/**
 * The pages of Settings (Wave 3.5 rule 5): the index lists them, each with a one-line summary
 * of what's set. Engine and diagnostic options are under About › Advanced.
 */
enum class SettingsSection(val tag: String) {
    ACCOUNT("settings-account"),
    PLAYBACK("settings-playback"),
    NOW_PLAYING("settings-now-playing"),
    DOWNLOADS("settings-downloads"),
    HISTORY("settings-history"),
    DEVICE("settings-device"),
    ABOUT("settings-about"),
    ADVANCED("settings-advanced"),

    /** Account › Your devices. */
    YOUR_DEVICES("settings-your-devices"),
}

/** A section's name: the index row's title and its page's title. */
@Composable
fun settingsSectionTitle(s: SettingsSection): String = stringResource(
    when (s) {
        SettingsSection.ACCOUNT -> R.string.section_account
        SettingsSection.PLAYBACK -> R.string.settings_playback
        SettingsSection.NOW_PLAYING -> R.string.now_playing
        SettingsSection.DOWNLOADS -> R.string.title_downloads
        SettingsSection.HISTORY -> R.string.section_history
        SettingsSection.DEVICE -> R.string.section_this_device
        SettingsSection.ABOUT -> R.string.section_about
        SettingsSection.ADVANCED -> R.string.settings_advanced
        SettingsSection.YOUR_DEVICES -> R.string.your_devices
    },
)

/** The index's one-line summaries that need the app's state (null: none shown). */
data class SettingsSummaries(
    val playback: String? = null,
    val nowPlaying: String? = null,
    val history: String? = null,
)

/** About's line for the server's player protocol, with the plugin versions it means. */
@Composable
internal fun serverProtocolText(protocol: UInt?): String = when {
    protocol == null -> stringResource(R.string.about_server_protocol_none)
    protocol >= io.github.jamezuk.lyrionanywhere.data.MIN_PLAYER_PROTOCOL -> stringResource(R.string.about_server_protocol, protocol.toInt())
    else -> stringResource(R.string.about_server_protocol_old, protocol.toInt())
}

/**
 * Settings: with no [section], the index (Account, Playback, Now playing, Downloads,
 * Listening history, This device, About), each row opening its page through [onSection];
 * with one, that page. A page's rows come from the slots below, so nothing moved is lost.
 */
@Composable
fun SettingsScreen(
    state: SettingsUiState,
    appVersion: String,
    libraryVersion: String,
    onSignOut: (removeDownloads: Boolean) -> Unit,
    onLicences: () -> Unit,
    modifier: Modifier = Modifier,
    /** The page shown; null for the index. */
    section: SettingsSection? = null,
    onSection: (SettingsSection) -> Unit = {},
    summaries: SettingsSummaries = SettingsSummaries(),
    onSound: () -> Unit = {},
    onDownloads: () -> Unit = {},
    onStorage: () -> Unit = {},
    /** The default-player choice (D20), when the app has players to offer. */
    defaultPlayer: (@Composable () -> Unit)? = null,
    onRename: (String) -> Unit = {},
    /** Listening history: record or not, view, clear (docs/plans/wave1-design.md §8). */
    history: (@Composable () -> Unit)? = null,
    /** The sleep timer's fade-out setting (Wave 1), when the app has players. */
    sleepFade: (@Composable () -> Unit)? = null,
    /** The lyrics peek setting (Wave 1), when the app has players. */
    lyrics: (@Composable () -> Unit)? = null,
    /** The stream level on Wi-Fi and mobile data, when the app has players. */
    streaming: (@Composable () -> Unit)? = null,
    /** Ratings: stars or a heart, and clearing them (Wave 2). */
    ratings: (@Composable () -> Unit)? = null,
    /** The visualiser on Now playing and its style (Wave 2 part 3), when the app has players. */
    visualiser: (@Composable () -> Unit)? = null,
    /** This device: haptics, the theme's colours and car mode (Wave 2 §5). */
    device: (@Composable () -> Unit)? = null,
    /** Gaps in the signal: reading the next track ahead (rolling cache), with this phone's player. */
    gaps: (@Composable () -> Unit)? = null,
    /** The one-time offer to let the player run unrestricted (top of Settings; null: none). */
    backgroundPrompt: (@Composable () -> Unit)? = null,
    /** The same option in Settings › Playback (null: not needed). */
    background: (@Composable () -> Unit)? = null,
    /** Change the password: current, new, the two-step code (null without two-step), sign out others. */
    onChangePassword: (current: String, new: String, code: String?, signOutOthers: Boolean) -> Unit = { _, _, _, _ -> },
    onClosePasswordForm: () -> Unit = {},
    /** Your devices: read the list, sign one out, sign out all the others. */
    onLoadDevices: () -> Unit = {},
    onSignOutDevice: (MyDevice) -> Unit = {},
    onSignOutOtherDevices: () -> Unit = {},
    /** The clock, for "Last used 3 hours ago" (Unix seconds). */
    nowSeconds: () -> Long = { System.currentTimeMillis() / 1000 },
    /**
     * About › Car connections: the last callers the media session decided on, one line each,
     * newest first (empty: none yet; null: the row isn't shown).
     */
    carConnections: List<String>? = null,
    /** About: the server's player protocol (null: not given), once read ([serverProtocolChecked]). */
    serverProtocol: UInt? = null,
    serverProtocolChecked: Boolean = false,
) {
    var confirm by rememberSaveable { mutableStateOf(false) }
    var renaming by rememberSaveable { mutableStateOf(false) }
    var changingPassword by rememberSaveable { mutableStateOf(false) }
    // Your devices: the one device (by id) or all the others (-1) waiting for a yes.
    var confirmSignOut by rememberSaveable { mutableStateOf<Long?>(null) }
    // Known not to be offered by this server (null, not yet known, is not a refusal).
    val devicesRefused = state.account?.canManageDevices == false
    val playback = defaultPlayer != null || streaming != null || sleepFade != null || state.sound != null
    val nowPlaying = lyrics != null || visualiser != null || ratings != null
    val thisDevice = device != null || state.deviceName.isNotEmpty()
    LazyColumn(modifier.fillMaxSize().testTag(section?.tag ?: "settings-index")) {
        when (section) {
            null -> {
                if (backgroundPrompt != null) item { backgroundPrompt() }
                item {
                    NavRow(
                        settingsSectionTitle(SettingsSection.ACCOUNT),
                        listOf(state.username, state.serverName).filter { it.isNotEmpty() }.joinToString(" · "),
                        { onSection(SettingsSection.ACCOUNT) },
                        icon = Icons.Outlined.Person,
                        tag = SettingsSection.ACCOUNT.tag,
                    )
                }
                if (playback) {
                    item {
                        NavRow(settingsSectionTitle(SettingsSection.PLAYBACK), summaries.playback, { onSection(SettingsSection.PLAYBACK) }, icon = Icons.Outlined.PlayCircle, tag = SettingsSection.PLAYBACK.tag)
                    }
                }
                if (nowPlaying) {
                    item {
                        NavRow(settingsSectionTitle(SettingsSection.NOW_PLAYING), summaries.nowPlaying, { onSection(SettingsSection.NOW_PLAYING) }, icon = Icons.Outlined.Album, tag = SettingsSection.NOW_PLAYING.tag)
                    }
                }
                if (state.downloadsOffered) {
                    item {
                        NavRow(
                            settingsSectionTitle(SettingsSection.DOWNLOADS),
                            downloadsSummary(state),
                            { onSection(SettingsSection.DOWNLOADS) },
                            icon = Icons.Outlined.Download,
                            tag = SettingsSection.DOWNLOADS.tag,
                        )
                    }
                }
                if (history != null) {
                    item {
                        NavRow(settingsSectionTitle(SettingsSection.HISTORY), summaries.history, { onSection(SettingsSection.HISTORY) }, icon = Icons.Outlined.History, tag = SettingsSection.HISTORY.tag)
                    }
                }
                if (thisDevice) {
                    item {
                        NavRow(settingsSectionTitle(SettingsSection.DEVICE), state.deviceName.ifEmpty { null }, { onSection(SettingsSection.DEVICE) }, icon = Icons.Outlined.Smartphone, tag = SettingsSection.DEVICE.tag)
                    }
                }
                item {
                    NavRow(
                        settingsSectionTitle(SettingsSection.ABOUT),
                        stringResource(R.string.settings_about_summary, appVersion),
                        { onSection(SettingsSection.ABOUT) },
                        icon = Icons.Outlined.Info,
                        tag = SettingsSection.ABOUT.tag,
                    )
                }
            }
            SettingsSection.ACCOUNT -> {
                item {
                    ListItem(
                        leadingContent = { Icon(Icons.Outlined.Person, contentDescription = null) },
                        headlineContent = { Text(state.username) },
                        supportingContent = {
                            Text(stringResource(if (state.account?.isAdmin == true) R.string.role_admin else R.string.role_listener))
                        },
                    )
                }
                item {
                    ListItem(
                        leadingContent = { Icon(Icons.Outlined.Dns, contentDescription = null) },
                        headlineContent = { Text(stringResource(R.string.server)) },
                        supportingContent = {
                            if (state.serverSecure) {
                                Text(state.serverName)
                            } else {
                                // Plain http:// (docs/plans/trusted-networks.md): said beside the
                                // server for as long as this sign-in lasts.
                                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                        Text(state.serverName)
                                        UnencryptedBadge()
                                    }
                                    Text(
                                        stringResource(R.string.unencrypted_explained),
                                        style = MaterialTheme.typography.bodySmall,
                                        modifier = Modifier.testTag("server-unencrypted-explained"),
                                    )
                                }
                            }
                        },
                    )
                }
                state.account?.let { a ->
                    item {
                        ListItem(
                            leadingContent = { Icon(Icons.Outlined.Security, contentDescription = null) },
                            headlineContent = { Text(stringResource(R.string.two_step_verification)) },
                            supportingContent = {
                                Text(stringResource(if (a.twoStepEnabled) R.string.two_step_on else R.string.two_step_off))
                            },
                        )
                    }
                    // Only where the server offers it (older gateways don't).
                    if (a.canChangePassword) {
                        item {
                            ListItem(
                                leadingContent = { Icon(Icons.Outlined.Password, contentDescription = null) },
                                headlineContent = { Text(stringResource(R.string.password_change)) },
                                supportingContent = { Text(stringResource(R.string.password_change_row)) },
                                modifier = Modifier
                                    .testTag("change-password")
                                    .clickable(enabled = !state.passwordChanging) { changingPassword = true },
                            )
                        }
                        if (changingPassword) {
                            item {
                                ChangePasswordEditor(
                                    twoStep = a.twoStepEnabled,
                                    busy = state.passwordChanging,
                                    error = state.passwordError,
                                    changed = state.passwordChanged,
                                    onSubmit = onChangePassword,
                                    onDismiss = {
                                        changingPassword = false
                                        onClosePasswordForm()
                                    },
                                )
                            }
                        }
                    }
                    // Only where the server offers it (older gateways don't).
                    if (a.canManageDevices) {
                        item {
                            NavRow(
                                settingsSectionTitle(SettingsSection.YOUR_DEVICES),
                                stringResource(R.string.your_devices_row),
                                { onSection(SettingsSection.YOUR_DEVICES) },
                                icon = Icons.Outlined.Devices,
                                tag = SettingsSection.YOUR_DEVICES.tag,
                            )
                        }
                    }
                }
                item {
                    ListItem(
                        leadingContent = { Icon(Icons.AutoMirrored.Outlined.Logout, contentDescription = null) },
                        headlineContent = { Text(stringResource(R.string.action_sign_out)) },
                        supportingContent = { Text(stringResource(R.string.sign_out_explained)) },
                        modifier = Modifier.clickable(enabled = !state.signingOut) { confirm = true },
                    )
                }
            }
            SettingsSection.PLAYBACK -> {
                if (defaultPlayer != null) item { defaultPlayer() }
                if (streaming != null) item { streaming() }
                if (background != null) item { background() }
                if (sleepFade != null) item { sleepFade() }
                state.sound?.let { sound ->
                    item {
                        NavRow(
                            stringResource(R.string.title_sound),
                            if (sound.bitPerfect) {
                                stringResource(R.string.sound_row_bit_perfect)
                            } else {
                                stringResource(R.string.sound_row_processing, stageNames(sound.activeStages))
                            },
                            onSound,
                            icon = Icons.Outlined.GraphicEq,
                            tag = "settings-sound",
                        )
                    }
                }
            }
            SettingsSection.NOW_PLAYING -> {
                if (lyrics != null) item { lyrics() }
                if (visualiser != null) item { visualiser() }
                if (ratings != null) {
                    item { HorizontalDivider() }
                    item { ratings() }
                }
            }
            SettingsSection.DOWNLOADS -> {
                item {
                    NavRow(
                        stringResource(R.string.title_downloads),
                        stringResource(R.string.storage_used, formatBytes(state.downloadedBytes)),
                        onDownloads,
                        icon = Icons.Outlined.Download,
                        tag = "settings-downloads-manager",
                    )
                }
                item {
                    NavRow(stringResource(R.string.title_storage), stringResource(R.string.storage_row), onStorage, icon = Icons.Outlined.Storage, tag = "settings-download-options")
                }
            }
            SettingsSection.HISTORY -> if (history != null) item { history() }
            SettingsSection.DEVICE -> {
                if (state.deviceName.isNotEmpty()) {
                    item {
                        ListItem(
                            leadingContent = { Icon(Icons.Outlined.Smartphone, contentDescription = null) },
                            headlineContent = { Text(stringResource(R.string.this_phone_name)) },
                            supportingContent = {
                                val error = state.renameError?.let { errorMessage(it) }
                                Text(
                                    error ?: stringResource(R.string.this_phone_name_explained, state.deviceName),
                                    // A failed rename is announced as it appears.
                                    modifier = if (error != null) {
                                        Modifier.semantics { liveRegion = LiveRegionMode.Polite }
                                    } else {
                                        Modifier
                                    },
                                )
                            },
                            trailingContent = { Text(stringResource(R.string.action_rename)) },
                            modifier = Modifier.clickable(enabled = !state.renaming) { renaming = !renaming },
                        )
                    }
                    if (renaming) {
                        item {
                            RenameDeviceEditor(
                                current = state.deviceName,
                                onRename = { name ->
                                    renaming = false
                                    onRename(name)
                                },
                                onDismiss = { renaming = false },
                            )
                        }
                    }
                }
                if (device != null) item { device() }
            }
            SettingsSection.ABOUT -> {
                item {
                    ListItem(
                        leadingContent = {
                            Image(painterResource(R.drawable.logo), contentDescription = null, modifier = Modifier.size(40.dp))
                        },
                        headlineContent = { Text(stringResource(R.string.app_name_full)) },
                        supportingContent = { Text(stringResource(R.string.version_line, appVersion, libraryVersion)) },
                    )
                }
                if (serverProtocolChecked) {
                    item {
                        ListItem(
                            leadingContent = { Icon(Icons.Outlined.Dns, contentDescription = null) },
                            headlineContent = { Text(serverProtocolText(serverProtocol)) },
                            modifier = Modifier.testTag("about-server-protocol"),
                        )
                    }
                }
                item {
                    ListItem(
                        leadingContent = { Icon(Icons.Outlined.Gavel, contentDescription = null) },
                        headlineContent = { Text(stringResource(R.string.licences)) },
                        modifier = Modifier.clickable(onClick = onLicences),
                    )
                }
                item {
                    val uriHandler = LocalUriHandler.current
                    ListItem(
                        leadingContent = { Icon(Icons.Outlined.PrivacyTip, contentDescription = null) },
                        headlineContent = { Text(stringResource(R.string.privacy_policy)) },
                        supportingContent = { Text(stringResource(R.string.privacy_policy_body)) },
                        modifier = Modifier
                            .testTag("privacy-policy")
                            .clickable {
                                // No browser (or a locked-down profile) must not crash the app.
                                runCatching { uriHandler.openUri(PRIVACY_POLICY_URL) }
                            },
                    )
                }
                if (carConnections != null) {
                    item {
                        ListItem(
                            leadingContent = { Icon(Icons.Outlined.DirectionsCar, contentDescription = null) },
                            headlineContent = { Text(stringResource(R.string.car_connections)) },
                            supportingContent = {
                                Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                                    if (carConnections.isEmpty()) {
                                        Text(stringResource(R.string.car_connections_none))
                                    } else {
                                        carConnections.forEach { Text(it) }
                                    }
                                    Text(
                                        stringResource(R.string.car_connections_explained),
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                            },
                            modifier = Modifier.testTag("car-connections"),
                        )
                    }
                }
                if (gaps != null) {
                    item {
                        NavRow(
                            settingsSectionTitle(SettingsSection.ADVANCED),
                            stringResource(R.string.settings_advanced_summary),
                            { onSection(SettingsSection.ADVANCED) },
                            icon = Icons.Outlined.Tune,
                            tag = SettingsSection.ADVANCED.tag,
                        )
                    }
                }
            }
            SettingsSection.ADVANCED -> if (gaps != null) item { gaps() }
            // A page restored onto a server that doesn't offer it (an older gateway): said
            // plainly, and nothing is asked of the server.
            SettingsSection.YOUR_DEVICES -> if (devicesRefused) {
                item {
                    Text(
                        stringResource(R.string.your_devices_not_offered),
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.padding(16.dp).testTag("your-devices-not-offered"),
                    )
                }
            } else {
                yourDevicesItems(
                    devices = state.devices,
                    busy = state.devicesBusy,
                    message = state.devicesMessage,
                    nowSeconds = nowSeconds,
                    onSignOut = { confirmSignOut = it.id },
                    onSignOutOthers = { confirmSignOut = ALL_OTHERS },
                    onRetry = onLoadDevices,
                )
            }
        }
    }
    if (section == SettingsSection.YOUR_DEVICES && !devicesRefused) {
        LaunchedEffect(Unit) { onLoadDevices() }
    }
    confirmSignOut?.let { target ->
        val others = state.devices.orEmpty().filterNot { it.current }
        val device = others.firstOrNull { it.id == target }
        if (target != ALL_OTHERS && device == null) {
            // Gone from the list meanwhile: nothing to ask about.
            LaunchedEffect(target) { confirmSignOut = null }
        } else {
            SignOutDeviceDialog(
                device = device,
                others = others.size,
                onConfirm = {
                    confirmSignOut = null
                    if (device != null) onSignOutDevice(device) else onSignOutOtherDevices()
                },
                onDismiss = { confirmSignOut = null },
            )
        }
    }
    if (confirm) {
        SignOutDialog(
            downloadedTracks = state.downloadedTracks,
            downloadedBytes = state.downloadedBytes,
            onSignOut = { remove ->
                confirm = false
                onSignOut(remove)
            },
            onDismiss = { confirm = false },
        )
    }
}

/** "Unencrypted", beside a server reached over plain http://. */
@Composable
fun UnencryptedBadge(modifier: Modifier = Modifier) {
    Surface(
        color = MaterialTheme.colorScheme.errorContainer,
        contentColor = MaterialTheme.colorScheme.onErrorContainer,
        shape = MaterialTheme.shapes.small,
        modifier = modifier.testTag("server-unencrypted"),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp)) {
            Icon(Icons.Outlined.LockOpen, contentDescription = null, modifier = Modifier.size(14.dp))
            Text(
                stringResource(R.string.unencrypted_badge),
                style = MaterialTheme.typography.labelSmall,
                modifier = Modifier.padding(start = 4.dp),
            )
        }
    }
}

/** [SettingsScreen]'s confirm target meaning every other device. */
private const val ALL_OTHERS = -1L

/** "1.18 GB · 34 tracks" (the Downloads row of the index). */
@Composable
private fun downloadsSummary(state: SettingsUiState): String = listOf(
    formatBytes(state.downloadedBytes),
    androidx.compose.ui.platform.LocalResources.current.getQuantityString(R.plurals.track_count, state.downloadedTracks, state.downloadedTracks),
).joinToString(" · ")

@Composable
fun LicencesScreen(modifier: Modifier = Modifier) {
    LazyColumn(modifier.fillMaxSize()) {
        item {
            Text(
                stringResource(R.string.licences_intro),
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(16.dp),
            )
        }
        items(LICENCES) { l ->
            ListItem(headlineContent = { Text(l.name) }, supportingContent = { Text(l.licence) })
        }
    }
}

/**
 * "Sign out?" With downloads on the device it asks, as the web app does, whether to keep
 * them (the default) or remove them; kept downloads show again only after signing in.
 */
@Composable
fun SignOutDialog(
    downloadedTracks: Int,
    downloadedBytes: Long,
    onSignOut: (removeDownloads: Boolean) -> Unit,
    onDismiss: () -> Unit,
) {
    val hasDownloads = downloadedTracks > 0
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.sign_out_confirm_title)) },
        text = {
            // Scrolls when large text makes it taller than the dialog.
            Text(
                if (hasDownloads) {
                    stringResource(R.string.sign_out_downloads_body, formatBytes(downloadedBytes))
                } else {
                    stringResource(R.string.sign_out_confirm_body)
                },
                modifier = Modifier.verticalScroll(rememberScrollState()),
            )
        },
        confirmButton = {
            if (hasDownloads) {
                androidx.compose.foundation.layout.Column(horizontalAlignment = androidx.compose.ui.Alignment.End) {
                    TextButton(onClick = { onSignOut(false) }) { Text(stringResource(R.string.sign_out_keep_downloads)) }
                    TextButton(onClick = { onSignOut(true) }) { Text(stringResource(R.string.sign_out_remove_downloads)) }
                    TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) }
                }
            } else {
                TextButton(onClick = { onSignOut(false) }) { Text(stringResource(R.string.action_sign_out)) }
            }
        },
        dismissButton = if (hasDownloads) {
            null
        } else {
            { TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) } }
        },
    )
}

/** Rename this phone, in place in the list: a name of 1 to 64 characters. */
@Composable
fun RenameDeviceEditor(current: String, onRename: (String) -> Unit, onDismiss: () -> Unit) {
    var name by rememberSaveable { mutableStateOf(current) }
    val valid = validDeviceName(name)
    androidx.compose.foundation.layout.Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
        OutlinedTextField(
            value = name,
            onValueChange = { name = it.take(80) },
            singleLine = true,
            label = { Text(stringResource(R.string.rename_phone_label)) },
            supportingText = { Text(stringResource(R.string.rename_phone_hint)) },
            isError = valid == null,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { if (valid != null && valid != current) onRename(valid) }),
            modifier = Modifier.testTag("rename-field"),
        )
        androidx.compose.foundation.layout.Row {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) }
            TextButton(
                onClick = { valid?.let(onRename) },
                enabled = valid != null && valid != current,
                modifier = Modifier.testTag("rename-save"),
            ) {
                Text(stringResource(R.string.action_rename))
            }
        }
    }
}
