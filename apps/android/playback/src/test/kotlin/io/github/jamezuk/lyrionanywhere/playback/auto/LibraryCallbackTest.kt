package io.github.jamezuk.lyrionanywhere.playback.auto

import android.os.Looper
import androidx.annotation.OptIn
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.LibraryResult
import androidx.media3.session.MediaLibraryService.MediaLibrarySession
import androidx.media3.session.MediaSession
import androidx.media3.session.SessionCommand
import androidx.media3.session.SessionError
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.BrowseNode
import io.github.jamezuk.lyrionanywhere.playback.LmsSessionPlayer
import io.github.jamezuk.lyrionanywhere.playback.RecordingActions
import io.github.jamezuk.lyrionanywhere.playback.SessionSnapshot
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.util.concurrent.ExecutionException
import java.util.concurrent.TimeUnit

/**
 * The exported library session's gate: who connects with what, and that library calls and
 * play requests are refused for anyone but a browsing caller.
 */
@OptIn(UnstableApi::class)
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class LibraryCallbackTest {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
    private val source = FakeAutoSource()
    private val codec = FakeCodec()
    private val tree = AutoCatalogue(source, { it::class.simpleName ?: "?" }, codec)

    private fun controller(pkg: String, uid: Int) =
        MediaSession.ControllerInfo.createTestOnlyControllerInfo(pkg, 0, uid, 0, 0, false, android.os.Bundle.EMPTY, true)

    private val car = controller("com.google.android.projection.gearhead", 10_001)
    private val watch = controller("com.example.watch", 10_002)
    private val stranger = controller("com.example.stranger", 10_003)

    private val decisions = mapOf(
        car.packageName to CallerAccess.BROWSE,
        watch.packageName to CallerAccess.CONTROL,
        stranger.packageName to CallerAccess.REJECT,
    )
    private val callback = LibraryCallback({ decisions.getValue(it.packageName) }, { tree }, scope)
    private val player = LmsSessionPlayer(Looper.getMainLooper(), MutableStateFlow(SessionSnapshot()), RecordingActions(), scope)
    private val session = MediaLibrarySession.Builder(ApplicationProvider.getApplicationContext<android.content.Context>(), player, callback).build()

    @After
    fun tearDown() {
        session.release()
        player.release()
        scope.cancel()
    }

    private fun <T> done(f: com.google.common.util.concurrent.ListenableFuture<T>): T {
        shadowOf(Looper.getMainLooper()).idle()
        return f.get(5, TimeUnit.SECONDS)
    }

    @Test
    fun theCarMayBrowseTheWatchMayOnlyControlAndStrangersAreRejected() {
        val car = callback.onConnect(session, car)
        assertTrue(car.isAccepted)
        assertTrue(car.availableSessionCommands.contains(SessionCommand.COMMAND_CODE_LIBRARY_GET_CHILDREN))
        assertTrue(car.availableSessionCommands.contains(SessionCommand.COMMAND_CODE_LIBRARY_SEARCH))

        val watch = callback.onConnect(session, watch)
        assertTrue(watch.isAccepted)
        assertFalse(watch.availableSessionCommands.contains(SessionCommand.COMMAND_CODE_LIBRARY_GET_CHILDREN))
        assertFalse(watch.availableSessionCommands.contains(SessionCommand.COMMAND_CODE_LIBRARY_GET_LIBRARY_ROOT))
        assertTrue(watch.availablePlayerCommands.contains(Player.COMMAND_PLAY_PAUSE))
        assertFalse("no playing by id", watch.availablePlayerCommands.contains(Player.COMMAND_SET_MEDIA_ITEM))
        assertFalse(watch.availablePlayerCommands.contains(Player.COMMAND_CHANGE_MEDIA_ITEMS))

        assertFalse(callback.onConnect(session, stranger).isAccepted)
    }

    @Test
    fun libraryCallsAreRefusedForAnyoneButABrowsingCaller() {
        for (who in listOf(watch, stranger)) {
            assertEquals(SessionError.ERROR_PERMISSION_DENIED, done(callback.onGetLibraryRoot(session, who, null)).resultCode)
            val children = done(callback.onGetChildren(session, who, tree.root().mediaId, 0, 10, null))
            assertEquals(SessionError.ERROR_PERMISSION_DENIED, children.resultCode)
            assertEquals(SessionError.ERROR_PERMISSION_DENIED, done(callback.onSearch(session, who, "x", null)).resultCode)
            assertEquals(
                SessionError.ERROR_PERMISSION_DENIED,
                done(callback.onGetItem(session, who, tree.root().mediaId)).resultCode,
            )
        }
        assertTrue(source.library.calls.isEmpty())
    }

    @Test
    fun theCarBrowsesTheTree() {
        val root = done(callback.onGetLibraryRoot(session, car, null))
        assertEquals(LibraryResult.RESULT_SUCCESS, root.resultCode)
        val tabs = done(callback.onGetChildren(session, car, root.value!!.mediaId, 0, 10, null))
        assertEquals(listOf("Home", "Library", "Downloads"), tabs.value!!.map { it.mediaMetadata.title.toString() })
        val bad = done(callback.onGetChildren(session, car, "file:///srv/music", 0, 10, null))
        assertEquals(SessionError.ERROR_BAD_VALUE, bad.resultCode)
    }

    @Test
    fun searchResultsArePagedPerCaller() {
        done(callback.onSearch(session, car, "so", null))
        val page = done(callback.onGetSearchResult(session, car, "so", 0, 2, null))
        assertEquals(2, page.value!!.size)
        // Another caller never sees this caller's results.
        val other = controller("com.google.android.projection.gearhead", 10_009)
        assertTrue(done(callback.onGetSearchResult(session, other, "so", 0, 2, null)).value!!.isEmpty())
    }

    @Test
    fun playRequestsAreCheckedAndStrippedToAnId() {
        val album = codec.id(BrowseNode.Album(7))!!
        val fromCar = MediaItem.Builder().setMediaId(album).setUri("https://example.com/evil.mp3").build()
        val ok = done(callback.onAddMediaItems(session, car, mutableListOf(fromCar)))
        assertEquals(album, ok.single().mediaId)
        assertEquals(null, ok.single().localConfiguration)

        fun refused(who: MediaSession.ControllerInfo, items: MutableList<MediaItem>) {
            try {
                done(callback.onAddMediaItems(session, who, items))
                throw AssertionError("accepted $items for ${who.packageName}")
            } catch (_: ExecutionException) {
            }
        }
        refused(watch, mutableListOf(MediaItem.Builder().setMediaId(album).build()))
        refused(stranger, mutableListOf(MediaItem.Builder().setMediaId(album).build()))
        refused(car, mutableListOf(MediaItem.Builder().setMediaId("al/7").build()))
        refused(car, mutableListOf(MediaItem.Builder().setMediaId(codec.id(BrowseNode.Artists)!!).build()))
        refused(car, mutableListOf())
        refused(car, MutableList(2) { MediaItem.Builder().setMediaId(album).build() })
    }
}
