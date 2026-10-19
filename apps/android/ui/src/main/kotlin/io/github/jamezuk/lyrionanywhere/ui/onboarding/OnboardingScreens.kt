package io.github.jamezuk.lyrionanywhere.ui.onboarding

import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.draw.alpha
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
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
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.autofill.ContentType
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.contentType
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.Dp
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
        Text(
            text,
            color = MaterialTheme.colorScheme.error,
            style = MaterialTheme.typography.bodyMedium,
            // Read out when it appears, without moving focus away from the form.
            modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
        )
    }
}

/** A refused sign-in or pairing reads as "wrong details", not "signed out". */
@Composable
private fun authErrorMessage(error: AppError, refused: Int): String =
    if (error == AppError.SignedOut) stringResource(refused) else errorMessage(error)

/** A button's label, or a spinner while busy: the spinner carries the label, so the button keeps its name. */
@Composable
private fun BusyLabel(busy: Boolean, label: String) {
    // The label stays (invisible while busy), so the button keeps its name for TalkBack and
    // its size; the spinner itself says nothing (it would be a separate, unlabelled stop).
    Box(contentAlignment = Alignment.Center) {
        Text(label, Modifier.alpha(if (busy) 0f else 1f))
        if (busy) CircularProgressIndicator(Modifier.size(20.dp).clearAndSetSemantics {}, strokeWidth = 2.dp)
    }
}

@Composable
fun ServerScreen(
    state: ServerUiState,
    onAddressChange: (String) -> Unit,
    onSubmit: () -> Unit,
    onScan: () -> Unit = {},
) {
    val addressFocus = remember { FocusRequester() }
    // Ready to type on first show, unless an address is already filled in.
    LaunchedEffect(Unit) { if (state.address.isEmpty() && !state.busy) addressFocus.requestFocus() }
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
            modifier = Modifier.fillMaxWidth().focusRequester(addressFocus),
        )
        ErrorText(state.error?.let { errorMessage(it) })
        Button(
            onClick = onSubmit,
            enabled = !state.busy && state.address.isNotBlank(),
            modifier = Modifier.fillMaxWidth(),
        ) { BusyLabel(state.busy, stringResource(R.string.action_continue)) }
        OutlinedButton(onClick = onScan, enabled = !state.busy, modifier = Modifier.fillMaxWidth()) {
            Icon(Icons.Outlined.QrCodeScanner, contentDescription = null)
            Text(stringResource(R.string.action_scan_qr), Modifier.padding(start = 8.dp))
        }
        Text(
            stringResource(R.string.server_scan_note),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
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
    val usernameFocus = remember { FocusRequester() }
    LaunchedEffect(Unit) {
        if (state.username.isEmpty() && !state.busy && !state.needsCode) usernameFocus.requestFocus()
    }
    val failed = state.error != null
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
            isError = failed,
            enabled = !state.busy && !state.needsCode,
            keyboardOptions = KeyboardOptions(
                keyboardType = KeyboardType.Text,
                imeAction = ImeAction.Next,
                autoCorrectEnabled = false,
            ),
            modifier = Modifier
                .fillMaxWidth()
                .focusRequester(usernameFocus)
                .semantics { contentType = ContentType.Username },
        )
        OutlinedTextField(
            value = state.password,
            onValueChange = onPasswordChange,
            label = { Text(stringResource(R.string.password)) },
            singleLine = true,
            isError = failed,
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
                isError = failed,
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
    onConfirm: () -> Unit = {},
    onCancel: () -> Unit = {},
    backLabel: String = stringResource(R.string.action_back_to_sign_in),
    /** The camera was refused for good: explain, and offer the app's settings instead. */
    cameraBlocked: Boolean = false,
    onOpenSettings: () -> Unit = {},
) {
    val confirm = state.confirm
    if (confirm != null) {
        PairConfirmation(confirm, busy = state.busy, onConfirm = onConfirm, onCancel = onCancel)
        return
    }
    FormColumn {
        Text(
            stringResource(R.string.pair_title),
            style = MaterialTheme.typography.headlineMedium,
            modifier = Modifier.semantics { heading() },
        )
        Text(stringResource(R.string.pair_body), style = MaterialTheme.typography.bodyLarge)
        if (cameraPermitted) {
            // A square that fits the window in either orientation: on a phone held sideways
            // the column's full width would be taller than the screen, and the code could not
            // be framed.
            val screenHeight = LocalConfiguration.current.screenHeightDp
            BoxWithConstraints(Modifier.fillMaxWidth()) {
                QrScanner(
                    active = state.scanning && !state.busy,
                    onScanned = onScanned,
                    modifier = Modifier.size(scannerSide(maxWidth, screenHeight)).align(Alignment.Center),
                )
            }
        } else if (cameraBlocked) {
            // Android won't ask again: say why there is no scanner, and where to turn it on.
            // Typing the code (below) still works.
            Text(
                stringResource(R.string.camera_denied),
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.testTag("camera-denied"),
            )
            OutlinedButton(onClick = onOpenSettings, modifier = Modifier.fillMaxWidth().testTag("camera-settings")) {
                Text(stringResource(R.string.action_open_settings))
            }
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
            isError = state.error != null,
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
        ) { BusyLabel(state.busy, stringResource(R.string.action_continue)) }
        TextButton(onClick = onBack, enabled = !state.busy) {
            Text(backLabel)
        }
    }
}

/**
 * The scanner preview's side: the column's width, but never more than about half the
 * window's height, so the whole square is on screen in landscape too (the camera frames
 * ZXing reads are the full frames whatever the preview's size).
 */
internal fun scannerSide(maxWidth: Dp, screenHeightDp: Int): Dp =
    minOf(maxWidth, (screenHeightDp * 0.55f).dp)

/**
 * Before anything is sent: which server the code signs in to (and the one it replaces), so
 * a QR code or link from someone else can't quietly sign this phone in somewhere else.
 */
@Composable
private fun PairConfirmation(confirm: PairConfirm, busy: Boolean, onConfirm: () -> Unit, onCancel: () -> Unit) {
    FormColumn {
        Text(
            stringResource(R.string.pair_confirm_title, confirm.serverName),
            style = MaterialTheme.typography.headlineMedium,
            modifier = Modifier.semantics { heading() },
        )
        Text(stringResource(R.string.pair_confirm_body), style = MaterialTheme.typography.bodyLarge)
        Text(
            stringResource(R.string.pair_confirm_server, confirm.serverUrl),
            style = MaterialTheme.typography.bodyMedium,
        )
        Text(
            stringResource(R.string.pair_confirm_code, confirm.code),
            style = MaterialTheme.typography.bodyMedium,
        )
        if (confirm.replaces != null) {
            Text(
                stringResource(R.string.pair_confirm_replaces, confirm.replaces),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Button(onClick = onConfirm, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
            BusyLabel(busy, stringResource(R.string.action_sign_in))
        }
        TextButton(onClick = onCancel, enabled = !busy) {
            Text(stringResource(R.string.action_cancel))
        }
    }
}
