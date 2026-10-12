package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.PairingInvite
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairConfirm
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairScreen
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairUiState
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairingLinks
import io.github.jamezuk.lyrionanywhere.ui.onboarding.RefusedLinkDialog
import io.github.jamezuk.lyrionanywhere.ui.onboarding.ServerScreen
import io.github.jamezuk.lyrionanywhere.ui.onboarding.ServerUiState
import io.github.jamezuk.lyrionanywhere.ui.onboarding.SwitchAccountDialog
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** Links the app is opened with: kept only if valid, and never acted on by themselves. */
class PairingLinksTest {
    private val account = FakeAccount()
    private val read = mutableListOf<String>()
    private val links = PairingLinks { text -> read += text; account.readPairingCode(text) }

    private val good = "lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=ABCDEFGHJK"

    @Test
    fun aValidLinkIsHeldUntilTakenOnce() {
        assertTrue(links.offer(good))
        assertEquals(PairingInvite("https://music.example.com/", "ABCDEFGHJK"), links.pending.value)
        assertFalse(links.refused.value)
        // Holding it does nothing else.
        assertTrue(account.paired.isEmpty())
        assertEquals(0, account.signedOut)
        assertEquals("ABCDEFGHJK", links.take()?.code)
        assertNull("taken once", links.take())
    }

    @Test
    fun hostileLinksAreDroppedAndReported() {
        for (bad in listOf(
            null,
            "",
            "   ",
            // Not the pairing scheme or host at all: never even parsed.
            "https://music.example.com/?code=ABCDEFGHJK",
            "http://lyrion-anywhere/pair?code=ABCDEFGHJK",
            "intent://pair?code=ABCDEFGHJK#Intent;scheme=lyrion-anywhere;end",
            "javascript:alert(1)//lyrion-anywhere://pair?code=ABCDEFGHJK",
            "content://lyrion-anywhere.pair/x",
            "file:///sdcard/lyrion-anywhere://pair",
            "lyrion-anywhere://evil?code=ABCDEFGHJK",
            "lyrion-anywhere:pair?code=ABCDEFGHJK",
            // A bare code isn't a link.
            "ABCDEFGHJK",
            // Overlong.
            "lyrion-anywhere://pair?code=ABCDEFGHJK&server=" + "a".repeat(PairingLinks.MAX_LINK),
        )) {
            assertFalse("$bad", links.offer(bad))
            assertNull("$bad", links.pending.value)
            assertTrue("$bad", links.refused.value)
            links.acknowledgeRefused()
        }
        assertTrue("only well-formed links reach the parser: $read", read.isEmpty())
        // Links that look right but the strict check refuses.
        for (bad in listOf(
            "lyrion-anywhere://pair?server=http%3A%2F%2Fmusic.example.com&code=ABCDEFGHJK",
            "lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=ABCDEFGHJK&token=x",
            "lyrion-anywhere://pair?code=ABCDEFGHJK&code=EFGHJKLMNP",
            "lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com",
            "lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=%3Cscript%3E",
        )) {
            assertFalse(bad, links.offer(bad))
            assertNull(bad, links.pending.value)
            assertTrue(bad, links.refused.value)
            links.acknowledgeRefused()
        }
        assertTrue(account.paired.isEmpty())
        assertEquals(0, account.signedOut)
    }

    @Test
    fun aBadLinkDoesNotDropAGoodOneWaiting() {
        assertTrue(links.offer(good))
        assertFalse(links.offer("lyrion-anywhere://evil"))
        assertEquals("ABCDEFGHJK", links.pending.value?.code)
        // A newer valid link replaces it; dismissing forgets it.
        assertTrue(links.offer("lyrion-anywhere://pair?code=EFGHJKLMNP"))
        assertEquals("EFGHJKLMNP", links.pending.value?.code)
        links.dismiss()
        assertNull(links.pending.value)
    }

    @Test
    fun onlyPairingLinksReachTheParser() {
        val strict = PairingLinks { ApiResult.Ok(PairingInvite(null, "X")) }
        // Anything not starting with the pairing link prefix is refused before it.
        assertFalse(strict.offer("ABCDEFGHJK"))
        assertTrue(strict.offer("LYRION-ANYWHERE://PAIR?code=ABCDEFGHJK"))
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class PairingScreenTests {
    @get:Rule
    val compose = createComposeRule()

    private fun pair(state: PairUiState, onConfirm: () -> Unit = {}, onCancel: () -> Unit = {}, onSubmit: () -> Unit = {}) {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                PairScreen(
                    state = state,
                    cameraPermitted = false,
                    onRequestCamera = {},
                    onScanned = {},
                    onCodeChange = {},
                    onSubmit = onSubmit,
                    onBack = {},
                    onConfirm = onConfirm,
                    onCancel = onCancel,
                )
            }
        }
    }

    @Test
    fun theServerIsShownAndConfirmedBeforePairing() {
        var confirmed = 0
        var cancelled = 0
        pair(
            PairUiState(
                scanning = false,
                confirm = PairConfirm("ABCDE-FGHJK", "lms.example.lan", "https://lms.example.lan/", replaces = "music.example.com"),
            ),
            onConfirm = { confirmed++ },
            onCancel = { cancelled++ },
        )
        compose.onNodeWithText("Sign in to lms.example.lan?").assertIsDisplayed()
        compose.onNodeWithText("Server: https://lms.example.lan/").assertIsDisplayed()
        compose.onNodeWithText("Code: ABCDE-FGHJK").assertIsDisplayed()
        compose.onNodeWithText("This replaces music.example.com as the server this app uses.").assertIsDisplayed()
        // No scanner or code field while asking.
        compose.onNodeWithText("Pairing code").assertDoesNotExist()
        compose.onNode(hasText("Cancel") and hasClickAction()).performClick()
        assertEquals(1, cancelled)
        compose.onNode(hasText("Sign in") and hasClickAction()).performClick()
        assertEquals(1, confirmed)
    }

    @Test
    fun scanningAndTypingStayAvailable() {
        var submitted = 0
        pair(PairUiState(code = "ABCDEFGHJK"), onSubmit = { submitted++ })
        compose.onNode(hasText("Scan QR code") and hasClickAction()).assertIsDisplayed()
        compose.onNodeWithText("Pairing code").assertIsDisplayed()
        compose.onNode(hasText("Continue") and hasClickAction()).performClick()
        assertEquals(1, submitted)
    }

    @Test
    fun theWelcomeScreenOffersScanning() {
        var scans = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                ServerScreen(ServerUiState(), onAddressChange = {}, onSubmit = {}, onScan = { scans++ })
            }
        }
        compose.onNode(hasText("Scan QR code") and hasClickAction()).performClick()
        assertEquals(1, scans)
    }

    @Test
    fun aLinkWhileSignedInAsksBeforeSigningOut() {
        var go = 0
        var stay = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SwitchAccountDialog(current = "music.example.com", target = "lms.example.lan", onContinue = { go++ }, onCancel = { stay++ })
            }
        }
        compose.onNodeWithText("Sign in with this link?").assertIsDisplayed()
        compose.onNodeWithText("music.example.com", substring = true).assertIsDisplayed()
        compose.onNodeWithText("lms.example.lan", substring = true).assertIsDisplayed()
        compose.onNode(hasText("Cancel") and hasClickAction()).performClick()
        assertEquals(1, stay)
        assertEquals(0, go)
        compose.onNode(hasText("Sign out and continue") and hasClickAction()).performClick()
        assertEquals(1, go)
    }

    @Test
    fun aLinkToTheSameServerSaysSo() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SwitchAccountDialog(current = "music.example.com", target = null, onContinue = {}, onCancel = {})
            }
        }
        compose.onNodeWithText("already signed in to music.example.com", substring = true).assertIsDisplayed()
    }

    @Test
    fun aRefusedLinkIsExplained() {
        var closed = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) { RefusedLinkDialog(onDismiss = { closed++ }) }
        }
        compose.onNodeWithText("Not a pairing link").assertIsDisplayed()
        compose.onNode(hasText("Close") and hasClickAction()).performClick()
        assertEquals(1, closed)
    }
}
