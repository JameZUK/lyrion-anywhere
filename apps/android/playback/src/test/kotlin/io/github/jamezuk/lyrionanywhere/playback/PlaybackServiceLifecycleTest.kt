package io.github.jamezuk.lyrionanywhere.playback

import android.app.Application
import android.app.ForegroundServiceStartNotAllowedException
import android.content.Context
import android.content.Intent
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.data.ArtRepository
import io.github.jamezuk.lyrionanywhere.data.EventsRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.android.controller.ServiceController
import org.robolectric.annotation.Config

/** Safe mode (the last run crashed): nothing is set up. */
class SafeModeHost : Application(), PlaybackHost {
    override val localPlayer: LocalPlayerRepository get() = error("not set up in safe mode")
    override val players: PlayersRepository get() = error("not set up in safe mode")
    override val events: EventsRepository get() = error("not set up in safe mode")
    override val art: ArtRepository get() = error("not set up in safe mode")
    override val playbackReady: Boolean get() = false
}

/** A started app; "Play on this phone" as [state] says. */
open class ReadyHost(state: LocalPlayerState) : Application(), PlaybackHost {
    override val localPlayer = FakeLocal(state)
    override val players = FakePlayers()
    override val events = FakeEvents()
    override val art = FakeArt()
}

class OffHost : ReadyHost(LocalPlayerState())

class OnHost : ReadyHost(running(LocalPlayback.PAUSED))

/**
 * The media session service is exported (cars and assistants bind to it), so any app can
 * start it, including with startForegroundService: Android then requires it to go to the
 * foreground, or ends the app with a crash (and the next start would be safe mode). Media3
 * goes to the foreground only while something plays, so the service does it itself when
 * started by anything but the app: a placeholder, removed at once, and it stops if there is
 * nothing to play. The app's own starts show nothing (review, 12 November).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class PlaybackServiceLifecycleTest {
    private val context: Context get() = ApplicationProvider.getApplicationContext()
    private val services = mutableListOf<ServiceController<PlaybackService>>()

    /** Every service made is destroyed (its media session released: their ids are process-wide). */
    @After
    fun tearDown() {
        services.forEach { runCatching { it.destroy() } }
        shadowOf(Looper.getMainLooper()).idle()
    }

    private fun foreign() = Intent(context, PlaybackService::class.java)

    private fun start(intent: Intent?, refuse: Boolean = false): PlaybackService {
        val c = Robolectric.buildService(PlaybackService::class.java, intent).create().also { services += it }
        if (refuse) shadowOf(c.get()).setThrowInStartForeground(ForegroundServiceStartNotAllowedException("test: refused"))
        c.get().onStartCommand(intent, 0, 1)
        shadowOf(Looper.getMainLooper()).idle()
        return c.get()
    }

    /** Went to the foreground (as Android requires) and straight out again. */
    private fun placeholderCameAndWent(s: PlaybackService) {
        val shadow = shadowOf(s)
        assertNotEquals("went to the foreground", 0, shadow.lastForegroundNotificationId)
        assertTrue("and left it", shadow.isForegroundStopped)
        assertFalse("the notification went with it", shadow.isLastForegroundNotificationAttached)
    }

    @Test
    @Config(application = SafeModeHost::class)
    fun inSafeModeItStillGoesToTheForegroundThenStops() {
        val s = start(foreign())
        placeholderCameAndWent(s)
        assertTrue(shadowOf(s).isStoppedBySelf)
    }

    @Test
    @Config(application = OffHost::class)
    fun startedByAnotherAppWithNothingToPlayItStops() {
        val s = start(foreign())
        placeholderCameAndWent(s)
        assertTrue(shadowOf(s).isStoppedBySelf)
    }

    @Test
    @Config(application = OnHost::class)
    fun startedByAnotherAppWhileThePhonePlaysItLeavesOnlyThePlaceholder() {
        val s = start(foreign())
        placeholderCameAndWent(s)
        assertFalse("the phone's player keeps its session", shadowOf(s).isStoppedBySelf)
    }

    @Test
    @Config(application = OnHost::class)
    fun theAppsOwnStartShowsNothing() {
        val s = start(PlaybackService.startIntent(context))
        assertEquals("no placeholder for the app's own start", 0, shadowOf(s).lastForegroundNotificationId)
        assertFalse(shadowOf(s).isStoppedBySelf)
    }

    @Test
    @Config(application = OnHost::class)
    fun aGuessedTokenIsAnotherApps() {
        placeholderCameAndWent(start(foreign().putExtra(PlaybackService.EXTRA_START, "guess")))
    }

    @Test
    @Config(application = OffHost::class)
    fun aRefusedForegroundIsHarmless() {
        // Started with plain startService from the background: Android refuses the foreground,
        // and nothing requires it then.
        val s = start(foreign(), refuse = true)
        assertTrue(shadowOf(s).isStoppedBySelf)
    }

    @Test
    @Config(application = OnHost::class)
    fun aRestartWithNoIntentNeedsNoForeground() {
        val s = start(null)
        assertEquals(0, shadowOf(s).lastForegroundNotificationId)
    }

    @Test
    @Config(application = OnHost::class)
    fun createdAndDestroyedCleanly() {
        val c = Robolectric.buildService(PlaybackService::class.java, PlaybackService.startIntent(context)).create()
        c.startCommand(0, 1)
        shadowOf(Looper.getMainLooper()).idle()
        c.destroy()
        // Its session is released: another service can make one again.
        start(PlaybackService.startIntent(context))
    }
}
