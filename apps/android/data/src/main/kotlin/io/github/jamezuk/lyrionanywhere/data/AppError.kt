package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.ApiException
import kotlin.coroutines.cancellation.CancellationException

/** Why something failed, in terms the UI can act on. Never carries tokens or passwords. */
sealed interface AppError {
    /** The server can't be reached. The sign-in is kept; try again later. */
    data object Offline : AppError

    /** Not signed in (or the sign-in ended). */
    data object SignedOut : AppError

    /** The password was right; a two-step code is needed. */
    data object TwoStepRequired : AppError

    /** Refused by the server's policy. */
    data class Forbidden(val detail: String) : AppError

    /** Too many attempts: wait this long. */
    data class RateLimited(val retryAfterSeconds: Long) : AppError

    /** The server or the music server failed. */
    data class Server(val status: Int, val detail: String) : AppError

    /** The input was refused (the detail is fit to show). */
    data class InvalidInput(val detail: String) : AppError

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

/** Map anything thrown by the Rust layer (cancellation is never swallowed). */
fun Throwable.toAppError(): AppError = when (this) {
    is CancellationException -> throw this
    is ApiException.Network -> AppError.Offline
    is ApiException.Unauthorised -> AppError.SignedOut
    is ApiException.TwoStepRequired -> AppError.TwoStepRequired
    is ApiException.Forbidden -> AppError.Forbidden(detail)
    is ApiException.RateLimited -> AppError.RateLimited(retryAfterSecs.toLong())
    is ApiException.Server -> AppError.Server(status.toInt(), detail)
    is ApiException.InvalidInput -> AppError.InvalidInput(detail)
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
