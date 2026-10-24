package io.github.jamezuk.lyrionanywhere.ui.onboarding

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.core.PairingInvite
import io.github.jamezuk.lyrionanywhere.data.AccountRepository
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.needsUnencryptedConfirmation
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class ServerUiState(
    val address: String = "",
    val busy: Boolean = false,
    val error: AppError? = null,
    /**
     * The address is plain `http://` to a private network (a VPN): the warning is showing,
     * and nothing is saved until the user chooses Connect anyway.
     */
    val confirmUnencrypted: Boolean = false,
)

/** Welcome: enter the server's address. */
class ServerViewModel(private val account: AccountRepository) : ViewModel() {
    private val _state = MutableStateFlow(
        ServerUiState(
            address = when (val s = account.state.value) {
                is AuthState.SignedOut -> s.server.url
                else -> ""
            },
        ),
    )
    val state: StateFlow<ServerUiState> = _state.asStateFlow()

    fun onAddressChange(value: String) {
        if (_state.value.confirmUnencrypted) return
        _state.update { it.copy(address = value.take(2048), error = null) }
    }

    /** Validate and use the address; `onDone` runs once it is saved. */
    fun submit(onDone: () -> Unit) = use(confirmed = false, onDone)

    /** The warning's Connect anyway: use the address, unencrypted. */
    fun connectUnencrypted(onDone: () -> Unit) {
        if (!_state.value.confirmUnencrypted) return
        _state.update { it.copy(confirmUnencrypted = false) }
        use(confirmed = true, onDone)
    }

    /** The warning's Cancel: nothing is used, and the address stays as it was typed. */
    fun cancelUnencrypted() {
        _state.update { it.copy(confirmUnencrypted = false) }
    }

    private fun use(confirmed: Boolean, onDone: () -> Unit) {
        val address = _state.value.address
        if (_state.value.busy || (_state.value.confirmUnencrypted && !confirmed)) return
        _state.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            when (val r = account.useServer(address, confirmedUnencrypted = confirmed)) {
                is ApiResult.Ok -> {
                    _state.update { it.copy(busy = false, address = r.value.url) }
                    onDone()
                }
                is ApiResult.Err -> _state.update {
                    if (!confirmed && r.error.needsUnencryptedConfirmation()) {
                        it.copy(busy = false, confirmUnencrypted = true)
                    } else {
                        it.copy(busy = false, error = r.error)
                    }
                }
            }
        }
    }
}

data class SignInUiState(
    val username: String = "",
    val password: String = "",
    val code: String = "",
    /** The server asked for a two-step code. */
    val needsCode: Boolean = false,
    val busy: Boolean = false,
    val error: AppError? = null,
) {
    val canSubmit: Boolean
        get() = !busy && username.isNotBlank() && password.isNotEmpty() &&
            (!needsCode || code.count { it.isDigit() } == 6)
}

/** Sign in with a user name and password (and a two-step code when asked). */
class SignInViewModel(private val account: AccountRepository) : ViewModel() {
    private val _state = MutableStateFlow(SignInUiState())
    val state: StateFlow<SignInUiState> = _state.asStateFlow()

    fun onUsernameChange(v: String) = _state.update { it.copy(username = v.take(128), error = null) }

    fun onPasswordChange(v: String) = _state.update { it.copy(password = v.take(1024), error = null) }

    fun onCodeChange(v: String) =
        _state.update { it.copy(code = v.filter { c -> c.isDigit() }.take(6), error = null) }

    fun submit() {
        val s = _state.value
        if (!s.canSubmit) return
        _state.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            val r = account.signIn(s.username.trim(), s.password, s.code.takeIf { s.needsCode })
            _state.update {
                when (r) {
                    // Signed in: the app moves on; forget the password at once.
                    is ApiResult.Ok -> SignInUiState(username = it.username)
                    is ApiResult.Err -> when (r.error) {
                        AppError.TwoStepRequired -> it.copy(busy = false, needsCode = true, code = "")
                        else -> it.copy(
                            busy = false,
                            error = r.error,
                            // A wrong code: ask for a fresh one.
                            code = "",
                        )
                    }
                }
            }
        }
    }
}

/** What the user is asked to confirm before pairing. */
data class PairConfirm(
    /** The code, grouped for reading ("ABCDE-FGHJK"). */
    val code: String,
    /** The server it signs in to, for display ("music.example.com"). */
    val serverName: String,
    val serverUrl: String,
    /** The server this replaces, when the code names a different one. */
    val replaces: String? = null,
    /** The server is plain `http://` to a private network: the user is warned before pairing. */
    val unencrypted: Boolean = false,
)

data class PairUiState(
    val code: String = "",
    val busy: Boolean = false,
    val error: AppError? = null,
    /** The scanner is paused after a scan until the result is known. */
    val scanning: Boolean = true,
    /** A code was read (scanned, typed or from a link): the user confirms the server first. */
    val confirm: PairConfirm? = null,
    /** The unencrypted-connection warning is showing (after Sign in, for an http:// server). */
    val confirmUnencrypted: Boolean = false,
)

/**
 * Pair by scanning the QR code from the web app or the LMS plugin (or typing the code, or
 * from a pairing link the app was opened with). Nothing is sent until the user has seen the
 * server and confirmed.
 */
class PairViewModel(private val account: AccountRepository) : ViewModel() {
    private val _state = MutableStateFlow(PairUiState())
    val state: StateFlow<PairUiState> = _state.asStateFlow()

    /** The invite being confirmed. */
    private var invite: PairingInvite? = null

    fun onCodeChange(v: String) = _state.update { it.copy(code = v.take(64), error = null) }

    fun submit() = review(_state.value.code)

    /** A QR code was read. */
    fun onScanned(text: String) {
        val s = _state.value
        if (!s.scanning || s.busy || s.confirm != null) return
        review(text)
    }

    /** A pairing link the app was opened with (already checked by [PairingLinks]). */
    fun offer(link: PairingInvite) {
        if (_state.value.busy) return
        show(link)
    }

    private fun review(text: String) {
        if (_state.value.busy) return
        when (val r = account.readPairingCode(text)) {
            is ApiResult.Ok -> show(r.value)
            is ApiResult.Err -> _state.update { it.copy(error = r.error) }
        }
    }

    private fun show(link: PairingInvite) {
        val current = (account.state.value as? AuthState.SignedOut)?.server
        // The server in use was agreed to when it was chosen; a new http:// one asks first.
        var unencrypted = false
        val target = when (val url = link.server) {
            null -> current
            current?.url -> current
            else -> when (val checked = account.checkServerAddress(url)) {
                is ApiResult.Ok -> checked.value
                is ApiResult.Err -> {
                    val agreed = if (checked.error.needsUnencryptedConfirmation()) {
                        account.checkServerAddress(url, confirmedUnencrypted = true) as? ApiResult.Ok
                    } else {
                        null
                    }
                    if (agreed == null) {
                        _state.update { it.copy(error = checked.error) }
                        return
                    }
                    unencrypted = true
                    agreed.value
                }
            }
        }
        if (target == null) {
            _state.update { it.copy(error = AppError.Reason.NO_SERVER.error()) }
            return
        }
        invite = link
        _state.update {
            it.copy(
                error = null,
                scanning = false,
                confirm = PairConfirm(
                    code = groupCode(link.code),
                    serverName = target.name,
                    serverUrl = target.url,
                    replaces = current?.takeIf { c -> c.url != target.url }?.name,
                    unencrypted = unencrypted,
                ),
            )
        }
    }

    /** Not this one: back to scanning or typing. */
    fun cancel() {
        if (_state.value.busy) return
        invite = null
        _state.update { it.copy(confirm = null, confirmUnencrypted = false, scanning = true, error = null) }
    }

    /** The user confirmed the server: pair (after the warning, for an http:// server). */
    fun confirm() {
        val confirm = _state.value.confirm ?: return
        if (_state.value.busy || invite == null) return
        if (confirm.unencrypted) {
            _state.update { it.copy(confirmUnencrypted = true) }
            return
        }
        pair(confirmedUnencrypted = false)
    }

    /** The warning's Connect anyway. */
    fun connectUnencrypted() {
        if (!_state.value.confirmUnencrypted) return
        _state.update { it.copy(confirmUnencrypted = false) }
        pair(confirmedUnencrypted = true)
    }

    /** The warning's Cancel: nothing is sent; the server and code stay on screen. */
    fun cancelUnencrypted() {
        _state.update { it.copy(confirmUnencrypted = false) }
    }

    private fun pair(confirmedUnencrypted: Boolean) {
        val link = invite ?: return
        if (_state.value.busy) return
        _state.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            val r = account.pair(link, confirmedUnencrypted)
            invite = null
            _state.update {
                when (r) {
                    is ApiResult.Ok -> PairUiState(scanning = false)
                    is ApiResult.Err -> it.copy(busy = false, scanning = true, confirm = null, error = r.error)
                }
            }
        }
    }

    companion object {
        /** "ABCDEFGHJK" as "ABCDE-FGHJK" (other lengths as they are). */
        fun groupCode(code: String): String =
            if (code.length == 10) "${code.take(5)}-${code.drop(5)}" else code
    }
}
