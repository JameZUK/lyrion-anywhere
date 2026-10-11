package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.ApiException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class AppErrorTest {
    @Test
    fun rustErrorsMapToAppErrors() {
        assertEquals(AppError.Offline("timed out"), ApiException.Network("timed out").toAppError())
        assertEquals(AppError.SignedOut, ApiException.Unauthorised().toAppError())
        assertEquals(AppError.TwoStepRequired, ApiException.TwoStepRequired().toAppError())
        assertEquals(AppError.Forbidden("no"), ApiException.Forbidden("no").toAppError())
        assertEquals(AppError.RateLimited(90), ApiException.RateLimited(90u).toAppError())
        assertEquals(AppError.Server(502, "x"), ApiException.Server(502u, "x").toAppError())
        assertEquals(AppError.InvalidInput("bad"), ApiException.InvalidInput("bad").toAppError())
        assertEquals(AppError.Unexpected("IllegalStateException"), IllegalStateException("secret").toAppError())
    }

    @Test
    fun apiCallWrapsFailuresButNeverCancellation() = runTest {
        assertEquals(ApiResult.Ok(3), apiCall { 3 })
        assertEquals(ApiResult.Err(AppError.Offline("x")), apiCall { throw ApiException.Network("x") })
        assertThrows(CancellationException::class.java) {
            kotlinx.coroutines.runBlocking { apiCall { throw CancellationException("stop") } }
        }
    }
}
