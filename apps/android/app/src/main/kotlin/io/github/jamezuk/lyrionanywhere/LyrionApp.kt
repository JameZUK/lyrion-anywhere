package io.github.jamezuk.lyrionanywhere

import android.app.Application
import android.content.Context
import coil3.ImageLoader
import coil3.PlatformContext
import coil3.SingletonImageLoader
import coil3.memory.MemoryCache
import coil3.request.crossfade
import io.github.jamezuk.lyrionanywhere.core.installCrashReport
import io.github.jamezuk.lyrionanywhere.data.ArtRepository
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.NowPlayingHub
import io.github.jamezuk.lyrionanywhere.widget.Controls
import io.github.jamezuk.lyrionanywhere.widget.NowPlayingWidget
import io.github.jamezuk.lyrionanywhere.widget.PlayPauseTileService
import io.github.jamezuk.lyrionanywhere.widget.WidgetModel
import kotlinx.coroutines.CoroutineExceptionHandler
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.dropWhile
import kotlinx.coroutines.flow.map
import io.github.jamezuk.lyrionanywhere.data.EventsRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerRepository
import io.github.jamezuk.lyrionanywhere.downloads.DownloadService
import io.github.jamezuk.lyrionanywhere.auto.AppAutoSource
import io.github.jamezuk.lyrionanywhere.playback.PlaybackHost
import io.github.jamezuk.lyrionanywhere.playback.auto.AutoArtCache
import io.github.jamezuk.lyrionanywhere.playback.auto.AutoArtUris
import io.github.jamezuk.lyrionanywhere.playback.auto.AutoCatalogue
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import android.content.Intent
import java.io.File
import io.github.jamezuk.lyrionanywhere.playback.auto.AutoSource
import io.github.jamezuk.lyrionanywhere.playback.auto.ResourceLabels
import io.github.jamezuk.lyrionanywhere.rust.NativeInit
import io.github.jamezuk.lyrionanywhere.playback.auto.CarConnectionLog
import io.github.jamezuk.lyrionanywhere.ui.art.ArtworkFetcher
import io.github.jamezuk.lyrionanywhere.ui.art.ArtworkKeyer
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

class LyrionApp : Application(), SingletonImageLoader.Factory, PlaybackHost, LaunchHost {
    lateinit var container: AppContainer
        private set

    // Everything below is created on first use, never while Android constructs the app, so a
    // failure there is recorded by [CrashLog] rather than killing the app before it can.

    /** Work that outlives any screen (resuming the sign-in, the phone's player). */
    val appScope: CoroutineScope by lazy {
        CoroutineScope(
            SupervisorJob() + Dispatchers.Main.immediate +
                CoroutineExceptionHandler { _, e -> CrashLog.record(this, "Background task failed", e) },
        )
    }

    /** What plays, for the home-screen widget and the Quick Settings tile. */
    override val nowPlaying: NowPlayingHub by lazy { NowPlayingHub() }

    /** The media session's last caller decisions, for Settings › About (memory only). */
    override val carConnections: CarConnectionLog by lazy { CarConnectionLog() }

    /** The widget's and tile's buttons. */
    val controls: Controls by lazy { Controls({ container.players }, nowPlaying) }

    /**
     * The app crashed repeatedly: start nothing and let [MainActivity] show the report (see
     * [CrashLog]), so a crash at start-up can't repeat itself on every launch.
     */
    var safeMode = false
        private set

    /** One crash's report (not a loop), for [MainActivity] to show once. */
    private var crashReport: String? = null

    /** The report to show once, if there is one; it is then forgotten. */
    fun takeCrashReport(): String? {
        val report = crashReport ?: return null
        crashReport = null
        runCatching { CrashLog.clear(this) }
        return report
    }

    /** Whether the normal start-up ran ([container] is ready). */
    val started: Boolean get() = ::container.isInitialized

    /** Crash recording starts here, before content providers (e.g. WorkManager's) run. */
    override fun attachBaseContext(base: Context) {
        super.attachBaseContext(base)
        CrashLog.install(base)
    }

    @OptIn(FlowPreview::class)
    override fun onCreate() {
        super.onCreate()
        val start = CrashLog.start(this)
        if (start.mode == CrashLog.StartMode.SAFE) {
            safeMode = true
            return
        }
        if (start.mode == CrashLog.StartMode.REPORT_ONCE) crashReport = start.report
        // Load the Rust library and give its TLS verifier Android's TrustManager, before
        // anything touches the network.
        NativeInit.init(this)
        installCrashReport(CrashLog.nativeFile(this).path)
        container = AppContainer(this, appScope)
        appScope.launch { container.account.start() }
        appScope.launch {
            container.account.state.collect {
                if (it is AuthState.SignedOut) {
                    nowPlaying.clear()
                    // The car's covers belong to the account: their grants go, then the files.
                    autoArt.uris().forEach { u -> runCatching { revokeUriPermission(u, Intent.FLAG_GRANT_READ_URI_PERMISSION) } }
                    autoArt.clear()
                }
            }
        }
        // Redraw the widget and tile only when what they show changes. This follows state
        // the app has anyway; it never wakes the app or schedules work.
        appScope.launch {
            // A fresh process knows nothing yet: keep the widget as it was until it does.
            nowPlaying.shown.map(WidgetModel::from).dropWhile { it.kind == WidgetModel.Kind.NOTHING }
                .distinctUntilChanged().debounce(WIDGET_DEBOUNCE_MS).collect {
                NowPlayingWidget.render(this@LyrionApp)
            }
        }
        appScope.launch {
            nowPlaying.phone.map { it?.playing to it?.title }.distinctUntilChanged().collect {
                PlayPauseTileService.refresh(this@LyrionApp)
            }
        }
        // Downloads keep going in the background through a foreground service (dataSync),
        // started when they become busy; it stops itself when they are done.
        appScope.launch {
            container.offline.state.map { (it.downloads?.busy ?: 0u) > 0u }.distinctUntilChanged().collect { busy ->
                if (busy) DownloadService.start(this@LyrionApp)
            }
        }
    }

    // The playback service's view of the app.
    override val localPlayer: LocalPlayerRepository get() = container.localPlayer
    override val players: PlayersRepository get() = container.players
    override val events: EventsRepository get() = container.events
    override val art: ArtRepository get() = container.art
    override val offlinePlayer: OfflinePlayerRepository get() = container.offlinePlayer
    override val backgroundPlayback get() = container.backgroundPlayback
    override val appForeground get() = container.appForeground

    override suspend fun offlineArtwork(albumRef: String): ByteArray? = container.offline.artwork(albumRef)

    // Safe mode (the last run crashed) builds nothing, and the session then accepts no one.
    override val playbackReady: Boolean get() = started

    /** The car's browse tree (Android Auto). */
    override val autoSource: AutoSource by lazy { AppAutoSource(container) }

    /**
     * Covers for Android Auto's lists (Wave 2 §5.2): fetched through the gateway (or read from
     * the downloads), kept under opaque keys in the app's cache, and served by its own
     * non-exported provider to Android Auto alone.
     */
    override val autoArt: AutoArtCache by lazy {
        AutoArtCache(File(cacheDir, "auto-art"), AutoArtCache.authorityFor(packageName), fetch = { key: String ->
            if (!started) {
                null
            } else if (key.startsWith(AutoArtUris.OFFLINE_PREFIX)) {
                container.offline.artwork(key.removePrefix(AutoArtUris.OFFLINE_PREFIX))
            } else {
                (container.art.image(key, AutoArtCache.SIZE_PX) as? ApiResult.Ok)?.value?.bytes
            }
        })
    }

    /**
     * A spoken "play …" from outside the app (the assistant): the best match in the library
     * (or the downloads) plays on this phone, through the same search and checks as the
     * car's. Nothing but the query text is used; with no query, playback carries on.
     */
    override fun playFromSearch(query: String?) {
        if (!started) return
        val q = query?.let(AutoCatalogue::clean)?.take(AutoCatalogue.MAX_QUERY)
        appScope.launch { AutoCatalogue(autoSource, ResourceLabels(this@LyrionApp)).play(null, q) }
    }

    /** Whether "Play on this phone" is on (the saved setting, so it is right at a cold start). */
    override suspend fun playsOnPhone(): Boolean = started && container.settings.settings.first().playOnPhone

    /** Pairing links only fill in the pairing screen (see [LaunchHandler]). */
    override fun offerPairingLink(link: String) {
        if (started) container.pairingLinks.offer(link)
    }

    private companion object {
        const val WIDGET_DEBOUNCE_MS = 500L
    }

    /** Artwork goes through the Rust Art API (bearer token, size cap); memory cache only. */
    override fun newImageLoader(context: PlatformContext): ImageLoader =
        ImageLoader.Builder(context)
            .components {
                add(ArtworkKeyer())
                add(ArtworkFetcher.Factory(container.art) { ref -> container.offline.artwork(ref) })
            }
            .memoryCache { MemoryCache.Builder().maxSizePercent(context, 0.2).build() }
            .diskCache(null)
            .crossfade(true)
            .build()
}
