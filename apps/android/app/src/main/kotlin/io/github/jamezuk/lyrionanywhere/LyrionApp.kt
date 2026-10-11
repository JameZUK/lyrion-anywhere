package io.github.jamezuk.lyrionanywhere

import android.app.Application
import coil3.ImageLoader
import coil3.PlatformContext
import coil3.SingletonImageLoader
import coil3.memory.MemoryCache
import coil3.request.crossfade
import io.github.jamezuk.lyrionanywhere.rust.NativeInit
import io.github.jamezuk.lyrionanywhere.ui.art.ArtworkFetcher
import io.github.jamezuk.lyrionanywhere.ui.art.ArtworkKeyer
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

class LyrionApp : Application(), SingletonImageLoader.Factory {
    lateinit var container: AppContainer
        private set

    /** Work that outlives any screen (resuming the sign-in). */
    private val appScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    override fun onCreate() {
        super.onCreate()
        // Load the Rust library and give its TLS verifier Android's TrustManager, before
        // anything touches the network.
        NativeInit.init(this)
        container = AppContainer(this)
        appScope.launch { container.account.start() }
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
