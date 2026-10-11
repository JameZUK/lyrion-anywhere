package io.github.jamezuk.lyrionanywhere.ui.onboarding

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.LibraryMusic
import androidx.compose.material.icons.outlined.QrCodeScanner
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.autofill.ContentType
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentType
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.errorMessage

/** A centred, scrollable column that keeps forms readable on wide screens. */
@Composable
private fun FormColumn(content: @Composable ColumnScope.() -> Unit) {
    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .imePadding()
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Column(
            Modifier.widthIn(max = 480.dp).fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) { content() }
    }
}

@Composable
private fun ErrorText(text: String?) {
    if (text != null) {
        Text(text, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium)
    }
}

/** A refused sign-in or pairing reads as "wrong details", not "signed out". */
@Composable
private fun authErrorMessage(error: AppError, refused: Int): String =
    if (error == AppError.SignedOut) stringResource(refused) else errorMessage(error)

@Composable
private fun BusyLabel(busy: Boolean, label: String) {
    if (busy) {
        CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
    } else {
        Text(label)
    }
}

@Composable
fun ServerScreen(
    state: ServerUiState,
    onAddressChange: (String) -> Unit,
    onSubmit: () -> Unit,
) {
    FormColumn {
        Icon(
            Icons.Outlined.LibraryMusic,
            contentDescription = null,
            modifier = Modifier.size(64.dp).align(Alignment.CenterHorizontally),
            tint = MaterialTheme.colorScheme.primary,
        )
        Text(
            stringResource(R.string.welcome_title),
            style = MaterialTheme.typography.headlineMedium,
            modifier = Modifier.semantics { heading() },
        )
        Text(stringResource(R.string.welcome_body), style = MaterialTheme.typography.bodyLarge)
        OutlinedTextField(
            value = state.address,
            onValueChange = onAddressChange,
            label = { Text(stringResource(R.string.server_address)) },
            placeholder = { Text(stringResource(R.string.server_address_hint)) },
            singleLine = true,
            isError = state.error != null,
            enabled = !state.busy,
            keyboardOptions = KeyboardOptions(
                keyboardType = KeyboardType.Uri,
                imeAction = ImeAction.Go,
                autoCorrectEnabled = false,
            ),
            keyboardActions = KeyboardActions(onGo = { onSubmit() }),
            modifier = Modifier.fillMaxWidth(),
        )
        ErrorText(state.error?.let { errorMessage(it) })
        Button(
            onClick = onSubmit,
            enabled = !state.busy && state.address.isNotBlank(),
            modifier = Modifier.fillMaxWidth(),
        ) { BusyLabel(state.busy, stringResource(R.string.action_continue)) }
        Text(
            stringResource(R.string.server_https_note),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
fun SignInScreen(
    serverName: String,
    state: SignInUiState,
    onUsernameChange: (String) -> Unit,
    onPasswordChange: (String) -> Unit,
    onCodeChange: (String) -> Unit,
    onSubmit: () -> Unit,
    onPair: () -> Unit,
    onChangeServer: () -> Unit,
    revoked: Boolean = false,
) {
    var showPassword by rememberSaveable { mutableStateOf(false) }
    FormColumn {
        Text(
            stringResource(R.string.sign_in_title),
            style = MaterialTheme.typography.headlineMedium,
            modifier = Modifier.semantics { heading() },
        )
        Text(
            stringResource(R.string.sign_in_server, serverName),
            style = MaterialTheme.typography.bodyLarge,
        )
        if (revoked) {
            Text(
                stringResource(R.string.sign_in_revoked),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        OutlinedTextField(
            value = state.username,
            onValueChange = onUsernameChange,
            label = { Text(stringResource(R.string.username)) },
            singleLine = true,
            enabled = !state.busy && !state.needsCode,
            keyboardOptions = KeyboardOptions(
                keyboardType = KeyboardType.Text,
                imeAction = ImeAction.Next,
                autoCorrectEnabled = false,
            ),
            modifier = Modifier.fillMaxWidth().semantics { contentType = ContentType.Username },
        )
        OutlinedTextField(
            value = state.password,
            onValueChange = onPasswordChange,
            label = { Text(stringResource(R.string.password)) },
            singleLine = true,
            enabled = !state.busy && !state.needsCode,
            visualTransformation = if (showPassword) VisualTransformation.None else PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(
                keyboardType = KeyboardType.Password,
                imeAction = if (state.needsCode) ImeAction.Next else ImeAction.Done,
            ),
            keyboardActions = KeyboardActions(onDone = { onSubmit() }),
            trailingIcon = {
                IconButton(onClick = { showPassword = !showPassword }) {
                    Icon(
                        if (showPassword) Icons.Outlined.VisibilityOff else Icons.Outlined.Visibility,
                        contentDescription = stringResource(
                            if (showPassword) R.string.hide_password else R.string.show_password,
                        ),
                    )
                }
            },
            modifier = Modifier.fillMaxWidth().semantics { contentType = ContentType.Password },
        )
        if (state.needsCode) {
            Text(stringResource(R.string.two_step_prompt), style = MaterialTheme.typography.bodyMedium)
            OutlinedTextField(
                value = state.code,
                onValueChange = onCodeChange,
                label = { Text(stringResource(R.string.two_step_code)) },
                singleLine = true,
                enabled = !state.busy,
                keyboardOptions = KeyboardOptions(
                    keyboardType = KeyboardType.NumberPassword,
                    imeAction = ImeAction.Done,
                ),
                keyboardActions = KeyboardActions(onDone = { onSubmit() }),
                modifier = Modifier.fillMaxWidth().semantics { contentType = ContentType.SmsOtpCode },
            )
        }
        ErrorText(state.error?.let { authErrorMessage(it, R.string.error_wrong_credentials) })
        Button(
            onClick = onSubmit,
            enabled = state.canSubmit,
            modifier = Modifier.fillMaxWidth(),
        ) { BusyLabel(state.busy, stringResource(R.string.action_sign_in)) }
        OutlinedButton(onClick = onPair, enabled = !state.busy, modifier = Modifier.fillMaxWidth()) {
            Icon(Icons.Outlined.QrCodeScanner, contentDescription = null)
            Text(stringResource(R.string.action_pair_instead), Modifier.padding(start = 8.dp))
        }
        TextButton(onClick = onChangeServer, enabled = !state.busy) {
            Text(stringResource(R.string.action_change_server))
        }
    }
}

@Composable
fun PairScreen(
    state: PairUiState,
    cameraPermitted: Boolean,
    onRequestCamera: () -> Unit,
    onScanned: (String) -> Unit,
    onCodeChange: (String) -> Unit,
    onSubmit: () -> Unit,
    onBack: () -> Unit,
) {
    FormColumn {
        Text(
            stringResource(R.string.pair_title),
            style = MaterialTheme.typography.headlineMedium,
            modifier = Modifier.semantics { heading() },
        )
        Text(stringResource(R.string.pair_body), style = MaterialTheme.typography.bodyLarge)
        if (cameraPermitted) {
            QrScanner(
                active = state.scanning && !state.busy,
                onScanned = onScanned,
                modifier = Modifier.fillMaxWidth(),
            )
        } else {
            OutlinedButton(onClick = onRequestCamera, modifier = Modifier.fillMaxWidth()) {
                Icon(Icons.Outlined.QrCodeScanner, contentDescription = null)
                Text(stringResource(R.string.action_scan_qr), Modifier.padding(start = 8.dp))
            }
        }
        OutlinedTextField(
            value = state.code,
            onValueChange = onCodeChange,
            label = { Text(stringResource(R.string.pairing_code)) },
            singleLine = true,
            enabled = !state.busy,
            keyboardOptions = KeyboardOptions(
                keyboardType = KeyboardType.Ascii,
                imeAction = ImeAction.Done,
                autoCorrectEnabled = false,
            ),
            keyboardActions = KeyboardActions(onDone = { onSubmit() }),
            modifier = Modifier.fillMaxWidth(),
        )
        ErrorText(state.error?.let { authErrorMessage(it, R.string.error_bad_pairing_code) })
        Button(
            onClick = onSubmit,
            enabled = !state.busy && state.code.isNotBlank(),
            modifier = Modifier.fillMaxWidth(),
        ) { BusyLabel(state.busy, stringResource(R.string.action_pair)) }
        TextButton(onClick = onBack, enabled = !state.busy) {
            Text(stringResource(R.string.action_back_to_sign_in))
        }
    }
}
