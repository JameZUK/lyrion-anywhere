package io.github.jamezuk.lyrionanywhere.ui.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Logout
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.Gavel
import androidx.compose.material.icons.outlined.Person
import androidx.compose.material.icons.outlined.Security
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.core.AccountInfo
import io.github.jamezuk.lyrionanywhere.data.AccountRepository
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.library.SectionHeader
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class SettingsUiState(
    val serverName: String = "",
    val username: String = "",
    val account: AccountInfo? = null,
    val signingOut: Boolean = false,
)

class SettingsViewModel(private val account: AccountRepository) : ViewModel() {
    private val _state = MutableStateFlow(SettingsUiState())
    val state: StateFlow<SettingsUiState> = _state.asStateFlow()

    init {
        (account.state.value as? AuthState.SignedInState)?.let { s ->
            _state.update { it.copy(serverName = s.server.name, username = s.who.user.username) }
        }
        viewModelScope.launch {
            (account.account() as? ApiResult.Ok)?.let { r -> _state.update { it.copy(account = r.value) } }
        }
    }

    fun signOut() {
        if (_state.value.signingOut) return
        _state.update { it.copy(signingOut = true) }
        viewModelScope.launch {
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
    Licence("Jetpack Compose, AndroidX, CameraX", "Apache-2.0"),
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
    onSignOut: () -> Unit,
    onLicences: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var confirm by rememberSaveable { mutableStateOf(false) }
    LazyColumn(modifier.fillMaxSize()) {
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
                leadingContent = { Icon(Icons.Outlined.Info, contentDescription = null) },
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
    }
    if (confirm) {
        AlertDialog(
            onDismissRequest = { confirm = false },
            title = { Text(stringResource(R.string.sign_out_confirm_title)) },
            text = { Text(stringResource(R.string.sign_out_confirm_body)) },
            confirmButton = {
                TextButton(onClick = {
                    confirm = false
                    onSignOut()
                }) { Text(stringResource(R.string.action_sign_out)) }
            },
            dismissButton = {
                TextButton(onClick = { confirm = false }) { Text(stringResource(R.string.action_cancel)) }
            },
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
