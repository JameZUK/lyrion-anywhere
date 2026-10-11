package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.CrossfeedSetting
import io.github.jamezuk.lyrionanywhere.core.DitherSetting
import io.github.jamezuk.lyrionanywhere.core.DspSettings
import io.github.jamezuk.lyrionanywhere.core.Levelling

/** Loudness levelling on this phone's player. */
enum class LevellingMode {
    OFF,

    /** ReplayGain track gain from the file's tags. */
    REPLAYGAIN_TRACK,

    /** ReplayGain album gain where tagged, otherwise track gain. */
    REPLAYGAIN_ALBUM,

    /** The gateway's EBU R128 analysis of each track, falling back to ReplayGain tags. */
    R128,
}

/** Headphone crossfeed presets. */
enum class CrossfeedMode { OFF, LIGHT, MODERATE, NATURAL }

/** Requantising when the bit depth is reduced (only then). */
enum class DitherMode { TPDF, TPDF_SHAPED, OFF }

/** An imported AutoEq `ParametricEQ.txt` profile, as checked by the engine. */
data class EqProfile(
    /** Shown in Settings, e.g. the file's name. */
    val name: String,
    val text: String,
    val filters: Int,
    val preampDb: Double,
)

/**
 * The "Sound" settings. The default is bit-perfect: every stage off. Bit-perfect USB
 * output is on by default (it only applies with a USB DAC on Android 14+ that offers it).
 */
data class SoundSettings(
    val preampDb: Double = 0.0,
    val levelling: LevellingMode = LevellingMode.OFF,
    /** The true-peak limiter, protecting the other stages from clipping. */
    val limiter: Boolean = true,
    val limiterCeilingDb: Double = -1.0,
    val eqEnabled: Boolean = false,
    val eq: EqProfile? = null,
    val crossfeed: CrossfeedMode = CrossfeedMode.OFF,
    val dither: DitherMode = DitherMode.TPDF,
    /** Ask Android for bit-perfect output to a USB DAC (Android 14+). */
    val bitPerfectUsb: Boolean = true,
) {
    /** The stages that change samples (empty: bit-perfect). The limiter and dither only
     *  ever act with one of these. */
    val activeStages: List<SoundStage>
        get() = buildList {
            if (preampDb != 0.0) add(SoundStage.PREAMP)
            if (levelling != LevellingMode.OFF) add(SoundStage.LEVELLING)
            if (eqEnabled && eq != null) add(SoundStage.EQ)
            if (crossfeed != CrossfeedMode.OFF) add(SoundStage.CROSSFEED)
        }

    /** Nothing changes the samples. */
    val bitPerfect: Boolean get() = activeStages.isEmpty()

    /** The same settings with every number forced into the range the screen offers (the
     *  engine clamps again, to its own ranges). Non-finite values reset to the defaults. */
    fun sanitised(): SoundSettings {
        fun clamp(v: Double, default: Double, range: ClosedFloatingPointRange<Double>) =
            if (v.isFinite()) v.coerceIn(range) else default
        return copy(
            preampDb = clamp(preampDb, 0.0, PREAMP_RANGE),
            limiterCeilingDb = clamp(limiterCeilingDb, -1.0, CEILING_RANGE),
            eq = eq?.takeIf { it.text.length <= MAX_EQ_CHARS },
            eqEnabled = eqEnabled && eq != null && eq.text.length <= MAX_EQ_CHARS,
        )
    }

    companion object {
        val PREAMP_RANGE = -12.0..12.0
        val CEILING_RANGE = -6.0..0.0

        /** The engine refuses profiles over 64 KiB. */
        const val MAX_EQ_CHARS = 64 * 1024
    }
}

enum class SoundStage { PREAMP, LEVELLING, EQ, CROSSFEED }

/** The levelling target, LUFS (ReplayGain's reference, so tagged and analysed tracks agree). */
const val LEVELLING_TARGET_LUFS = -18.0

/** Bit-perfect: every DSP stage off. */
val DEFAULT_DSP = DspSettings(
    enabled = false,
    preampDb = 0.0,
    levelling = Levelling.OFF,
    targetLufs = LEVELLING_TARGET_LUFS,
    limiter = true,
    analysisLevelling = false,
    limiterCeilingDb = -1.0,
    eqProfile = null,
    crossfeed = CrossfeedSetting.OFF,
    dither = DitherSetting.TPDF,
)

/** The engine's DSP for the Sound settings. Bit-perfect ([DEFAULT_DSP]) when no stage is on. */
fun dspFor(sound: SoundSettings): DspSettings {
    val s = sound.sanitised()
    if (s.bitPerfect) return DEFAULT_DSP.copy(dither = s.dither.toCore())
    return DEFAULT_DSP.copy(
        enabled = true,
        preampDb = s.preampDb,
        levelling = when (s.levelling) {
            LevellingMode.OFF -> Levelling.OFF
            LevellingMode.REPLAYGAIN_TRACK, LevellingMode.R128 -> Levelling.TRACK
            LevellingMode.REPLAYGAIN_ALBUM -> Levelling.ALBUM
        },
        analysisLevelling = s.levelling == LevellingMode.R128,
        limiter = s.limiter,
        limiterCeilingDb = s.limiterCeilingDb,
        eqProfile = s.eq?.text?.takeIf { s.eqEnabled },
        crossfeed = when (s.crossfeed) {
            CrossfeedMode.OFF -> CrossfeedSetting.OFF
            CrossfeedMode.LIGHT -> CrossfeedSetting.LIGHT
            CrossfeedMode.MODERATE -> CrossfeedSetting.MODERATE
            CrossfeedMode.NATURAL -> CrossfeedSetting.NATURAL
        },
        dither = s.dither.toCore(),
    )
}

private fun DitherMode.toCore() = when (this) {
    DitherMode.TPDF -> DitherSetting.TPDF
    DitherMode.TPDF_SHAPED -> DitherSetting.TPDF_SHAPED
    DitherMode.OFF -> DitherSetting.OFF
}

/** The engine's verdict on an EQ profile. */
sealed interface EqCheck {
    data class Ok(val filters: Int, val preampDb: Double, val flat: Boolean) : EqCheck

    data class Invalid(val reason: String) : EqCheck
}
