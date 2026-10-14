package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.AccountInfo
import io.github.jamezuk.lyrionanywhere.core.OwnDevice
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.PasswordChangeError
import io.github.jamezuk.lyrionanywhere.data.PasswordChangeResult
import io.github.jamezuk.lyrionanywhere.ui.settings.PasswordFormProblem
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsUiState
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsViewModel
import io.github.jamezuk.lyrionanywhere.ui.settings.passwordFormProblem
import io.github.jamezuk.lyrionanywhere.ui.settings.passwordFormReady
import io.github.jamezuk.lyrionanywhere.ui.settings.twoStepDigits
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

private const val OLD = "old password 1"
private const val NEW = "a new passphrase"

class PasswordFormTest {
    @Test
    fun theFormIsCheckedBeforeSending() {
        assertNull(passwordFormProblem("", "", ""))
        assertEquals(PasswordFormProblem.TooShort, passwordFormProblem(OLD, "short", ""))
        assertEquals(PasswordFormProblem.Same, passwordFormProblem(OLD, OLD, OLD))
        assertEquals(PasswordFormProblem.Mismatch, passwordFormProblem(OLD, NEW, "$NEW!"))
        assertNull(passwordFormProblem(OLD, NEW, NEW))
        // Ten characters of any script are ten.
        assertNull(passwordFormProblem(OLD, "🎵".repeat(10), ""))

        assertTrue(passwordFormReady(OLD, NEW, NEW, twoStep = false, code = ""))
        assertFalse(passwordFormReady("", NEW, NEW, twoStep = false, code = ""))
        assertFalse(passwordFormReady(OLD, NEW, "", twoStep = false, code = ""))
        assertFalse(passwordFormReady(OLD, NEW, NEW, twoStep = true, code = "12345"))
        assertTrue(passwordFormReady(OLD, NEW, NEW, twoStep = true, code = "123 456"))
        assertEquals("123456", twoStepDigits(" 123 456 "))
        assertNull(twoStepDigits("12345a"))
    }
}

class ChangePasswordViewModelTest : MainDispatcherTest() {
    @Test
    fun aChangeShowsItsOutcome() = runTest(dispatcher) {
        val account = FakeAccount()
        account._state.value = AuthState.SignedInState(SERVER, WHO)
        val vm = SettingsViewModel(account)
        assertTrue(vm.state.value.account!!.canChangePassword)

        account.passwordResult = PasswordChangeResult.Failed(PasswordChangeError.WrongPassword)
        vm.changePassword(OLD, NEW, "123456", false)
        assertEquals(listOf<Any?>(OLD, NEW, "123456", false), account.passwordChanges.single())
        assertEquals(PasswordChangeError.WrongPassword, vm.state.value.passwordError)
        assertFalse(vm.state.value.passwordChanging)

        val done = PasswordChangeResult.Changed(1, emptyList())
        account.passwordResult = done
        vm.changePassword(OLD, NEW, "654321", true)
        assertEquals(done, vm.state.value.passwordChanged)
        assertNull(vm.state.value.passwordError)

        vm.closePasswordForm()
        assertNull(vm.state.value.passwordChanged)
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class ChangePasswordScreenTests {
    @get:Rule
    val compose = createComposeRule()

    private fun account(canChange: Boolean = true, twoStep: Boolean = false) =
        AccountInfo("alice", false, twoStep, false, canChange)

    /** Settings with a fake form behind it: submissions recorded, outcome settable. */
    private class Form(initial: SettingsUiState) {
        var state by mutableStateOf(initial)
        val sent = mutableListOf<List<Any?>>()
    }

    private fun show(form: Form) {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SettingsScreen(
                    state = form.state,
                    appVersion = "1",
                    libraryVersion = "1",
                    onSignOut = {},
                    onLicences = {},
                    // Settings › Account (Wave 3.5: Settings is an index of pages).
                    section = io.github.jamezuk.lyrionanywhere.ui.settings.SettingsSection.ACCOUNT,
                    onChangePassword = { c, n, code, others -> form.sent += listOf(c, n, code, others) },
                )
            }
        }
    }

    @Test
    fun hiddenWhenTheServerCantChangePasswords() {
        val form = Form(SettingsUiState(username = "alice", account = null))
        show(form)
        compose.onNodeWithTag("change-password").assertDoesNotExist()
        form.state = form.state.copy(account = account(canChange = false))
        compose.onNodeWithTag("change-password").assertDoesNotExist()
        compose.onNodeWithText("Two-step verification").assertIsDisplayed()
        form.state = form.state.copy(account = account(canChange = true))
        compose.onNodeWithTag("change-password").assertIsDisplayed()
    }

    @Test
    fun theFormSendsTheNewPasswordOnce() {
        val form = Form(SettingsUiState(username = "alice", account = account()))
        show(form)
        compose.onNodeWithTag("change-password").performClick()
        compose.onNodeWithTag("password-code").assertDoesNotExist()
        val submit = compose.onNodeWithTag("password-submit")
        submit.assertIsNotEnabled()
        compose.onNodeWithTag("password-current").performTextInput(OLD)
        compose.onNodeWithTag("password-new").performTextInput(NEW)
        compose.onNodeWithTag("password-confirm").performTextInput("$NEW?")
        compose.onNodeWithText("The new passwords don’t match.").assertIsDisplayed()
        submit.assertIsNotEnabled()
        compose.onNodeWithTag("password-sign-out-others").performClick()
        // Fix the confirmation.
        compose.onNodeWithTag("password-confirm").performTextReplacement(NEW)
        compose.onNodeWithText("The new passwords don’t match.").assertDoesNotExist()
        submit.assertIsEnabled()
        submit.performClick()
        assertEquals(listOf<Any?>(OLD, NEW, null, true), form.sent.single())
    }

    @Test
    fun twoStepAccountsAreAskedForTheCode() {
        val form = Form(SettingsUiState(username = "alice", account = account(twoStep = true)))
        show(form)
        compose.onNodeWithTag("change-password").performClick()
        compose.onNodeWithTag("password-current").performTextInput(OLD)
        compose.onNodeWithTag("password-new").performTextInput(NEW)
        compose.onNodeWithTag("password-confirm").performTextInput(NEW)
        val submit = compose.onNodeWithTag("password-submit")
        submit.assertIsNotEnabled()
        compose.onNodeWithTag("password-code").performTextInput("123 456")
        submit.assertIsEnabled()
        submit.performClick()
        assertEquals(listOf<Any?>(OLD, NEW, "123456", false), form.sent.single())
    }

    @Test
    fun aWrongPasswordIsShownInTheForm() {
        val form = Form(SettingsUiState(username = "alice", account = account()))
        show(form)
        compose.onNodeWithTag("change-password").performClick()
        form.state = form.state.copy(passwordError = PasswordChangeError.WrongPassword)
        compose.onNodeWithText("The current password or two-step code isn’t right.").assertIsDisplayed()
        form.state = form.state.copy(passwordError = PasswordChangeError.LockedOut(15))
        compose.onNodeWithText("Too many attempts. Try again in 15 minutes.").assertIsDisplayed()
        form.state = form.state.copy(passwordError = PasswordChangeError.Refused("password must be at least 10 characters"))
        compose.onNodeWithText("Not changed: password must be at least 10 characters.").assertIsDisplayed()
    }

    @Test
    fun successSaysWhichDevicesAreStillSignedIn() {
        val form = Form(SettingsUiState(username = "alice", account = account()))
        show(form)
        compose.onNodeWithTag("change-password").performClick()
        form.state = form.state.copy(
            passwordChanged = PasswordChangeResult.Changed(
                0,
                listOf(OwnDevice(2, "Kitchen", "Firefox on Linux", null), OwnDevice(3, "Tablet", null, null)),
            ),
        )
        compose.onNodeWithText("Password changed. This phone stays signed in.").assertIsDisplayed()
        compose.onNodeWithText("Still signed in with your account:").assertIsDisplayed()
        compose.onNodeWithText("Kitchen (Firefox on Linux)").assertIsDisplayed()
        compose.onNodeWithText("Tablet").assertIsDisplayed()
        form.state = form.state.copy(passwordChanged = PasswordChangeResult.Changed(2, emptyList()))
        compose.onNodeWithText("2 other devices were signed out.").assertIsDisplayed()
        compose.onNodeWithText("No other devices are signed in.").assertIsDisplayed()
    }
}
