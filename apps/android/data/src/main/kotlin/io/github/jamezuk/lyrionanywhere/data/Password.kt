package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.OwnDevice
import io.github.jamezuk.lyrionanywhere.core.PasswordChanged

/** Why a password change failed, in the terms the form shows. Never carries a password. */
sealed interface PasswordChangeError {
    /** The current password, or the two-step code, isn't right. */
    data object WrongPassword : PasswordChangeError

    /** Two-step verification is on: the code is needed. */
    data object CodeRequired : PasswordChangeError

    /** Refused as given: [reason] is the policy's (or the server's) words, fit to show. */
    data class Refused(val reason: String) : PasswordChangeError

    /** Too many wrong tries: wait about this many minutes (at least 1). */
    data class LockedOut(val minutes: Long) : PasswordChangeError

    /** The server couldn't be reached; nothing was changed. */
    data object Offline : PasswordChangeError

    /** The sign-in has ended. */
    data object SignedOut : PasswordChangeError

    /** Anything else (the server failed). */
    data class Other(val error: AppError) : PasswordChangeError
}

/** What a password change came to. */
sealed interface PasswordChangeResult {
    /** Changed. This phone stays signed in; [otherDevices] are still signed in too. */
    data class Changed(val signedOut: Int, val otherDevices: List<OwnDevice>) : PasswordChangeResult

    data class Failed(val error: PasswordChangeError) : PasswordChangeResult
}

/** Map a failed change (`POST /v1/account/password`) to what the form says. */
fun AppError.toPasswordChangeError(): PasswordChangeError = when (this) {
    // The gateway answers a wrong password or code with 403 (not 401, which is a token).
    is AppError.Forbidden -> PasswordChangeError.WrongPassword
    AppError.TwoStepRequired -> PasswordChangeError.CodeRequired
    is AppError.InvalidInput -> PasswordChangeError.Refused(detail)
    is AppError.RateLimited -> PasswordChangeError.LockedOut(((retryAfterSeconds + 59) / 60).coerceAtLeast(1))
    is AppError.Offline -> PasswordChangeError.Offline
    AppError.SignedOut -> PasswordChangeError.SignedOut
    else -> PasswordChangeError.Other(this)
}

/** Run a password change through the Rust layer and say what it came to. */
suspend fun runPasswordChange(call: suspend () -> PasswordChanged): PasswordChangeResult =
    when (val r = apiCall(call)) {
        is ApiResult.Ok -> PasswordChangeResult.Changed(r.value.signedOut.toInt(), r.value.otherDevices)
        is ApiResult.Err -> PasswordChangeResult.Failed(r.error.toPasswordChangeError())
    }
