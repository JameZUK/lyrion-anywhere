package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.offline.ContinueOnState
import io.github.jamezuk.lyrionanywhere.data.offline.HandOffResult
import io.github.jamezuk.lyrionanywhere.ui.offline.ContinueChoice
import io.github.jamezuk.lyrionanywhere.ui.offline.ContinueOnPrompt
import io.github.jamezuk.lyrionanywhere.ui.offline.continueChoices
import io.github.jamezuk.lyrionanywhere.ui.offline.defaultContinueChoice
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** "Back online. Continue on <player>?" (SPEC §8.4). */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class ContinueOnPromptTests {
    @get:Rule
    val compose = createComposeRule()

    private val phone = "02:4c:41:00:00:01"
    private val players = listOf(
        Player(phone, "Pixel", null, true, true, false),
        Player("aa:bb:cc:dd:ee:01", "Kitchen", null, true, true, false),
        Player("aa:bb:cc:dd:ee:02", "", null, true, false, false),
    )
    private val choices = continueChoices(players, phone, "This phone", phoneAvailable = true)

    @Test
    fun theChoicesAreThisPhoneThenTheListedPlayersWithTheLastUsedFirst() {
        assertEquals(
            listOf(ContinueChoice(null, "This phone"), ContinueChoice("aa:bb:cc:dd:ee:01", "Kitchen"), ContinueChoice("aa:bb:cc:dd:ee:02", "aa:bb:cc:dd:ee:02")),
            choices,
        )
        assertEquals(choices[1], defaultContinueChoice(choices, "aa:bb:cc:dd:ee:01", phone))
        assertEquals(choices[0], defaultContinueChoice(choices, phone, phone))
        assertEquals(choices[0], defaultContinueChoice(choices, null, phone))
        // A player not listed for this account is never the default.
        assertEquals(choices[0], defaultContinueChoice(choices, "aa:bb:cc:dd:ee:99", phone))
        val noPhone = continueChoices(players, phone, "This phone", phoneAvailable = false)
        assertTrue(noPhone.none { it.id == null })
        assertEquals(noPhone[0], defaultContinueChoice(noPhone, null, phone))
        assertNull(defaultContinueChoice(emptyList(), null, phone))
    }

    @Test
    fun offersTheDefaultAndContinuesOnTheChosenPlayer() {
        val picked = mutableListOf<ContinueChoice?>()
        var notNow = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                ContinueOnPrompt(
                    state = ContinueOnState(tracks = 12, playing = true),
                    choices = choices,
                    default = choices[1],
                    onContinue = { picked += it },
                    onNotNow = { notNow++ },
                    onDismissResult = {},
                )
            }
        }
        compose.onNodeWithText("Back online. Continue on Kitchen?").assertIsDisplayed()
        // One line since Wave 3.5: ▾ says what would move and lists the players.
        compose.onNodeWithTag("continue-on-choose").performClick()
        compose.onNodeWithText("12 tracks from your downloads, playing now").assertIsDisplayed()
        compose.onNodeWithTag("continue-on-choice-aa:bb:cc:dd:ee:01").assertIsSelected()
        compose.onNodeWithTag("continue-on-choice-aa:bb:cc:dd:ee:01").performClick()
        compose.onNodeWithText("Continue").performClick()
        compose.onNodeWithTag("continue-on-choose").performClick()
        compose.onNodeWithTag("continue-on-choice-phone").performClick()
        compose.onNodeWithText("Back online. Continue on This phone?").assertIsDisplayed()
        compose.onNodeWithText("Continue").performClick()
        compose.onNodeWithContentDescription("Not now").performClick()
        assertEquals(listOf(choices[1], choices[0]), picked)
        assertEquals(1, notNow)
    }

    @Test
    fun nothingIsShownWithoutAQueue() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                ContinueOnPrompt(ContinueOnState(), choices, choices[0], {}, {}, {})
            }
        }
        compose.onNodeWithTag("continue-on").assertDoesNotExist()
        compose.onNodeWithTag("continue-on-result").assertDoesNotExist()
    }

    @Test
    fun whileBusyNothingCanBeChangedAndTheResultIsShown() {
        var state by mutableStateOf(ContinueOnState(tracks = 3, busy = "Kitchen"))
        var dismissed = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                ContinueOnPrompt(state, choices, choices[1], {}, {}, { dismissed++ })
            }
        }
        compose.onNodeWithText("Moving your downloads queue to Kitchen…").assertIsDisplayed()
        compose.onNodeWithText("Continue").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Not now").assertIsNotEnabled()

        // It failed: the prompt stays, with why, and the downloads carry on.
        state = ContinueOnState(tracks = 3, result = HandOffResult.Failed(AppError.Offline("connection lost")), resultTarget = "Kitchen")
        compose.onNodeWithText("Couldn’t continue on Kitchen", substring = true).assertIsDisplayed()
        compose.onNodeWithText("Continue").assertExists()

        // It worked, with tracks skipped.
        state = ContinueOnState(result = HandOffResult.Moved(10, 2), resultTarget = "Kitchen")
        compose.onNodeWithText("Now playing on Kitchen. 2 tracks were skipped: they aren’t in the library any more.").assertIsDisplayed()
        compose.onNodeWithText("Dismiss").performClick()
        assertEquals(1, dismissed)
    }
}
