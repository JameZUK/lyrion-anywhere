package io.github.jamezuk.lyrionanywhere.ui.car

import android.annotation.SuppressLint
import android.bluetooth.BluetoothClass
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.ContextCompat
import androidx.core.content.IntentCompat
import io.github.jamezuk.lyrionanywhere.data.AudioOutput
import io.github.jamezuk.lyrionanywhere.data.isBluetoothAddress

/** A Bluetooth connection change car mode cares about. */
sealed interface CarBtEvent {
    data class Connected(val device: CarDevice) : CarBtEvent

    data class Disconnected(val address: String) : CarBtEvent
}

/**
 * Car mode's view of Bluetooth (Wave 2 §5.1): which devices are connected and whether each
 * is car audio. Everything needs BLUETOOTH_CONNECT on Android 12+ (BLUETOOTH, granted at
 * install, before), and every call is checked for it and guarded: without it nothing is read
 * and car mode only opens from its button. What is read (names and addresses) stays on the
 * phone.
 */
object CarBluetooth {
    /** The permission asked for when an automatic option is first chosen (Android 12+). */
    // The name itself (Manifest.permission.BLUETOOTH_CONNECT is API 31); it is only asked for there.
    const val PERMISSION = "android.permission.BLUETOOTH_CONNECT"

    fun permissionNeeded(sdk: Int = Build.VERSION.SDK_INT): Boolean = sdk >= Build.VERSION_CODES.S

    fun granted(context: Context): Boolean =
        !permissionNeeded() || ContextCompat.checkSelfPermission(context, PERMISSION) == PackageManager.PERMISSION_GRANTED

    /** Car audio: a car kit or a hands-free unit (the classes car head units announce). */
    fun isCarClass(deviceClass: Int?): Boolean =
        deviceClass == BluetoothClass.Device.AUDIO_VIDEO_CAR_AUDIO || deviceClass == BluetoothClass.Device.AUDIO_VIDEO_HANDSFREE

    /** A device as car mode keeps it (null: unreadable, or not allowed to read it). */
    @SuppressLint("MissingPermission") // Callers check granted() first; a refusal is caught.
    fun deviceOf(d: BluetoothDevice?): CarDevice? {
        if (d == null) return null
        return try {
            val address = d.address?.uppercase() ?: return null
            if (!isBluetoothAddress(address)) return null
            CarDevice(address, AudioOutput.cleanName(d.name), isCarClass(d.bluetoothClass?.deviceClass))
        } catch (_: SecurityException) {
            null
        }
    }

    /**
     * A broadcast as an event: only the two connection actions, with a device; anything else
     * (another action, no device, an unreadable one) is ignored.
     */
    fun eventOf(intent: Intent?): CarBtEvent? {
        val action = intent?.action ?: return null
        if (action != BluetoothDevice.ACTION_ACL_CONNECTED && action != BluetoothDevice.ACTION_ACL_DISCONNECTED) return null
        val device = try {
            IntentCompat.getParcelableExtra(intent, BluetoothDevice.EXTRA_DEVICE, BluetoothDevice::class.java)
        } catch (_: RuntimeException) {
            null
        }
        val d = deviceOf(device) ?: return null
        return if (action == BluetoothDevice.ACTION_ACL_CONNECTED) CarBtEvent.Connected(d) else CarBtEvent.Disconnected(d.address)
    }

    /** The paired devices, for choosing "This device" (empty without the permission). */
    @SuppressLint("MissingPermission")
    fun bonded(context: Context): List<CarDevice> {
        if (!granted(context)) return emptyList()
        return try {
            val adapter = context.getSystemService(BluetoothManager::class.java)?.adapter ?: return emptyList()
            adapter.bondedDevices.orEmpty().mapNotNull(::deviceOf).sortedBy { it.name.lowercase() }
        } catch (_: SecurityException) {
            emptyList()
        }
    }

    /**
     * The audio devices connected now (media and hands-free), handed to [onResult] on the
     * main thread; empty without the permission or with Bluetooth off.
     */
    @SuppressLint("MissingPermission")
    fun connectedNow(context: Context, onResult: (List<CarDevice>) -> Unit) {
        if (!granted(context)) return onResult(emptyList())
        val adapter = try {
            context.getSystemService(BluetoothManager::class.java)?.adapter?.takeIf { it.isEnabled }
        } catch (_: SecurityException) {
            null
        } ?: return onResult(emptyList())
        val profiles = listOf(BluetoothProfile.A2DP, BluetoothProfile.HEADSET)
        val found = LinkedHashMap<String, CarDevice>()
        var pending = profiles.size
        fun done() {
            if (--pending == 0) onResult(found.values.toList())
        }
        for (p in profiles) {
            val listener = object : BluetoothProfile.ServiceListener {
                override fun onServiceConnected(profile: Int, proxy: BluetoothProfile) {
                    try {
                        proxy.connectedDevices.orEmpty().mapNotNull(::deviceOf).forEach { found[it.address] = it }
                    } catch (_: SecurityException) {
                    }
                    runCatching { adapter.closeProfileProxy(profile, proxy) }
                    done()
                }

                override fun onServiceDisconnected(profile: Int) {}
            }
            val asked = try {
                adapter.getProfileProxy(context, listener, p)
            } catch (_: SecurityException) {
                false
            }
            if (!asked) done()
        }
    }
}

/**
 * Listens for Bluetooth connections while the app is in front (registered at run time, not
 * in the manifest, and not exported: only the system's broadcasts reach it).
 */
class CarBluetoothWatcher(private val context: Context, private val onEvent: (CarBtEvent) -> Unit) {
    private var registered = false

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(c: Context, intent: Intent) {
            CarBluetooth.eventOf(intent)?.let(onEvent)
        }
    }

    /** Start listening; false (and nothing registered) without the permission. */
    fun start(): Boolean {
        if (registered) return true
        if (!CarBluetooth.granted(context)) return false
        val filter = IntentFilter().apply {
            addAction(BluetoothDevice.ACTION_ACL_CONNECTED)
            addAction(BluetoothDevice.ACTION_ACL_DISCONNECTED)
        }
        registered = runCatching {
            ContextCompat.registerReceiver(context, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
        }.isSuccess
        return registered
    }

    fun stop() {
        if (!registered) return
        registered = false
        runCatching { context.unregisterReceiver(receiver) }
    }

    val listening: Boolean get() = registered
}
