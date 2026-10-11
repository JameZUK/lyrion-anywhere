package io.github.jamezuk.lyrionanywhere.playback.auto

import androidx.annotation.OptIn
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.LibraryResult
import androidx.media3.session.MediaLibraryService.LibraryParams
import androidx.media3.session.MediaLibraryService.MediaLibrarySession
import androidx.media3.session.MediaSession
import androidx.media3.session.SessionError
import com.google.common.collect.ImmutableList
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture
import com.google.common.util.concurrent.SettableFuture
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/** Run [block] in [this] scope, as a future for Media3. Failures become [fallback]. */
fun <T> CoroutineScope.listenable(fallback: () -> T, block: suspend () -> T): ListenableFuture<T> {
    val f = SettableFuture.create<T>()
    launch {
        try {
            f.set(block())
        } catch (e: CancellationException) {
            f.set(fallback())
            throw e
        } catch (_: Exception) {
            f.set(fallback())
        }
    }
    return f
}

/**
 * The media library session's callback: who may connect ([CallerPolicy]), and the car's
 * browse tree, search and play requests ([AutoCatalogue]).
 *
 * - **onConnect** decides per caller: [CallerAccess.BROWSE] gets the library commands;
 *   [CallerAccess.CONTROL] gets transport controls only (no library commands, and no way to
 *   set media items by id); everyone else is rejected.
 * - Library calls check the caller again, and every id the car sends back is parsed by the
 *   Rust grammar before anything is looked up.
 * - Play requests ([onAddMediaItems]) accept only ids that parse as playable, or a spoken
 *   search; the items then go to the session player, which plays them on this phone.
 */
@OptIn(UnstableApi::class)
class LibraryCallback(
    private val access: (MediaSession.ControllerInfo) -> CallerAccess,
    private val catalogue: () -> AutoCatalogue?,
    private val scope: CoroutineScope,
) : MediaLibrarySession.Callback {
    /** The last search's results, per caller and query (the car pages through them). */
    private val searches = LinkedHashMap<Pair<Int, String>, List<MediaItem>>()

    override fun onConnect(session: MediaSession, controller: MediaSession.ControllerInfo): MediaSession.ConnectionResult =
        when (access(controller)) {
            CallerAccess.BROWSE -> MediaSession.ConnectionResult.AcceptedResultBuilder(session)
                .setAvailableSessionCommands(MediaSession.ConnectionResult.DEFAULT_SESSION_AND_LIBRARY_COMMANDS)
                .build()
            CallerAccess.CONTROL -> MediaSession.ConnectionResult.AcceptedResultBuilder(session)
                .setAvailableSessionCommands(MediaSession.ConnectionResult.DEFAULT_SESSION_COMMANDS)
                .setAvailablePlayerCommands(CONTROL_ONLY)
                .build()
            CallerAccess.REJECT -> MediaSession.ConnectionResult.reject()
        }

    private fun allowed(controller: MediaSession.ControllerInfo) = access(controller) == CallerAccess.BROWSE

    private fun <T : Any> denied(): ListenableFuture<LibraryResult<T>> =
        Futures.immediateFuture(LibraryResult.ofError<T>(SessionError.ERROR_PERMISSION_DENIED))

    override fun onGetLibraryRoot(
        session: MediaLibrarySession,
        browser: MediaSession.ControllerInfo,
        params: LibraryParams?,
    ): ListenableFuture<LibraryResult<MediaItem>> {
        if (!allowed(browser)) return denied()
        val c = catalogue() ?: return Futures.immediateFuture(LibraryResult.ofError(SessionError.ERROR_INVALID_STATE))
        return Futures.immediateFuture(LibraryResult.ofItem(c.root(), params))
    }

    override fun onGetChildren(
        session: MediaLibrarySession,
        browser: MediaSession.ControllerInfo,
        parentId: String,
        page: Int,
        pageSize: Int,
        params: LibraryParams?,
    ): ListenableFuture<LibraryResult<ImmutableList<MediaItem>>> {
        if (!allowed(browser)) return denied()
        val c = catalogue() ?: return Futures.immediateFuture(LibraryResult.ofError(SessionError.ERROR_INVALID_STATE))
        return scope.listenable({ LibraryResult.ofError(SessionError.ERROR_IO) }) {
            when (val items = c.children(parentId, page, pageSize)) {
                null -> LibraryResult.ofError(SessionError.ERROR_BAD_VALUE)
                else -> LibraryResult.ofItemList(items, params)
            }
        }
    }

    override fun onGetItem(
        session: MediaLibrarySession,
        browser: MediaSession.ControllerInfo,
        mediaId: String,
    ): ListenableFuture<LibraryResult<MediaItem>> {
        if (!allowed(browser)) return denied()
        val c = catalogue() ?: return Futures.immediateFuture(LibraryResult.ofError(SessionError.ERROR_INVALID_STATE))
        return scope.listenable({ LibraryResult.ofError(SessionError.ERROR_IO) }) {
            c.item(mediaId)?.let { LibraryResult.ofItem(it, null) } ?: LibraryResult.ofError(SessionError.ERROR_BAD_VALUE)
        }
    }

    override fun onSearch(
        session: MediaLibrarySession,
        browser: MediaSession.ControllerInfo,
        query: String,
        params: LibraryParams?,
    ): ListenableFuture<LibraryResult<Void>> {
        if (!allowed(browser)) return denied()
        val c = catalogue() ?: return Futures.immediateFuture(LibraryResult.ofError(SessionError.ERROR_INVALID_STATE))
        val q = query.take(AutoCatalogue.MAX_QUERY)
        scope.launch {
            val found = runCatching { c.search(q) }.getOrDefault(emptyList())
            synchronized(searches) {
                searches[browser.uid to q] = found
                while (searches.size > MAX_SEARCHES) searches.remove(searches.keys.first())
            }
            session.notifySearchResultChanged(browser, q, found.size, params)
        }
        return Futures.immediateFuture(LibraryResult.ofVoid(params))
    }

    override fun onGetSearchResult(
        session: MediaLibrarySession,
        browser: MediaSession.ControllerInfo,
        query: String,
        page: Int,
        pageSize: Int,
        params: LibraryParams?,
    ): ListenableFuture<LibraryResult<ImmutableList<MediaItem>>> {
        if (!allowed(browser)) return denied()
        val found = synchronized(searches) { searches[browser.uid to query.take(AutoCatalogue.MAX_QUERY)] }.orEmpty()
        val size = pageSize.coerceIn(1, PAGE_MAX)
        val start = page.coerceAtLeast(0).toLong() * size
        val items = if (start >= found.size) emptyList() else found.subList(start.toInt(), minOf(found.size, start.toInt() + size))
        return Futures.immediateFuture(LibraryResult.ofItemList(items, params))
    }

    /**
     * What the car asked to play: only ids of the grammar that can be played, or a spoken
     * search (no id). The items reach the session player, which plays them on this phone.
     */
    override fun onAddMediaItems(
        mediaSession: MediaSession,
        controller: MediaSession.ControllerInfo,
        mediaItems: MutableList<MediaItem>,
    ): ListenableFuture<MutableList<MediaItem>> {
        val c = catalogue()
        if (!allowed(controller) || c == null || mediaItems.isEmpty() || mediaItems.size > MAX_ITEMS ||
            !mediaItems.all { c.acceptsPlayRequest(it) }
        ) {
            return Futures.immediateFailedFuture(UnsupportedOperationException("not playable here"))
        }
        // Only the id and a spoken query are passed on: never a URI from outside.
        return Futures.immediateFuture(
            mediaItems.map { m ->
                MediaItem.Builder()
                    .setMediaId(m.mediaId)
                    .setRequestMetadata(
                        MediaItem.RequestMetadata.Builder()
                            .setSearchQuery(m.requestMetadata.searchQuery?.take(AutoCatalogue.MAX_QUERY))
                            .build(),
                    )
                    .setMediaMetadata(m.mediaMetadata)
                    .build()
            }.toMutableList(),
        )
    }

    companion object {
        const val MAX_SEARCHES = 8
        const val PAGE_MAX = 100
        const val MAX_ITEMS = 1

        /** Transport only: no setting media items (playing by id) for control-only callers. */
        val CONTROL_ONLY: Player.Commands = MediaSession.ConnectionResult.DEFAULT_PLAYER_COMMANDS.buildUpon()
            .remove(Player.COMMAND_SET_MEDIA_ITEM)
            .remove(Player.COMMAND_CHANGE_MEDIA_ITEMS)
            .build()
    }
}
