package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.CrossfeedSetting
import io.github.jamezuk.lyrionanywhere.core.DitherSetting
import io.github.jamezuk.lyrionanywhere.core.DspSettings
import io.github.jamezuk.lyrionanywhere.core.Levelling
import io.github.jamezuk.lyrionanywhere.core.TransitionMode

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

/** How tracks are joined on this phone's player. */
enum class TransitionsMode {
    /** Gapless and bit-perfect (the default). */
    OFF,

    /** A fixed crossfade between tracks of different albums. */
    CROSSFADE,

    /** Shaped by the server's analysis: silence trimmed, the fade follows the music. */
    SMART,
}

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
    /** Crossfades between tracks (never within an album). Off keeps playback gapless. */
    val transitions: TransitionsMode = TransitionsMode.OFF,
    /** The crossfade's length (the most a smart transition uses), seconds. */
    val crossfadeSecs: Double = DEFAULT_CROSSFADE_SECS,
    /**
     * EQ profiles bound to outputs (Wave 2 §5.3): used instead of [eq] while that output
     * plays. Device names stay on this phone.
     */
    val outputEq: List<OutputEq> = emptyList(),
) {
    /** The stages that change samples (empty: bit-perfect). The limiter and dither only
     *  ever act with one of these. */
    val activeStages: List<SoundStage>
        get() = buildList {
            if (preampDb != 0.0) add(SoundStage.PREAMP)
            if (levelling != LevellingMode.OFF) add(SoundStage.LEVELLING)
            if (eqEnabled && eq != null) add(SoundStage.EQ)
            if (crossfeed != CrossfeedMode.OFF) add(SoundStage.CROSSFEED)
            if (transitions != TransitionsMode.OFF) add(SoundStage.CROSSFADE)
        }

    /** Nothing changes the samples. */
    val bitPerfect: Boolean get() = activeStages.isEmpty()

    /** The same settings with every number forced into the range the screen offers (the
     *  engine clamps again, to its own ranges). Non-finite values reset to the defaults. */
    fun sanitised(): SoundSettings {
        fun clamp(v: Double, default: Double, range: ClosedFloatingPointRange<Double>) =
            if (v.isFinite()) v.coerceIn(range) else default
        val general = eq?.takeIf { it.text.length <= MAX_EQ_CHARS }
        val bound = OutputEq.sanitise(outputEq)
        return copy(
            preampDb = clamp(preampDb, 0.0, PREAMP_RANGE),
            limiterCeilingDb = clamp(limiterCeilingDb, -1.0, CEILING_RANGE),
            eq = general,
            // The switch needs a profile somewhere: the general one or an output's.
            eqEnabled = eqEnabled && (general != null || bound.any { it.profile != null }),
            crossfadeSecs = clamp(crossfadeSecs, DEFAULT_CROSSFADE_SECS, CROSSFADE_RANGE),
            outputEq = bound,
        )
    }

    /** The binding for [output]: its own device's first, else any of its kind (null: none). */
    fun bindingFor(output: AudioOutput?): OutputEq? {
        if (output == null) return null
        return outputEq.firstOrNull { it.kind == output.kind && it.deviceName.isNotEmpty() && it.deviceName == output.name }
            ?: outputEq.firstOrNull { it.kind == output.kind && it.deviceName.isEmpty() }
    }

    /**
     * These settings as they apply on [output]: a bound profile replaces the general one
     * (a binding with no profile means no EQ there). Unbound outputs use the general one.
     */
    fun forOutput(output: AudioOutput?): SoundSettings {
        val b = bindingFor(output) ?: return this
        return copy(eq = b.profile, eqEnabled = eqEnabled && b.profile != null)
    }

    companion object {
        val PREAMP_RANGE = -12.0..12.0
        val CEILING_RANGE = -6.0..0.0
        val CROSSFADE_RANGE = 1.0..10.0
        const val DEFAULT_CROSSFADE_SECS = 6.0

        /** The engine refuses profiles over 64 KiB. */
        const val MAX_EQ_CHARS = 64 * 1024
    }
}

enum class SoundStage { PREAMP, LEVELLING, EQ, CROSSFEED, CROSSFADE }

/** The kinds of output an EQ profile can be bound to (Wave 2 §5.3). */
enum class OutputClass {
    SPEAKER,
    WIRED,

    /** A USB DAC, by its product name. */
    USB,

    /** A Bluetooth device, by its name. */
    BLUETOOTH,
    ;

    /** Only USB DACs and Bluetooth devices are told apart by name. */
    val named: Boolean get() = this == USB || this == BLUETOOTH
}

/** Where the sound goes now: the kind of output and, for a named kind, the device's name. */
data class AudioOutput(val kind: OutputClass, val name: String = "") {
    companion object {
        /** A device name fit to keep and compare: printable, trimmed, at most 64 characters. */
        fun cleanName(name: String?): String = name.orEmpty().filter { !it.isISOControl() }.trim().take(64)

        fun of(kind: OutputClass, name: String?): AudioOutput = AudioOutput(kind, if (kind.named) cleanName(name) else "")
    }
}

/**
 * An EQ profile bound to an output: a kind of output and, for USB or Bluetooth, one
 * device's name (empty: any of that kind). A null [profile] means no EQ on that output.
 */
data class OutputEq(val kind: OutputClass, val deviceName: String = "", val profile: EqProfile?) {
    companion object {
        /** At most this many bindings are kept. */
        const val MAX = 8

        /**
         * Bindings fit to keep: names cleaned (and dropped where the kind has none), profiles
         * over the size limit or holding the stored form's separators dropped, one binding
         * per output (the last wins), at most [MAX].
         */
        fun sanitise(list: List<OutputEq>): List<OutputEq> {
            val out = LinkedHashMap<Pair<OutputClass, String>, OutputEq>()
            for (b in list) {
                val name = if (b.kind.named) AudioOutput.cleanName(b.deviceName) else ""
                val profile = b.profile
                    ?.takeIf { it.text.length <= SoundSettings.MAX_EQ_CHARS && it.text.none { c -> c == RECORD || c == FIELD } }
                    ?.let { it.copy(name = it.name.filter { c -> !c.isISOControl() }.trim().take(64).ifEmpty { "EQ" }) }
                if (b.profile != null && profile == null) continue
                val key = b.kind to name
                out.remove(key)
                out[key] = OutputEq(b.kind, name, profile)
            }
            return out.values.toList().takeLast(MAX)
        }

        private const val RECORD = '\u001E'
        private const val FIELD = '\u001F'

        /**
         * Bindings as one stored string: records of kind, device name, whether there is a
         * profile, its name, filter count, preamp and text. The separators are control
         * characters, which no kept name or profile holds.
         */
        fun encode(list: List<OutputEq>): String = sanitise(list).joinToString(RECORD.toString()) { b ->
            val p = b.profile
            listOf(
                b.kind.name,
                b.deviceName,
                if (p == null) "0" else "1",
                p?.name.orEmpty(),
                (p?.filters ?: 0).toString(),
                (p?.preampDb ?: 0.0).toString(),
                p?.text.orEmpty(),
            ).joinToString(FIELD.toString())
        }

        /** The longest stored string read: anything longer is ignored. */
        const val MAX_STORED = (SoundSettings.MAX_EQ_CHARS + 1024) * MAX

        /** Stored bindings, read defensively: anything malformed is skipped. */
        fun decode(stored: String?): List<OutputEq> {
            if (stored.isNullOrEmpty() || stored.length > MAX_STORED) return emptyList()
            val out = mutableListOf<OutputEq>()
            for (record in stored.split(RECORD)) {
                if (out.size >= MAX) break
                val f = record.split(FIELD)
                if (f.size != 7) continue
                val kind = OutputClass.entries.firstOrNull { it.name == f[0] } ?: continue
                val profile = when (f[2]) {
                    "0" -> null
                    "1" -> {
                        val filters = f[4].toIntOrNull()?.takeIf { it in 0..1_000 } ?: continue
                        val preamp = f[5].toDoubleOrNull()?.takeIf { it.isFinite() } ?: continue
                        if (f[6].isEmpty()) continue
                        EqProfile(f[3], f[6], filters, preamp)
                    }
                    else -> continue
                }
                out += OutputEq(kind, f[1], profile)
            }
            return sanitise(out)
        }
    }
}

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
    transitions = TransitionMode.OFF,
    crossfadeS = SoundSettings.DEFAULT_CROSSFADE_SECS,
)

/** The engine's DSP for the Sound settings. Bit-perfect ([DEFAULT_DSP]) when no stage is on. */
fun dspFor(sound: SoundSettings): DspSettings {
    val s = sound.sanitised()
    return chainFor(s).copy(
        transitions = when (s.transitions) {
            TransitionsMode.OFF -> TransitionMode.OFF
            TransitionsMode.CROSSFADE -> TransitionMode.CROSSFADE
            TransitionsMode.SMART -> TransitionMode.SMART
        },
        crossfadeS = s.crossfadeSecs,
    )
}

/** The DSP chain's part (transitions are separate: they never switch the chain on). */
private fun chainFor(s: SoundSettings): DspSettings {
    if ((s.activeStages - SoundStage.CROSSFADE).isEmpty()) return DEFAULT_DSP.copy(dither = s.dither.toCore())
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
