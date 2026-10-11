package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsUiState
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsViewModel
import io.github.jamezuk.lyrionanywhere.ui.settings.validDeviceName
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

class DeviceNameTest {
    @Test
    fun namesAreOneToSixtyFourCharactersWithoutControls() {
        assertEquals("Kitchen phone", validDeviceName("  Kitchen phone "))
        assertEquals("é".repeat(64), validDeviceName("é".repeat(64)))
        assertEquals("🎵".repeat(64), validDeviceName("🎵".repeat(64)))
        for (bad in listOf("", "   ", "x".repeat(65), "a\nb", "a\u0000b", "a\u007fb")) {
            assertNull(bad, validDeviceName(bad))
        }
    }
}

class RenameViewModelTest : MainDispatcherTest() {
    @Test
    fun renamingShowsTheStoredNameOrTheError() = runTest(dispatcher) {
        val account = FakeAccount()
        account._state.value = AuthState.SignedInState(SERVER, WHO)
        val vm = SettingsViewModel(account)
        assertEquals("Phone", vm.state.value.deviceName)

        account.renameResult = ApiResult.Ok("bDenb")
        vm.rename("  <b>Den</b> ")
        assertEquals(listOf("<b>Den</b>"), account.renames)
        assertEquals("bDenb", vm.state.value.deviceName)
        assertFalse(vm.state.value.renaming)

        // Invalid names never reach the server.
        vm.rename("   ")
        vm.rename("x".repeat(65))
        assertEquals(1, account.renames.size)

        val refused = AppError.InvalidInput("another of this user's devices already has that name")
        account.renameResult = ApiResult.Err(refused)
        vm.rename("Tablet")
        assertEquals(refused, vm.state.value.renameError)
        assertEquals("bDenb", vm.state.value.deviceName)
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class RenameScreenTests {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun thisPhoneCanBeRenamedFromSettings() {
        val renamed = mutableListOf<String>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SettingsScreen(
                    state = SettingsUiState(username = "alice", deviceName = "Pixel 8"),
                    appVersion = "1",
                    libraryVersion = "1",
                    onSignOut = {},
                    onLicences = {},
                    onRename = { renamed += it },
                )
            }
        }
        compose.onNodeWithText("Its player is called “Pixel 8” in your music server").assertIsDisplayed()
        compose.onNodeWithTag("rename-field").assertDoesNotExist()
        compose.onNodeWithText("This phone").performClick()
        val save = compose.onNodeWithTag("rename-save")
        val field = compose.onNodeWithTag("rename-field")
        save.assertIsNotEnabled() // unchanged
        field.performTextReplacement("   ")
        save.assertIsNotEnabled()
        field.performTextReplacement(" Kitchen ")
        save.assertIsEnabled()
        save.performClick()
        assertEquals(listOf("Kitchen"), renamed)
        compose.onNodeWithTag("rename-field").assertDoesNotExist()
    }

    @Test
    fun noRenameRowWithoutASignIn() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SettingsScreen(SettingsUiState(username = "alice"), "1", "1", {}, {})
            }
        }
        compose.onNodeWithText("This phone").assertDoesNotExist()
    }
}
