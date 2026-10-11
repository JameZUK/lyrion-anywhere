package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.Discovery
import io.github.jamezuk.lyrionanywhere.core.DiscoveryOptions
import io.github.jamezuk.lyrionanywhere.core.DstmState
import io.github.jamezuk.lyrionanywhere.core.RadioPlan
import io.github.jamezuk.lyrionanywhere.core.RadioSeed
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * "Start radio" and "More like this" (A6), from what the server offers: AudioMuse-AI's
 * context-menu entries, or LMS's Don't Stop The Music. Nothing is assumed: an item with
 * neither gets no radio, and the app hides the action.
 */
interface DiscoveryRepository {
    /** What can be offered for [seed] on [player] (remembered for a while). */
    suspend fun options(player: String, seed: RadioSeed): ApiResult<DiscoveryOptions>

    /** Start radio from [seed] on [player] as [plan] says. */
    suspend fun startRadio(player: String, seed: RadioSeed, plan: RadioPlan): ApiResult<Unit>
}

/** A small cache with an expiry, keyed by text (bounded). */
class ExpiringCache<V>(
    private val clock: () -> Long,
    private val ttlMs: Long = SuitabilityCache.TTL_MS,
    private val max: Int = 200,
) {
    private data class Entry<V>(val value: V, val at: Long)

    private val lock = Mutex()
    private val entries = LinkedHashMap<String, Entry<V>>()

    suspend fun get(key: String): V? = lock.withLock {
        entries[key]?.takeIf { clock() - it.at in 0 until ttlMs }?.value
    }

    suspend fun put(key: String, value: V) = lock.withLock {
        entries.remove(key)
        entries[key] = Entry(value, clock())
        while (entries.size > max) entries.remove(entries.keys.first())
    }

    /** Forget entries whose key starts with [prefix] (everything when null). */
    suspend fun forget(prefix: String? = null) = lock.withLock {
        if (prefix == null) entries.clear() else entries.keys.removeAll { it.startsWith(prefix) }
    }
}

/** The cache key for an item. */
fun RadioSeed.key(): String = when (this) {
    is RadioSeed.Track -> "track:$id"
    is RadioSeed.Album -> "album:$id"
    is RadioSeed.Artist -> "artist:$id"
}

/** The Rust [Discovery] for the signed-in server; answers are remembered for ten minutes per
 *  connection, player and item (a new sign-in starts afresh). */
class RustDiscoveryRepository(
    private val source: ConnectionSource,
    clock: () -> Long,
) : DiscoveryRepository {
    private val options = ExpiringCache<DiscoveryOptions>(clock)
    private val dstm = ExpiringCache<DstmState>(clock)
    private var connection: Connection? = null
    private val lock = Mutex()

    /** The connection, with the caches emptied when it changed. */
    private suspend fun current(): Connection? {
        val c = source.connection.value
        val changed = lock.withLock {
            val changed = c !== connection
            connection = c
            changed
        }
        if (changed) {
            options.forget()
            dstm.forget()
        }
        return c
    }

    override suspend fun options(player: String, seed: RadioSeed): ApiResult<DiscoveryOptions> {
        val c = current() ?: return ApiResult.Err(AppError.SignedOut)
        val key = "$player|${seed.key()}"
        options.get(key)?.let { return ApiResult.Ok(it) }
        val r = apiCall {
            Discovery(c.session).use { d ->
                val known = dstm.get(player) ?: d.dontStopTheMusic(player)?.also { dstm.put(player, it) }
                d.options(player, seed, known)
            }
        }
        if (r is ApiResult.Ok) options.put(key, r.value)
        return r
    }

    override suspend fun startRadio(player: String, seed: RadioSeed, plan: RadioPlan): ApiResult<Unit> {
        val c = current() ?: return ApiResult.Err(AppError.SignedOut)
        val r = apiCall { Discovery(c.session).use { it.startRadio(player, seed, plan) } }
        if (plan.enableDstm) {
            // Don't Stop The Music changed (or may have): ask again next time.
            dstm.forget(player)
            options.forget("$player|")
        }
        return r
    }
}
