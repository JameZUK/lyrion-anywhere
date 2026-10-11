package io.github.jamezuk.lyrionanywhere.rust

import android.content.Context

/**
 * Loads the Rust library and gives its TLS certificate verifier (rustls-platform-verifier)
 * access to Android's TrustManager. Call once, before any network call, e.g. from
 * `Application.onCreate`.
 */
object NativeInit {
    @Volatile
    private var initialised = false

    @Synchronized
    fun init(context: Context) {
        if (initialised) return
        System.loadLibrary("anywhere_mobile")
        initPlatformVerifier(context.applicationContext)
        initialised = true
    }

    @JvmStatic
    private external fun initPlatformVerifier(context: Context)
}
