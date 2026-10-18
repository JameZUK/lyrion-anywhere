package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.text.TextLayoutResult
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
import io.github.jamezuk.lyrionanywhere.data.AccountRepository
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.DevicesMessage
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsSection
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsUiState
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsViewModel
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.CompletableDeferred
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

/** The fake account, with the list read held until the test lets it through. */
private class GatedAccount(val inner: FakeAccount) : AccountRepository by inner {
    var gate: CompletableDeferred<Unit>? = null
    var loads = 0

    override suspend fun myDevices(): ApiResult<List<MyDevice>> {
        loads++
        gate?.await()
        return inner.myDevices()
    }
}

/** M21: messages that go stale, overlapping actions, and a failed first load. */
class YourDevicesMessagesTest : MainDispatcherTest() {
    private fun signedIn(): FakeAccount = FakeAccount().apply {
        _state.value = AuthState.SignedInState(SERVER, WHO)
        accountInfo = AccountInfo("alice", false, true, false, true, true)
        devices = mutableListOf(THIS, LAPTOP, KITCHEN)
    }

    @Test
    fun aLoadThatWorksClearsAnEarlierLoadFailure() = runTest(dispatcher) {
        val account = signedIn()
        account.devicesError = AppError.Offline()
        val vm = SettingsViewModel(account)
        vm.loadDevices()
        assertEquals(DevicesMessage.Failed(AppError.Offline()), vm.state.value.devicesMessage)
        account.devicesError = null
        vm.loadDevices()
        assertEquals(listOf(THIS, LAPTOP, KITCHEN), vm.state.value.devices)
        assertNull(vm.state.value.devicesMessage)
    }

    @Test
    fun aFailedReloadKeepsTheSignOutMessageAndDropsTheDevice() = runTest(dispatcher) {
        val account = signedIn()
        val vm = SettingsViewModel(account)
        vm.loadDevices()
        // The shared rate limit refuses the reload straight after the sign-out.
        account.devicesError = AppError.RateLimited(60)
        vm.signOutDevice(LAPTOP)
        assertEquals(DevicesMessage.SignedOut("Laptop"), vm.state.value.devicesMessage)
        assertEquals(listOf(THIS, KITCHEN), vm.state.value.devices)
        assertFalse(vm.state.value.devicesBusy)

        vm.signOutOtherDevices()
        assertEquals(DevicesMessage.SignedOutOthers(1), vm.state.value.devicesMessage)
        assertEquals(listOf(THIS), vm.state.value.devices)
    }

    @Test
    fun anAlreadyGoneDeviceLeavesTheListEvenIfTheReloadFails() = runTest(dispatcher) {
        val account = signedIn()
        val vm = SettingsViewModel(account)
        vm.loadDevices()
        account.devices.remove(LAPTOP)
        account.devicesError = AppError.ServerBusy(30)
        vm.signOutDevice(LAPTOP)
        assertEquals(DevicesMessage.AlreadyGone, vm.state.value.devicesMessage)
        assertEquals(listOf(THIS, KITCHEN), vm.state.value.devices)
    }

    @Test
    fun noSecondSignOutWhileTheListIsBeingReadAgain() = runTest(dispatcher) {
        val account = GatedAccount(signedIn())
        val vm = SettingsViewModel(account)
        vm.loadDevices()
        val gate = CompletableDeferred<Unit>().also { account.gate = it }
        vm.signOutDevice(LAPTOP)
        // Signed out; the list is still being read: busy, so nothing else can start.
        assertEquals(listOf(2L), account.inner.signedOutDevices)
        assertTrue(vm.state.value.devicesBusy)
        vm.signOutDevice(KITCHEN)
        vm.signOutOtherDevices()
        assertEquals(listOf(2L), account.inner.signedOutDevices)
        assertEquals(0, account.inner.othersSignedOut)
        gate.complete(Unit)
        assertFalse(vm.state.value.devicesBusy)
        assertEquals(listOf(THIS, KITCHEN), vm.state.value.devices)
        assertEquals(DevicesMessage.SignedOut("Laptop"), vm.state.value.devicesMessage)
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

    /**
     * Review, 15 October: a back stack restored onto a server that doesn't offer Your
     * devices (an older gateway) opened the page anyway, asked for the list, and showed the
     * gateway's 404 text as the app's own message.
     */
    @Test
    fun aRestoredPageOnAServerWithoutItAsksForNothing() {
        val page = Page(SettingsUiState(username = "alice", account = account(devices = false)), SettingsSection.YOUR_DEVICES)
        show(page)
        compose.waitForIdle()
        assertEquals(0, page.loads)
        compose.onNodeWithTag("your-devices-not-offered").assertIsDisplayed()
        compose.onNodeWithTag("sign-out-others").assertDoesNotExist()
    }

    @Test
    fun aNotFoundAnswerIsSaidInTheAppsOwnWords() {
        val gone = AppError.InvalidInput("not found: /v1/devices", AppError.Reason.NOT_FOUND)
        val page = Page(SettingsUiState(username = "alice", devicesMessage = DevicesMessage.Failed(gone)), SettingsSection.YOUR_DEVICES)
        show(page)
        compose.onNodeWithText("/v1/devices", substring = true).assertDoesNotExist()
        compose.onNodeWithText("isn’t on the server", substring = true).assertExists()
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

    /** M21: a failed first load offers Try again, which reads the list again. */
    @Test
    fun aFailedFirstLoadOffersRetry() {
        val page = Page(
            SettingsUiState(username = "alice", devicesMessage = DevicesMessage.Failed(AppError.Offline())),
            SettingsSection.YOUR_DEVICES,
        )
        show(page)
        compose.waitForIdle()
        assertEquals(1, page.loads)
        compose.onNodeWithText("Try again").assertIsDisplayed().performClick()
        assertEquals(2, page.loads)
    }

    @Test
    fun noSignOutWhileBusy() {
        val page = Page(
            SettingsUiState(username = "alice", devices = listOf(THIS, LAPTOP), devicesBusy = true),
            SettingsSection.YOUR_DEVICES,
        )
        show(page)
        compose.onNodeWithTag("sign-out-device-2").assertIsNotEnabled()
        compose.onNodeWithTag("sign-out-others").assertIsNotEnabled()
    }

    private fun messageColour(): Color {
        val results = mutableListOf<TextLayoutResult>()
        compose.onNodeWithTag("devices-message").fetchSemanticsNode()
            .config[SemanticsActions.GetTextLayoutResult].action!!.invoke(results)
        return results.single().layoutInput.style.color
    }

    @Test
    fun aFailureIsShownInTheErrorColour() {
        var error = Color.Unspecified
        var normal = Color.Unspecified
        var state by mutableStateOf(
            SettingsUiState(
                username = "alice",
                devices = listOf(THIS, LAPTOP),
                devicesMessage = DevicesMessage.Failed(AppError.RateLimited(60)),
            ),
        )
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                error = MaterialTheme.colorScheme.error
                normal = MaterialTheme.colorScheme.onSurface
                SettingsScreen(
                    state = state,
                    appVersion = "1",
                    libraryVersion = "1",
                    onSignOut = {},
                    onLicences = {},
                    section = SettingsSection.YOUR_DEVICES,
                    onSection = {},
                    onLoadDevices = {},
                    onSignOutDevice = {},
                    onSignOutOtherDevices = {},
                    nowSeconds = { NOW },
                )
            }
        }
        compose.onNodeWithTag("devices-message").assertIsDisplayed()
        assertEquals(error, messageColour())
        state = state.copy(devicesMessage = DevicesMessage.SignedOut("Laptop"))
        compose.waitForIdle()
        assertEquals(normal, messageColour())
        assertTrue(error != normal)
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
