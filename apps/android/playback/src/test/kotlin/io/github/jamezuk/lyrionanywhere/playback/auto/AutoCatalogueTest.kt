package io.github.jamezuk.lyrionanywhere.playback.auto

import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.BrowseNode
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.Selection
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** The car's browse tree: what each node holds, search, and playing on this phone (D20). */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class AutoCatalogueTest {
    private val source = FakeAutoSource()
    private val codec = FakeCodec()
    private val tree = AutoCatalogue(source, { node -> node::class.simpleName ?: "?" }, codec)

    private fun id(node: BrowseNode) = codec.id(node)!!

    private fun titles(items: List<MediaItem>?) = items!!.map { it.mediaMetadata.title.toString() }

    /** Nothing the car is given carries a location: no URI, no artwork URI, no extras. */
    private fun assertInert(items: List<MediaItem>) {
        for (m in items) {
            assertNull(m.localConfiguration)
            assertNull(m.mediaMetadata.artworkUri)
            assertNull(m.mediaMetadata.extras)
            assertNull(m.requestMetadata.mediaUri)
            assertFalse(m.mediaId.contains("/") || m.mediaId.contains(":"))
        }
    }

    @Test
    fun theTabsFollowTheConnection() = runTest {
        val root = tree.root()
        assertTrue(root.mediaMetadata.isBrowsable == true)
        assertEquals(listOf("Home", "Library", "Downloads"), titles(tree.children(root.mediaId, 0, 20)))
        source.isOnline = false
        assertEquals(listOf("Downloads"), titles(tree.children(root.mediaId, 0, 20)))
        source.isOnline = true
        source.albumsHere.clear()
        assertEquals(listOf("Home", "Library"), titles(tree.children(root.mediaId, 0, 20)))
        assertTrue("a cold start waits for the sign-in", source.settled > 0)
    }

    @Test
    fun homeOffersToCarryOnOnlyWithAQueue() = runTest {
        assertEquals(listOf("NewMusic", "Favourites", "Playlists"), titles(tree.children(id(BrowseNode.Home), 0, 20)))
        source.queued = true
        val home = tree.children(id(BrowseNode.Home), 0, 20)!!
        assertEquals("Resume", home[0].mediaMetadata.title)
        assertTrue(home[0].mediaMetadata.isPlayable == true)
        assertFalse(home[0].mediaMetadata.isBrowsable == true)
    }

    @Test
    fun libraryListsArePagedThroughTheGatewayRepositories() = runTest {
        val first = tree.children(id(BrowseNode.Albums), 0, 10)!!
        assertEquals(10, first.size)
        assertEquals("Album 1", first[0].mediaMetadata.title)
        val second = tree.children(id(BrowseNode.Albums), 2, 10)!!
        assertEquals("Album 21", second[0].mediaMetadata.title)
        assertTrue(source.library.calls.contains("albums 20 10 TITLE"))
        tree.children(id(BrowseNode.NewMusic), 0, 5)
        assertTrue(source.library.calls.contains("albums 0 5 NEWEST"))
        // Pages are bounded whatever the car asks.
        tree.children(id(BrowseNode.Albums), 0, 100_000)
        assertTrue(source.library.calls.contains("albums 0 100 TITLE"))
        assertEquals(emptyList<MediaItem>(), tree.children(id(BrowseNode.Albums), -1, 10))
        assertInert(first)
        // Albums open and play; artists open.
        assertTrue(first.all { it.mediaMetadata.isBrowsable == true && it.mediaMetadata.isPlayable == true })
        val artists = tree.children(id(BrowseNode.Artists), 0, 10)!!
        assertEquals(listOf("Artist", "Band"), titles(artists))
        assertTrue(artists.all { it.mediaMetadata.isPlayable == false })
        assertEquals(MediaMetadata.MEDIA_TYPE_ARTIST, artists[0].mediaMetadata.mediaType)
    }

    @Test
    fun albumsPlaylistsAndFavouritesOpen() = runTest {
        val tracks = tree.children(id(BrowseNode.Album(7)), 0, 5)!!
        assertEquals(5, tracks.size)
        assertTrue(tracks.all { it.mediaMetadata.isPlayable == true && it.mediaMetadata.isBrowsable == false })
        assertEquals(BrowseNode.AlbumTrack(7, 2u), codec.parse(tracks[2].mediaId))
        val rest = tree.children(id(BrowseNode.Album(7)), 2, 5)!!
        assertEquals(BrowseNode.AlbumTrack(7, 10u), codec.parse(rest[0].mediaId))
        assertEquals(2, rest.size)
        assertEquals(emptyList<MediaItem>(), tree.children(id(BrowseNode.Album(7)), 9, 5))

        val pl = tree.children(id(BrowseNode.Playlist(5)), 0, 20)!!
        assertEquals(BrowseNode.PlaylistTrack(5, 1u), codec.parse(pl[1].mediaId))

        val favs = tree.children(id(BrowseNode.Favourites), 0, 20)!!
        assertEquals(BrowseNode.Favourite("f.1"), codec.parse(favs[0].mediaId))
        assertEquals(BrowseNode.FavouriteFolder("f.2"), codec.parse(favs[1].mediaId))
        tree.children(favs[1].mediaId, 0, 20)
        assertTrue(source.library.calls.contains("favourites 0 20 f.2"))
        assertInert(tracks + pl + favs)
    }

    @Test
    fun unknownAndUnbrowsableIdsAreRefused() = runTest {
        for (bad in listOf("", "al/7", "file:///srv/music/a.flac", "https://music.example.com/x", "n999999", "../../etc")) {
            assertNull(bad, tree.children(bad, 0, 10))
            assertNull(bad, tree.item(bad))
            // (An empty id with no query means "carry on", tested below.)
            if (bad.isNotEmpty()) assertFalse(bad, tree.play(bad, null))
        }
        // Playable but not browsable: no children.
        assertNull(tree.children(id(BrowseNode.Track(42)), 0, 10))
        assertTrue(source.library.calls.isEmpty())
        assertTrue(source.players.enqueued.isEmpty())
    }

    @Test
    fun offlineTheServersTreeIsEmptyAndTheDownloadsRemain() = runTest {
        source.isOnline = false
        assertEquals(emptyList<MediaItem>(), tree.children(id(BrowseNode.Albums), 0, 10))
        assertTrue("nothing is asked of an unreachable server", source.library.calls.isEmpty())
        val albums = tree.children(id(BrowseNode.Downloads), 0, 10)!!
        assertEquals(listOf("Record", "Another"), titles(albums))
        val tracks = tree.children(albums[0].mediaId, 0, 10)!!
        assertEquals(listOf("Song 1", "Song 2"), titles(tracks))
        assertInert(albums + tracks)
        // Playing a downloaded track plays its album from there, from the downloads.
        assertTrue(tree.play(tracks[1].mediaId, null))
        assertEquals(1, source.played.single().second)
        assertEquals(2, source.played.single().first.size)
        // Server items can't be played offline.
        assertFalse(tree.play(id(BrowseNode.Album(7)), null))
        assertTrue(source.players.enqueued.isEmpty())
    }

    @Test
    fun playingAlwaysGoesToThisPhone() = runTest {
        val phone = source.phoneId
        assertTrue(tree.play(id(BrowseNode.AlbumTrack(7, 3u)), null))
        assertTrue(tree.play(id(BrowseNode.Album(7)), null))
        assertTrue(tree.play(id(BrowseNode.PlaylistTrack(5, 1u)), null))
        assertTrue(tree.play(id(BrowseNode.Track(42)), null))
        assertEquals(
            listOf(
                listOf(phone, Selection.Album(7), Placement.PLAY_NOW, 3),
                listOf(phone, Selection.Album(7), Placement.PLAY_NOW, null),
                listOf(phone, Selection.Playlist(5), Placement.PLAY_NOW, 1),
                listOf(phone, Selection.Track(42), Placement.PLAY_NOW, null),
            ),
            source.players.enqueued,
        )
        assertTrue(tree.play(id(BrowseNode.Favourite("f.1")), null))
        assertEquals(listOf("f.1"), source.players.favourites)
        assertTrue(tree.play(id(BrowseNode.Resume), null))
        assertEquals(1, source.resumed)
        // Not playable: refused.
        assertFalse(tree.play(id(BrowseNode.Artist(3)), null))
        assertFalse(tree.play(id(BrowseNode.Albums), null))
        // The phone's player couldn't be reached: reported as failed.
        source.phoneId = null
        assertFalse(tree.play(id(BrowseNode.Album(7)), null))
        // A downloaded track index out of range is refused.
        assertFalse(tree.play(id(BrowseNode.DownloadedTrack("a7", 5u)), null))
    }

    @Test
    fun searchCoversTheLibraryAndTheDownloads() = runTest {
        val found = tree.search("so")
        val nodes = found.map { codec.parse(it.mediaId) }
        assertEquals(BrowseNode.Track(42), nodes[0])
        assertTrue(BrowseNode.Album(7) in nodes)
        assertTrue(BrowseNode.Artist(3) in nodes)
        // "Song 1" and "Song 2" are downloaded.
        assertTrue(BrowseNode.DownloadedTrack("a7", 0u) in nodes)
        assertInert(found)
        assertEquals(emptyList<MediaItem>(), tree.search("   "))
        // Long queries are cut before they reach the server.
        tree.search("x".repeat(5000))
        assertTrue(source.library.calls.any { it == "search " + "x".repeat(AutoCatalogue.MAX_QUERY) })
        source.isOnline = false
        source.library.calls.clear()
        assertTrue(tree.search("song").all { codec.parse(it.mediaId) is BrowseNode.DownloadedTrack })
        assertTrue(source.library.calls.isEmpty())
    }

    @Test
    fun aSpokenRequestPlaysTheBestMatch() = runTest {
        assertTrue(tree.play(null, "so what"))
        assertEquals(listOf(source.phoneId, Selection.Track(42), Placement.PLAY_NOW, null), source.players.enqueued.single())
        // "Play music" with nothing named: carry on.
        assertTrue(tree.play("", null))
        assertEquals(1, source.resumed)
        source.library.fail = true
        source.albumsHere.clear()
        assertFalse(tree.play(null, "nothing here"))
    }

    @Test
    fun playRequestsAreCheckedBeforeTheyReachThePlayer() {
        assertTrue(tree.acceptsPlayRequest(MediaItem.Builder().setMediaId(id(BrowseNode.Album(7))).build()))
        assertFalse(tree.acceptsPlayRequest(MediaItem.Builder().setMediaId(id(BrowseNode.Artists)).build()))
        assertFalse(tree.acceptsPlayRequest(MediaItem.Builder().setMediaId("al/7").build()))
        assertFalse(tree.acceptsPlayRequest(MediaItem.Builder().setMediaId("").build()))
        val spoken = MediaItem.Builder().setRequestMetadata(
            MediaItem.RequestMetadata.Builder().setSearchQuery("jazz").build(),
        ).build()
        assertTrue(tree.acceptsPlayRequest(spoken))
    }

    @Test
    fun textIsCleanedForTheCar() {
        assertEquals("Hello", AutoCatalogue.clean(" Hel\u0000lo\n "))
        assertEquals(200, AutoCatalogue.clean("x".repeat(1000)).length)
    }

    @Test
    fun theRootItemIsAFolder() {
        val r = tree.root()
        assertNotNull(codec.parse(r.mediaId))
        assertEquals(MediaMetadata.MEDIA_TYPE_FOLDER_MIXED, r.mediaMetadata.mediaType)
    }
}
