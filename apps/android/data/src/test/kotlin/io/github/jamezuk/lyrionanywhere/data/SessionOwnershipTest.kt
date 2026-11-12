package io.github.jamezuk.lyrionanywhere.data

import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import io.github.jamezuk.lyrionanywhere.core.ApiException
import io.github.jamezuk.lyrionanywhere.core.PairingInvite
import io.github.jamezuk.lyrionanywhere.core.SecureStore
import io.github.jamezuk.lyrionanywhere.core.SignedIn
import io.github.jamezuk.lyrionanywhere.core.StoredCredentials
import io.github.jamezuk.lyrionanywhere.data.offline.who
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/**
 * H1 (review of 2026-11-12): the account repository owns the Rust session and keeps it for as
 * long as it uses the server. A sign-in's [Connection] closes only its own objects, so signing
 * in again after a sign-out, a revocation or a pairing link uses a live session (the fakes
 * throw once closed, as the binding does). Retry acts only on a sign-in still waiting.
 */
class SessionOwnershipTest {
    @get:Rule val tmp = TemporaryFolder()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    @After
    fun tearDown() = scope.cancel()

    private var rigs = 0

    private val server = ServerInfo("https://music.example.com/", "music.example.com", true)
    private val other = ServerInfo("https://other.example.com/", "other.example.com", true)

    private class MemoryStore : SecureStore {
        var stored: StoredCredentials? = null
        override fun load(): StoredCredentials? = stored
        override fun save(credentials: StoredCredentials) {
            stored = credentials
        }
        override fun clear() {
            stored = null
        }
    }

    /**
     * Signs in as the Rust session does, and tells its observer when the sign-in ends.
     * [resume]: what a restore does with the saved sign-in.
     */
    private class Signer(saved: Boolean, private val resume: (Signer) -> SignedIn) : TestSession() {
        var who: SignedIn? = if (saved) who("alice") else null
        var restores = 0

        override fun onHasSignIn() = who != null
        override fun onCurrent() = who

        override suspend fun onRestore(): SignedIn {
            restores++
            return resume(this)
        }

        override suspend fun onLogin(username: String, password: String, twoStepCode: String?): SignedIn =
            who(username).also { who = it }

        override suspend fun onPair(code: String): SignedIn = who("paired").also { who = it }

        override suspend fun onLogout(): Boolean {
            end(revoked = false)
            return true
        }

        /** The sign-in ends here (as the Rust session's `forget`): the observer is told. */
        fun end(revoked: Boolean) {
            who = null
            observer?.onSignedOut(revoked)
        }
    }

    private inner class Rig(saved: Boolean = false) {
        private val file = tmp.root.resolve("s${rigs++}.preferences_pb")
        val settings = DataStoreSettings(PreferenceDataStoreFactory.create(scope = scope) { file })
        val sessions = mutableListOf<Signer>()

        /** Whether a restore reaches the server. */
        var reachable = true

        /** Set: the server refuses a restore, and the session reports it as revoked (or not). */
        var refusedAsRevoked: Boolean? = null

        /** The restore is refused and the session's observer is told nothing. */
        var refusedUntold = false

        val repo = RustAccountRepository(
            settings = settings,
            secureStore = MemoryStore(),
            deviceName = "Test phone",
            io = Dispatchers.Unconfined,
            parseServer = { url -> info(url) },
            openSession = { url -> info(url) to Signer(saved, ::restoreOn).also { sessions += it } },
            connect = { info, s -> testConnection(info, s) },
        )

        fun info(url: String) = if (url == other.url) other else server

        private fun restoreOn(s: Signer): SignedIn {
            if (!reachable) throw ApiException.Network("test: no network")
            if (refusedUntold) throw ApiException.Unauthorised()
            refusedAsRevoked?.let { revoked ->
                // As the Rust session does: the observer first, then the error.
                s.end(revoked)
                throw ApiException.Unauthorised()
            }
            return s.who ?: throw ApiException.Unauthorised()
        }

        val session: Signer get() = sessions.last()
        val connection: Connection? get() = repo.connection.value

        /** Signed in on [server] with a password. */
        suspend fun signedIn(): Rig {
            assertTrue(repo.useServer(server.url) is ApiResult.Ok)
            assertEquals(ApiResult.Ok(Unit), repo.signIn("alice", "secret", null))
            return this
        }
    }

    private fun assertSignedIn(r: Rig, user: String) {
        val s = r.repo.state.value
        assertTrue("got $s", s is AuthState.SignedInState)
        assertEquals(user, (s as AuthState.SignedInState).who.user.username)
        val c = r.connection
        assertTrue("a connection while signed in", c != null)
        assertSame("the connection is over the repository's session", r.session, c!!.session)
        assertFalse("its objects are open", c.partsClosed)
        assertFalse("the session is open", r.session.closed)
    }

    private fun refusedToSignOutFirst(r: ApiResult<Unit>) =
        r is ApiResult.Err && (r.error as? AppError.InvalidInput)?.reason == AppError.Reason.SIGN_OUT_FIRST

    @Test
    fun signingInAgainAfterASignOutUsesTheLiveSession() = runBlocking {
        val r = Rig().signedIn()
        val first = r.connection!!
        assertTrue(r.repo.signOut())
        assertEquals(AuthState.SignedOut(server), r.repo.state.value)
        assertNull(r.connection)
        assertTrue("the sign-in's objects are closed", first.partsClosed)
        assertFalse("the session is not: the repository still uses it", r.session.closed)

        assertEquals(ApiResult.Ok(Unit), r.repo.signIn("bob", "secret", null))
        assertSignedIn(r, "bob")
        assertEquals(1, r.sessions.size)
        assertNotSame(first, r.connection)
    }

    @Test
    fun signingInAgainAfterARevocationUsesTheLiveSession() = runBlocking {
        val r = Rig().signedIn()
        val first = r.connection!!
        // The server ended the sign-in (a refused refresh, from any call).
        r.session.end(revoked = true)
        assertEquals(AuthState.SignedOut(server, revoked = true), r.repo.state.value)
        assertNull(r.connection)
        assertTrue(first.partsClosed)
        assertFalse(r.session.closed)

        assertEquals(ApiResult.Ok(Unit), r.repo.signIn("alice", "secret", null))
        assertSignedIn(r, "alice")
    }

    @Test
    fun anEndedSignInIsNotARevocation() = runBlocking {
        val r = Rig().signedIn()
        // An expired sign-in, as the engine reports it: ended, not revoked (downloads stay).
        r.session.end(revoked = false)
        assertEquals(AuthState.SignedOut(server, revoked = false), r.repo.state.value)
        assertEquals(ApiResult.Ok(Unit), r.repo.signIn("alice", "secret", null))
        assertSignedIn(r, "alice")
    }

    @Test
    fun aPairingLinkToTheSameServerWhileSignedIn() = runBlocking {
        val r = Rig().signedIn()
        val first = r.connection!!
        val invite = PairingInvite(server.url, "ABCDEFGHJK")
        // Straight to pair() while signed in: refused, and nothing of the sign-in changes.
        val refused = r.repo.pair(invite)
        assertTrue("$refused", refusedToSignOutFirst(refused))
        assertSame(first, r.connection)
        assertSignedIn(r, "alice")

        // The app's way: the user agrees to sign out, then pairs on the pairing screen.
        r.repo.signOut()
        assertTrue(first.partsClosed)
        assertEquals(ApiResult.Ok(Unit), r.repo.pair(invite))
        assertSignedIn(r, "paired")
        assertEquals(1, r.sessions.size)
    }

    @Test
    fun signingInWhileSignedInIsRefused() = runBlocking {
        val r = Rig().signedIn()
        val first = r.connection!!
        val refused = r.repo.signIn("bob", "secret", null)
        assertTrue("$refused", refusedToSignOutFirst(refused))
        assertSame(first, r.connection)
        assertSignedIn(r, "alice")
    }

    @Test
    fun retryWhileSignedInLeavesTheSignInAlone() = runBlocking {
        val r = Rig(saved = true)
        r.settings.setServerUrl(server.url)
        r.repo.start()
        assertSignedIn(r, "alice")
        val first = r.connection!!
        val state = r.repo.state.value
        assertEquals(1, r.session.restores)

        // Offline mode's Retry while signed in, or a retry loop that lost the race.
        r.repo.retry()
        assertEquals(1, r.session.restores)
        assertSame(state, r.repo.state.value)
        assertSame(first, r.connection)
        assertSignedIn(r, "alice")
    }

    @Test
    fun retryWhenUnreachableRestoresAndOtherwiseDoesNothing() = runBlocking {
        val r = Rig(saved = true)
        r.settings.setServerUrl(server.url)
        r.reachable = false
        r.repo.start()
        assertEquals(AuthState.Unreachable(server, "test: no network"), r.repo.state.value)
        r.reachable = true
        r.repo.retry()
        assertSignedIn(r, "alice")

        // Signed out, there is nothing to retry.
        r.repo.signOut()
        val restores = r.session.restores
        r.repo.retry()
        assertEquals(restores, r.session.restores)
        assertEquals(AuthState.SignedOut(server), r.repo.state.value)
    }

    @Test
    fun aRestoreTheServerRefusesIsARevocationOnlyIfTheSessionSaysSo() = runBlocking {
        for (revoked in listOf(true, false)) {
            val r = Rig(saved = true)
            r.settings.setServerUrl(server.url)
            r.refusedAsRevoked = revoked
            r.repo.start()
            assertEquals("revoked = $revoked", AuthState.SignedOut(server, revoked), r.repo.state.value)
            assertNull(r.connection)
        }
    }

    @Test
    fun aRestoreRefusedWithoutTheSessionSayingWhyIsNotARevocation() = runBlocking {
        val r = Rig(saved = true)
        r.settings.setServerUrl(server.url)
        // Refused, with nothing saying the server revoked this device: its downloads stay.
        r.refusedUntold = true
        r.repo.start()
        assertEquals(1, r.session.restores)
        assertEquals(AuthState.SignedOut(server, revoked = false), r.repo.state.value)
    }

    @Test
    fun theRepositoryClosesASessionOnlyWhenItLeavesTheServer() = runBlocking {
        val r = Rig().signedIn()
        val first = r.session
        r.repo.signOut()
        assertFalse(first.closed)
        assertTrue(r.repo.useServer(other.url) is ApiResult.Ok)
        assertEquals(1, first.closes)
        assertFalse(r.session.closed)
        r.repo.forgetServer()
        assertTrue(r.session.closed)
        assertEquals(AuthState.NoServer, r.repo.state.value)
    }

    @Test
    fun aReplacedSessionCanNoLongerSignTheAppOut() = runBlocking {
        val r = Rig().signedIn()
        val old = r.session
        r.repo.signOut()
        assertTrue(r.repo.useServer(other.url) is ApiResult.Ok)
        assertEquals(ApiResult.Ok(Unit), r.repo.signIn("alice", "secret", null))
        val live = r.connection!!
        // The old session's engine, still finishing, reports its sign-in ended.
        old.observer!!.onSignedOut(true)
        assertSame(live, r.connection)
        assertTrue(r.repo.state.value is AuthState.SignedInState)
        assertFalse(live.partsClosed)
    }
}
