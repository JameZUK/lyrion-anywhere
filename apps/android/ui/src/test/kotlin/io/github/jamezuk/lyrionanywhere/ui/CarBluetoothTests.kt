package io.github.jamezuk.lyrionanywhere.ui

import android.Manifest
import android.app.Application
import android.bluetooth.BluetoothClass
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.content.Intent
import android.os.Looper
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.data.CarAutoOpen
import io.github.jamezuk.lyrionanywhere.data.CarModeSetting
import io.github.jamezuk.lyrionanywhere.ui.car.CarAction
import io.github.jamezuk.lyrionanywhere.ui.car.CarAutoOpenEffect
import io.github.jamezuk.lyrionanywhere.ui.car.CarAutoOpener
import io.github.jamezuk.lyrionanywhere.ui.car.CarBluetooth
import io.github.jamezuk.lyrionanywhere.ui.car.CarBluetoothWatcher
import io.github.jamezuk.lyrionanywhere.ui.car.CarBtEvent
import io.github.jamezuk.lyrionanywhere.ui.car.CarDevice
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

private const val CAR = "0A:1B:2C:3D:4E:5F"
private const val BUDS = "0A:1B:2C:3D:4E:60"

/**
 * Car mode's Bluetooth side (Wave 2 §5.1): the permission is checked before anything is
 * read, only the two connection broadcasts are acted on, and a receiver is registered only
 * while permitted (and never exported).
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class CarBluetoothTests {
    private val app = ApplicationProvider.getApplicationContext<Application>()
    private val adapter = app.getSystemService(BluetoothManager::class.java).adapter

    private fun device(address: String, name: String, deviceClass: Int?): BluetoothDevice {
        val d = adapter.getRemoteDevice(address)
        shadowOf(d).setName(name)
        if (deviceClass != null) {
            val ctor = BluetoothClass::class.java.getDeclaredConstructor(Int::class.javaPrimitiveType)
            ctor.isAccessible = true
            shadowOf(d).setBluetoothClass(ctor.newInstance(deviceClass))
        }
        return d
    }

    private fun acl(action: String, d: BluetoothDevice?) = Intent(action).apply { d?.let { putExtra(BluetoothDevice.EXTRA_DEVICE, it) } }

    private val car get() = device(CAR, "Example Car Kit", BluetoothClass.Device.AUDIO_VIDEO_CAR_AUDIO)
    private val buds get() = device(BUDS, "Example Headphones", BluetoothClass.Device.AUDIO_VIDEO_HEADPHONES)

    @Test
    fun onlyTheConnectionBroadcastsWithADeviceAreRead() {
        assertEquals(CarBtEvent.Connected(CarDevice(CAR, "Example Car Kit", true)), CarBluetooth.eventOf(acl(BluetoothDevice.ACTION_ACL_CONNECTED, car)))
        assertEquals(CarBtEvent.Disconnected(CAR), CarBluetooth.eventOf(acl(BluetoothDevice.ACTION_ACL_DISCONNECTED, car)))
        assertEquals(
            CarBtEvent.Connected(CarDevice(BUDS, "Example Headphones", false)),
            CarBluetooth.eventOf(acl(BluetoothDevice.ACTION_ACL_CONNECTED, buds)),
        )
        // Anything else is nothing.
        assertNull(CarBluetooth.eventOf(null))
        assertNull(CarBluetooth.eventOf(acl(BluetoothDevice.ACTION_ACL_CONNECTED, null)))
        assertNull(CarBluetooth.eventOf(acl(BluetoothDevice.ACTION_BOND_STATE_CHANGED, car)))
        assertNull(CarBluetooth.eventOf(acl("io.example.FAKE_ACL_CONNECTED", car)))
        // A device extra of the wrong type.
        assertNull(CarBluetooth.eventOf(Intent(BluetoothDevice.ACTION_ACL_CONNECTED).putExtra(BluetoothDevice.EXTRA_DEVICE, "0A:1B:2C:3D:4E:5F")))
        // Hands-free counts as car audio; no class at all doesn't.
        assertTrue(CarBluetooth.isCarClass(BluetoothClass.Device.AUDIO_VIDEO_HANDSFREE))
        assertFalse(CarBluetooth.isCarClass(null))
        assertEquals(false, CarBluetooth.deviceOf(device("0A:1B:2C:3D:4E:61", "Unknown", null))?.carAudio)
    }

    @Test
    fun namesAreCleanedBeforeTheyAreKept() {
        val noisy = device("0A:1B:2C:3D:4E:62", "Car\u0000\n" + "x".repeat(200), BluetoothClass.Device.AUDIO_VIDEO_CAR_AUDIO)
        val d = CarBluetooth.deviceOf(noisy)!!
        assertTrue(d.name.none { it.isISOControl() } && d.name.length <= 64)
    }

    @Test
    fun withoutThePermissionNothingIsRegisteredOrRead() {
        shadowOf(app).denyPermissions(Manifest.permission.BLUETOOTH_CONNECT)
        assertTrue(CarBluetooth.permissionNeeded())
        assertFalse(CarBluetooth.granted(app))
        val events = mutableListOf<CarBtEvent>()
        val w = CarBluetoothWatcher(app) { events += it }
        assertFalse(w.start())
        assertFalse(w.listening)
        app.sendBroadcast(acl(BluetoothDevice.ACTION_ACL_CONNECTED, car))
        shadowOf(Looper.getMainLooper()).idle()
        assertTrue(events.isEmpty())
        shadowOf(adapter).setBondedDevices(setOf(car))
        assertTrue(CarBluetooth.bonded(app).isEmpty())
        var connected: List<CarDevice>? = null
        CarBluetooth.connectedNow(app) { connected = it }
        assertEquals(emptyList<CarDevice>(), connected)
    }

    @Test
    fun withThePermissionConnectionsArriveUntilStopped() {
        shadowOf(app).grantPermissions(Manifest.permission.BLUETOOTH_CONNECT)
        val events = mutableListOf<CarBtEvent>()
        val w = CarBluetoothWatcher(app) { events += it }
        assertTrue(w.start())
        assertTrue("twice is harmless", w.start())
        app.sendBroadcast(acl(BluetoothDevice.ACTION_ACL_CONNECTED, car))
        app.sendBroadcast(acl(BluetoothDevice.ACTION_ACL_CONNECTED, null))
        app.sendBroadcast(acl(BluetoothDevice.ACTION_ACL_DISCONNECTED, car))
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(listOf(CarBtEvent.Connected(CarDevice(CAR, "Example Car Kit", true)), CarBtEvent.Disconnected(CAR)), events)
        w.stop()
        w.stop()
        assertFalse(w.listening)
        app.sendBroadcast(acl(BluetoothDevice.ACTION_ACL_CONNECTED, car))
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(2, events.size)
        // The paired devices, for choosing one, sorted by name.
        shadowOf(adapter).setBondedDevices(setOf(car, buds))
        assertEquals(listOf("Example Car Kit", "Example Headphones"), CarBluetooth.bonded(app).map { it.name })
    }

    @Test
    @Config(sdk = [30])
    fun beforeAndroid12NoPermissionIsAskedFor() {
        assertFalse(CarBluetooth.permissionNeeded())
        assertTrue(CarBluetooth.granted(app))
    }
}

/** Car mode opening and leaving by itself, as the app sees Bluetooth while in front. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class CarAutoOpenEffectTests {
    @get:Rule
    val compose = createComposeRule()

    private val app = ApplicationProvider.getApplicationContext<Application>()
    private val adapter = app.getSystemService(BluetoothManager::class.java).adapter

    private fun carDevice(): BluetoothDevice {
        val d = adapter.getRemoteDevice(CAR)
        shadowOf(d).setName("Example Car Kit")
        val ctor = BluetoothClass::class.java.getDeclaredConstructor(Int::class.javaPrimitiveType)
        ctor.isAccessible = true
        shadowOf(d).setBluetoothClass(ctor.newInstance(BluetoothClass.Device.AUDIO_VIDEO_CAR_AUDIO))
        return d
    }

    @Test
    fun aCarConnectingOpensAndDisconnectingLeaves() {
        shadowOf(app).grantPermissions(Manifest.permission.BLUETOOTH_CONNECT)
        val actions = mutableListOf<CarAction>()
        var lost = 0
        compose.setContent {
            CarAutoOpenEffect(CarModeSetting(CarAutoOpen.ANY_CAR), CarAutoOpener(), onAction = { actions += it }, onPermissionLost = { lost++ })
        }
        compose.waitForIdle()
        app.sendBroadcast(Intent(BluetoothDevice.ACTION_ACL_CONNECTED).putExtra(BluetoothDevice.EXTRA_DEVICE, carDevice()))
        shadowOf(Looper.getMainLooper()).idle()
        app.sendBroadcast(Intent(BluetoothDevice.ACTION_ACL_DISCONNECTED).putExtra(BluetoothDevice.EXTRA_DEVICE, carDevice()))
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(listOf(CarAction.OPEN, CarAction.LEAVE), actions.filter { it != CarAction.NONE })
        assertEquals(0, lost)
    }

    @Test
    fun offListensForNothing() {
        shadowOf(app).grantPermissions(Manifest.permission.BLUETOOTH_CONNECT)
        val actions = mutableListOf<CarAction>()
        compose.setContent { CarAutoOpenEffect(CarModeSetting(), CarAutoOpener(), onAction = { actions += it }, onPermissionLost = {}) }
        compose.waitForIdle()
        app.sendBroadcast(Intent(BluetoothDevice.ACTION_ACL_CONNECTED).putExtra(BluetoothDevice.EXTRA_DEVICE, carDevice()))
        shadowOf(Looper.getMainLooper()).idle()
        assertTrue(actions.isEmpty())
    }

    @Test
    fun aRevokedPermissionPutsTheSettingBackToOff() {
        shadowOf(app).denyPermissions(Manifest.permission.BLUETOOTH_CONNECT)
        val actions = mutableListOf<CarAction>()
        var lost = 0
        compose.setContent {
            CarAutoOpenEffect(CarModeSetting(CarAutoOpen.ANY_CAR), CarAutoOpener(), onAction = { actions += it }, onPermissionLost = { lost++ })
        }
        compose.waitForIdle()
        assertEquals(1, lost)
        app.sendBroadcast(Intent(BluetoothDevice.ACTION_ACL_CONNECTED).putExtra(BluetoothDevice.EXTRA_DEVICE, carDevice()))
        shadowOf(Looper.getMainLooper()).idle()
        assertTrue(actions.isEmpty())
    }
}
