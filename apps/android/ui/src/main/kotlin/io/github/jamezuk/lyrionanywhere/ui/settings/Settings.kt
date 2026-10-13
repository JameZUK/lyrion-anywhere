package io.github.jamezuk.lyrionanywhere.ui.settings

import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
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
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material.icons.outlined.Storage
import androidx.compose.material.icons.outlined.Gavel
import androidx.compose.material.icons.outlined.GraphicEq
import androidx.compose.material.icons.outlined.Person
import androidx.compose.material.icons.outlined.PrivacyTip
import androidx.compose.material.icons.outlined.Security
import androidx.compose.material.icons.outlined.Smartphone
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
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
import io.github.jamezuk.lyrionanywhere.data.AccountRepository
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
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
                it.copy(serverName = s.server.name, username = s.who.user.username, deviceName = s.who.device.name)
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

@Composable
fun SettingsScreen(
    state: SettingsUiState,
    appVersion: String,
    libraryVersion: String,
    onSignOut: (removeDownloads: Boolean) -> Unit,
    onLicences: () -> Unit,
    modifier: Modifier = Modifier,
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
) {
    var confirm by rememberSaveable { mutableStateOf(false) }
    var renaming by rememberSaveable { mutableStateOf(false) }
    LazyColumn(modifier.fillMaxSize()) {
        if (defaultPlayer != null) {
            item { SectionHeader(stringResource(R.string.section_default_player)) }
            item { defaultPlayer() }
        }
        if (sleepFade != null) {
            item { sleepFade() }
            if (lyrics == null) item { HorizontalDivider() }
        }
        if (lyrics != null) {
            item { lyrics() }
            if (streaming == null) item { HorizontalDivider() }
        }
        if (visualiser != null) {
            item { visualiser() }
            if (streaming == null) item { HorizontalDivider() }
        }
        if (streaming != null) {
            item { streaming() }
            item { HorizontalDivider() }
        }
        if (ratings != null) {
            item { ratings() }
            item { HorizontalDivider() }
        }
        state.sound?.let { sound ->
            item { SectionHeader(stringResource(R.string.section_playback)) }
            item {
                ListItem(
                    leadingContent = { Icon(Icons.Outlined.GraphicEq, contentDescription = null) },
                    headlineContent = { Text(stringResource(R.string.title_sound)) },
                    supportingContent = {
                        Text(
                            if (sound.bitPerfect) {
                                stringResource(R.string.sound_row_bit_perfect)
                            } else {
                                stringResource(R.string.sound_row_processing, stageNames(sound.activeStages))
                            },
                        )
                    },
                    modifier = Modifier.clickable(onClick = onSound),
                )
            }
            item { HorizontalDivider() }
        }
        if (history != null) {
            item { SectionHeader(stringResource(R.string.section_history)) }
            item { history() }
            item { HorizontalDivider() }
        }
        if (state.downloadsOffered) {
            item { SectionHeader(stringResource(R.string.section_offline)) }
            item {
                ListItem(
                    leadingContent = { Icon(Icons.Outlined.Download, contentDescription = null) },
                    headlineContent = { Text(stringResource(R.string.title_downloads)) },
                    supportingContent = { Text(stringResource(R.string.storage_used, formatBytes(state.downloadedBytes))) },
                    modifier = Modifier.clickable(onClick = onDownloads),
                )
            }
            item {
                ListItem(
                    leadingContent = { Icon(Icons.Outlined.Storage, contentDescription = null) },
                    headlineContent = { Text(stringResource(R.string.title_storage)) },
                    supportingContent = { Text(stringResource(R.string.storage_row)) },
                    modifier = Modifier.clickable(onClick = onStorage),
                )
            }
            item { HorizontalDivider() }
        }
        item { SectionHeader(stringResource(R.string.section_account)) }
        item {
            ListItem(
                leadingContent = { Icon(Icons.Outlined.Person, contentDescription = null) },
                headlineContent = { Text(state.username) },
                supportingContent = {
                    Text(
                        stringResource(
                            if (state.account?.isAdmin == true) R.string.role_admin else R.string.role_listener,
                        ),
                    )
                },
            )
        }
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
        item {
            ListItem(
                leadingContent = { Icon(Icons.Outlined.Dns, contentDescription = null) },
                headlineContent = { Text(stringResource(R.string.server)) },
                supportingContent = { Text(state.serverName) },
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
        }
        item {
            ListItem(
                leadingContent = { Icon(Icons.AutoMirrored.Outlined.Logout, contentDescription = null) },
                headlineContent = { Text(stringResource(R.string.action_sign_out)) },
                supportingContent = { Text(stringResource(R.string.sign_out_explained)) },
                modifier = Modifier.clickable(enabled = !state.signingOut) { confirm = true },
            )
        }
        item { HorizontalDivider() }
        item { SectionHeader(stringResource(R.string.section_about)) }
        item {
            ListItem(
                leadingContent = {
                    Image(painterResource(R.drawable.logo), contentDescription = null, modifier = Modifier.size(40.dp))
                },
                headlineContent = { Text(stringResource(R.string.app_name_full)) },
                supportingContent = { Text(stringResource(R.string.version_line, appVersion, libraryVersion)) },
            )
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
