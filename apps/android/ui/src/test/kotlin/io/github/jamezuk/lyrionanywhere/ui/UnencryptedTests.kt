package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.PairingInvite
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.ServerInfo
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairConfirm
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairScreen
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairUiState
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairViewModel
import io.github.jamezuk.lyrionanywhere.ui.onboarding.ServerScreen
import io.github.jamezuk.lyrionanywhere.ui.onboarding.ServerViewModel
import io.github.jamezuk.lyrionanywhere.ui.onboarding.UnencryptedDialog
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsSection
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsUiState
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsViewModel
import io.github.jamezuk.lyrionanywhere.data.AuthState
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

private const val VPN = "http://10.8.0.1:9443/"
private val VPN_SERVER = ServerInfo(VPN, "10.8.0.1:9443", secure = false)

/** Trusted networks (docs/plans/trusted-networks.md): the view models ask before http://. */
class UnencryptedViewModelTest : MainDispatcherTest() {
    @Test
    fun aPrivateHttpAddressAsksBeforeItIsUsed() = runTest(dispatcher) {
        val account = FakeAccount().apply { useServerResult = ApiResult.Ok(VPN_SERVER) }
        val vm = ServerViewModel(account)
        vm.onAddressChange(VPN)
        var done = 0
        vm.submit { done++ }
        assertTrue("the warning shows", vm.state.value.confirmUnencrypted)
        assertNull("it isn't an error", vm.state.value.error)
        assertEquals(0, done)
        assertEquals(listOf(VPN to false), account.useServerCalls)
        // The address can't change under the question.
        vm.onAddressChange("https://elsewhere.example.com")
        assertEquals(VPN, vm.state.value.address)
        // Submitting again while asked does nothing.
        vm.submit { done++ }
        assertEquals(1, account.useServerCalls.size)

        vm.connectUnencrypted { done++ }
        assertFalse(vm.state.value.confirmUnencrypted)
        assertEquals(listOf(VPN to false, VPN to true), account.useServerCalls)
        assertEquals(1, done)
    }

    @Test
    fun cancelUsesNothingAndKeepsTheAddress() = runTest(dispatcher) {
        val account = FakeAccount()
        val vm = ServerViewModel(account)
        vm.onAddressChange("http://[fd00::5]:9443")
        vm.submit {}
        assertTrue(vm.state.value.confirmUnencrypted)
        vm.cancelUnencrypted()
        assertFalse(vm.state.value.confirmUnencrypted)
        assertEquals("http://[fd00::5]:9443", vm.state.value.address)
        assertTrue("never used confirmed", account.useServerCalls.none { it.second })
        // Connect anyway after cancelling (a stale tap) does nothing either.
        vm.connectUnencrypted {}
        assertTrue(account.useServerCalls.none { it.second })
    }

    @Test
    fun httpsAndRefusedAddressesNeverAsk() = runTest(dispatcher) {
        val account = FakeAccount()
        val vm = ServerViewModel(account)
        vm.onAddressChange("https://music.example.com")
        var done = 0
        vm.submit { done++ }
        assertFalse(vm.state.value.confirmUnencrypted)
        assertEquals(1, done)
        // A name over http is refused outright (by the rule, here the fake's result).
        account.useServerResult = ApiResult.Err(AppError.Reason.PLAIN_HTTP.error())
        for (a in listOf("http://music.example.com", "http://203.0.113.10:9443", "http://localhost:9443")) {
            vm.onAddressChange(a)
            vm.submit { done++ }
            assertFalse(a, vm.state.value.confirmUnencrypted)
            assertEquals(a, AppError.Reason.PLAIN_HTTP, (vm.state.value.error as AppError.InvalidInput).reason)
        }
        assertEquals(1, done)
        assertTrue(account.useServerCalls.none { it.second })
    }

    @Test
    fun pairingWithAPrivateHttpServerAsksBeforeSending() = runTest(dispatcher) {
        val account = FakeAccount().apply { checkServer = { ApiResult.Ok(VPN_SERVER) } }
        val vm = PairViewModel(account)
        vm.onScanned("lyrion-anywhere://pair?server=http%3A%2F%2F10.8.0.1%3A9443&code=ABCDEFGHJK")
        val confirm = vm.state.value.confirm!!
        assertTrue(confirm.unencrypted)
        assertEquals(VPN, confirm.serverUrl)
        vm.confirm()
        assertTrue("the warning shows", vm.state.value.confirmUnencrypted)
        assertTrue("nothing sent yet", account.paired.isEmpty())
        vm.cancelUnencrypted()
        assertFalse(vm.state.value.confirmUnencrypted)
        assertTrue(account.paired.isEmpty())
        vm.connectUnencrypted()
        assertTrue("a stale Connect anyway sends nothing", account.paired.isEmpty())
        vm.confirm()
        vm.connectUnencrypted()
        assertEquals(VPN, account.paired.single().server)
        assertEquals(listOf(true), account.pairedConfirmed)
    }

    @Test
    fun pairingWithHttpsNeverAsks() = runTest(dispatcher) {
        val account = FakeAccount()
        val vm = PairViewModel(account)
        vm.offer(PairingInvite("https://music.example.com/", "ABCDEFGHJK"))
        assertFalse(vm.state.value.confirm!!.unencrypted)
        vm.confirm()
        assertFalse(vm.state.value.confirmUnencrypted)
        assertEquals(listOf(false), account.pairedConfirmed)
    }

    @Test
    fun aPairingLinkToANameOverHttpIsRefused() = runTest(dispatcher) {
        val account = FakeAccount()
        val vm = PairViewModel(account)
        vm.onScanned("lyrion-anywhere://pair?server=http%3A%2F%2Fmusic.example.com&code=ABCDEFGHJK")
        assertNull(vm.state.value.confirm)
        assertTrue(vm.state.value.error is AppError.InvalidInput)
        assertTrue(account.paired.isEmpty())
    }

    @Test
    fun settingsKnowsTheServerIsUnencrypted() = runTest(dispatcher) {
        val account = FakeAccount().apply { _state.value = AuthState.SignedInState(VPN_SERVER, WHO) }
        assertFalse(SettingsViewModel(account).state.value.serverSecure)
        account._state.value = AuthState.SignedInState(SERVER, WHO)
        assertTrue(SettingsViewModel(account).state.value.serverSecure)
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class UnencryptedScreenTests {
    @get:Rule
    val compose = createComposeRule()

    private val title = "Unencrypted connection"
    private val body = "This server address isn’t encrypted (http://). Anyone on the same network could read your password and what you play. Only continue if this is your own VPN or a network you trust."

    @Test
    fun theDialogSaysWhatItMeansAndOffersAChoice() {
        var connected = 0
        var cancelled = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) { UnencryptedDialog(onConnect = { connected++ }, onCancel = { cancelled++ }) }
        }
        compose.onNodeWithText(title).assertIsDisplayed()
        compose.onNodeWithText(body).assertIsDisplayed()
        compose.onNode(hasText("Cancel") and hasClickAction()).performClick()
        assertEquals(1, cancelled)
        compose.onNode(hasText("Connect anyway") and hasClickAction()).performClick()
        assertEquals(1, connected)
        compose.assertAccessible()
    }

    /** The welcome screen with its real view model over the fake account. */
    private fun welcome(account: FakeAccount, onDone: () -> Unit = {}): ServerViewModel {
        val vm = ServerViewModel(account)
        compose.setContent {
            val ui by vm.state.collectAsState()
            LyrionTheme(dynamicColour = false) {
                ServerScreen(
                    state = ui,
                    onAddressChange = vm::onAddressChange,
                    onSubmit = { vm.submit(onDone) },
                    onConnectUnencrypted = { vm.connectUnencrypted(onDone) },
                    onCancelUnencrypted = vm::cancelUnencrypted,
                )
            }
        }
        return vm
    }

    private fun type(address: String) {
        compose.onNode(hasSetTextAction()).performTextReplacement(address)
        compose.onNode(hasText("Continue") and hasClickAction()).performClick()
        compose.waitForIdle()
    }

    @Test
    fun onlyAPrivateHttpAddressShowsTheDialog() {
        val account = FakeAccount()
        var done = 0
        welcome(account) { done++ }
        type("https://music.example.com")
        compose.onNodeWithTag("unencrypted-dialog").assertDoesNotExist()
        assertEquals(1, done)
        type("music.example.com")
        compose.onNodeWithTag("unencrypted-dialog").assertDoesNotExist()
        type("http://10.8.0.1:9443")
        compose.onNodeWithText(title).assertIsDisplayed()
        assertEquals("not used before the answer", 2, done)
    }

    @Test
    fun cancelLeavesTheAddressAndSignsNothingIn() {
        val account = FakeAccount()
        var done = 0
        val vm = welcome(account) { done++ }
        type("http://10.8.0.1:9443")
        compose.onNodeWithTag("unencrypted-cancel").performClick()
        compose.waitForIdle()
        compose.onNodeWithTag("unencrypted-dialog").assertDoesNotExist()
        compose.onNodeWithText("http://10.8.0.1:9443").assertIsDisplayed()
        assertEquals("http://10.8.0.1:9443", vm.state.value.address)
        assertEquals(0, done)
        assertTrue(account.useServerCalls.none { it.second })
    }

    @Test
    fun connectAnywayUsesTheServer() {
        val account = FakeAccount().apply { useServerResult = ApiResult.Ok(VPN_SERVER) }
        var done = 0
        welcome(account) { done++ }
        type("http://10.8.0.1:9443")
        compose.onNodeWithTag("unencrypted-connect").performClick()
        compose.waitForIdle()
        compose.onNodeWithTag("unencrypted-dialog").assertDoesNotExist()
        assertEquals(1, done)
        assertEquals("http://10.8.0.1:9443" to true, account.useServerCalls.last())
    }

    @Test
    fun thePairingConfirmationMarksAnUnencryptedServerAndAsks() {
        var connected = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                PairScreen(
                    state = PairUiState(
                        scanning = false,
                        confirm = PairConfirm("ABCDE-FGHJK", "10.8.0.1:9443", VPN, unencrypted = true),
                        confirmUnencrypted = true,
                    ),
                    cameraPermitted = false,
                    onRequestCamera = {},
                    onScanned = {},
                    onCodeChange = {},
                    onSubmit = {},
                    onBack = {},
                    onConnectUnencrypted = { connected++ },
                )
            }
        }
        compose.onNodeWithText(title).assertIsDisplayed()
        compose.onNodeWithTag("unencrypted-connect").performClick()
        assertEquals(1, connected)
    }

    private fun account(secure: Boolean) {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SettingsScreen(
                    state = SettingsUiState(serverName = if (secure) "music.example.com" else "10.8.0.1:9443", serverSecure = secure, username = "alice"),
                    appVersion = "1",
                    libraryVersion = "1",
                    section = SettingsSection.ACCOUNT,
                    onSignOut = {},
                    onLicences = {},
                )
            }
        }
    }

    @Test
    fun settingsShowsTheBadgeForAnUnencryptedServer() {
        account(secure = false)
        // The row reads as one (merged) item; the badge is inside it.
        compose.onNodeWithTag("server-unencrypted", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithText("Unencrypted").assertIsDisplayed()
        compose.onNodeWithTag("server-unencrypted-explained", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithText("10.8.0.1:9443").assertIsDisplayed()
        compose.assertAccessible()
    }

    @Test
    fun settingsShowsNoBadgeForAnEncryptedServer() {
        account(secure = true)
        compose.onNodeWithText("music.example.com").assertIsDisplayed()
        compose.onNodeWithTag("server-unencrypted", useUnmergedTree = true).assertDoesNotExist()
        compose.onNodeWithTag("server-unencrypted-explained", useUnmergedTree = true).assertDoesNotExist()
        compose.onNodeWithText("Unencrypted").assertDoesNotExist()
    }
}
