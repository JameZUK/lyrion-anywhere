package io.github.jamezuk.lyrionanywhere.playback

import android.content.Context
import android.media.AudioDeviceInfo
import android.media.AudioManager
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.data.MAX_LOUDNESS_REFERENCE_DB
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** Loudness compensation following the phone's media volume: the rules, and the follower. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class PhoneVolumeLoudnessTest {
    /** A curve like the speaker's: 15 steps, from −58 dB to 0 dB. */
    private val speaker: (Int) -> Float? = { i -> LoudnessReference.fallbackDb(i, 15).let { -it.toFloat() } }

    @Test
    fun theSystemsOwnCurveIsFollowed() {
        assertEquals(17.0, LoudnessReference.attenuationDb(9, 15, AudioDeviceInfo.TYPE_BUILTIN_SPEAKER, false) { -17f }!!, 1e-6)
        assertEquals(0.0, LoudnessReference.attenuationDb(15, 15, AudioDeviceInfo.TYPE_WIRED_HEADPHONES, false) { 0f }!!, 1e-6)
        // Muted is the most; a nonsense boost from the system counts as none.
        assertEquals(MAX_LOUDNESS_REFERENCE_DB, LoudnessReference.attenuationDb(0, 15, null, false, speaker)!!, 0.0)
        assertEquals(0.0, LoudnessReference.attenuationDb(15, 15, null, false) { 6f }!!, 0.0)
        assertEquals(MAX_LOUDNESS_REFERENCE_DB, LoudnessReference.attenuationDb(3, 15, null, false) { Float.NEGATIVE_INFINITY }!!, 0.0)
    }

    @Test
    fun withoutTheSystemsAnswerTheDefaultCurveStandsIn() {
        assertEquals(58.0, LoudnessReference.fallbackDb(1, 100), 1e-9)
        assertEquals(40.0, LoudnessReference.fallbackDb(20, 100), 1e-9)
        assertEquals(17.0, LoudnessReference.fallbackDb(60, 100), 1e-9)
        assertEquals(0.0, LoudnessReference.fallbackDb(100, 100), 1e-9)
        assertEquals(28.5, LoudnessReference.fallbackDb(40, 100), 1e-9)
        assertEquals(MAX_LOUDNESS_REFERENCE_DB, LoudnessReference.fallbackDb(0, 15), 0.0)
        // Out-of-range indices are held to the range; no range is full volume.
        assertEquals(0.0, LoudnessReference.fallbackDb(99, 15), 0.0)
        assertEquals(0.0, LoudnessReference.fallbackDb(3, 0), 0.0)
        // It only falls: louder never attenuates more.
        val curve = (1..15).map { LoudnessReference.fallbackDb(it, 15) }
        assertTrue(curve.zipWithNext().all { (a, b) -> b <= a })
        assertEquals(LoudnessReference.fallbackDb(7, 15), LoudnessReference.attenuationDb(7, 15, null, false) { i -> if (i == 1) -58f else null }!!, 1e-9)
        assertEquals(LoudnessReference.fallbackDb(7, 15), LoudnessReference.attenuationDb(7, 15, null, false) { Float.NaN }!!, 1e-9)
    }

    @Test
    fun aFixedVolumeOrAbsoluteBluetoothVolumeFallsBackToThePlayerLevel() {
        assertNull(LoudnessReference.attenuationDb(5, 15, AudioDeviceInfo.TYPE_BUILTIN_SPEAKER, true, speaker))
        assertNull(LoudnessReference.attenuationDb(5, 0, null, false, speaker))
        // Absolute volume: the phone's curve for the headset is flat at 0 dB.
        for (type in listOf(AudioDeviceInfo.TYPE_BLUETOOTH_A2DP, 26, 27, 30, AudioDeviceInfo.TYPE_HEARING_AID)) {
            assertNull(LoudnessReference.attenuationDb(5, 15, type, false) { 0f })
            // Not knowing the curve, a Bluetooth device is taken to keep its own volume.
            assertNull(LoudnessReference.attenuationDb(5, 15, type, false) { null })
        }
        // A Bluetooth device the phone turns down itself is followed.
        assertEquals(
            LoudnessReference.fallbackDb(5, 15),
            LoudnessReference.attenuationDb(5, 15, AudioDeviceInfo.TYPE_BLUETOOTH_A2DP, false, speaker)!!,
            1e-4,
        )
    }

    @Test
    fun theFollowerPassesOnChangesOnlyWhileActive() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val audio = context.getSystemService(AudioManager::class.java)
        val seen = mutableListOf<Double?>()
        val follower = PhoneVolumeFollower(context, audio) { seen += it }
        follower.refresh()
        assertTrue("nothing while inactive", seen.isEmpty())
        audio.setStreamVolume(AudioManager.STREAM_MUSIC, 5, 0)
        follower.setActive(true)
        assertEquals(1, seen.size)
        // The same again is not passed on.
        follower.refresh()
        assertEquals(1, seen.size)
        audio.setStreamVolume(AudioManager.STREAM_MUSIC, 0, 0)
        follower.refresh()
        assertEquals(MAX_LOUDNESS_REFERENCE_DB, seen.last()!!, 0.0)
        follower.setActive(false)
        audio.setStreamVolume(AudioManager.STREAM_MUSIC, 3, 0)
        follower.refresh()
        assertEquals(2, seen.size)
    }
}
