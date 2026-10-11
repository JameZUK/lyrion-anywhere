package io.github.jamezuk.lyrionanywhere.playback

import android.content.Context
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.MixerOffer
import io.github.jamezuk.lyrionanywhere.core.OfferBlocker
import io.github.jamezuk.lyrionanywhere.core.PcmEncoding
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.shadows.AudioDeviceInfoBuilder

/** The platform side of bit-perfect USB output, as pure functions (no device needed), and
 *  the `AudioManager` wrapper's refusals under Robolectric. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class UsbDacTest {
    private val stereo = AudioFormat.CHANNEL_OUT_STEREO

    private fun mixer(encoding: Int, rate: Int, mask: Int = stereo, behaviour: Int = MIXER_BEHAVIOR_BIT_PERFECT) =
        PlatformMixer(encoding, rate, mask, behaviour)

    @Test
    fun platformMixersMapToTheEnginesOffers() {
        assertEquals(
            MixerOffer(PcmEncoding.I16, 44_100u, 2u, true),
            toOffer(mixer(AudioFormat.ENCODING_PCM_16BIT, 44_100)),
        )
        assertEquals(
            MixerOffer(PcmEncoding.I24_PACKED, 96_000u, 2u, true),
            toOffer(mixer(AudioFormat.ENCODING_PCM_24BIT_PACKED, 96_000)),
        )
        assertEquals(
            MixerOffer(PcmEncoding.I32, 192_000u, 2u, true),
            toOffer(mixer(AudioFormat.ENCODING_PCM_32BIT, 192_000)),
        )
        // Default mixing is listed too, and kept apart.
        assertEquals(
            MixerOffer(PcmEncoding.FLOAT, 48_000u, 2u, false),
            toOffer(mixer(AudioFormat.ENCODING_PCM_FLOAT, 48_000, behaviour = 0)),
        )
        assertEquals(1u, toOffer(mixer(AudioFormat.ENCODING_PCM_16BIT, 48_000, AudioFormat.CHANNEL_OUT_MONO))!!.channels)
        assertEquals(6u, toOffer(mixer(AudioFormat.ENCODING_PCM_16BIT, 48_000, AudioFormat.CHANNEL_OUT_5POINT1))!!.channels)
    }

    @Test
    fun unplayableOrNonsenseMixersAreDropped() {
        // Encodings AAudio can't open (8-bit, compressed), nonsense rates, no channels.
        assertNull(toOffer(mixer(AudioFormat.ENCODING_PCM_8BIT, 44_100)))
        assertNull(toOffer(mixer(AudioFormat.ENCODING_E_AC3, 44_100)))
        assertNull(toOffer(mixer(AudioFormat.ENCODING_AC3, 48_000)))
        assertNull(toOffer(mixer(AudioFormat.ENCODING_INVALID, 48_000)))
        for (rate in listOf(0, -1, 7_999, 768_001, Int.MAX_VALUE, Int.MIN_VALUE)) {
            assertNull("$rate Hz", toOffer(mixer(AudioFormat.ENCODING_PCM_16BIT, rate)))
        }
        assertNull(toOffer(mixer(AudioFormat.ENCODING_PCM_16BIT, 44_100, mask = 0)))
        // Whatever the platform lists, nothing throws.
        val r = java.util.Random(7)
        repeat(5_000) {
            toOffer(PlatformMixer(r.nextInt(), r.nextInt(), r.nextInt(), r.nextInt()))
        }
    }

    @Test
    fun theChosenOfferIsFoundInThePlatformList() {
        val list = listOf(
            mixer(AudioFormat.ENCODING_PCM_FLOAT, 48_000, behaviour = 0),
            mixer(AudioFormat.ENCODING_AC3, 48_000),
            mixer(AudioFormat.ENCODING_PCM_16BIT, 48_000),
            mixer(AudioFormat.ENCODING_PCM_24BIT_PACKED, 48_000),
        )
        assertEquals(3, indexOfOffer(MixerOffer(PcmEncoding.I24_PACKED, 48_000u, 2u, true), list))
        assertEquals(2, indexOfOffer(MixerOffer(PcmEncoding.I16, 48_000u, 2u, true), list))
        // Not listed (any field different): never a wrong one.
        assertEquals(-1, indexOfOffer(MixerOffer(PcmEncoding.I16, 44_100u, 2u, true), list))
        assertEquals(-1, indexOfOffer(MixerOffer(PcmEncoding.I16, 48_000u, 2u, false), list))
        assertEquals(-1, indexOfOffer(MixerOffer(PcmEncoding.I32, 48_000u, 2u, true), list))
        assertEquals(-1, indexOfOffer(MixerOffer(PcmEncoding.I16, 48_000u, 2u, true), emptyList()))
    }

    @Test
    fun theUsbDacIsPickedAndOtherOutputsIgnored() {
        val speaker = OutputDevice(1, AudioDeviceInfo.TYPE_BUILTIN_SPEAKER, "Phone")
        val bt = OutputDevice(2, AudioDeviceInfo.TYPE_BLUETOOTH_A2DP, "Headphones")
        val headset = OutputDevice(3, AudioDeviceInfo.TYPE_USB_HEADSET, "Headset")
        val dac = OutputDevice(9, AudioDeviceInfo.TYPE_USB_DEVICE, "Example DAC")
        val dock = OutputDevice(4, AudioDeviceInfo.TYPE_DOCK, "Dock")
        assertNull(pickUsbDac(listOf(speaker, bt, dock)))
        assertEquals(headset, pickUsbDac(listOf(speaker, headset)))
        assertEquals("a DAC before a headset", dac, pickUsbDac(listOf(headset, speaker, dac)))
        assertEquals("then the lowest id", dac, pickUsbDac(listOf(dac.copy(id = 12), dac)))
        assertFalse(isUsbDac(AudioDeviceInfo.TYPE_USB_ACCESSORY))
    }

    @Test
    fun blockersAreReportedInOrder() {
        val dac = OutputDevice(9, AudioDeviceInfo.TYPE_USB_DEVICE, "Example DAC")
        assertEquals(OfferBlocker.TURNED_OFF, offerBlocker(false, dac, 35))
        assertEquals(OfferBlocker.NO_USB_DAC, offerBlocker(true, null, 35))
        assertEquals(OfferBlocker.NO_USB_DAC, offerBlocker(true, null, 29))
        assertEquals(OfferBlocker.ANDROID_TOO_OLD, offerBlocker(true, dac, 33))
        assertNull(offerBlocker(true, dac, 34))
        assertNull(offerBlocker(true, dac, 37))
    }

    @Test
    fun namesAreCleanAndBounded() {
        assertEquals("USB DAC", dacName(OutputDevice(1, AudioDeviceInfo.TYPE_USB_DEVICE, "  ")))
        assertEquals("Example DAC", dacName(OutputDevice(1, AudioDeviceInfo.TYPE_USB_DEVICE, "Example\u0000 DAC")))
        assertEquals(64, dacName(OutputDevice(1, AudioDeviceInfo.TYPE_USB_DEVICE, "x".repeat(500))).length)
        assertEquals("USB audio · Example DAC", routeLabel("USB audio", RouteKind.USB, "Example DAC"))
        assertEquals("Bluetooth · Headphones", routeLabel("Bluetooth", RouteKind.BLUETOOTH, "Headphones"))
        assertEquals("Headphones", routeLabel("Headphones", RouteKind.WIRED, "Some name"))
        assertEquals("Bluetooth", routeLabel("Bluetooth", RouteKind.BLUETOOTH, null))
    }

    @Test
    fun theNegotiatorRefusesWithoutADacOrWhenOff() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val audio = context.getSystemService(AudioManager::class.java)
        var enabled = true
        val n = UsbDacNegotiator(audio) { enabled }
        assertEquals(OfferBlocker.NO_USB_DAC, n.offers().blocker)
        assertTrue(n.offers().offers.isEmpty())
        // Nothing to apply to: refused; clearing is always fine.
        assertFalse(n.apply(MixerOffer(PcmEncoding.I16, 44_100u, 2u, true)))
        assertTrue(n.apply(null))

        val dac = AudioDeviceInfoBuilder.newBuilder().setType(AudioDeviceInfo.TYPE_USB_DEVICE).build()
        shadowOf(audio).setOutputDevices(listOf(dac))
        val offers = n.offers()
        assertNull("a DAC on Android 14+: worth asking", offers.blocker)
        assertTrue(offers.device.isNotEmpty())
        // This (simulated) DAC lists no mixer attributes, so nothing matches.
        assertFalse(n.apply(MixerOffer(PcmEncoding.I16, 44_100u, 2u, true)))
        enabled = false
        assertEquals(OfferBlocker.TURNED_OFF, n.offers().blocker)
    }
}
