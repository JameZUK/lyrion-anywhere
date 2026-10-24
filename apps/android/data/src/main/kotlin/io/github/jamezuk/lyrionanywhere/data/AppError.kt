package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.ApiException
import kotlin.coroutines.cancellation.CancellationException

/** Why something failed, in terms the UI can act on. Never carries tokens or passwords. */
sealed interface AppError {
    /**
     * The server can't be reached. The sign-in is kept; try again later. [detail] is the
     * network layer's reason (e.g. a DNS, connection or certificate failure), shown small
     * under the message so a failure can be diagnosed; it never holds tokens or passwords.
     */
    data class Offline(val detail: String = "") : AppError

    /** Not signed in (or the sign-in ended). */
    data object SignedOut : AppError

    /** The password was right; a two-step code is needed. */
    data object TwoStepRequired : AppError

    /** Refused by the server's policy. */
    data class Forbidden(val detail: String) : AppError

    /** Too many attempts: wait this long. */
    data class RateLimited(val retryAfterSeconds: Long) : AppError

    /**
     * The music server is busy (it stopped answering for a while; the gateway is reachable):
     * nothing was done. Try again in a moment, at the earliest after [retryAfterSeconds].
     */
    data class ServerBusy(val retryAfterSeconds: Long) : AppError

    /** The server or the music server failed. */
    data class Server(val status: Int, val detail: String) : AppError

    /**
     * The input was refused. With a known [reason] the app says it in its own words (from
     * its resources); with [Reason.OTHER] the [detail] is all there is (fit to show: bounded,
     * printable).
     */
    data class InvalidInput(val detail: String, val reason: Reason = Reason.OTHER) : AppError

    /** Why input was refused, when it is known: each is worded by the app, not the server. */
    enum class Reason {
        /** Only the [InvalidInput.detail] explains it. */
        OTHER,

        /** The server has no such thing (its 404): removed, or the server needs updating. */
        NOT_FOUND,

        /** It changed on the server meanwhile (its 409). */
        CONFLICT,

        /** Sign out before choosing another server. */
        SIGN_OUT_FIRST,

        /** No server is chosen yet. */
        CHOOSE_SERVER_FIRST,

        /** The pairing code has no server, and none is chosen. */
        NO_SERVER,

        /**
         * An `http://` server address this build never takes: a name, a public address, or
         * loopback outside debug builds (`https://`, or `http://` to a private address).
         */
        PLAIN_HTTP,

        /**
         * An `http://` server address on a private network (a VPN): allowed once the user has
         * confirmed the unencrypted connection. The app asks, then tries again.
         */
        CONFIRM_UNENCRYPTED,

        /** A smart filter shows 1 to 500 albums. */
        FILTER_LIMIT,

        /** A menu too deep to pin to Home. */
        MENU_TOO_DEEP,

        /** The queue changed before the edit arrived. */
        QUEUE_CHANGED,

        /** Smart downloads wait for Wi-Fi. */
        WAITING_FOR_WIFI,
        ;

        /** This reason as an error (its detail is only for logs and tests). */
        fun error(): InvalidInput = InvalidInput(name.lowercase().replace('_', ' '), this)
    }

    /** Anything else (a bug, or storage failing). */
    data class Unexpected(val detail: String) : AppError
}

/** The result of a call through the Rust layer. */
sealed interface ApiResult<out T> {
    data class Ok<T>(val value: T) : ApiResult<T>

    data class Err(val error: AppError) : ApiResult<Nothing>
}

inline fun <T, R> ApiResult<T>.map(f: (T) -> R): ApiResult<R> = when (this) {
    is ApiResult.Ok -> ApiResult.Ok(f(value))
    is ApiResult.Err -> this
}

fun <T> ApiResult<T>.getOrNull(): T? = (this as? ApiResult.Ok)?.value

fun <T> ApiResult<T>.errorOrNull(): AppError? = (this as? ApiResult.Err)?.error

/** Why another account's downloads were refused (see [ApiException.WrongAccount]). */
const val WRONG_ACCOUNT = "These downloads belong to another account."

/** Map anything thrown by the Rust layer (cancellation is never swallowed). */
fun Throwable.toAppError(): AppError = when (this) {
    is CancellationException -> throw this
    is ApiException.Network -> AppError.Offline(detail)
    is ApiException.Unauthorised -> AppError.SignedOut
    is ApiException.TwoStepRequired -> AppError.TwoStepRequired
    is ApiException.Forbidden -> AppError.Forbidden(detail)
    is ApiException.RateLimited -> AppError.RateLimited(retryAfterSecs.toLong())
    is ApiException.ServerBusy -> AppError.ServerBusy(retryAfterSecs.toLong())
    is ApiException.Server -> AppError.Server(status.toInt(), detail)
    is ApiException.InvalidInput -> AppError.InvalidInput(detail)
    // Typed by status in Rust (404, 409): said in the app's own words, not the server's text.
    is ApiException.NotFound -> AppError.InvalidInput(detail, AppError.Reason.NOT_FOUND)
    is ApiException.Conflict -> AppError.InvalidInput(detail, AppError.Reason.CONFLICT)
    // Another account's downloads were passed: refused, nothing sent.
    is ApiException.WrongAccount -> AppError.Forbidden(WRONG_ACCOUNT)
    else -> AppError.Unexpected(this::class.java.simpleName)
}

/** Run a call, turning failures into [ApiResult.Err]. */
suspend fun <T> apiCall(block: suspend () -> T): ApiResult<T> = try {
    ApiResult.Ok(block())
} catch (e: CancellationException) {
    throw e
} catch (e: Exception) {
    ApiResult.Err(e.toAppError())
}
