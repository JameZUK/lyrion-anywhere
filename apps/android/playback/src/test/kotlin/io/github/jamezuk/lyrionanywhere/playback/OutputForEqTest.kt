package io.github.jamezuk.lyrionanywhere.playback

import android.media.AudioDeviceInfo
import io.github.jamezuk.lyrionanywhere.data.AudioOutput
import io.github.jamezuk.lyrionanywhere.data.OutputClass
import org.junit.Assert.assertEquals
import org.junit.Test

/** Per-output EQ (Wave 2 §5.3): what Android reports, as the output a profile is bound to. */
class OutputForEqTest {
    @Test
    fun eachKindOfDeviceMapsToItsOutput() {
        assertEquals(AudioOutput(OutputClass.SPEAKER), audioOutputOf(null))
        assertEquals(AudioOutput(OutputClass.SPEAKER), audioOutputOf(OutputDevice(1, AudioDeviceInfo.TYPE_BUILTIN_SPEAKER, "Phone")))
        assertEquals(AudioOutput(OutputClass.WIRED), audioOutputOf(OutputDevice(2, AudioDeviceInfo.TYPE_WIRED_HEADPHONES, "Headset")))
        assertEquals(
            AudioOutput(OutputClass.USB, "Example DAC"),
            audioOutputOf(OutputDevice(3, AudioDeviceInfo.TYPE_USB_DEVICE, " Example DAC\u0000")),
        )
        assertEquals(
            AudioOutput(OutputClass.BLUETOOTH, "Example Headphones"),
            audioOutputOf(OutputDevice(4, AudioDeviceInfo.TYPE_BLUETOOTH_A2DP, "Example Headphones")),
        )
        // Anything else (an earpiece, HDMI) counts as the speaker.
        assertEquals(AudioOutput(OutputClass.SPEAKER), audioOutputOf(OutputDevice(5, AudioDeviceInfo.TYPE_HDMI, "TV")))
    }

    @Test
    fun theBestOutputIsTheOneMediaPlaysOn() {
        val speaker = OutputDevice(1, AudioDeviceInfo.TYPE_BUILTIN_SPEAKER, "")
        val bt = OutputDevice(2, AudioDeviceInfo.TYPE_BLUETOOTH_A2DP, "Example Headphones")
        val usb = OutputDevice(3, AudioDeviceInfo.TYPE_USB_DEVICE, "Example DAC")
        assertEquals(bt, bestOutput(listOf(speaker, bt)))
        assertEquals(usb, bestOutput(listOf(speaker, bt, usb)))
        assertEquals(null, bestOutput(emptyList()))
        // Long names are cut, so a name never grows without bound.
        assertEquals(64, audioOutputOf(OutputDevice(6, AudioDeviceInfo.TYPE_BLUETOOTH_A2DP, "x".repeat(500))).name.length)
    }
}
