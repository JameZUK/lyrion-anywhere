package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.core.syncRatings
import io.github.jamezuk.lyrionanywhere.core.DownloadObserver
import io.github.jamezuk.lyrionanywhere.core.DownloadPolicy
import io.github.jamezuk.lyrionanywhere.core.DownloadRequest
import io.github.jamezuk.lyrionanywhere.core.Downloader
import io.github.jamezuk.lyrionanywhere.core.FormatChoice
import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.core.OfflineAlbum
import io.github.jamezuk.lyrionanywhere.core.OfflineLibrary
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.core.PlaysSyncOutcome
import io.github.jamezuk.lyrionanywhere.core.SmartPlan
import io.github.jamezuk.lyrionanywhere.core.planSmart
import io.github.jamezuk.lyrionanywhere.core.removeOfflineData
import io.github.jamezuk.lyrionanywhere.core.syncPlays
import io.github.jamezuk.lyrionanywhere.data.Connection
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/** Whose downloads: a user on a server. */
data class Owner(val serverUrl: String, val username: String)

/** The parts of the Rust [OfflineLibrary] the app uses (a seam for tests). Blocking: call
 *  from a background dispatcher. */
interface OfflineStore : AutoCloseable {
    fun tracks(): List<OfflineTrack>

    fun albums(): List<OfflineAlbum>

    fun usedBytes(): Long

    fun artwork(albumRef: String): ByteArray?

    fun remove(keys: List<String>)

    fun removeAlbum(albumRef: String)

    fun removeAll()

    fun pendingPlays(): Int

    fun planSmart(wanted: List<DownloadRequest>): SmartPlan

    /** Queue a rating made offline (sent with the plays); false if out of range. */
    fun rateOffline(trackId: Long, trackKey: String, rating: Int, at: Long): Boolean = false
}

/** The download manager (a seam for tests). */
interface DownloaderHandle {
    fun enqueue(requests: List<DownloadRequest>, choice: FormatChoice)

    fun pause(trackId: Long?)

    fun resume(trackId: Long?)

    fun cancel(trackId: Long?)

    fun clearFinished()

    fun setPolicy(policy: DownloadPolicy)

    fun setNetwork(network: NetworkKind)

    fun shutdown()
}

/** What needs the signed-in session: downloading and sending plays (a seam for tests). */
interface OnlineLink {
    fun downloader(
        store: OfflineStore,
        policy: DownloadPolicy,
        network: NetworkKind,
        observer: DownloadObserver,
    ): DownloaderHandle

    suspend fun syncPlays(store: OfflineStore): PlaysSyncOutcome

    /** Send ratings made offline. */
    suspend fun syncRatings(store: OfflineStore) {}
}

/**
 * The real store over the Rust library. The repository closes it at a sign-out, a revocation
 * or a change of user while other work may still hold it, so the library is handed to other
 * Rust calls only through [lend].
 */
class RustOfflineStore(@PublishedApi internal val library: OfflineLibrary) : OfflineStore {
    /** One while open, plus one per [lend] in progress: the library closes when it reaches 0. */
    @PublishedApi internal val uses = AtomicLong(1)
    private val closed = AtomicBoolean(false)

    /**
     * Runs [block] with the library, kept open until [block] returns even if the store is
     * closed meanwhile. For passing the library to another Rust call: UniFFI refuses a closed
     * object's own methods, but one passed as an argument is not checked and would reach freed
     * memory. Throws [IllegalStateException] once the store is closed.
     */
    inline fun <R> lend(block: (OfflineLibrary) -> R): R {
        acquire()
        try {
            return block(library)
        } finally {
            release()
        }
    }

    @PublishedApi internal fun acquire() {
        while (true) {
            val n = uses.get()
            check(n > 0 && !isClosed()) { "the offline library is closed" }
            if (uses.compareAndSet(n, n + 1)) return
        }
    }

    @PublishedApi internal fun isClosed() = closed.get()

    @PublishedApi internal fun release() {
        if (uses.decrementAndGet() == 0L) library.close()
    }

    override fun tracks() = library.tracks()

    override fun albums() = library.albums()

    override fun usedBytes() = library.usedBytes().toLong()

    override fun artwork(albumRef: String) = library.artwork(albumRef)?.bytes

    override fun remove(keys: List<String>) {
        library.remove(keys)
    }

    override fun removeAlbum(albumRef: String) {
        library.removeAlbum(albumRef)
    }

    override fun removeAll() = library.removeAll()

    override fun pendingPlays() = library.pendingPlays().toInt()

    override fun planSmart(wanted: List<DownloadRequest>) = lend { planSmart(it, wanted) }

    override fun rateOffline(trackId: Long, trackKey: String, rating: Int, at: Long) =
        rating in 0..100 && at >= 0 && library.rateOffline(trackId, trackKey, rating.toUInt(), at.toULong())

    /** Closes the library now, or once the last [lend] in progress ends. */
    override fun close() {
        if (closed.compareAndSet(false, true)) release()
    }

    companion object {
        fun open(root: String, owner: Owner): OfflineStore =
            RustOfflineStore(OfflineLibrary.open(root, owner.serverUrl, owner.username))

        /** Delete everything kept for [owner] (a revoked device, or "remove" at sign-out). */
        fun wipe(root: String, owner: Owner) = removeOfflineData(root, owner.serverUrl, owner.username)
    }
}

private class RustDownloader(private val d: Downloader) : DownloaderHandle {
    override fun enqueue(requests: List<DownloadRequest>, choice: FormatChoice) = d.enqueue(requests, choice)

    override fun pause(trackId: Long?) = d.pause(trackId)

    override fun resume(trackId: Long?) = d.resume(trackId)

    override fun cancel(trackId: Long?) = d.cancel(trackId)

    override fun clearFinished() = d.clearFinished()

    override fun setPolicy(policy: DownloadPolicy) = d.setPolicy(policy)

    override fun setNetwork(network: NetworkKind) = d.setNetwork(network)

    override fun shutdown() {
        d.shutdown()
        d.close()
    }
}

/** The real link over the signed-in [Connection]. */
class RustOnlineLink(private val connection: Connection) : OnlineLink {
    override fun downloader(
        store: OfflineStore,
        policy: DownloadPolicy,
        network: NetworkKind,
        observer: DownloadObserver,
    ): DownloaderHandle =
        RustDownloader((store as RustOfflineStore).lend { Downloader(connection.session, it, policy, network, observer) })

    override suspend fun syncPlays(store: OfflineStore): PlaysSyncOutcome =
        (store as RustOfflineStore).lend { syncPlays(connection.session, it) }

    override suspend fun syncRatings(store: OfflineStore) {
        (store as RustOfflineStore).lend { syncRatings(connection.session, it) }
    }
}
