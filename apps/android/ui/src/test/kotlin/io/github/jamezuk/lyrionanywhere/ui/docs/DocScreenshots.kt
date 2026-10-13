package io.github.jamezuk.lyrionanywhere.ui.docs

import android.graphics.Bitmap
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.hasScrollToNodeAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.AccountInfo
import io.github.jamezuk.lyrionanywhere.core.DownloadOrigin
import io.github.jamezuk.lyrionanywhere.core.DownloadState
import io.github.jamezuk.lyrionanywhere.core.DownloadFormat
import io.github.jamezuk.lyrionanywhere.core.DownloadItem
import io.github.jamezuk.lyrionanywhere.core.Favourite
import io.github.jamezuk.lyrionanywhere.core.HomeLayout
import io.github.jamezuk.lyrionanywhere.core.HomeShelf
import io.github.jamezuk.lyrionanywhere.core.OfflineAlbum
import io.github.jamezuk.lyrionanywhere.core.SavedPlaylist
import io.github.jamezuk.lyrionanywhere.core.ShelfKind
import io.github.jamezuk.lyrionanywhere.core.SignalPathInfo
import io.github.jamezuk.lyrionanywhere.core.SignalStage
import io.github.jamezuk.lyrionanywhere.core.SignalVerdict
import io.github.jamezuk.lyrionanywhere.core.Station
import io.github.jamezuk.lyrionanywhere.core.StationSpec
import io.github.jamezuk.lyrionanywhere.data.BusiestDay
import io.github.jamezuk.lyrionanywhere.data.ListeningStats
import io.github.jamezuk.lyrionanywhere.data.LibraryAlbumOrder
import io.github.jamezuk.lyrionanywhere.data.LibraryChip
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.MonthTotal
import io.github.jamezuk.lyrionanywhere.data.NewArtist
import io.github.jamezuk.lyrionanywhere.data.NowPlayingLayout
import io.github.jamezuk.lyrionanywhere.data.NowPlayingPrefs
import io.github.jamezuk.lyrionanywhere.data.ReadAheadSettings
import io.github.jamezuk.lyrionanywhere.data.ReplayCache
import io.github.jamezuk.lyrionanywhere.data.ReplayCacheUsage
import io.github.jamezuk.lyrionanywhere.data.SoundSettings
import io.github.jamezuk.lyrionanywhere.data.StatsPeriod
import io.github.jamezuk.lyrionanywhere.data.StatsRange
import io.github.jamezuk.lyrionanywhere.data.StatsTotals
import io.github.jamezuk.lyrionanywhere.data.Streak
import io.github.jamezuk.lyrionanywhere.data.TopArtist
import io.github.jamezuk.lyrionanywhere.data.TopEntry
import io.github.jamezuk.lyrionanywhere.data.Waveform
import io.github.jamezuk.lyrionanywhere.data.statsPeriodChoices
import io.github.jamezuk.lyrionanywhere.ui.FakeSharer
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.car.CarListsUi
import io.github.jamezuk.lyrionanywhere.ui.car.CarModeScreen
import io.github.jamezuk.lyrionanywhere.ui.carActions
import io.github.jamezuk.lyrionanywhere.ui.components.MainTab
import io.github.jamezuk.lyrionanywhere.ui.components.HomeSettingsButton
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumActions
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumBrowser
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumDetail
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumScreen
import io.github.jamezuk.lyrionanywhere.ui.library.HomeActions
import io.github.jamezuk.lyrionanywhere.ui.library.HomeFeed
import io.github.jamezuk.lyrionanywhere.ui.library.HomeScreen
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryScreen
import io.github.jamezuk.lyrionanywhere.ui.library.Loadable
import io.github.jamezuk.lyrionanywhere.ui.library.SparseState
import io.github.jamezuk.lyrionanywhere.ui.localStatus
import io.github.jamezuk.lyrionanywhere.ui.nowPlayingActions
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsActions
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsUiState
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnActions
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnList
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.QueueActions
import io.github.jamezuk.lyrionanywhere.ui.player.QueueEntry
import io.github.jamezuk.lyrionanywhere.ui.player.QueueScreen
import io.github.jamezuk.lyrionanywhere.ui.player.QueueScreenActions
import io.github.jamezuk.lyrionanywhere.ui.player.SignalPathPanel
import io.github.jamezuk.lyrionanywhere.ui.settings.ReadAheadSetting
import io.github.jamezuk.lyrionanywhere.ui.settings.ReplayCacheSetting
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsUiState
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundActions
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundUiState
import io.github.jamezuk.lyrionanywhere.ui.stats.StatsActions
import io.github.jamezuk.lyrionanywhere.ui.stats.StatsScreen
import io.github.jamezuk.lyrionanywhere.ui.stats.StatsUiState
import io.github.jamezuk.lyrionanywhere.ui.stats.StoryUiState
import io.github.jamezuk.lyrionanywhere.ui.stats.YearStoryScreen
import io.github.jamezuk.lyrionanywhere.ui.stats.storyCards
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import java.io.File
import java.time.LocalDate
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/*
 * The documentation screenshots (the PNGs in docs/images/android): the app's real screens and
 * theme, rendered on the JVM by Robolectric's native graphics, fed with the invented library
 * in [DocLibrary] and generated cover art ([DocArt]). Nothing comes from a real installation.
 *
 * Every normal test run renders them all and checks each is a real picture (not blank);
 * they are written to docs/images/android only when asked:
 *
 *     ./gradlew :ui:testDebugUnitTest --tests '*DocScreenshots*' -PrecordDocScreenshots
 *
 * (see docs/RELEASING.md › Android screenshots).
 */

private val TODAY: LocalDate = LocalDate.of(2026, 10, 13)

/** Shared by the phone and tablet classes: render, check and (when recording) write. */
abstract class DocShots {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @Before
    fun setUp() {
        DocArt.install(ApplicationProvider.getApplicationContext())
        compose.runOnUiThread { WindowCompat.setDecorFitsSystemWindows(compose.activity.window, false) }
    }

    /** Give the app a status bar and gesture bar, as a phone would. */
    fun applyInsets() {
        compose.runOnUiThread {
            val content = compose.activity.findViewById<ViewGroup>(android.R.id.content)
            val density = content.resources.displayMetrics.density
            val top = (STATUS_BAR_DP * density).toInt()
            val bottom = (NAV_BAR_DP * density).toInt()
            val insets = WindowInsetsCompat.Builder()
                .setInsets(WindowInsetsCompat.Type.statusBars(), Insets.of(0, top, 0, 0))
                .setInsets(WindowInsetsCompat.Type.navigationBars(), Insets.of(0, 0, 0, bottom))
                .build()
            ViewCompat.dispatchApplyWindowInsets(content, insets)
        }
    }

    /** Scroll the screen's list up by [dp], slowly (no fling), to show what is below. */
    fun scrollUp(dp: Int) {
        compose.onAllNodes(hasScrollAction()).onFirst().performTouchInput {
            val px = dp * density
            swipe(Offset(centerX, centerY + px / 2), Offset(centerX, centerY - px / 2), durationMillis = 1_500)
        }
    }

    private fun settle() {
        repeat(4) {
            compose.mainClock.advanceTimeBy(500)
            compose.waitForIdle()
        }
    }

    fun shot(
        name: String,
        darkIcons: Boolean? = null,
        prepare: () -> Unit = {},
        content: @Composable () -> Unit,
    ) {
        compose.setContent { LyrionTheme(dynamicColour = false) { DeviceFrame(darkIcons) { content() } } }
        compose.waitForIdle()
        applyInsets()
        settle()
        prepare()
        settle()
        val image = compose.onRoot().captureToImage().asAndroidBitmap()
        DocShotFiles.check(name, image)
        DocShotFiles.write(name, image)
    }
}

/** Checks each picture, and writes it when recording (`-PrecordDocScreenshots`). */
object DocShotFiles {
    /** Where to write them: set by the Gradle build only when recording. */
    private val dir: File? = System.getProperty("docScreenshots.dir")?.takeIf { it.isNotBlank() }?.let(::File)

    /** Widest picture written (larger screens are scaled down to this). */
    private const val MAX_WIDTH = 1080

    fun check(name: String, image: Bitmap) {
        assertTrue("$name is empty", image.width > 100 && image.height > 100)
        // A real screen has plenty of colours; a blank or failed render has a handful.
        val colours = HashSet<Int>()
        for (y in 0 until image.height step 7) {
            for (x in 0 until image.width step 7) colours += image.getPixel(x, y)
        }
        assertTrue("$name looks blank (${colours.size} colours)", colours.size > 40)
    }

    fun write(name: String, image: Bitmap) {
        val out = dir ?: return
        out.mkdirs()
        val scaled = if (image.width > MAX_WIDTH) {
            Bitmap.createScaledBitmap(image, MAX_WIDTH, image.height * MAX_WIDTH / image.width, true)
        } else {
            image
        }
        // Opaque RGB (no alpha channel): a good deal smaller than Android's RGBA PNGs.
        val pixels = IntArray(scaled.width * scaled.height)
        scaled.getPixels(pixels, 0, scaled.width, 0, 0, scaled.width, scaled.height)
        val rgb = java.awt.image.BufferedImage(scaled.width, scaled.height, java.awt.image.BufferedImage.TYPE_INT_RGB)
        rgb.setRGB(0, 0, scaled.width, scaled.height, pixels, 0, scaled.width)
        val writer = javax.imageio.ImageIO.getImageWritersByFormatName("png").next()
        val params = writer.defaultWriteParam.apply {
            // The strongest compression (lossless either way).
            compressionMode = javax.imageio.ImageWriteParam.MODE_EXPLICIT
            compressionQuality = 0f
        }
        val file = File(out, "$name.png")
        file.delete()
        javax.imageio.ImageIO.createImageOutputStream(file).use { stream ->
            writer.output = stream
            writer.write(null, javax.imageio.IIOImage(rgb, null, null), params)
        }
        writer.dispose()
    }
}

/** What the screenshots show: one listener's invented library and players. */
object DocStates {
    /** A gentle, invented waveform for the seek bar. */
    private fun waveform(trackId: Long, secs: Double): Waveform {
        val n = 240
        val peaks = ByteArray(n) { i ->
            val t = i / n.toDouble()
            val swell = 0.55 + 0.35 * kotlin.math.sin(t * Math.PI * 3.2) * kotlin.math.sin(t * Math.PI)
            val grain = 0.12 * kotlin.math.sin(i * 1.7) + 0.08 * kotlin.math.sin(i * 0.37)
            ((swell + grain).coerceIn(0.08, 1.0) * 255).toInt().toByte()
        }
        return Waveform(trackId, peaks, secs)
    }

    val PHONE_PATH = SignalPathInfo(
        sourceCodec = "FLAC",
        sourceLossless = true,
        sourceBits = 24u,
        sourceRate = 96_000u,
        sourceChannels = 2u,
        stages = listOf(SignalStage("Decode (lossless)", true, false), SignalStage("Volume (fixed, 100%)", true, false)),
        dspActive = false,
        outputDevice = "USB audio interface",
        outputRate = 96_000u,
        outputFormat = "24-bit integer",
        outputBits = 24u,
        direct = true,
        fixedVolume = true,
        resampled = false,
        engineBitPerfect = true,
        bitPerfect = true,
        notBitPerfect = emptyList(),
        verdict = SignalVerdict.BIT_PERFECT,
        summary = "",
    )

    fun player(selected: String = DocLibrary.LOUNGE, prefs: NowPlayingPrefs = NowPlayingPrefs()): PlayerUiState {
        val phone = selected == DocLibrary.PHONE_ID
        val status = DocLibrary.status(name = if (phone) "Phone" else "Lounge")
        val current = status.current!!
        return PlayerUiState(
            players = DocLibrary.PLAYERS,
            selectedId = selected,
            status = status,
            statusAt = 0,
            queue = DocLibrary.QUEUE.mapIndexed { i, t -> QueueEntry(i + 1L, t) },
            loading = false,
            live = true,
            local = LocalPlayerState(
                enabled = true,
                status = localStatus(DocLibrary.PHONE_ID).copy(name = "Phone", readAhead = true, cushionMs = 240_000uL),
                signalPath = PHONE_PATH,
            ),
            waveform = waveform(current.id!!, current.durationSecs!!),
            playingOn = mapOf(DocLibrary.LOUNGE to current, DocLibrary.STUDY to DocLibrary.tracks(6)[2]),
            lyrics = DocLibrary.LYRICS,
            lyricsTrackId = current.id,
            nowPlaying = prefs,
        )
    }

    /** The mini player: this phone playing (the usual case). */
    val mini get() = player(selected = DocLibrary.PHONE_ID)

    val homeActions = HomeActions({}, {}, {}, {}, {}, {}, {})

    fun home(): HomeFeed {
        val byIndex = DocLibrary.ALBUMS
        return HomeFeed(
            recent = Loadable.Ready(DocLibrary.ADDED.map { byIndex[it] }),
            random = Loadable.Ready(listOf(5, 9, 2, 11, 7, 3).map { byIndex[it] }),
            favourites = Loadable.Ready(listOf(Favourite("f0.0", "Morning Radio", null, false), Favourite("f0.1", "Late Jazz", null, false))),
            playlists = Loadable.Ready(listOf(SavedPlaylist(5, "Road trip"), SavedPlaylist(6, "Sunday morning"))),
            layout = HomeLayout(
                listOf(ShelfKind.RECENT, ShelfKind.STATIONS, ShelfKind.NEW, ShelfKind.MOST).map { HomeShelf(it, null, false) },
                false,
            ),
            layoutLoaded = true,
            history = mapOf(
                ShelfKind.RECENT to Loadable.Ready(DocLibrary.LAST_PLAYED.map { byIndex[it] }),
                ShelfKind.MOST to Loadable.Ready(listOf(0, 6, 3, 1, 10, 7).map { byIndex[it] }),
            ),
            stations = Loadable.Ready(
                listOf(
                    Station(1, "Forgotten favourites", StationSpec.Forgotten, null),
                    Station(2, "Fresh this month", StationSpec.Fresh, null),
                ),
            ),
        )
    }

    fun stats(): ListeningStats {
        val top = listOf(0, 6, 3, 1, 10, 7, 4, 8, 2, 9)
        return ListeningStats(
            enabled = true,
            retentionDays = 396,
            period = StatsRange(1_790_000_000, 1_792_600_000),
            totals = StatsTotals(412, 82_800, 301, 64, 41, 27),
            topArtists = top.mapIndexed { i, a ->
                val e = DocLibrary.ENTRIES[a]
                TopArtist(DocLibrary.ARTISTS.indexOf(e.artist) + 1L, e.artist, 48L - i * 4, 11_000L - i * 900)
            },
            topAlbums = top.mapIndexed { i, a -> TopEntry(a + 1L, 30L - i * 2, 7_000L - i * 500) },
            topTracks = top.mapIndexed { i, a -> TopEntry(DocLibrary.tracks(a)[i % 5].id!!, 14L - i, 3_000L - i * 200) },
            newArtists = listOf(NewArtist(10, "Selene Park", 1_791_000_000)),
            byHour = List(24) { h -> listOf(0, 0, 0, 0, 0, 0, 2, 8, 12, 6, 4, 3, 5, 6, 4, 3, 5, 9, 14, 22, 30, 48, 26, 8)[h].toLong() },
            byWeekday = listOf(42, 30, 48, 51, 62, 120, 88),
            byMonth = emptyList(),
            longestStreak = Streak(9, "2026-10-02", "2026-10-10"),
            busiestDay = BusiestDay("2026-10-04", 14_400),
        )
    }

    val statsAlbums = DocLibrary.ALBUMS.associateBy { it.id }
    val statsTracks = DocLibrary.ENTRIES.indices.flatMap { DocLibrary.tracks(it) }.associateBy { it.id!! }

    fun statsState() = StatsUiState(
        today = TODAY,
        period = StatsPeriod.Month(2026, 10),
        choices = statsPeriodChoices(TODAY, 396),
        loading = false,
        stats = stats().normalised(),
        albums = statsAlbums,
        tracks = statsTracks,
        error = null,
    )

    fun story() = StoryUiState(
        2026,
        loading = false,
        cards = storyCards(
            stats().copy(byMonth = listOf(MonthTotal("2026-01", 30, 6_000), MonthTotal("2026-06", 220, 41_000), MonthTotal("2026-10", 412, 82_800))),
            statsAlbums,
            statsTracks,
        ),
    )

    fun downloads(): DownloadsUiState {
        fun album(i: Int, size: Long, at: Long) = DocLibrary.ENTRIES[i].let { e ->
            OfflineAlbum("a${i + 1}", i + 1L, e.title, e.artist, e.tracks.size.toUInt(), size.toULong(), true, at.toULong(), DownloadOrigin.MANUAL)
        }
        val tidal = DocLibrary.tracks(8)
        return DownloadsUiState(
            inProgress = listOf(
                DownloadItem(tidal[3].id!!, 9, tidal[3].title, DownloadState.ACTIVE, 21_000_000uL, 34_000_000uL, DownloadFormat.ORIGINAL, DownloadOrigin.MANUAL, null, ""),
                DownloadItem(tidal[4].id!!, 9, tidal[4].title, DownloadState.QUEUED, 0uL, 0uL, DownloadFormat.ORIGINAL, DownloadOrigin.MANUAL, null, ""),
                DownloadItem(tidal[5].id!!, 9, tidal[5].title, DownloadState.QUEUED, 0uL, 0uL, DownloadFormat.ORIGINAL, DownloadOrigin.MANUAL, null, ""),
            ),
            albums = listOf(
                album(0, 412_000_000, 1_792_000_000),
                album(1, 236_000_000, 1_791_000_000),
                album(4, 198_000_000, 1_790_000_000),
                album(6, 254_000_000, 1_789_000_000),
                album(10, 171_000_000, 1_788_000_000),
            ),
            trackCount = 34,
            usedBytes = 1_271_000_000,
            capBytes = 8_000_000_000,
            canDownload = true,
        )
    }
}

/** The phone screenshots, dark (the app follows the system theme). */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h914dp-night-420dpi")
class DocScreenshotsPhone : DocShots() {
    @Test
    fun nowPlaying() = shot("now-playing") {
        PhoneShell(tab = null, mini = null) {
            NowPlayingScreen(DocStates.player(), clock = { 0L }, actions = nowPlayingActions(onClose = {}))
        }
    }

    @Test
    fun home() = shot("home") {
        PhoneShell(tab = MainTab.HOME, mini = DocStates.mini) {
            DocScreen(stringResource(R.string.nav_home), actions = { HomeSettingsButton(onClick = {}) }) {
                HomeScreen(DocStates.home(), DocStates.homeActions)
            }
        }
    }

    @Test
    fun library() = shot("library") {
        PhoneShell(tab = MainTab.LIBRARY, mini = DocStates.mini) {
            DocScreen(stringResource(R.string.nav_library)) {
                LibraryScreen(
                    libraries = emptyList(),
                    chosenLibrary = null,
                    onChooseLibrary = {},
                    onNewMusic = {},
                    onOpen = {},
                    chip = LibraryChip.ALBUMS,
                    onChip = {},
                    chipContent = {
                        AlbumBrowser(
                            SparseState(total = DocLibrary.ALBUMS.size, pages = mapOf(0 to DocLibrary.ALBUMS)),
                            sort = LibraryAlbumOrder.TITLE,
                            grid = true,
                            onSort = {},
                            onGrid = {},
                            onEnsure = {},
                            onRetry = {},
                            onAlbum = {},
                            onMore = {},
                        )
                    },
                )
            }
        }
    }

    @Test
    fun album() = shot("album", prepare = { scrollUp(110) }) {
        PhoneShell(tab = MainTab.LIBRARY, mini = DocStates.mini) {
            DocScreen(stringResource(R.string.title_album), back = true) {
                AlbumScreen(
                    Loadable.Ready(AlbumDetail(DocLibrary.ALBUMS[1], DocLibrary.tracks(1))),
                    AlbumActions(onRetry = {}, onPlay = { _, _ -> }, onShuffle = {}, onPlayTrack = {}, onArtist = { _, _ -> }, onMore = {}, onDownload = {}),
                )
            }
        }
    }

    @Test
    fun queue() = shot("queue") {
        PhoneShell(tab = MainTab.QUEUE, mini = DocStates.mini) {
            DocScreen(stringResource(R.string.nav_queue)) {
                QueueScreen(DocStates.player(), QueueScreenActions(QueueActions(onJump = {})))
            }
        }
    }

    @Test
    fun playOn() = shot("play-on") {
        SheetOver(
            behind = {
                PhoneShell(tab = null, mini = null) {
                    NowPlayingScreen(DocStates.player(), clock = { 0L }, actions = nowPlayingActions(onClose = {}))
                }
            },
        ) {
            PlayOnList(
                DocStates.player(),
                PlayOnActions(onControl = {}, onBackToPhone = {}, onMoveHere = {}, onBringToPhone = {}, onSync = { _, _ -> }, onAllRooms = {}),
                Modifier.padding(bottom = 24.dp),
            )
        }
    }

    @Test
    fun signalPath() = shot("signal-path") {
        val phone = DocStates.player(selected = DocLibrary.PHONE_ID)
        SheetOver(
            behind = {
                PhoneShell(tab = null, mini = null) {
                    NowPlayingScreen(phone, clock = { 0L }, actions = nowPlayingActions(onClose = {}))
                }
            },
        ) {
            Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 24.dp)) {
                SignalPathPanel(phone.local.signalPath, cushionMs = phone.local.cushionMs)
            }
        }
    }

    @Test
    fun soundSettings() = shot("sound-settings") {
        PhoneShell(tab = null, mini = DocStates.mini) {
            DocScreen(stringResource(R.string.title_sound), back = true) {
                SoundScreen(SoundUiState(SoundSettings(), playerLevel = 60), object : SoundActions {})
            }
        }
    }

    @Test
    fun gapsSettings() = shot(
        "gaps-settings",
        prepare = {
            compose.onAllNodes(hasScrollToNodeAction()).onFirst().performScrollToNode(hasTestTag("read-ahead-settings"))
        },
    ) {
        PhoneShell(tab = null, mini = DocStates.mini) {
            DocScreen(stringResource(R.string.title_settings), back = true) {
                SettingsScreen(
                    state = SettingsUiState(
                        username = "alex",
                        account = AccountInfo("alex", false, true, false),
                        sound = SoundSettings(),
                        downloadsOffered = true,
                        downloadedBytes = 1_271_000_000,
                        deviceName = "Phone",
                    ),
                    appVersion = "1.0.0",
                    libraryVersion = "1.0.0",
                    onSignOut = {},
                    onLicences = {},
                    gaps = {
                        ReadAheadSetting(ReadAheadSettings()) {}
                        ReplayCacheSetting(
                            bytes = ReplayCache.DEFAULT_BYTES,
                            usage = ReplayCacheUsage(612_000_000, 48),
                            downloadsAllowed = true,
                            onSize = {},
                            onClear = {},
                        )
                    },
                )
            }
        }
    }

    @Test
    fun stats() = shot("stats") {
        PhoneShell(tab = null, mini = DocStates.mini) {
            DocScreen(stringResource(R.string.stats_title), back = true) {
                StatsScreen(DocStates.statsState(), StatsActions())
            }
        }
    }

    @Test
    fun yearStory() = shot(
        "year-story",
        prepare = {
            compose.onNodeWithTag("story-next").performClick()
            compose.onNodeWithTag("story-next").performClick()
        },
    ) {
        YearStoryScreen(DocStates.story(), onClose = {}, onRetry = {}, sharer = FakeSharer())
    }

    @Test
    fun carMode() = shot("car-mode", darkIcons = false) {
        CarModeScreen(
            DocStates.player(),
            CarListsUi(
                favourites = listOf(Favourite("f0.0", "Morning Radio", null, false)),
                stations = listOf(Station(1, "Forgotten favourites", StationSpec.Forgotten, null)),
                loading = false,
            ),
            clock = { 0L },
            actions = carActions(),
        )
    }

    @Test
    fun downloads() = shot("downloads") {
        PhoneShell(tab = null, mini = DocStates.mini) {
            DocScreen(stringResource(R.string.title_downloads), back = true) {
                DownloadsScreen(DocStates.downloads(), DownloadsActions(onStorage = {}))
            }
        }
    }
}

/** Now playing in the light theme. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h914dp-notnight-420dpi")
class DocScreenshotsPhoneLight : DocShots() {
    @Test
    fun nowPlayingLight() = shot("now-playing-light") {
        PhoneShell(tab = null, mini = null) {
            NowPlayingScreen(DocStates.player(), clock = { 0L }, actions = nowPlayingActions(onClose = {}))
        }
    }
}

/** Now playing on a tablet (landscape): the cover, the controls and the queue side by side. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w1280dp-h800dp-night-xhdpi")
class DocScreenshotsTablet : DocShots() {
    @Test
    fun tabletNowPlaying() = shot("tablet-now-playing") {
        TabletShell(tab = null) {
            NowPlayingScreen(
                DocStates.player(prefs = NowPlayingPrefs(layout = NowPlayingLayout.LYRICS, queuePanel = true)),
                clock = { 0L },
                actions = nowPlayingActions(onClose = {}),
                wide = true,
            )
        }
    }
}
