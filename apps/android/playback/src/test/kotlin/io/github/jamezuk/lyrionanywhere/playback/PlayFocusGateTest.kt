package io.github.jamezuk.lyrionanywhere.playback

import android.content.Context
import android.media.AudioManager
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.data.BackgroundPlayback
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/**
 * Audio focus when LMS starts this phone playing (a resume from another controller, often
 * with the screen off). Android 15+ refuses focus to an app that is neither on screen nor
 * running a foreground service; that refusal must never pause the whole sync group.
 *
 * Runs the real [AudioFocusHandler] over Robolectric's AudioManager, which refuses focus
 * the way Android does: granted only while the app is on screen or the playback service
 * is in the foreground, and never during a call.
 */
@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class PlayFocusGateTest {
    private val audio = ApplicationProvider.getApplicationContext<Context>().getSystemService(AudioManager::class.java)
    private val shadow = shadowOf(audio)
    private val handler = AudioFocusHandler(audio) {}
    private val test = TestScope()
    private val counter = BackgroundPlayback()

    /** What happened, in order. */
    private val events = mutableListOf<String>()

    private var onScreen = false
    private var serviceForeground = false
    private var inCall = false

    /** Each nudge to the foreground takes effect on this nudge (null: never; Android refused). */
    private var foregroundOnNudge: Int? = 1
    private var nudges = 0

    private val gate = PlayFocusGate(
        scope = test,
        bringToForeground = {
            nudges++
            events += "foreground"
            if (foregroundOnNudge?.let { nudges >= it } == true) serviceForeground = true
        },
        request = {
            val allowed = (onScreen || serviceForeground) && !inCall
            shadow.setNextFocusRequestResponse(
                if (allowed) AudioManager.AUDIOFOCUS_REQUEST_GRANTED else AudioManager.AUDIOFOCUS_REQUEST_FAILED,
            )
            handler.request().also { events += if (it) "request:granted" else "request:refused" }
        },
        onScreen = { onScreen },
        serviceForeground = { serviceForeground },
        inCall = { inCall },
        clock = { test.testScheduler.currentTime },
    ) { decision ->
        events += decision.name
        if (decision == FocusDecision.PLAY_WITHOUT_FOCUS) counter.focusRefusedInBackground()
    }

    private fun settle() {
        test.advanceTimeBy(10_000)
        test.runCurrent()
    }

    private fun pauses() = events.count { it == FocusDecision.PAUSE.name }

    @Test
    fun aResumeFromElsewhereWithTheScreenOffGoesToTheForegroundBeforeAskingForFocus() {
        gate.onPlay()
        settle()
        assertEquals(listOf("foreground", "request:granted", "GRANTED"), events)
        assertEquals(0, pauses())
        assertEquals(0, counter.focusRefusals.value)
    }

    @Test
    fun aBackgroundRefusalKeepsPlayingAndIsCountedNeverPausingTheGroup() {
        // Android would not let the service into the foreground: focus is refused throughout.
        foregroundOnNudge = null
        gate.onPlay()
        settle()
        assertEquals("the foreground was asked for before focus", "foreground", events.first())
        assertEquals(0, pauses())
        assertEquals(FocusDecision.PLAY_WITHOUT_FOCUS.name, events.last())
        assertEquals(1, events.count { it == FocusDecision.PLAY_WITHOUT_FOCUS.name })
        assertEquals(1, counter.focusRefusals.value)
        assertTrue("retried before concluding", events.count { it == "request:refused" } > 1)
        assertTrue("concluded within about 2 s", test.testScheduler.currentTime <= 10_000)
    }

    @Test
    fun theBackgroundConclusionComesOnlyAfterAboutTwoSeconds() {
        foregroundOnNudge = null
        gate.onPlay()
        test.runCurrent()
        test.advanceTimeBy(1_500)
        assertTrue("nothing concluded yet: $events", events.none { it == FocusDecision.PLAY_WITHOUT_FOCUS.name })
        test.advanceTimeBy(1_000)
        test.runCurrent()
        assertEquals(FocusDecision.PLAY_WITHOUT_FOCUS.name, events.last())
    }

    @Test
    fun refusedThenGrantedOnARetryPlaysWithoutAPause() {
        // Media3 takes the foreground a moment later (the session's state catches up).
        foregroundOnNudge = 3
        gate.onPlay()
        settle()
        assertEquals(0, pauses())
        assertEquals(FocusDecision.GRANTED.name, events.last())
        assertEquals(1, events.count { it == "request:granted" })
        assertTrue(events.first() == "foreground")
        assertEquals(0, counter.focusRefusals.value)
    }

    @Test
    fun aCallWhileTheAppIsOnScreenStillPausesAtOnceAndOnlyOnce() {
        onScreen = true
        inCall = true
        gate.onPlay()
        test.runCurrent()
        assertEquals("paused at once, without waiting for retries", 1, pauses())
        // LMS's status bounces (paused, then playing again from a stale report).
        gate.onStop()
        test.advanceTimeBy(300)
        gate.onPlay()
        settle()
        assertEquals(1, pauses())
        assertEquals(0, counter.focusRefusals.value)
    }

    @Test
    fun aRefusalInTheForegroundServiceIsAGenuineConflictAndPausesOnce() {
        // The service is in the foreground, yet focus is refused (another app's call).
        inCall = true
        gate.onPlay()
        settle()
        assertEquals(1, pauses())
        assertEquals(0, counter.focusRefusals.value)
    }

    @Test
    fun onePlayTransitionMakesOneFocusDecision() {
        onScreen = true
        inCall = true
        gate.onPlay()
        gate.onPlay()
        test.advanceTimeBy(70)
        gate.onPlay()
        settle()
        assertEquals(1, pauses())
        assertEquals(1, events.count { it in FocusDecision.entries.map(FocusDecision::name) })
    }

    @Test
    fun repeatedReportsWhileTheBackgroundDecisionIsPendingAreOneDecision() {
        foregroundOnNudge = null
        gate.onPlay()
        test.advanceTimeBy(300)
        gate.onPlay()
        settle()
        assertEquals(1, events.count { it in FocusDecision.entries.map(FocusDecision::name) })
        assertEquals(1, counter.focusRefusals.value)
    }

    @Test
    fun aStopWhileRetryingDecidesNothing() {
        foregroundOnNudge = null
        gate.onPlay()
        test.advanceTimeBy(300)
        gate.onStop()
        settle()
        assertEquals(0, events.count { it in FocusDecision.entries.map(FocusDecision::name) })
    }

    @Test
    fun aLaterPlayAfterTheCooldownIsDecidedAfresh() {
        onScreen = true
        inCall = true
        gate.onPlay()
        test.runCurrent()
        gate.onStop()
        test.advanceTimeBy(PlayFocusGate.PAUSE_COOLDOWN_MS + 1)
        gate.onPlay()
        test.runCurrent()
        assertEquals(2, pauses())
    }
}

/** Why focus was refused: pure rules. */
class FocusRefusalsTest {
    @Test
    fun onlyARefusalWithTheAppWhollyInTheBackgroundIsExplainedByTheBackgroundRule() {
        for (screen in listOf(false, true)) for (fg in listOf(false, true)) for (call in listOf(false, true)) {
            val expected = if (!screen && !fg && !call) FocusRefusal.BACKGROUND else FocusRefusal.CONFLICT
            assertEquals("$screen $fg $call", expected, FocusRefusals.classify(screen, fg, call))
        }
    }

    @Test
    fun onScreenOrInACallNoRetryCanHelp() {
        for (screen in listOf(false, true)) for (call in listOf(false, true)) {
            assertEquals("$screen $call", screen || call, FocusRefusals.conclusive(screen, call))
        }
    }
}
