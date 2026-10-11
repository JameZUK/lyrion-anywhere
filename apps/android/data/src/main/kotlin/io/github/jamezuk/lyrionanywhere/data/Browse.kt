package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.Suitability
import io.github.jamezuk.lyrionanywhere.core.VirtualLibrary
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * Which LMS virtual library to browse ("Libraries" in LMS's My Music). Offered only when
 * LMS reports any; the choice is remembered on this device, and applies only while LMS
 * still has that library.
 */
class VirtualLibraries(
    private val library: LibraryRepository,
    private val settings: SettingsStore,
) {
    private val _available = MutableStateFlow<List<VirtualLibrary>>(emptyList())

    /** Libraries LMS reports; empty when it has none (the picker is then hidden). */
    val available: StateFlow<List<VirtualLibrary>> = _available.asStateFlow()

    /** The library browse commands are narrowed to, or null for the whole library. */
    val chosen: Flow<String?> = combine(settings.settings, _available) { s, list ->
        s.libraryId?.takeIf { id -> list.any { it.id == id } }
    }.distinctUntilChanged()

    /** Ask LMS which libraries exist (failures leave the whole library). */
    suspend fun refresh() {
        _available.value = (library.libraries() as? ApiResult.Ok)?.value.orEmpty()
    }

    suspend fun choose(id: String?) {
        if (id == null || _available.value.any { it.id == id }) settings.setLibrary(id)
    }
}

/**
 * How usable each app or plugin is from here, judged from its own top menu: remembered for
 * a while ([ttlMs]) so the Apps page doesn't ask again on every visit.
 */
class SuitabilityCache(
    private val clock: () -> Long,
    private val ttlMs: Long = TTL_MS,
) {
    private data class Entry(val value: Suitability, val at: Long)

    private val lock = Mutex()
    private val entries = LinkedHashMap<String, Entry>()

    suspend fun get(key: String): Suitability? = lock.withLock {
        entries[key]?.takeIf { clock() - it.at < ttlMs }?.value
    }

    suspend fun put(key: String, value: Suitability) = lock.withLock {
        entries[key] = Entry(value, clock())
        // Bounded, whatever LMS lists.
        while (entries.size > MAX_ENTRIES) entries.remove(entries.keys.first())
    }

    companion object {
        const val TTL_MS = 10 * 60_000L
        const val MAX_ENTRIES = 500
    }
}
