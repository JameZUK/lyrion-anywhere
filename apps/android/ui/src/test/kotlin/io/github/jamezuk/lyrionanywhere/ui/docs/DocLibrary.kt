package io.github.jamezuk.lyrionanywhere.ui.docs

import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.LyricLine
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.Quality
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.core.TrackLyrics
import java.math.BigInteger
import java.security.MessageDigest

/**
 * The invented library the documentation screenshots show: the same albums, artists, tracks
 * and players as the web demo's stand-in server (`tools/demo/fake_lms.py`), so the Android
 * and web screenshots tell one story. Every name here is made up.
 */
object DocLibrary {
    class Entry(val title: String, val artist: String, val year: Int, val tracks: List<String>, val genre: String)

    val ENTRIES = listOf(
        Entry("Night Ferry", "The Paper Lanterns", 2019, listOf("Harbour Lights", "Last Crossing", "Night Ferry", "Paper Moons", "Undertow", "Gulls at Dawn", "Coastline", "Home Port"), "Folk"),
        Entry("Glasshouse", "Marisol Vega", 2021, listOf("Glasshouse", "Fern & Frost", "Morning Light", "Small Weather", "Greenhouse Effect", "Seedlings", "Petrichor"), "Indie"),
        Entry("Static Bloom", "Northbound Signal", 2017, listOf("Transmission", "Static Bloom", "Radio Silence", "Low Orbit", "Signal Fire", "Northbound"), "Electronic"),
        Entry("Saltwater Hymns", "Ada & the Undertow", 2015, listOf("Saltwater Hymn", "Driftwood", "The Long Tide", "Lantern Song", "Fathoms", "Shoreline"), "Folk"),
        Entry("Small Hours", "Kestrel Lane", 2022, listOf("3 a.m.", "Streetlights", "Small Hours", "Night Bus", "Neon Rain", "First Train"), "Indie"),
        Entry("Winter Études", "Orchard Quartet", 2012, listOf("Étude No. 1", "Étude No. 2", "Snowfield", "Étude No. 3", "Hoarfrost", "Étude No. 4"), "Classical"),
        Entry("Parallax", "Lumen Drift", 2020, listOf("Parallax", "Afterimage", "Wavelength", "Prism", "Refraction", "Event Horizon", "Lumen"), "Electronic"),
        Entry("Analogue Summer", "The Copper Hours", 2018, listOf("Tape Hiss", "Analogue Summer", "Polaroid", "Long Drive", "Sun Flare", "Cassette Heart"), "Indie"),
        Entry("Tidal Lines", "Haruka Mori", 2023, listOf("Tidal Lines", "Rock Pools", "Moonpull", "Sea Glass", "Ebb", "Flood"), "Ambient"),
        Entry("Field Recordings", "Brass Atlas", 2016, listOf("Field Recording I", "Meadowlark", "Hedgerow", "Field Recording II", "Barn Owl", "Evensong"), "Ambient"),
        Entry("Blue Hour Radio", "Selene Park", 2024, listOf("Blue Hour", "Late Show", "Request Line", "Dedication", "Sign-off"), "Jazz"),
        Entry("Hymns for the Road", "Tin Roof Choir", 2014, listOf("Open Road", "Tin Roof", "Mile Marker", "Rest Stop", "Homeward"), "Folk"),
    )

    val ARTISTS = (ENTRIES.map { it.artist } + "Various Artists").distinct().sorted()

    /** Albums in the order they were "added" (most recent first), as the demo has them. */
    val ADDED = listOf(10, 8, 4, 6, 1, 11, 0, 2, 7, 3, 9, 5)

    /** Recently played, most recent first. */
    val LAST_PLAYED = listOf(6, 0, 3, 7, 1, 10, 4)

    fun artworkKey(albumId: Long) = (1000 + albumId).toString()

    fun album(index: Int): Album {
        val e = ENTRIES[index]
        val id = index + 1L
        return Album(id, e.title, e.artist, ARTISTS.indexOf(e.artist) + 1L, e.year, artworkKey(id), false)
    }

    val ALBUMS: List<Album> = ENTRIES.indices.map(::album)

    /** An invented length for a track, as the demo works it out (150 to 329 seconds). */
    fun seconds(title: String): Int {
        val d = MessageDigest.getInstance("SHA-1").digest(title.toByteArray())
        return 150 + BigInteger(1, d).mod(BigInteger.valueOf(180)).toInt()
    }

    val CD = Quality("FLAC", "FLAC 16/44.1", true, false, 44_100u, 16u, null)
    val HI_RES = Quality("FLAC", "FLAC 24/96", true, true, 96_000u, 24u, null)

    /** The tracks of album [index] (ids as the demo numbers them). */
    fun tracks(index: Int, hiRes: Boolean = index == 0): List<Track> {
        val e = ENTRIES[index]
        val first = 101L + ENTRIES.take(index).sumOf { it.tracks.size }
        val albumId = index + 1L
        return e.tracks.mapIndexed { n, title ->
            Track(
                id = first + n,
                title = title,
                artist = e.artist,
                album = e.title,
                albumId = albumId,
                artistId = ARTISTS.indexOf(e.artist) + 1L,
                durationSecs = seconds(title).toDouble(),
                trackNumber = (n + 1).toUInt(),
                artwork = artworkKey(albumId),
                queueIndex = null,
                discNumber = null,
                discCount = null,
                genre = e.genre,
                format = if (hiRes) "FLAC 24/96" else "FLAC 16/44.1",
                year = e.year,
                quality = if (hiRes) HI_RES else CD,
            )
        }
    }

    // The players: three rooms in LMS, and this phone.
    const val PHONE_ID = "02:00:00:00:00:01"
    const val LOUNGE = "aa:bb:cc:00:00:01"
    const val STUDY = "aa:bb:cc:00:00:02"
    const val GARDEN = "aa:bb:cc:00:00:03"
    val PLAYERS = listOf(
        Player(PHONE_ID, "Phone", "Lyrion Anywhere", true, true, false),
        Player(LOUNGE, "Lounge", "squeezelite", true, true, true),
        Player(STUDY, "Study", "squeezelite", true, true, false),
        Player(GARDEN, "Garden Room", "squeezelite", true, false, false),
    )

    /** The queue: Night Ferry, then a few more albums' worth. */
    val QUEUE: List<Track> = (tracks(0) + tracks(1).take(4)).mapIndexed { i, t -> t.copy(queueIndex = i.toUInt()) }

    /** The track playing: "Night Ferry", 1 min 23 s in. */
    const val NOW_INDEX = 2
    const val ELAPSED = 83.4

    fun status(name: String = "Lounge", mode: PlaybackMode = PlaybackMode.PLAYING) = PlayerStatus(
        name = name,
        connected = true,
        powered = true,
        mode = mode,
        elapsedSecs = ELAPSED,
        durationSecs = QUEUE[NOW_INDEX].durationSecs,
        volume = 42u,
        muted = false,
        shuffle = ShuffleMode.OFF,
        repeat = RepeatMode.OFF,
        queueIndex = NOW_INDEX.toUInt(),
        queueLength = QUEUE.size.toUInt(),
        canSeek = true,
        current = QUEUE[NOW_INDEX],
        next = QUEUE[NOW_INDEX + 1],
    )

    /** "Night Ferry"'s lyrics, written for the demo (synced). */
    val LYRICS = TrackLyrics(
        false,
        true,
        listOf(
            12_000 to "The harbour lights are folding up like paper",
            19_500 to "The gulls have gone to sleep along the pier",
            27_000 to "I've packed the things I'll need and left the rest behind me",
            34_500 to "There's nothing left to keep me waiting here",
            44_000 to "So take the night ferry, take the long way round",
            51_500 to "Count the lanterns as they leave the sound",
            59_000 to "Every mile of water is a mile I've found",
            66_500 to "On the night ferry, homeward bound",
            76_000 to "The engine hums a hymn below the decking",
            83_500 to "The stars are swinging on the mast above",
            91_000 to "I write your name in salt along the railing",
            98_500 to "And watch the spray come up and wash it off",
        ).map { (ms, text) -> LyricLine(ms.toUInt(), text) },
    )
}
