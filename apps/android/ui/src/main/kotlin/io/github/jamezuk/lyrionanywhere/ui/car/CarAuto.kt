package io.github.jamezuk.lyrionanywhere.ui.car

import io.github.jamezuk.lyrionanywhere.data.CarAutoOpen
import io.github.jamezuk.lyrionanywhere.data.CarModeSetting
import io.github.jamezuk.lyrionanywhere.data.isBluetoothAddress

/** A Bluetooth device as car mode sees it: its address, name and whether it is car audio. */
data class CarDevice(val address: String, val name: String, val carAudio: Boolean)

/** What car mode should do after something happened. */
enum class CarAction { NONE, OPEN, LEAVE }

/**
 * When car mode opens and leaves by itself (Wave 2 §5.1). Pure, tested on the JVM.
 *
 * - **Opens** while the app is in front, or when it is next opened, if a matching car is
 *   connected (Android doesn't let an app open itself from the background).
 * - **Leaves** when the car that opened it disconnects; car mode opened by its button never
 *   leaves by itself.
 * - Leaving by hand while the car is still connected isn't undone: it opens again only after
 *   that car disconnects and comes back.
 */
class CarAutoOpener {
    private val connected = LinkedHashMap<String, CarDevice>()
    private val dismissed = mutableSetOf<String>()

    /** The car whose connection opened car mode (null: closed, or opened by hand). */
    var openedBy: String? = null
        private set

    /** Car mode is showing. */
    var open = false
        private set

    /** Whether [d] is the car [setting] waits for. */
    fun matches(setting: CarModeSetting, d: CarDevice): Boolean = when (setting.auto) {
        CarAutoOpen.OFF -> false
        CarAutoOpen.ANY_CAR -> d.carAudio
        CarAutoOpen.THIS_DEVICE -> isBluetoothAddress(setting.deviceAddress) && d.address.equals(setting.deviceAddress, ignoreCase = true)
    }

    /** A device connected (seen while the app is in front). */
    fun connected(d: CarDevice, setting: CarModeSetting, foreground: Boolean): CarAction {
        if (!isBluetoothAddress(d.address)) return CarAction.NONE
        connected[d.address.uppercase()] = d.copy(address = d.address.uppercase())
        return check(setting, foreground)
    }

    /** A device disconnected: car mode leaves if that car opened it. */
    fun disconnected(address: String): CarAction {
        val a = address.uppercase()
        connected.remove(a)
        dismissed.remove(a)
        if (open && openedBy == a) {
            open = false
            openedBy = null
            return CarAction.LEAVE
        }
        return CarAction.NONE
    }

    /**
     * The devices connected now (read as the app comes to the front): forget the rest, leave
     * if the car that opened car mode has gone meanwhile, else open for a matching car.
     */
    fun snapshot(devices: List<CarDevice>, setting: CarModeSetting, foreground: Boolean): CarAction {
        val now = devices.filter { isBluetoothAddress(it.address) }.associateBy { it.address.uppercase() }
        connected.clear()
        now.forEach { (a, d) -> connected[a] = d.copy(address = a) }
        dismissed.retainAll(now.keys)
        val by = openedBy
        if (open && by != null && by !in now) {
            open = false
            openedBy = null
            return CarAction.LEAVE
        }
        return check(setting, foreground)
    }

    /** Open for a matching connected car, if car mode isn't showing already. */
    fun check(setting: CarModeSetting, foreground: Boolean): CarAction {
        if (!foreground || open || setting.auto == CarAutoOpen.OFF) return CarAction.NONE
        val car = connected.values.firstOrNull { matches(setting, it) && it.address !in dismissed } ?: return CarAction.NONE
        open = true
        openedBy = car.address
        return CarAction.OPEN
    }

    /** Opened by its button: it stays until left by hand. */
    fun openedByHand() {
        open = true
        openedBy = null
    }

    /** Left by hand: not opened again for the cars connected now, until they reconnect. */
    fun leftByHand(setting: CarModeSetting) {
        connected.values.filter { matches(setting, it) }.forEach { dismissed += it.address }
        open = false
        openedBy = null
    }
}

/**
 * Choosing how car mode opens (Settings). The automatic choices need Android's
 * BLUETOOTH_CONNECT permission (Android 12+), asked for only when one is first chosen.
 */
object CarSettingFlow {
    enum class Step {
        /** Keep the choice as it is. */
        APPLY,

        /** Ask for the permission first. */
        ASK,

        /** Pick the device (from the paired ones). */
        PICK,

        /** The permission was refused: back to Off, with a note. */
        REFUSED,
    }

    /** What choosing [option] leads to, with or without the permission. */
    fun choose(option: CarAutoOpen, granted: Boolean): Step = when {
        option == CarAutoOpen.OFF -> Step.APPLY
        !granted -> Step.ASK
        option == CarAutoOpen.THIS_DEVICE -> Step.PICK
        else -> Step.APPLY
    }

    /** After Android answered the permission prompt for [option]. */
    fun answered(option: CarAutoOpen, granted: Boolean): Step = when {
        !granted -> Step.REFUSED
        option == CarAutoOpen.THIS_DEVICE -> Step.PICK
        else -> Step.APPLY
    }

    /** The setting kept for a device picked from the paired ones. */
    fun picked(d: CarDevice): CarModeSetting =
        CarModeSetting(CarAutoOpen.THIS_DEVICE, d.name, d.address).sanitised()

    /** A setting that needs the permission, when it is gone (revoked in Android's settings): Off. */
    fun withoutPermission(setting: CarModeSetting, granted: Boolean): CarModeSetting =
        if (!granted && setting.auto != CarAutoOpen.OFF) CarModeSetting() else setting
}
