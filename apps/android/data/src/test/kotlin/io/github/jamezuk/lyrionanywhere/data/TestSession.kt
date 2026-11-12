package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.AccountInfo
import io.github.jamezuk.lyrionanywhere.core.Analysis
import io.github.jamezuk.lyrionanywhere.core.Art
import io.github.jamezuk.lyrionanywhere.core.Events
import io.github.jamezuk.lyrionanywhere.core.Me
import io.github.jamezuk.lyrionanywhere.core.Menus
import io.github.jamezuk.lyrionanywhere.core.MusicLibrary
import io.github.jamezuk.lyrionanywhere.core.MyDevice
import io.github.jamezuk.lyrionanywhere.core.NoHandle
import io.github.jamezuk.lyrionanywhere.core.PasswordChanged
import io.github.jamezuk.lyrionanywhere.core.Players
import io.github.jamezuk.lyrionanywhere.core.Routine
import io.github.jamezuk.lyrionanywhere.core.Server
import io.github.jamezuk.lyrionanywhere.core.Session
import io.github.jamezuk.lyrionanywhere.core.SessionObserver
import io.github.jamezuk.lyrionanywhere.core.SignedIn
import io.github.jamezuk.lyrionanywhere.core.StationRunner

/**
 * A [Session] for tests that behaves as the generated binding does once closed: every call
 * after [close] throws IllegalStateException, as a destroyed UniFFI object does ("… object
 * has already been destroyed"). Fakes extend it and script what they need through the `on…`
 * hooks, which run only while the session is open; one not scripted fails the call with
 * [UnsupportedOperationException] (never mistaken for a destroyed object).
 *
 * Never override [close] to do nothing in a fake: that hid H1 (a closed session used again).
 */
abstract class TestSession : Session(NoHandle) {
    /** The observer the repository gave (null: none yet). */
    @Volatile var observer: SessionObserver? = null
        private set

    /** How many times [close] was called. */
    @Volatile var closes = 0
        private set

    val closed: Boolean get() = uniffiIsDestroyed

    /** Throws as the binding does once the object is destroyed. */
    protected fun alive() = check(!uniffiIsDestroyed) { "${javaClass.simpleName} object has already been destroyed" }

    private fun unscripted(call: String): Nothing = throw UnsupportedOperationException("$call is not scripted")

    protected open fun onHasSignIn(): Boolean = false
    protected open fun onCurrent(): SignedIn? = null
    protected open suspend fun onRestore(): SignedIn = unscripted("restore")
    protected open suspend fun onLogin(username: String, password: String, twoStepCode: String?): SignedIn = unscripted("login")
    protected open suspend fun onPair(code: String): SignedIn = unscripted("pair")
    protected open suspend fun onLogout(): Boolean = unscripted("logout")
    protected open suspend fun onMe(): SignedIn = unscripted("me")
    protected open suspend fun onAccount(): AccountInfo = unscripted("account")
    protected open suspend fun onChangePassword(current: String, new: String, twoStepCode: String?, signOutOthers: Boolean): PasswordChanged =
        unscripted("changePassword")
    protected open suspend fun onMyDevices(): List<MyDevice> = unscripted("myDevices")
    protected open suspend fun onRefresh(): Unit = unscripted("refresh")
    protected open suspend fun onRenameDevice(name: String): String = unscripted("renameDevice")
    protected open suspend fun onServerProtocol(): UInt? = unscripted("serverProtocol")
    protected open suspend fun onSignOutDevice(deviceId: Long): Boolean = unscripted("signOutDevice")
    protected open suspend fun onSignOutOtherDevices(): UInt = unscripted("signOutOtherDevices")

    final override fun close() {
        closes++
        super.close()
    }

    final override fun setObserver(observer: SessionObserver) {
        alive()
        this.observer = observer
    }

    final override fun hasSignIn(): Boolean {
        alive()
        return onHasSignIn()
    }

    final override fun current(): SignedIn? {
        alive()
        return onCurrent()
    }

    final override suspend fun restore(): SignedIn {
        alive()
        return onRestore()
    }

    final override suspend fun login(username: String, password: String, twoStepCode: String?): SignedIn {
        alive()
        return onLogin(username, password, twoStepCode)
    }

    final override suspend fun pair(code: String): SignedIn {
        alive()
        return onPair(code)
    }

    final override suspend fun logout(): Boolean {
        alive()
        return onLogout()
    }

    final override suspend fun me(): SignedIn {
        alive()
        return onMe()
    }

    final override suspend fun account(): AccountInfo {
        alive()
        return onAccount()
    }

    final override suspend fun changePassword(current: String, new: String, twoStepCode: String?, signOutOthers: Boolean): PasswordChanged {
        alive()
        return onChangePassword(current, new, twoStepCode, signOutOthers)
    }

    final override suspend fun myDevices(): List<MyDevice> {
        alive()
        return onMyDevices()
    }

    final override suspend fun refresh() {
        alive()
        onRefresh()
    }

    final override suspend fun renameDevice(name: String): String {
        alive()
        return onRenameDevice(name)
    }

    final override fun server(): Server {
        alive()
        unscripted("server")
    }

    final override suspend fun serverProtocol(): UInt? {
        alive()
        return onServerProtocol()
    }

    final override suspend fun signOutDevice(deviceId: Long): Boolean {
        alive()
        return onSignOutDevice(deviceId)
    }

    final override suspend fun signOutOtherDevices(): UInt {
        alive()
        return onSignOutOtherDevices()
    }
}

/**
 * A [Connection] over [session] made of objects that never reach Rust (a JVM test can't
 * build the real ones). Closing it destroys them, as it does the real ones.
 */
fun testConnection(server: ServerInfo, session: Session): Connection = Connection(
    server,
    session,
    MusicLibrary(NoHandle),
    Players(NoHandle),
    Art(NoHandle),
    Events(NoHandle),
    Analysis(NoHandle),
    Menus(NoHandle),
    Me(NoHandle),
    StationRunner(NoHandle),
    Routine(NoHandle),
)

/** Whether every object [Connection.close] closes has been closed. */
val Connection.partsClosed: Boolean
    get() = listOf(
        library.uniffiIsDestroyed, players.uniffiIsDestroyed, art.uniffiIsDestroyed, events.uniffiIsDestroyed,
        analysis.uniffiIsDestroyed, menus.uniffiIsDestroyed, me.uniffiIsDestroyed, stations.uniffiIsDestroyed,
        routine.uniffiIsDestroyed,
    ).all { it }
