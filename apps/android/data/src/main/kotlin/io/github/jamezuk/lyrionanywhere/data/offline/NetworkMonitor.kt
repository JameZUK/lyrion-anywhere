package io.github.jamezuk.lyrionanywhere.data.offline

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** Which network the phone is on: Wi-Fi (unmetered), mobile data (metered) or none. */
interface NetworkMonitor {
    val kind: StateFlow<NetworkKind>
}

/** What a network's capabilities mean for downloads. Pure: tested on the JVM. */
fun networkKind(hasInternet: Boolean, unmetered: Boolean): NetworkKind = when {
    !hasInternet -> NetworkKind.OFFLINE
    unmetered -> NetworkKind.WIFI
    else -> NetworkKind.MOBILE
}

/**
 * The default network as the app sees it. Android reports the default network lost and
 * then the next one's capabilities within a moment whenever it picks the default again
 * (Wi-Fi scoring, a fallback to mobile data, a validation blip). Passing that loss on
 * dropped the phone player's socket about once a minute (live, 2026-10-15). So:
 * - a loss (or the default network losing INTERNET) becomes OFFLINE only once no default
 *   network with INTERNET has come back for [graceMs];
 * - a network with INTERNET shows at once, Wi-Fi or mobile data (the downloads policy
 *   follows it), and ends any loss waiting out its grace;
 * - a loss of a network other than the current default (the one it replaced, reported
 *   late) is ignored.
 *
 * Networks are only compared, so any value stands for one (Android's `Network`; strings in
 * tests). The grace runs on [scope]: a test dispatcher's clock on the JVM.
 */
class DefaultNetworkTracker(
    initialNetwork: Any?,
    initial: NetworkKind,
    private val scope: CoroutineScope,
    private val graceMs: Long = OFFLINE_GRACE_MS,
) {
    private val _kind = MutableStateFlow(initial)
    val kind: StateFlow<NetworkKind> = _kind.asStateFlow()

    private val lock = Any()
    private var current: Any? = initialNetwork.takeIf { initial != NetworkKind.OFFLINE }
    private var pendingLoss: Job? = null

    /** Bumped by every report: a loss waiting out its grace applies only if none came since. */
    private var generation = 0L

    fun onCapabilities(network: Any, kind: NetworkKind) = synchronized(lock) {
        if (kind == NetworkKind.OFFLINE) {
            // The default network without INTERNET: as good as lost.
            current = null
            startLoss()
            return@synchronized
        }
        generation++
        current = network
        pendingLoss?.cancel()
        pendingLoss = null
        _kind.value = kind
    }

    fun onLost(network: Any) = synchronized(lock) {
        // The network that was replaced, reported after its successor: old news.
        if (current != null && current != network) return@synchronized
        current = null
        startLoss()
    }

    private fun startLoss() {
        if (pendingLoss != null || _kind.value == NetworkKind.OFFLINE) return
        val gen = ++generation
        pendingLoss = scope.launch {
            delay(graceMs)
            synchronized(lock) {
                if (generation == gen) {
                    _kind.value = NetworkKind.OFFLINE
                    pendingLoss = null
                }
            }
        }
    }

    companion object {
        /** How long no default network must last before it counts as none. */
        const val OFFLINE_GRACE_MS = 1_500L
    }
}

/**
 * Follows the default network through `ConnectivityManager` (needs the normal
 * `ACCESS_NETWORK_STATE` permission). "Unmetered" counts as Wi-Fi, so a metered Wi-Fi
 * hotspot is treated as mobile data, as Android itself does for downloads. A loss shows
 * only after a grace (see [DefaultNetworkTracker]).
 */
class AndroidNetworkMonitor(
    context: Context,
    scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default),
) : NetworkMonitor {
    private val cm = context.getSystemService(ConnectivityManager::class.java)
    private val tracker: DefaultNetworkTracker
    override val kind: StateFlow<NetworkKind>

    private fun kindOf(caps: NetworkCapabilities) = networkKind(
        hasInternet = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET),
        unmetered = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED),
    )

    init {
        val active = cm?.activeNetwork
        val caps = active?.let { cm.getNetworkCapabilities(it) }
        tracker = DefaultNetworkTracker(active, caps?.let(::kindOf) ?: NetworkKind.OFFLINE, scope)
        kind = tracker.kind
        cm?.registerDefaultNetworkCallback(
            object : ConnectivityManager.NetworkCallback() {
                override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
                    tracker.onCapabilities(network, kindOf(caps))
                }

                override fun onLost(network: Network) {
                    tracker.onLost(network)
                }
            },
        )
    }
}
