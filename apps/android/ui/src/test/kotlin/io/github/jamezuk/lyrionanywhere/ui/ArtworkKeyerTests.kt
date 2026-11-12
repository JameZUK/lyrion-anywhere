package io.github.jamezuk.lyrionanywhere.ui

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import coil3.request.Options
import io.github.jamezuk.lyrionanywhere.core.Device
import io.github.jamezuk.lyrionanywhere.core.SignedIn
import io.github.jamezuk.lyrionanywhere.core.User
import io.github.jamezuk.lyrionanywhere.core.UserRole
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.ServerInfo
import io.github.jamezuk.lyrionanywhere.ui.art.Artwork
import io.github.jamezuk.lyrionanywhere.ui.art.ArtworkKeyer
import io.github.jamezuk.lyrionanywhere.ui.art.artScopeOf
import io.github.jamezuk.lyrionanywhere.ui.art.offlineArtKey
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * Covers are cached in memory by key, and an artwork key (`music/123/cover`, a downloaded
 * album's reference) names an image only on its own server: after a change of server the
 * same key must not bring back the old server's cover (review, 12 November).
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class ArtworkKeyerTests {
    private val options = Options(ApplicationProvider.getApplicationContext())
    private val a = ServerInfo("https://music.example.com/", "music.example.com", true)
    private val b = ServerInfo("https://other.example.com/", "other.example.com", true)
    private val who = SignedIn(User("alice", UserRole.LISTENER), Device(1, "Phone", "02:4c:41:00:00:01"))

    @Test
    fun theSameKeyOnAnotherServerIsAnotherImage() {
        var state: AuthState = AuthState.SignedInState(a, who)
        val keyer = ArtworkKeyer { artScopeOf(state) }
        val cover = Artwork("music/123/cover", 300)
        val offline = Artwork(offlineArtKey("a7"), 300)
        val onA = keyer.key(cover, options)
        val offlineOnA = keyer.key(offline, options)
        // The same server, whatever the sign-in state: the same cached image.
        state = AuthState.Unreachable(a)
        assertEquals(onA, keyer.key(cover, options))
        assertEquals(offlineOnA, keyer.key(offline, options))
        // Another server: never the first server's cover.
        state = AuthState.SignedOut(b)
        assertNotEquals(onA, keyer.key(cover, options))
        assertNotEquals(offlineOnA, keyer.key(offline, options))
        // Size still counts.
        assertNotEquals(keyer.key(cover, options), keyer.key(cover.copy(sizePx = 64), options))
    }

    @Test
    fun theScopeIsTheServerOnly() {
        assertEquals("", artScopeOf(AuthState.Starting))
        assertEquals("", artScopeOf(AuthState.NoServer))
        assertEquals(a.url, artScopeOf(AuthState.Restoring(a)))
        assertEquals(a.url, artScopeOf(AuthState.SignedInState(a, who)))
        assertEquals(b.url, artScopeOf(AuthState.SignedOut(b, revoked = true)))
    }
}
