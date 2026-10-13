package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.core.HomeLayout
import io.github.jamezuk.lyrionanywhere.core.ShelfKind
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.MeRepository
import io.github.jamezuk.lyrionanywhere.ui.home.HomeLayouts
import io.github.jamezuk.lyrionanywhere.ui.library.HomeFeedViewModel
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Home's layout: edited only from the one the server holds, never saved over after a failed load. */
class HomeLayoutSafetyTest : MainDispatcherTest() {
    private class FailingHome(var fail: Boolean) : MeRepository by FakeMe() {
        val saved = mutableListOf<HomeLayout>()

        override suspend fun home(): ApiResult<HomeLayout> =
            if (fail) ApiResult.Err(AppError.Offline("down")) else ApiResult.Ok(HomeLayouts.move(HomeLayouts.DEFAULT, 0, 1)!!)

        override suspend fun saveHome(layout: HomeLayout): ApiResult<HomeLayout> {
            saved += layout
            return ApiResult.Ok(layout)
        }

        override suspend fun resetHome(): ApiResult<HomeLayout> {
            saved += HomeLayouts.DEFAULT
            return ApiResult.Ok(HomeLayouts.DEFAULT)
        }
    }

    @Test
    fun aHomeThatFailedToLoadCanNotBeEditedOrSavedOver() = runTest(dispatcher) {
        val me = FailingHome(fail = true)
        val vm = HomeFeedViewModel(FakeLibrary(), me = me, player = { "aa:00:00:00:00:01" })
        // The default is shown, but it isn't the user's layout.
        assertEquals(HomeLayouts.DEFAULT, vm.state.value.layout)
        assertFalse(vm.state.value.layoutLoaded)
        assertTrue(vm.state.value.layoutError is AppError.Offline)
        vm.edit()
        assertNull(vm.state.value.draft)
        vm.resetDraft()
        vm.save()
        assertTrue(me.saved.isEmpty())
        // Once it loads, editing starts from the server's layout.
        me.fail = false
        vm.refresh()
        assertTrue(vm.state.value.layoutLoaded)
        assertNull(vm.state.value.layoutError)
        vm.edit()
        assertEquals(ShelfKind.STATIONS, vm.state.value.draft!!.shelves.first().kind)
        assertFalse(vm.draftChanged())
        vm.moveDown(0)
        assertTrue(vm.draftChanged())
        // A later failed refresh keeps the loaded layout editable.
        vm.cancelEdit()
        me.fail = true
        vm.refresh()
        assertTrue(vm.state.value.layoutLoaded)
        assertEquals(ShelfKind.STATIONS, vm.state.value.layout!!.shelves.first().kind)
    }
}
