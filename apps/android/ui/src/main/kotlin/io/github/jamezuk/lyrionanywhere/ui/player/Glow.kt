package io.github.jamezuk.lyrionanywhere.ui.player

import kotlin.math.exp

/**
 * The visualiser's Glow (shared design with the web app; the parameters match exactly): a
 * soft halo behind the cover in the cover's own colours, slowly turning while it plays, that
 * pulses on the beat rather than following the level.
 *
 * Plain Kotlin (no Android types), so the beat detection and the envelope are tested on the
 * JVM. Times are milliseconds on any monotonic clock.
 */
object Glow {
    /** The bands (of [io.github.jamezuk.lyrionanywhere.data.VisualFrame.bands]) that make the bass. */
    const val BASS_BANDS = 5

    /** The running mean of the bass covers this long. */
    const val MEAN_WINDOW_MS = 1_500L

    /** An onset is bass this many times above its running mean (and rising). */
    const val ONSET_RATIO = 1.3f

    /** No second onset sooner than this after one. */
    const val REFRACTORY_MS = 150L

    /** Bass below this is silence: nothing fires, however it compares with its mean. */
    const val MIN_ENERGY = 0.02f

    /** The mean needs at least this many earlier frames before anything can fire. */
    const val WARM_UP_FRAMES = 3

    /** The pulse rises over this long… */
    const val ATTACK_MS = 30f

    /** …then decays with this time constant. */
    const val DECAY_MS = 350f

    /** The halo's radius, from its base (1.0) to full pulse. */
    const val RADIUS_MIN = 1.0f
    const val RADIUS_MAX = 1.25f

    /** The halo's opacity, at rest to full pulse. */
    const val OPACITY_MIN = 0.35f
    const val OPACITY_MAX = 0.85f

    /** The cover's scale at full pulse (very small). */
    const val COVER_SCALE_MAX = 1.02f

    /** How much the smoothed bass level counts between beats (low, so beats stand out). */
    const val BREATH_WEIGHT = 0.15f

    /** One turn of the halo takes this long while playing (frozen when paused). */
    const val TURN_MS = 24_000L

    /** The halo's lobes (120° apart), and the most colours taken from the cover. */
    const val MAX_COLOURS = 3
    const val LOBES = 3

    /** The bass energy of a frame's [bands]: the mean of the low bands, 0–1; garbage counts as 0. */
    fun bassEnergy(bands: FloatArray?): Float {
        if (bands == null || bands.isEmpty()) return 0f
        val n = minOf(BASS_BANDS, bands.size)
        var sum = 0f
        for (i in 0 until n) {
            val v = bands[i]
            sum += if (v.isFinite()) v.coerceIn(0f, 1f) else 0f
        }
        return sum / n
    }

    /** How hard the halo is driven (0–1): the beat [pulse], plus a little of the smoothed [level]. */
    fun drive(pulse: Float, level: Float): Float {
        val p = if (pulse.isFinite()) pulse.coerceIn(0f, 1f) else 0f
        val l = if (level.isFinite()) level.coerceIn(0f, 1f) else 0f
        return (p + BREATH_WEIGHT * l).coerceIn(0f, 1f)
    }

    fun radius(drive: Float): Float = RADIUS_MIN + (RADIUS_MAX - RADIUS_MIN) * clamp01(drive)

    fun opacity(drive: Float): Float = OPACITY_MIN + (OPACITY_MAX - OPACITY_MIN) * clamp01(drive)

    /** The cover's scale: the beat only (not the breathing), so it never drifts. */
    fun coverScale(pulse: Float): Float = 1f + (COVER_SCALE_MAX - 1f) * clamp01(pulse)

    /** The halo's turn (degrees) after [playingMs] of playing time. */
    fun angle(playingMs: Long): Float = ((playingMs.coerceAtLeast(0) % TURN_MS).toFloat() / TURN_MS) * 360f

    private fun clamp01(v: Float) = if (v.isFinite()) v.coerceIn(0f, 1f) else 0f
}

/**
 * Finds beats in the bass: keeps the mean bass energy over the last [Glow.MEAN_WINDOW_MS];
 * an onset fires when the energy exceeds [Glow.ONSET_RATIO] × that mean and is rising, at
 * least [Glow.REFRACTORY_MS] after the last one. A steady tone never fires again; silence
 * never fires.
 */
class OnsetDetector {
    private val times = LongArray(CAPACITY)
    private val values = FloatArray(CAPACITY)
    private var head = 0
    private var count = 0
    private var sum = 0.0
    private var previous = 0f
    private var lastOnset = Long.MIN_VALUE
    private var lastTime = Long.MIN_VALUE

    /** A frame's bass [energy] (0–1) at [nowMs]; true when it is an onset. */
    fun step(energy: Float, nowMs: Long): Boolean {
        val e = if (energy.isFinite()) energy.coerceIn(0f, 1f) else 0f
        // A clock that goes backwards (or a reset host) starts the history again.
        if (lastTime != Long.MIN_VALUE && nowMs < lastTime) reset()
        lastTime = nowMs
        // Drop what has left the window.
        while (count > 0 && nowMs - times[head] > Glow.MEAN_WINDOW_MS) {
            sum -= values[head]
            head = (head + 1) % CAPACITY
            count--
        }
        val warm = count >= Glow.WARM_UP_FRAMES
        val mean = if (count > 0) (sum / count).toFloat() else 0f
        val onset = warm &&
            e > Glow.MIN_ENERGY &&
            e > Glow.ONSET_RATIO * mean &&
            e > previous &&
            (lastOnset == Long.MIN_VALUE || nowMs - lastOnset >= Glow.REFRACTORY_MS)
        if (onset) lastOnset = nowMs
        // Into the history (the oldest goes if the buffer is full).
        if (count == CAPACITY) {
            sum -= values[head]
            head = (head + 1) % CAPACITY
            count--
        }
        val tail = (head + count) % CAPACITY
        times[tail] = nowMs
        values[tail] = e
        sum += e
        count++
        if (count == 1) sum = e.toDouble() // keep rounding from drifting
        previous = e
        return onset
    }

    fun reset() {
        head = 0
        count = 0
        sum = 0.0
        previous = 0f
        lastOnset = Long.MIN_VALUE
        lastTime = Long.MIN_VALUE
    }

    private companion object {
        /** Frames kept at most: 1.5 s even at 4 ms a frame. */
        const val CAPACITY = 384
    }
}

/**
 * The pulse a beat kicks: up to 1 over [Glow.ATTACK_MS] (from wherever it was), then an
 * exponential decay with [Glow.DECAY_MS] as its time constant.
 */
class PulseEnvelope {
    private var kickedAt = Long.MIN_VALUE
    private var from = 0f

    /** A beat at [nowMs]. */
    fun kick(nowMs: Long) {
        from = value(nowMs)
        kickedAt = nowMs
    }

    /** The pulse (0–1) at [nowMs]. */
    fun value(nowMs: Long): Float {
        if (kickedAt == Long.MIN_VALUE) return 0f
        val t = (nowMs - kickedAt).toFloat()
        if (t < 0f) return from
        if (t < Glow.ATTACK_MS) return from + (1f - from) * (t / Glow.ATTACK_MS)
        val v = exp(-(t - Glow.ATTACK_MS) / Glow.DECAY_MS)
        return if (v < 1e-3f) 0f else v
    }

    fun reset() {
        kickedAt = Long.MIN_VALUE
        from = 0f
    }
}

/** The detector and the envelope together, fed one visualiser frame at a time. */
class GlowDriver {
    private val detector = OnsetDetector()
    private val envelope = PulseEnvelope()

    /** Onsets so far (for tests and diagnostics). */
    var onsets = 0
        private set

    /** A frame's [bands] (null: no frame, silence) at [nowMs]; returns the pulse (0–1). */
    fun step(bands: FloatArray?, nowMs: Long): Float {
        if (detector.step(Glow.bassEnergy(bands), nowMs)) {
            envelope.kick(nowMs)
            onsets++
        }
        return envelope.value(nowMs)
    }

    fun reset() {
        detector.reset()
        envelope.reset()
        onsets = 0
    }
}
