package io.github.jamezuk.lyrionanywhere.data.offline

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

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
 * Follows the default network through `ConnectivityManager` (needs the normal
 * `ACCESS_NETWORK_STATE` permission). "Unmetered" counts as Wi-Fi, so a metered Wi-Fi
 * hotspot is treated as mobile data, as Android itself does for downloads.
 */
class AndroidNetworkMonitor(context: Context) : NetworkMonitor {
    private val cm = context.getSystemService(ConnectivityManager::class.java)
    private val _kind = MutableStateFlow(current())
    override val kind: StateFlow<NetworkKind> = _kind.asStateFlow()

    private fun current(): NetworkKind {
        val caps = cm?.activeNetwork?.let { cm.getNetworkCapabilities(it) } ?: return NetworkKind.OFFLINE
        return kindOf(caps)
    }

    private fun kindOf(caps: NetworkCapabilities) = networkKind(
        hasInternet = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET),
        unmetered = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED),
    )

    init {
        cm?.registerDefaultNetworkCallback(
            object : ConnectivityManager.NetworkCallback() {
                override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
                    _kind.value = kindOf(caps)
                }

                override fun onLost(network: Network) {
                    _kind.value = NetworkKind.OFFLINE
                }
            },
        )
    }
}
