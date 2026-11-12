package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.core.NoHandle
import io.github.jamezuk.lyrionanywhere.core.OfflineLibrary
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The offline library handed to another Rust call (sending plays, smart downloads, the
 * download manager, the offline player) stays open until that call has it: UniFFI checks a
 * closed object's own methods but not one passed as an argument, so closing the store at a
 * sign-out or revocation must not free a library another call is about to pass to Rust.
 */
class RustOfflineStoreTest {
    @Test
    fun aLendGetsTheOpenLibrary() = runTest {
        val library = OfflineLibrary(NoHandle)
        val store = RustOfflineStore(library)
        assertSame(library, store.lend { it })
        assertFalse(library.uniffiIsDestroyed)
        store.close()
        assertTrue(library.uniffiIsDestroyed)
    }

    @Test
    fun closingWhileLentClosesTheLibraryOnlyOnceTheLendEnds() = runTest {
        val library = OfflineLibrary(NoHandle)
        val store = RustOfflineStore(library)
        val lent = CompletableDeferred<Unit>()
        val done = CompletableDeferred<Unit>()
        val use = async {
            store.lend {
                lent.complete(Unit)
                done.await()
                assertFalse("still open while lent", it.uniffiIsDestroyed)
            }
        }
        lent.await()
        store.close()
        assertFalse(library.uniffiIsDestroyed)
        done.complete(Unit)
        use.await()
        assertTrue(library.uniffiIsDestroyed)
    }

    @Test
    fun nothingIsLentOnceClosed() = runTest {
        val library = OfflineLibrary(NoHandle)
        val store = RustOfflineStore(library)
        store.close()
        store.close()
        var given = 0
        assertThrows(IllegalStateException::class.java) { store.lend { given++ } }
        assertEquals(0, given)
        assertTrue(library.uniffiIsDestroyed)
    }

    @Test
    fun aFailingLendStillLetsTheLibraryClose() = runTest {
        val library = OfflineLibrary(NoHandle)
        val store = RustOfflineStore(library)
        assertThrows(IllegalArgumentException::class.java) { store.lend { throw IllegalArgumentException("boom") } }
        store.close()
        assertTrue(library.uniffiIsDestroyed)
    }
}
