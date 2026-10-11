package io.github.jamezuk.lyrionanywhere.ui.onboarding

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.data.AccountRepository
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AuthState
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class ServerUiState(
    val address: String = "",
    val busy: Boolean = false,
    val error: AppError? = null,
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
        _state.update { it.copy(address = value.take(2048), error = null) }
    }

    /** Validate and use the address; `onDone` runs once it is saved. */
    fun submit(onDone: () -> Unit) {
        val address = _state.value.address
        if (_state.value.busy) return
        _state.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            when (val r = account.useServer(address)) {
                is ApiResult.Ok -> {
                    _state.update { it.copy(busy = false, address = r.value.url) }
                    onDone()
                }
                is ApiResult.Err -> _state.update { it.copy(busy = false, error = r.error) }
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

data class PairUiState(
    val code: String = "",
    val busy: Boolean = false,
    val error: AppError? = null,
    /** The scanner is paused after a scan until the result is known. */
    val scanning: Boolean = true,
)

/** Pair by scanning the QR code from the web app (or typing the code). */
class PairViewModel(private val account: AccountRepository) : ViewModel() {
    private val _state = MutableStateFlow(PairUiState())
    val state: StateFlow<PairUiState> = _state.asStateFlow()

    fun onCodeChange(v: String) = _state.update { it.copy(code = v.take(64), error = null) }

    fun submit() = pairWith(_state.value.code)

    /** A QR code was read. */
    fun onScanned(text: String) {
        if (!_state.value.scanning || _state.value.busy) return
        pairWith(text)
    }

    private fun pairWith(text: String) {
        if (_state.value.busy) return
        val invite = when (val r = account.readPairingCode(text)) {
            is ApiResult.Ok -> r.value
            is ApiResult.Err -> {
                _state.update { it.copy(error = r.error) }
                return
            }
        }
        _state.update { it.copy(busy = true, scanning = false, error = null) }
        viewModelScope.launch {
            val r = account.pair(invite)
            _state.update {
                when (r) {
                    is ApiResult.Ok -> PairUiState(scanning = false)
                    is ApiResult.Err -> it.copy(busy = false, scanning = true, error = r.error)
                }
            }
        }
    }
}
