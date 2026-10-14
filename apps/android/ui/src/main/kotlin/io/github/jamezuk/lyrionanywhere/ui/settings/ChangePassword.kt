package io.github.jamezuk.lyrionanywhere.ui.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.Checkbox
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.autofill.ContentType
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentType
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.data.PasswordChangeError
import io.github.jamezuk.lyrionanywhere.data.PasswordChangeResult
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.errorMessage

/** The server's password policy (it checks again): at least this many characters. */
const val MIN_PASSWORD = 10

/** What is wrong with the form as typed, before anything is sent. */
enum class PasswordFormProblem { TooShort, Same, Mismatch }

/** The first thing wrong with the new password as typed, or null (empty fields aren't wrong yet). */
fun passwordFormProblem(current: String, new: String, confirm: String): PasswordFormProblem? = when {
    new.isNotEmpty() && new.codePointCount(0, new.length) < MIN_PASSWORD -> PasswordFormProblem.TooShort
    new.isNotEmpty() && new == current -> PasswordFormProblem.Same
    confirm.isNotEmpty() && confirm != new -> PasswordFormProblem.Mismatch
    else -> null
}

/** The digits of a two-step code as typed (spaces dropped), if it is six digits. */
fun twoStepDigits(code: String): String? = code.filterNot { it.isWhitespace() }.takeIf { c -> c.length == 6 && c.all { it in '0'..'9' } }

/** Whether the form can be sent. */
fun passwordFormReady(current: String, new: String, confirm: String, twoStep: Boolean, code: String): Boolean =
    current.isNotEmpty() && new.isNotEmpty() && confirm == new &&
        passwordFormProblem(current, new, confirm) == null && (!twoStep || twoStepDigits(code) != null)

@Composable
fun passwordChangeErrorText(error: PasswordChangeError): String = when (error) {
    PasswordChangeError.WrongPassword -> stringResource(R.string.password_error_wrong)
    PasswordChangeError.CodeRequired -> stringResource(R.string.two_step_prompt)
    is PasswordChangeError.Refused -> stringResource(R.string.password_error_refused, error.reason)
    is PasswordChangeError.LockedOut ->
        pluralStringResource(R.plurals.error_rate_limited_minutes, error.minutes.toInt(), error.minutes.toInt())
    PasswordChangeError.Offline -> stringResource(R.string.password_error_offline)
    PasswordChangeError.SignedOut -> stringResource(R.string.password_error_signed_out)
    is PasswordChangeError.Other -> errorMessage(error.error)
}

/**
 * Change the password: the current one, the new one twice, and the two-step code when
 * two-step is on. Other devices stay signed in unless the box is ticked. Once changed, it
 * says which other devices are still signed in. In place in the settings list, as the
 * rename editor is.
 */
@Composable
fun ChangePasswordEditor(
    twoStep: Boolean,
    busy: Boolean,
    error: PasswordChangeError?,
    changed: PasswordChangeResult.Changed?,
    onSubmit: (current: String, new: String, code: String?, signOutOthers: Boolean) -> Unit,
    onDismiss: () -> Unit,
) {
    if (changed != null) {
        PasswordChangedNote(changed, onDismiss)
        return
    }
    // Passwords aren't saved across process death: plain remember, not rememberSaveable.
    var current by remember { mutableStateOf("") }
    var new by remember { mutableStateOf("") }
    var confirm by remember { mutableStateOf("") }
    var code by remember { mutableStateOf("") }
    var signOutOthers by rememberSaveable { mutableStateOf(false) }
    var show by remember { mutableStateOf(false) }
    val problem = passwordFormProblem(current, new, confirm)
    val ready = passwordFormReady(current, new, confirm, twoStep, code) && !busy
    val submit = { if (ready) onSubmit(current, new, if (twoStep) twoStepDigits(code) else null, signOutOthers) }
    val transformation = if (show) VisualTransformation.None else PasswordVisualTransformation()
    Column(
        Modifier.padding(horizontal = 16.dp, vertical = 8.dp).testTag("password-form"),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        OutlinedTextField(
            value = current,
            onValueChange = { current = it.take(1024) },
            label = { Text(stringResource(R.string.password_current)) },
            singleLine = true,
            enabled = !busy,
            isError = error == PasswordChangeError.WrongPassword,
            visualTransformation = transformation,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Next),
            trailingIcon = {
                IconButton(onClick = { show = !show }) {
                    Icon(
                        if (show) Icons.Outlined.VisibilityOff else Icons.Outlined.Visibility,
                        contentDescription = stringResource(if (show) R.string.hide_password else R.string.show_password),
                    )
                }
            },
            modifier = Modifier
                .fillMaxWidth()
                .testTag("password-current")
                .semantics { contentType = ContentType.Password },
        )
        OutlinedTextField(
            value = new,
            onValueChange = { new = it.take(1024) },
            label = { Text(stringResource(R.string.password_new)) },
            supportingText = {
                Text(
                    stringResource(
                        when (problem) {
                            PasswordFormProblem.TooShort -> R.string.password_too_short
                            PasswordFormProblem.Same -> R.string.password_same
                            else -> R.string.password_new_hint
                        },
                    ),
                )
            },
            singleLine = true,
            enabled = !busy,
            isError = problem == PasswordFormProblem.TooShort || problem == PasswordFormProblem.Same,
            visualTransformation = transformation,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Next),
            modifier = Modifier
                .fillMaxWidth()
                .testTag("password-new")
                .semantics { contentType = ContentType.NewPassword },
        )
        OutlinedTextField(
            value = confirm,
            onValueChange = { confirm = it.take(1024) },
            label = { Text(stringResource(R.string.password_confirm)) },
            supportingText = if (problem == PasswordFormProblem.Mismatch) {
                { Text(stringResource(R.string.password_mismatch)) }
            } else {
                null
            },
            singleLine = true,
            enabled = !busy,
            isError = problem == PasswordFormProblem.Mismatch,
            visualTransformation = transformation,
            keyboardOptions = KeyboardOptions(
                keyboardType = KeyboardType.Password,
                imeAction = if (twoStep) ImeAction.Next else ImeAction.Done,
            ),
            modifier = Modifier
                .fillMaxWidth()
                .testTag("password-confirm")
                .semantics { contentType = ContentType.NewPassword },
        )
        if (twoStep) {
            OutlinedTextField(
                value = code,
                onValueChange = { code = it.take(12) },
                label = { Text(stringResource(R.string.two_step_code)) },
                supportingText = { Text(stringResource(R.string.two_step_prompt)) },
                singleLine = true,
                enabled = !busy,
                isError = error == PasswordChangeError.CodeRequired,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword, imeAction = ImeAction.Done),
                modifier = Modifier
                    .fillMaxWidth()
                    .testTag("password-code")
                    .semantics { contentType = ContentType.SmsOtpCode },
            )
        }
        Row(
            Modifier
                .fillMaxWidth()
                .toggleable(value = signOutOthers, enabled = !busy, role = Role.Checkbox) { signOutOthers = it }
                .testTag("password-sign-out-others"),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Checkbox(checked = signOutOthers, onCheckedChange = null, enabled = !busy)
            Text(stringResource(R.string.password_sign_out_others), Modifier.weight(1f))
        }
        if (error != null) {
            Text(
                passwordChangeErrorText(error),
                color = MaterialTheme.colorScheme.error,
                style = MaterialTheme.typography.bodyMedium,
                // Announced as it appears.
                modifier = Modifier
                    .testTag("password-error")
                    .semantics { liveRegion = LiveRegionMode.Polite },
            )
        }
        Row {
            TextButton(onClick = onDismiss, enabled = !busy) { Text(stringResource(R.string.action_cancel)) }
            TextButton(onClick = submit, enabled = ready, modifier = Modifier.testTag("password-submit")) {
                Text(stringResource(R.string.password_change))
            }
        }
    }
}

/** Changed: who else is still signed in (or how many were signed out). */
@Composable
private fun PasswordChangedNote(changed: PasswordChangeResult.Changed, onDismiss: () -> Unit) {
    Column(
        Modifier
            .padding(horizontal = 16.dp, vertical = 8.dp)
            .testTag("password-changed")
            .semantics { liveRegion = LiveRegionMode.Polite },
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(stringResource(R.string.password_changed), style = MaterialTheme.typography.bodyLarge)
        if (changed.signedOut > 0) {
            Text(pluralStringResource(R.plurals.password_signed_out_others, changed.signedOut, changed.signedOut))
        }
        if (changed.otherDevices.isEmpty()) {
            Text(stringResource(R.string.password_no_other_devices))
        } else {
            Text(stringResource(R.string.password_still_signed_in))
            changed.otherDevices.forEach { d ->
                val platform = d.platform
                Text(
                    if (platform != null) stringResource(R.string.password_device_platform, d.name, platform) else d.name,
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        }
        TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_done)) }
    }
}
