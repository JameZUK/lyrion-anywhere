package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.MyDevice
import org.junit.Assert.assertEquals
import org.junit.Test

class DevicesTest {
    private val now = 1_800_000_000L

    private fun device(id: Long = 2, current: Boolean = false, lastSeen: Long? = null) =
        MyDevice(id, "Phone", "android", "Pixel 8", 1L, lastSeen, current)

    @Test
    fun lastUsedReadsInSteps() {
        assertEquals(LastUsed.Now, lastUsed(device(current = true, lastSeen = 1), now))
        assertEquals(LastUsed.Never, lastUsed(device(lastSeen = null), now))
        assertEquals(LastUsed.Never, lastUsed(device(lastSeen = 0), now))
        assertEquals(LastUsed.Never, lastUsed(device(lastSeen = -5), now))
        assertEquals(LastUsed.JustNow, lastUsed(device(lastSeen = now - 60), now))
        assertEquals(LastUsed.Minutes(5), lastUsed(device(lastSeen = now - 5 * 60), now))
        assertEquals(LastUsed.Hours(3), lastUsed(device(lastSeen = now - 3 * 3600), now))
        assertEquals(LastUsed.Days(4), lastUsed(device(lastSeen = now - 4 * 86_400), now))
        assertEquals(LastUsed.Months(4), lastUsed(device(lastSeen = now - 120 * 86_400), now))
        // A phone clock behind the server's never reads as the future.
        assertEquals(LastUsed.JustNow, lastUsed(device(lastSeen = now + 3600), now))
        // Nonsense from the server stays bounded.
        assertEquals(LastUsed.Months(Int.MAX_VALUE), lastUsed(device(lastSeen = 1), Long.MAX_VALUE))
    }

    @Test
    fun thisPhoneComesFirstAndOnlyOnce() {
        val a = device(1)
        val me = device(2, current = true)
        val b = device(3)
        val alsoMe = device(4, current = true)
        assertEquals(listOf(me, a, b), orderDevices(listOf(a, me, b, alsoMe)))
        assertEquals(listOf(a, b), orderDevices(listOf(a, b)))
        assertEquals(emptyList<MyDevice>(), orderDevices(emptyList()))
    }
}
