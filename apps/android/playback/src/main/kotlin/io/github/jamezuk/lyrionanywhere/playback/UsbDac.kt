package io.github.jamezuk.lyrionanywhere.playback

import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioMixerAttributes
import android.os.Build
import androidx.annotation.RequiresApi
import io.github.jamezuk.lyrionanywhere.core.MixerOffer
import io.github.jamezuk.lyrionanywhere.core.OfferBlocker
import io.github.jamezuk.lyrionanywhere.core.OutputNegotiator
import io.github.jamezuk.lyrionanywhere.core.OutputOffers
import io.github.jamezuk.lyrionanywhere.core.PcmEncoding

/*
 * Bit-perfect USB output (Android 14+, API 34): Android's preferred mixer attributes.
 *
 * The engine (Rust) decides which mixer configuration to use (`negotiate::plan`); this file
 * is the platform side, split in two:
 * - pure functions over plain values (testable on the JVM without a device): which output
 *   is a USB DAC, why bit-perfect is unavailable, how a platform mixer configuration maps
 *   to the engine's [MixerOffer] and back;
 * - [UsbDacNegotiator], the thin `AudioManager` wrapper the engine calls.
 */

/** API level of `AudioManager.getSupportedMixerAttributes` and `MIXER_BEHAVIOR_BIT_PERFECT`. */
const val BIT_PERFECT_API = Build.VERSION_CODES.UPSIDE_DOWN_CAKE

/** `AudioMixerAttributes.MIXER_BEHAVIOR_BIT_PERFECT` (API 34; the value, so pure code can
 *  use it on any API level). */
const val MIXER_BEHAVIOR_BIT_PERFECT = 1

/** An output device, as plain values. */
data class OutputDevice(val id: Int, val type: Int, val name: String)

/** One mixer configuration from `getSupportedMixerAttributes`, as plain values. */
data class PlatformMixer(val encoding: Int, val sampleRate: Int, val channelMask: Int, val behavior: Int)

/** USB audio outputs (a DAC or a USB headset); the dock and accessory types aren't DACs. */
fun isUsbDac(type: Int): Boolean =
    type == AudioDeviceInfo.TYPE_USB_DEVICE || type == AudioDeviceInfo.TYPE_USB_HEADSET

/** The USB DAC among the outputs, if any (a DAC before a headset; then the lowest id). */
fun pickUsbDac(outputs: List<OutputDevice>): OutputDevice? =
    outputs.filter { isUsbDac(it.type) }
        .minWithOrNull(
            compareBy<OutputDevice> { if (it.type == AudioDeviceInfo.TYPE_USB_DEVICE) 0 else 1 }.thenBy { it.id },
        )

/** Why there's nothing to offer, or null when the DAC's mixer configurations are worth asking for. */
fun offerBlocker(enabled: Boolean, dac: OutputDevice?, sdk: Int): OfferBlocker? = when {
    !enabled -> OfferBlocker.TURNED_OFF
    dac == null -> OfferBlocker.NO_USB_DAC
    sdk < BIT_PERFECT_API -> OfferBlocker.ANDROID_TOO_OLD
    else -> null
}

/** The engine's view of a platform mixer configuration; null for one it can't play (an
 *  encoding AAudio has no format for, a nonsense rate, no channels). */
fun toOffer(m: PlatformMixer): MixerOffer? {
    val encoding = when (m.encoding) {
        AudioFormat.ENCODING_PCM_16BIT -> PcmEncoding.I16
        AudioFormat.ENCODING_PCM_24BIT_PACKED -> PcmEncoding.I24_PACKED
        AudioFormat.ENCODING_PCM_32BIT -> PcmEncoding.I32
        AudioFormat.ENCODING_PCM_FLOAT -> PcmEncoding.FLOAT
        else -> return null
    }
    if (m.sampleRate !in 8_000..768_000) return null
    val channels = when (m.channelMask) {
        AudioFormat.CHANNEL_OUT_STEREO -> 2
        AudioFormat.CHANNEL_OUT_MONO -> 1
        else -> Integer.bitCount(m.channelMask)
    }
    if (channels == 0) return null
    return MixerOffer(encoding, m.sampleRate.toUInt(), channels.toUInt(), m.behavior == MIXER_BEHAVIOR_BIT_PERFECT)
}

/** Which platform configuration the engine chose (the first that maps to it), or -1. */
fun indexOfOffer(offer: MixerOffer, platform: List<PlatformMixer>): Int =
    platform.indexOfFirst { toOffer(it) == offer }

/** The display name of a USB DAC: its product name, or a generic one. */
fun dacName(dac: OutputDevice): String =
    dac.name.filter { !it.isISOControl() }.trim().take(64).ifEmpty { "USB DAC" }

/**
 * Android's side of bit-perfect output: lists the USB DAC's mixer configurations and
 * applies or clears the one the engine picked, for our media stream (usage media, the
 * engine's AAudio stream). Called on the engine's thread; `AudioManager` is thread-safe.
 */
class UsbDacNegotiator(
    private val audio: AudioManager,
    /** The "Bit-perfect USB output" setting. */
    private val enabled: () -> Boolean,
) : OutputNegotiator {
    private val attributes = AudioAttributes.Builder()
        .setUsage(AudioAttributes.USAGE_MEDIA)
        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
        .build()

    /** The device we set mixer attributes on (cleared there). */
    @Volatile private var appliedTo: AudioDeviceInfo? = null

    private fun usbDac(): Pair<OutputDevice, AudioDeviceInfo>? {
        val infos = audio.getDevices(AudioManager.GET_DEVICES_OUTPUTS).toList()
        val picked = pickUsbDac(infos.map { OutputDevice(it.id, it.type, it.productName?.toString() ?: "") })
            ?: return null
        return infos.firstOrNull { it.id == picked.id }?.let { picked to it }
    }

    override fun offers(): OutputOffers {
        val dac = usbDac()
        val name = dac?.first?.let(::dacName) ?: ""
        offerBlocker(enabled(), dac?.first, Build.VERSION.SDK_INT)?.let { return OutputOffers(name, it, emptyList()) }
        if (dac == null || Build.VERSION.SDK_INT < BIT_PERFECT_API) {
            return OutputOffers(name, OfferBlocker.ANDROID_TOO_OLD, emptyList())
        }
        return OutputOffers(name, null, mixerList(dac.second).map(::platformMixer).mapNotNull(::toOffer))
    }

    override fun apply(offer: MixerOffer?): Boolean {
        if (Build.VERSION.SDK_INT < BIT_PERFECT_API) return false
        if (offer == null) {
            appliedTo?.let { clear(it) }
            appliedTo = null
            return true
        }
        val info = usbDac()?.second ?: return false
        return set(info, offer)
    }

    @RequiresApi(BIT_PERFECT_API)
    private fun mixerList(info: AudioDeviceInfo): List<AudioMixerAttributes> =
        try {
            audio.getSupportedMixerAttributes(info)
        } catch (_: RuntimeException) {
            emptyList()
        }

    @RequiresApi(BIT_PERFECT_API)
    private fun platformMixer(a: AudioMixerAttributes) =
        PlatformMixer(a.format.encoding, a.format.sampleRate, a.format.channelMask, a.mixerBehavior)

    @RequiresApi(BIT_PERFECT_API)
    private fun set(info: AudioDeviceInfo, offer: MixerOffer): Boolean {
        val list = mixerList(info)
        val i = indexOfOffer(offer, list.map(::platformMixer))
        if (i < 0) return false
        val ok = try {
            audio.setPreferredMixerAttributes(attributes, info, list[i])
        } catch (_: RuntimeException) {
            false
        }
        if (ok) {
            appliedTo?.takeIf { it.id != info.id }?.let { clear(it) }
            appliedTo = info
        }
        return ok
    }

    @RequiresApi(BIT_PERFECT_API)
    private fun clear(info: AudioDeviceInfo) {
        try {
            audio.clearPreferredMixerAttributes(attributes, info)
        } catch (_: RuntimeException) {
            // The device has gone: nothing to clear.
        }
    }
}
