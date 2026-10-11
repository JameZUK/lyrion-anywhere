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
import kotlinx.coroutines.flow.dropWhile
import kotlinx.coroutines.flow.map
import io.github.jamezuk.lyrionanywhere.data.EventsRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import io.github.jamezuk.lyrionanywhere.playback.PlaybackHost
import io.github.jamezuk.lyrionanywhere.rust.NativeInit
import io.github.jamezuk.lyrionanywhere.ui.art.ArtworkFetcher
import io.github.jamezuk.lyrionanywhere.ui.art.ArtworkKeyer
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

class LyrionApp : Application(), SingletonImageLoader.Factory, PlaybackHost {
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

    /** The widget's and tile's buttons. */
    val controls: Controls by lazy { Controls({ container.players }, nowPlaying) }

    /**
     * The last run crashed: start nothing and let [MainActivity] show the report (see
     * [CrashLog]), so a crash at start-up can't repeat itself on every launch.
     */
    var safeMode = false
        private set

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
        if (runCatching { CrashLog.pending(this) }.getOrNull() != null) {
            safeMode = true
            return
        }
        // Load the Rust library and give its TLS verifier Android's TrustManager, before
        // anything touches the network.
        NativeInit.init(this)
        installCrashReport(CrashLog.nativeFile(this).path)
        container = AppContainer(this, appScope)
        appScope.launch { container.account.start() }
        appScope.launch {
            container.account.state.collect { if (it is AuthState.SignedOut) nowPlaying.clear() }
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
    }

    // The playback service's view of the app.
    override val localPlayer: LocalPlayerRepository get() = container.localPlayer
    override val players: PlayersRepository get() = container.players
    override val events: EventsRepository get() = container.events
    override val art: ArtRepository get() = container.art

    private companion object {
        const val WIDGET_DEBOUNCE_MS = 500L
    }

    /** Artwork goes through the Rust Art API (bearer token, size cap); memory cache only. */
    override fun newImageLoader(context: PlatformContext): ImageLoader =
        ImageLoader.Builder(context)
            .components {
                add(ArtworkKeyer())
                add(ArtworkFetcher.Factory(container.art))
            }
            .memoryCache { MemoryCache.Builder().maxSizePercent(context, 0.2).build() }
            .diskCache(null)
            .crossfade(true)
            .build()
}
