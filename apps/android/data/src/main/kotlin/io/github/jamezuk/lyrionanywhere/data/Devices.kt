package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.MyDevice

/** When a device was last used, in the steps the page says it ("3 hours ago"). */
sealed interface LastUsed {
    /** This phone. */
    data object Now : LastUsed

    /** The server has no record of it being used. */
    data object Never : LastUsed

    /** Within the last two minutes. */
    data object JustNow : LastUsed

    data class Minutes(val n: Int) : LastUsed

    data class Hours(val n: Int) : LastUsed

    data class Days(val n: Int) : LastUsed

    data class Months(val n: Int) : LastUsed
}

/**
 * How long ago [device] was last used, from the server's time (Unix seconds, to the minute)
 * and [nowSeconds]. A phone clock behind the server's never reads as the future.
 */
fun lastUsed(device: MyDevice, nowSeconds: Long): LastUsed {
    if (device.current) return LastUsed.Now
    val seen = device.lastSeen?.takeIf { it > 0 } ?: return LastUsed.Never
    val secs = (nowSeconds - seen).coerceAtLeast(0)
    if (secs < 120) return LastUsed.JustNow
    val minutes = secs / 60
    if (minutes < 120) return LastUsed.Minutes(minutes.toInt())
    val hours = minutes / 60
    if (hours < 48) return LastUsed.Hours(hours.toInt())
    val days = hours / 24
    if (days < 60) return LastUsed.Days(days.toInt())
    return LastUsed.Months((days / 30).coerceAtMost(Int.MAX_VALUE.toLong()).toInt())
}

/** What the last "Your devices" action came to: one message at a time. */
sealed interface DevicesMessage {
    /** [name] was signed out. */
    data class SignedOut(val name: String) : DevicesMessage

    /** [count] other devices were signed out. */
    data class SignedOutOthers(val count: Int) : DevicesMessage

    /** It had gone already (signed out elsewhere): as good as done. */
    data object AlreadyGone : DevicesMessage

    /** It failed; nothing was signed out. */
    data class Failed(val error: AppError) : DevicesMessage
}

/** This phone first, then the others as the server listed them (oldest first). */
fun orderDevices(devices: List<MyDevice>): List<MyDevice> =
    devices.filter { it.current }.take(1) + devices.filterNot { it.current }
