package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.AccountInfo
import io.github.jamezuk.lyrionanywhere.core.Analysis
import io.github.jamezuk.lyrionanywhere.core.Art
import io.github.jamezuk.lyrionanywhere.core.Events
import io.github.jamezuk.lyrionanywhere.core.MusicLibrary
import io.github.jamezuk.lyrionanywhere.core.PairingInvite
import io.github.jamezuk.lyrionanywhere.core.Players
import io.github.jamezuk.lyrionanywhere.core.SecureStore
import io.github.jamezuk.lyrionanywhere.core.Server
import io.github.jamezuk.lyrionanywhere.core.Session
import io.github.jamezuk.lyrionanywhere.core.SessionObserver
import io.github.jamezuk.lyrionanywhere.core.SignedIn
import io.github.jamezuk.lyrionanywhere.core.parsePairingInvite
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** A validated server address. */
data class ServerInfo(val url: String, val name: String, val secure: Boolean)

/** Where the app stands with its server. */
sealed interface AuthState {
    /** Reading settings and the saved sign-in. */
    data object Starting : AuthState

    /** No server chosen yet. */
    data object NoServer : AuthState

    /** A server, but nobody signed in. `revoked`: the server ended the last sign-in. */
    data class SignedOut(val server: ServerInfo, val revoked: Boolean = false) : AuthState

    /** A saved sign-in is being checked with the server. */
    data class Restoring(val server: ServerInfo) : AuthState

    /** A saved sign-in, but the server can't be reached (it is kept). */
    data class Unreachable(val server: ServerInfo, val detail: String = "") : AuthState

    data class SignedInState(val server: ServerInfo, val who: SignedIn) : AuthState
}

/** The Rust objects for the signed-in server. */
class Connection(val server: ServerInfo, val session: Session) {
    val library = MusicLibrary(session)
    val players = Players(session)
    val art = Art(session)
    val events = Events(session)
    val analysis = Analysis(session)

    fun close() {
        listOf(library, players, art, events, analysis, session).forEach { it.close() }
    }
}

interface ConnectionSource {
    /** Non-null while signed in. */
    val connection: StateFlow<Connection?>
}

interface AccountRepository {
    val state: StateFlow<AuthState>

    /** Read settings and resume a saved sign-in. */
    suspend fun start()

    /** Check an address without contacting it. */
    fun checkServerAddress(url: String): ApiResult<ServerInfo>

    /** Use this server (only while signed out). */
    suspend fun useServer(url: String): ApiResult<ServerInfo>

    /** Forget the server (only while signed out). */
    suspend fun forgetServer()

    suspend fun signIn(username: String, password: String, twoStepCode: String?): ApiResult<Unit>

    /** Read a scanned QR code or a typed pairing code. */
    fun readPairingCode(text: String): ApiResult<PairingInvite>

    /** Pair with a code; a QR code that names a server switches to it first. */
    suspend fun pair(invite: PairingInvite): ApiResult<Unit>

    /** Try the saved sign-in again (after [AuthState.Unreachable]). */
    suspend fun retry()

    /** Sign out. Returns false if the server couldn't be told (signed out here anyway). */
    suspend fun signOut(): Boolean

    suspend fun account(): ApiResult<AccountInfo>
}

/**
 * The real account repository over the Rust [Session]. Blocking work (creating the session
 * reads the Keystore-sealed token) runs on [io].
 */
class RustAccountRepository(
    private val settings: SettingsStore,
    private val secureStore: SecureStore,
    private val deviceName: String,
    private val io: CoroutineDispatcher,
) : AccountRepository, ConnectionSource {
    private val _state = MutableStateFlow<AuthState>(AuthState.Starting)
    override val state: StateFlow<AuthState> = _state.asStateFlow()
    private val _connection = MutableStateFlow<Connection?>(null)
    override val connection: StateFlow<Connection?> = _connection.asStateFlow()

    private val lock = Mutex()
    private var session: Session? = null
    private var server: ServerInfo? = null

    private val observer = object : SessionObserver {
        override fun onSignedOut(revoked: Boolean) {
            val s = server ?: return
            dropConnection()
            _state.value = AuthState.SignedOut(s, revoked)
        }
    }

    private fun info(s: Server) = ServerInfo(s.url(), s.displayName(), s.isSecure())

    private fun dropConnection() {
        _connection.value?.close()
        _connection.value = null
    }

    override fun checkServerAddress(url: String): ApiResult<ServerInfo> = try {
        Server(url).use { ApiResult.Ok(info(it)) }
    } catch (e: Exception) {
        ApiResult.Err(e.toAppError())
    }

    /** Create the session for `url` (on [io]: it reads the sealed token). */
    private suspend fun open(url: String): ApiResult<ServerInfo> = withContext(io) {
        try {
            val s = Server(url)
            val info = info(s)
            val sess = Session(s, secureStore, deviceName)
            sess.setObserver(observer)
            session?.close()
            dropConnection()
            session = sess
            server = info
            ApiResult.Ok(info)
        } catch (e: Exception) {
            ApiResult.Err(e.toAppError())
        }
    }

    override suspend fun start() = lock.withLock {
        val url = settings.settings.first().serverUrl
        if (url == null) {
            _state.value = AuthState.NoServer
            return@withLock
        }
        when (val opened = open(url)) {
            is ApiResult.Err -> _state.value = AuthState.NoServer
            is ApiResult.Ok -> restoreLocked(opened.value)
        }
    }

    private suspend fun restoreLocked(info: ServerInfo) {
        val sess = session ?: return
        if (!sess.hasSignIn()) {
            _state.value = AuthState.SignedOut(info)
            return
        }
        _state.value = AuthState.Restoring(info)
        when (val r = apiCall { sess.restore() }) {
            is ApiResult.Ok -> signedIn(info, sess, r.value)
            is ApiResult.Err -> _state.value = when (val e = r.error) {
                is AppError.Offline -> AuthState.Unreachable(info, e.detail)
                is AppError.RateLimited, is AppError.Server -> AuthState.Unreachable(info)
                else -> AuthState.SignedOut(info, revoked = true)
            }
        }
    }

    private fun signedIn(info: ServerInfo, sess: Session, who: SignedIn) {
        dropConnection()
        _connection.value = Connection(info, sess)
        _state.value = AuthState.SignedInState(info, who)
    }

    override suspend fun retry() = lock.withLock {
        server?.let { restoreLocked(it) } ?: Unit
    }

    override suspend fun useServer(url: String): ApiResult<ServerInfo> = lock.withLock {
        if (_state.value is AuthState.SignedInState) {
            return@withLock ApiResult.Err(AppError.InvalidInput("sign out first"))
        }
        val checked = checkServerAddress(url)
        if (checked is ApiResult.Err) return@withLock checked
        // A new server: any sign-in saved for the old one is no use.
        withContext(io) { runCatching { secureStore.clear() } }
        val r = open(url)
        if (r is ApiResult.Ok) {
            settings.setServerUrl(r.value.url)
            _state.value = AuthState.SignedOut(r.value)
        }
        r
    }

    override suspend fun forgetServer() = lock.withLock {
        if (_state.value is AuthState.SignedInState) return@withLock
        session?.close()
        session = null
        server = null
        withContext(io) { runCatching { secureStore.clear() } }
        settings.setServerUrl(null)
        _state.value = AuthState.NoServer
    }

    override suspend fun signIn(
        username: String,
        password: String,
        twoStepCode: String?,
    ): ApiResult<Unit> = lock.withLock {
        val sess = session ?: return@withLock ApiResult.Err(AppError.InvalidInput("choose a server first"))
        val info = server ?: return@withLock ApiResult.Err(AppError.InvalidInput("choose a server first"))
        apiCall { sess.login(username, password, twoStepCode) }.map { signedIn(info, sess, it) }
    }

    override fun readPairingCode(text: String): ApiResult<PairingInvite> = try {
        ApiResult.Ok(parsePairingInvite(text))
    } catch (e: Exception) {
        ApiResult.Err(e.toAppError())
    }

    override suspend fun pair(invite: PairingInvite): ApiResult<Unit> {
        val target = invite.server
        if (target != null && target != server?.url) {
            val switched = useServer(target)
            if (switched is ApiResult.Err) return switched
        }
        return lock.withLock {
            val sess = session ?: return@withLock ApiResult.Err(AppError.InvalidInput("choose a server first"))
            val info = server ?: return@withLock ApiResult.Err(AppError.InvalidInput("choose a server first"))
            apiCall { sess.pair(invite.code) }.map { signedIn(info, sess, it) }
        }
    }

    override suspend fun signOut(): Boolean = lock.withLock {
        val sess = session ?: return@withLock true
        val info = server
        val confirmed = (apiCall { sess.logout() } as? ApiResult.Ok)?.value ?: false
        dropConnection()
        if (info != null) _state.value = AuthState.SignedOut(info)
        confirmed
    }

    override suspend fun account(): ApiResult<AccountInfo> {
        val sess = _connection.value?.session ?: return ApiResult.Err(AppError.SignedOut)
        return apiCall { sess.account() }
    }
}
