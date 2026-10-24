package io.github.jamezuk.lyrionanywhere.data

import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import io.github.jamezuk.lyrionanywhere.core.ApiException
import io.github.jamezuk.lyrionanywhere.core.NoHandle
import io.github.jamezuk.lyrionanywhere.core.PairingInvite
import io.github.jamezuk.lyrionanywhere.core.SecureStore
import io.github.jamezuk.lyrionanywhere.core.Session
import io.github.jamezuk.lyrionanywhere.core.SessionObserver
import io.github.jamezuk.lyrionanywhere.core.SignedIn
import io.github.jamezuk.lyrionanywhere.core.StoredCredentials
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/**
 * The repository's decision on plain `http://` servers (docs/plans/trusted-networks.md):
 * nothing is opened, saved or sent to a private-network server until the user confirms; a
 * name or a public address over http is refused whatever the user says; https is unchanged.
 */
class UnencryptedServerTest {
    @get:Rule val tmp = TemporaryFolder()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    @After
    fun tearDown() = scope.cancel()

    private class NoStore : SecureStore {
        override fun load(): StoredCredentials? = null
        override fun save(credentials: StoredCredentials) {}
        override fun clear() {}
    }

    /**
     * Records what is sent; the calls then fail as if offline (a successful sign-in would
     * build the native library objects, which a JVM test can't).
     */
    private class FakeSession : Session(NoHandle) {
        val pairs = mutableListOf<String>()
        val logins = mutableListOf<String>()

        override fun hasSignIn() = false

        override suspend fun login(username: String, password: String, twoStepCode: String?): SignedIn {
            logins += username
            throw ApiException.Network("test: offline")
        }

        override suspend fun pair(code: String): SignedIn {
            pairs += code
            throw ApiException.Network("test: offline")
        }

        override fun setObserver(observer: SessionObserver) {}

        override fun close() {}
    }

    private class Rig(val repo: RustAccountRepository, val settings: SettingsStore, val opened: MutableList<String>, val sessions: MutableList<Session>)

    private var rigs = 0

    /** The Rust check stands in as "already validated": the address as given. */
    private fun rig(debug: Boolean = false): Rig {
        val settings = DataStoreSettings(PreferenceDataStoreFactory.create(scope = scope) { tmp.root.resolve("s${rigs++}.preferences_pb") })
        val opened = mutableListOf<String>()
        val sessions = mutableListOf<Session>()
        fun info(url: String) = ServerInfo(url, url.substringAfter("://").trimEnd('/'), url.startsWith("https://"))
        val repo = RustAccountRepository(
            settings = settings,
            secureStore = NoStore(),
            deviceName = "Test phone",
            io = Dispatchers.Unconfined,
            allowPlainLoopback = debug,
            parseServer = ::info,
            openSession = { url ->
                opened += url
                info(url) to FakeSession().also { sessions += it }
            },
        )
        return Rig(repo, settings, opened, sessions)
    }

    private val vpn = "http://10.8.0.1:9443/"

    @Test
    fun aPrivateServerIsNotUsedUntilConfirmed() = runBlocking {
        for (debug in listOf(false, true)) {
            val r = rig(debug)
            val refused = r.repo.useServer(vpn)
            assertTrue("$refused", (refused as ApiResult.Err).error.needsUnencryptedConfirmation())
            assertTrue(r.opened.isEmpty())
            assertNull(r.settings.settings.first().serverUrl)
            assertEquals(AuthState.Starting, r.repo.state.value)
            assertTrue(r.repo.checkServerAddress(vpn) is ApiResult.Err)

            val used = r.repo.useServer(vpn, confirmedUnencrypted = true)
            assertTrue("$used", used is ApiResult.Ok)
            assertFalse((used as ApiResult.Ok).value.secure)
            assertEquals(listOf(vpn), r.opened)
            assertEquals(vpn, r.settings.settings.first().serverUrl)
            assertEquals(AuthState.SignedOut(used.value), r.repo.state.value)
            // Signing in then goes ahead (the fake is offline).
            assertEquals(ApiResult.Err(AppError.Offline("test: offline")), r.repo.signIn("alice", "secret password", null))
            assertEquals(listOf("alice"), (r.sessions.single() as FakeSession).logins)
        }
    }

    @Test
    fun namesAndPublicAddressesOverHttpAreRefusedEvenIfConfirmed() = runBlocking {
        for (url in listOf("http://music.example.com/", "http://203.0.113.10:9443/", "http://[2001:db8::1]/")) {
            for (debug in listOf(false, true)) {
                val r = rig(debug)
                for (confirmed in listOf(false, true)) {
                    val got = r.repo.useServer(url, confirmed)
                    assertEquals(url, ApiResult.Err(PLAIN_HTTP_REFUSED), got)
                    assertEquals(url, ApiResult.Err(PLAIN_HTTP_REFUSED), r.repo.checkServerAddress(url, confirmed))
                }
                assertTrue(r.opened.isEmpty())
                assertNull(r.settings.settings.first().serverUrl)
            }
        }
    }

    @Test
    fun loopbackOverHttpStaysADebugBuildOnlyChoice() = runBlocking {
        val loop = "http://127.0.0.1:9443/"
        val release = rig(debug = false)
        assertEquals(ApiResult.Err(PLAIN_HTTP_REFUSED), release.repo.useServer(loop, confirmedUnencrypted = true))
        assertTrue(release.opened.isEmpty())
        val debug = rig(debug = true)
        assertTrue(debug.repo.useServer(loop) is ApiResult.Ok)
    }

    @Test
    fun httpsIsUnchanged() = runBlocking {
        val r = rig()
        val used = r.repo.useServer("https://music.example.com/")
        assertTrue("$used", used is ApiResult.Ok && used.value.secure)
        assertTrue(r.repo.checkServerAddress("https://10.0.0.10/") is ApiResult.Ok)
    }

    @Test
    fun pairingWithAPrivateServerAsksFirst() = runBlocking {
        val r = rig()
        val invite = PairingInvite(vpn, "ABCDEFGHJK")
        val refused = r.repo.pair(invite)
        assertTrue("$refused", (refused as ApiResult.Err).error.needsUnencryptedConfirmation())
        assertTrue("nothing opened or sent", r.opened.isEmpty())

        // Confirmed: the server is used and the code sent (the fake is offline).
        assertEquals(ApiResult.Err(AppError.Offline("test: offline")), r.repo.pair(invite, confirmedUnencrypted = true))
        assertEquals(listOf("ABCDEFGHJK"), (r.sessions.single() as FakeSession).pairs)
        assertFalse((r.repo.state.value as AuthState.SignedOut).server.secure)
    }

    @Test
    fun pairingWithANameOverHttpIsRefused() = runBlocking {
        val r = rig(debug = true)
        val got = r.repo.pair(PairingInvite("http://music.example.com/", "ABCDEFGHJK"), confirmedUnencrypted = true)
        assertEquals(ApiResult.Err(PLAIN_HTTP_REFUSED), got)
        assertTrue(r.opened.isEmpty())
    }
}
