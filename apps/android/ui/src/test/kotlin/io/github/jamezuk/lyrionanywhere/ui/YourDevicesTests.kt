package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.hasTestTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.AccountInfo
import io.github.jamezuk.lyrionanywhere.core.MyDevice
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.DevicesMessage
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsSection
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsUiState
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsViewModel
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val NOW = 1_800_000_000L

private val THIS = MyDevice(1, "Pixel", "android", "Pixel 8", 1, NOW - 60, true)
private val LAPTOP = MyDevice(2, "Laptop", "browser", "Firefox on Linux", 2, NOW - 3 * 3600, false)
private val KITCHEN = MyDevice(3, "Kitchen", "pwa", null, 3, null, false)

class YourDevicesViewModelTest : MainDispatcherTest() {
    private fun signedIn(): FakeAccount = FakeAccount().apply {
        _state.value = AuthState.SignedInState(SERVER, WHO)
        accountInfo = AccountInfo("alice", false, true, false, true, true)
        devices = mutableListOf(THIS, LAPTOP, KITCHEN)
    }

    @Test
    fun loadsSignsOneOutAndReloads() = runTest(dispatcher) {
        val account = signedIn()
        val vm = SettingsViewModel(account)
        assertTrue(vm.state.value.account!!.canManageDevices)
        assertNull(vm.state.value.devices)
        vm.loadDevices()
        assertEquals(listOf(THIS, LAPTOP, KITCHEN), vm.state.value.devices)

        vm.signOutDevice(LAPTOP)
        assertEquals(listOf(2L), account.signedOutDevices)
        assertEquals(DevicesMessage.SignedOut("Laptop"), vm.state.value.devicesMessage)
        assertEquals(listOf(THIS, KITCHEN), vm.state.value.devices)
        assertFalse(vm.state.value.devicesBusy)

        // Gone already (signed out elsewhere): said plainly.
        vm.signOutDevice(LAPTOP)
        assertEquals(DevicesMessage.AlreadyGone, vm.state.value.devicesMessage)
    }

    @Test
    fun thisPhoneIsNeverSentForSigningOut() = runTest(dispatcher) {
        val account = signedIn()
        val vm = SettingsViewModel(account)
        vm.loadDevices()
        vm.signOutDevice(THIS)
        assertTrue(account.signedOutDevices.isEmpty())
        assertNull(vm.state.value.devicesMessage)
    }

    @Test
    fun allOthersAreSignedOutAndCounted() = runTest(dispatcher) {
        val account = signedIn()
        val vm = SettingsViewModel(account)
        vm.loadDevices()
        vm.signOutOtherDevices()
        assertEquals(1, account.othersSignedOut)
        assertEquals(DevicesMessage.SignedOutOthers(2), vm.state.value.devicesMessage)
        assertEquals(listOf(THIS), vm.state.value.devices)
    }

    @Test
    fun failuresAreOneMessageAndNothingChanges() = runTest(dispatcher) {
        val account = signedIn()
        val vm = SettingsViewModel(account)
        vm.loadDevices()
        vm.signOutDevice(KITCHEN)
        account.signOutError = AppError.RateLimited(60)
        vm.signOutDevice(LAPTOP)
        assertEquals(DevicesMessage.Failed(AppError.RateLimited(60)), vm.state.value.devicesMessage)
        assertEquals(listOf(THIS, LAPTOP), vm.state.value.devices)

        account.devicesError = AppError.Offline()
        val fresh = SettingsViewModel(account)
        fresh.loadDevices()
        assertNull(fresh.state.value.devices)
        assertEquals(DevicesMessage.Failed(AppError.Offline()), fresh.state.value.devicesMessage)
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class YourDevicesScreenTests {
    @get:Rule
    val compose = createComposeRule()

    private class Page(initial: SettingsUiState, section: SettingsSection) {
        var state by mutableStateOf(initial)
        var section by mutableStateOf(section)
        val opened = mutableListOf<SettingsSection>()
        var loads = 0
        val signedOut = mutableListOf<MyDevice>()
        var others = 0
    }

    private fun show(page: Page) {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SettingsScreen(
                    state = page.state,
                    appVersion = "1",
                    libraryVersion = "1",
                    onSignOut = {},
                    onLicences = {},
                    section = page.section,
                    onSection = { page.opened += it },
                    onLoadDevices = { page.loads++ },
                    onSignOutDevice = { page.signedOut += it },
                    onSignOutOtherDevices = { page.others++ },
                    nowSeconds = { NOW },
                )
            }
        }
    }

    private fun account(devices: Boolean) = AccountInfo("alice", false, false, false, true, devices)

    @Test
    fun theRowShowsOnlyWhenTheServerOffersIt() {
        val page = Page(SettingsUiState(username = "alice", account = account(devices = false)), SettingsSection.ACCOUNT)
        show(page)
        compose.onNodeWithText("Two-step verification").assertIsDisplayed()
        compose.onNodeWithTag(SettingsSection.YOUR_DEVICES.tag).assertDoesNotExist()
        page.state = page.state.copy(account = account(devices = true))
        compose.onNodeWithTag(SettingsSection.YOUR_DEVICES.tag).assertIsDisplayed().performClick()
        assertEquals(listOf(SettingsSection.YOUR_DEVICES), page.opened)
    }

    @Test
    fun listsDevicesThisOneMarkedAndWithoutSignOut() {
        val page = Page(SettingsUiState(username = "alice", devices = listOf(LAPTOP, THIS, KITCHEN)), SettingsSection.YOUR_DEVICES)
        show(page)
        compose.waitForIdle()
        assertEquals(1, page.loads)
        compose.onNodeWithTag("device-1").assertTextContains("This device", substring = true)
        compose.onNodeWithTag("device-1").assertTextContains("Pixel 8 · In use now", substring = true)
        compose.onNodeWithTag("sign-out-device-1").assertDoesNotExist()
        compose.onNodeWithTag("device-2").assertTextContains("Firefox on Linux · Last used 3 hours ago", substring = true)
        compose.onNodeWithTag("device-3").assertTextContains("Installed web app · Not used yet", substring = true)
        compose.onNodeWithTag("sign-out-device-2").assertIsDisplayed()
        compose.onNodeWithTag("sign-out-others").assertIsDisplayed()
    }

    @Test
    fun signingOneOutAsksFirst() {
        val page = Page(SettingsUiState(username = "alice", devices = listOf(THIS, LAPTOP)), SettingsSection.YOUR_DEVICES)
        show(page)
        compose.onNodeWithTag("sign-out-device-2").performClick()
        compose.onNodeWithText("Sign out “Laptop”?").assertIsDisplayed()
        compose.onNodeWithText("Cancel").performClick()
        compose.onNodeWithTag("sign-out-device-dialog").assertDoesNotExist()
        assertTrue(page.signedOut.isEmpty())
        compose.onNodeWithTag("sign-out-device-2").performClick()
        compose.onNodeWithTag("sign-out-device-confirm").performClick()
        assertEquals(listOf(LAPTOP), page.signedOut)
        compose.onNodeWithTag("sign-out-device-dialog").assertDoesNotExist()
    }

    @Test
    fun signingAllOthersOutAsksFirst() {
        val page = Page(SettingsUiState(username = "alice", devices = listOf(THIS, LAPTOP, KITCHEN)), SettingsSection.YOUR_DEVICES)
        show(page)
        compose.onNodeWithTag("sign-out-others").performClick()
        compose.onNodeWithText("2 devices will have to sign in again. This phone stays signed in.").assertIsDisplayed()
        compose.onNodeWithTag("sign-out-device-confirm").performClick()
        assertEquals(1, page.others)
        assertTrue(page.signedOut.isEmpty())
    }

    @Test
    fun aloneThereIsNoSignOutAllAndOneMessageIsAnnounced() {
        val page = Page(
            SettingsUiState(username = "alice", devices = listOf(THIS), devicesMessage = DevicesMessage.SignedOutOthers(2)),
            SettingsSection.YOUR_DEVICES,
        )
        show(page)
        compose.onNodeWithTag("sign-out-others").assertDoesNotExist()
        compose.onNodeWithTag("no-other-devices").assertIsDisplayed()
        compose.onNodeWithTag("devices-message").assertTextContains("2 other devices were signed out.")
        page.state = page.state.copy(devicesMessage = DevicesMessage.AlreadyGone)
        compose.onNodeWithTag("devices-message").assertTextContains("That device was already signed out.")
    }

    @Test
    fun aDeviceGoneBeforeTheConfirmIsNotAskedAbout() {
        val page = Page(SettingsUiState(username = "alice", devices = listOf(THIS, LAPTOP)), SettingsSection.YOUR_DEVICES)
        show(page)
        compose.onNodeWithTag("sign-out-device-2").performClick()
        compose.onNodeWithTag("sign-out-device-dialog").assertIsDisplayed()
        page.state = page.state.copy(devices = listOf(THIS))
        compose.waitForIdle()
        compose.onNodeWithTag("sign-out-device-dialog").assertDoesNotExist()
        assertTrue(page.signedOut.isEmpty())
    }

    @Test
    fun hostileNamesAreShownAsText() {
        val evil = MyDevice(9, "<b>Evil</b> %s %1\$s", "browser", "<script>", 1, null, false)
        val page = Page(SettingsUiState(username = "alice", devices = listOf(THIS, evil)), SettingsSection.YOUR_DEVICES)
        show(page)
        compose.onNode(hasTestTag("settings-your-devices")).performScrollToNode(hasTestTag("device-9"))
        compose.onNodeWithTag("device-9").assertTextContains("<b>Evil</b> %s %1\$s", substring = true)
        compose.onNodeWithTag("device-9").assertTextContains("<script> · Not used yet", substring = true)
    }
}
