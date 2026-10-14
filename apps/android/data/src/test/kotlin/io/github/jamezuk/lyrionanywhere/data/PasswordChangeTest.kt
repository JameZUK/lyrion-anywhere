package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.ApiException
import io.github.jamezuk.lyrionanywhere.core.OwnDevice
import io.github.jamezuk.lyrionanywhere.core.PasswordChanged
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class PasswordChangeTest {
    private suspend fun failing(e: Exception) = runPasswordChange { throw e }

    @Test
    fun aChangeReportsTheDevicesStillSignedIn() = runTest {
        val kitchen = OwnDevice(2, "Kitchen", "Firefox on Linux", 1_700_000_000)
        assertEquals(
            PasswordChangeResult.Changed(0, listOf(kitchen)),
            runPasswordChange { PasswordChanged(0u, listOf(kitchen)) },
        )
        assertEquals(
            PasswordChangeResult.Changed(3, emptyList()),
            runPasswordChange { PasswordChanged(3u, emptyList()) },
        )
    }

    @Test
    fun aWrongPasswordOrCodeIsSaidSo() = runTest {
        assertEquals(
            PasswordChangeResult.Failed(PasswordChangeError.WrongPassword),
            failing(ApiException.Forbidden("the current password or two-step code is wrong")),
        )
    }

    @Test
    fun aMissingCodeIsAskedFor() = runTest {
        assertEquals(
            PasswordChangeResult.Failed(PasswordChangeError.CodeRequired),
            failing(ApiException.TwoStepRequired()),
        )
    }

    @Test
    fun thePolicysReasonIsShown() = runTest {
        assertEquals(
            PasswordChangeResult.Failed(PasswordChangeError.Refused("password must be at least 10 characters")),
            failing(ApiException.InvalidInput("password must be at least 10 characters")),
        )
    }

    @Test
    fun aLockoutSaysHowManyMinutes() = runTest {
        assertEquals(
            PasswordChangeResult.Failed(PasswordChangeError.LockedOut(15)),
            failing(ApiException.RateLimited(900u)),
        )
        // Rounded up, and never "0 minutes".
        assertEquals(PasswordChangeError.LockedOut(2), AppError.RateLimited(61).toPasswordChangeError())
        assertEquals(PasswordChangeError.LockedOut(1), AppError.RateLimited(1).toPasswordChangeError())
    }

    @Test
    fun networkAndSignInFailuresAreTheirOwn() = runTest {
        assertEquals(
            PasswordChangeResult.Failed(PasswordChangeError.Offline),
            failing(ApiException.Network("timed out")),
        )
        assertEquals(
            PasswordChangeResult.Failed(PasswordChangeError.SignedOut),
            failing(ApiException.Unauthorised()),
        )
        assertEquals(
            PasswordChangeResult.Failed(PasswordChangeError.Other(AppError.Server(500, "internal error"))),
            failing(ApiException.Server(500u, "internal error")),
        )
    }

    @Test
    fun cancellationIsNeverSwallowed() {
        assertThrows(CancellationException::class.java) {
            kotlinx.coroutines.runBlocking { runPasswordChange { throw CancellationException("stop") } }
        }
    }

    @Test
    fun theDefaultRepositoryIsSignedOut() = runTest {
        val repo = object : AccountRepository {
            override val state = kotlinx.coroutines.flow.MutableStateFlow<AuthState>(AuthState.NoServer)
            override suspend fun start() {}
            override fun checkServerAddress(url: String) = throw UnsupportedOperationException()
            override suspend fun useServer(url: String) = throw UnsupportedOperationException()
            override suspend fun forgetServer() {}
            override suspend fun signIn(username: String, password: String, twoStepCode: String?) =
                throw UnsupportedOperationException()
            override fun readPairingCode(text: String) = throw UnsupportedOperationException()
            override suspend fun pair(invite: io.github.jamezuk.lyrionanywhere.core.PairingInvite) =
                throw UnsupportedOperationException()
            override suspend fun retry() {}
            override suspend fun signOut() = true
            override suspend fun account() = throw UnsupportedOperationException()
        }
        assertEquals(
            PasswordChangeResult.Failed(PasswordChangeError.SignedOut),
            repo.changePassword("old password", "new password!", null, false),
        )
    }
}
