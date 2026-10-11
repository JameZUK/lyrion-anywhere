package io.github.jamezuk.lyrionanywhere.data.offline

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
}

/** The real store over the Rust library. */
class RustOfflineStore(val library: OfflineLibrary) : OfflineStore {
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

    override fun planSmart(wanted: List<DownloadRequest>) = planSmart(library, wanted)

    override fun close() = library.close()

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
        RustDownloader(Downloader(connection.session, (store as RustOfflineStore).library, policy, network, observer))

    override suspend fun syncPlays(store: OfflineStore): PlaysSyncOutcome =
        syncPlays(connection.session, (store as RustOfflineStore).library)
}
