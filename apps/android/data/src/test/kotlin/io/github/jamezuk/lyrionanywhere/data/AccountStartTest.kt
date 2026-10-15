package io.github.jamezuk.lyrionanywhere.data

import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import io.github.jamezuk.lyrionanywhere.core.ApiException
import io.github.jamezuk.lyrionanywhere.core.NoHandle
import io.github.jamezuk.lyrionanywhere.core.SecureStore
import io.github.jamezuk.lyrionanywhere.core.Session
import io.github.jamezuk.lyrionanywhere.core.SessionObserver
import io.github.jamezuk.lyrionanywhere.core.SignedIn
import io.github.jamezuk.lyrionanywhere.core.StoreException
import io.github.jamezuk.lyrionanywhere.core.StoredCredentials
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/**
 * M6: a saved sign-in that secure storage can't read just now (the Keystore restarting, an
 * I/O error) is not a sign-out. Start-up shows "can't open secure storage" with Retry, the
 * stored sign-in is left alone and the server is not told anything.
 */
class AccountStartTest {
    @get:Rule val tmp = TemporaryFolder()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    @After
    fun tearDown() = scope.cancel()

    private val server = ServerInfo("https://music.example.com", "music.example.com", true)
    private val saved = StoredCredentials(refreshToken = "saved-token", deviceId = 7)

    /** A store that is unreadable while [unavailable], else holds [saved] (or nothing). */
    private inner class FlakyStore : SecureStore {
        var unavailable = true
        var stored: StoredCredentials? = saved
        var clears = 0
        var saves = 0

        override fun load(): StoredCredentials? {
            if (unavailable) throw StoreException.Failed("unavailable: ProviderException")
            return stored
        }

        override fun save(credentials: StoredCredentials) {
            saves++
            stored = credentials
        }

        override fun clear() {
            clears++
            stored = null
        }
    }

    /** Behaves as the Rust session does: a store that fails to load reads as no sign-in. */
    private class FakeSession(store: SecureStore) : Session(NoHandle) {
        val refresh: String? = try {
            store.load()?.refreshToken
        } catch (_: Exception) {
            null
        }
        var restores = 0
        var logouts = 0

        override fun hasSignIn() = refresh != null

        override suspend fun restore(): SignedIn {
            restores++
            throw ApiException.Network("test: no network")
        }

        override suspend fun logout(): Boolean {
            logouts++
            return true
        }

        override fun setObserver(observer: SessionObserver) {}

        override fun close() {}
    }

    private fun rig(store: FlakyStore): Pair<RustAccountRepository, MutableList<FakeSession>> {
        val settings = DataStoreSettings(PreferenceDataStoreFactory.create(scope = scope) { tmp.root.resolve("s.preferences_pb") })
        runBlocking { settings.setServerUrl(server.url) }
        val sessions = mutableListOf<FakeSession>()
        val repo = RustAccountRepository(
            settings = settings,
            secureStore = store,
            deviceName = "Test phone",
            io = Dispatchers.Unconfined,
            openSession = { _ -> server to FakeSession(store).also { sessions += it } },
        )
        return repo to sessions
    }

    @Test
    fun anUnreadableStoreAtStartIsNotASignOut() = runBlocking {
        val store = FlakyStore()
        val (repo, sessions) = rig(store)
        repo.start()
        val state = repo.state.value
        assertTrue("got $state", state is AuthState.Unreachable && state.secureStorage)
        assertEquals(0, store.clears)
        assertEquals(0, store.saves)
        assertEquals(saved, store.stored)
        assertEquals(0, sessions.sumOf { it.logouts })
    }

    @Test
    fun retryOnceTheStoreIsReadableResumesTheSavedSignIn() = runBlocking {
        val store = FlakyStore()
        val (repo, sessions) = rig(store)
        repo.start()
        // Still unreadable: still waiting, nothing forgotten.
        repo.retry()
        assertTrue((repo.state.value as AuthState.Unreachable).secureStorage)
        store.unavailable = false
        repo.retry()
        // The saved sign-in is now checked with the server (which the fake can't reach).
        assertEquals(1, sessions.last().restores)
        assertEquals(AuthState.Unreachable(server, "test: no network"), repo.state.value)
        assertEquals(0, store.clears)
        assertEquals(0, sessions.sumOf { it.logouts })
    }

    @Test
    fun nothingSavedIsStillSignedOut() = runBlocking {
        val store = FlakyStore().apply {
            unavailable = false
            stored = null
        }
        val (repo, _) = rig(store)
        repo.start()
        assertEquals(AuthState.SignedOut(server), repo.state.value)
    }
}
