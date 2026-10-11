package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.Suitability
import io.github.jamezuk.lyrionanywhere.core.VirtualLibrary
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.lang.reflect.Proxy

private class Settings : SettingsStore {
    val state = MutableStateFlow(AppSettings())
    override val settings: Flow<AppSettings> = state

    override suspend fun setServerUrl(url: String?) = state.update { it.copy(serverUrl = url) }

    override suspend fun setSelectedPlayer(playerId: String?) = state.update { it.copy(selectedPlayerId = playerId) }

    override suspend fun setPlayOnPhone(on: Boolean) = state.update { it.copy(playOnPhone = on) }

    override suspend fun setSound(sound: SoundSettings) = state.update { it.copy(sound = sound) }

    override suspend fun setLibrary(id: String?) = state.update { it.copy(libraryId = id) }

    override suspend fun setDefaultPlayer(choice: DefaultPlayer, playerId: String?) =
        state.update { it.copy(defaultPlayer = choice, defaultPlayerId = playerId) }
}

/** A library repository that only lists [libs]; any other call fails the test. */
@Suppress("UNCHECKED_CAST")
private fun libraries(libs: () -> ApiResult<List<VirtualLibrary>>): LibraryRepository =
    Proxy.newProxyInstance(LibraryRepository::class.java.classLoader, arrayOf(LibraryRepository::class.java)) { _, m, _ ->
        if (m.name == "libraries") libs() else error("unexpected ${m.name}")
    } as LibraryRepository

class VirtualLibrariesTest {
    @Test
    fun aChoiceAppliesOnlyWhileLmsHasThatLibrary() = runBlocking {
        var list = listOf(VirtualLibrary("ab12", "Classical"))
        val settings = Settings()
        val repo = object : LibraryRepository by libraries({ ApiResult.Ok(list) }) {
            override suspend fun libraries() = ApiResult.Ok(list)
        }
        val v = VirtualLibraries(repo, settings)
        v.refresh()
        assertEquals(list, v.available.value)
        v.choose("ab12")
        assertEquals("ab12", v.chosen.first())
        // A library LMS doesn't list is never chosen.
        v.choose("cd34")
        assertEquals("ab12", settings.state.value.libraryId)
        // LMS dropped it: the whole library, though the choice is kept for if it returns.
        list = emptyList()
        v.refresh()
        assertNull(v.chosen.first())
        v.choose(null)
        assertNull(settings.state.value.libraryId)
    }

    @Test
    fun failuresLeaveTheWholeLibrary() = runBlocking {
        val settings = Settings().apply { state.value = AppSettings(libraryId = "ab12") }
        val repo = object : LibraryRepository by libraries({ ApiResult.Err(AppError.Offline()) }) {
            override suspend fun libraries(): ApiResult<List<VirtualLibrary>> = ApiResult.Err(AppError.Offline())
        }
        val v = VirtualLibraries(repo, settings)
        v.refresh()
        assertEquals(emptyList<VirtualLibrary>(), v.available.value)
        assertNull(v.chosen.first())
    }
}

class SuitabilityCacheTest {
    @Test
    fun remembersForAWhileAndStaysBounded() = runBlocking {
        var now = 0L
        val cache = SuitabilityCache({ now }, ttlMs = 1000)
        cache.put("a", Suitability.Usable)
        assertEquals(Suitability.Usable, cache.get("a"))
        now = 1001
        assertNull(cache.get("a"))
        repeat(SuitabilityCache.MAX_ENTRIES + 50) { cache.put("k$it", Suitability.Empty) }
        assertNull(cache.get("k0"))
        assertEquals(Suitability.Empty, cache.get("k${SuitabilityCache.MAX_ENTRIES + 49}"))
    }
}
