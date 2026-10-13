package io.github.jamezuk.lyrionanywhere.ui.docs

import androidx.compose.ui.test.getBoundsInRoot
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.ui.car.CarListsUi
import io.github.jamezuk.lyrionanywhere.ui.car.CarModeScreen
import io.github.jamezuk.lyrionanywhere.ui.carActions
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * Car mode keeps its controls clear of the status bar and the gesture bar: the app draws
 * edge to edge and car mode has the whole screen. Regression: its close and voice buttons
 * sat under the status bar, and its bottom row on the gesture bar (found while taking the
 * documentation screenshots).
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w411dp-h914dp")
class CarModeInsetsTest : DocShots() {
    @Test
    fun carModeStaysClearOfTheSystemBars() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                CarModeScreen(DocStates.player(), CarListsUi(loading = false), clock = { 0L }, actions = carActions())
            }
        }
        compose.waitForIdle()
        applyInsets()
        compose.waitForIdle()
        val exit = compose.onNodeWithTag("car-exit").getBoundsInRoot()
        assertTrue("the close button is under the status bar (top ${exit.top})", exit.top >= STATUS_BAR_DP.dp)
        val screen = compose.onNodeWithTag("car-mode").getBoundsInRoot()
        val bottomRow = compose.onNodeWithTag("car-favourites").getBoundsInRoot()
        assertTrue(
            "the bottom row is on the gesture bar (${bottomRow.bottom} of ${screen.bottom})",
            bottomRow.bottom <= screen.bottom - NAV_BAR_DP.dp,
        )
    }
}
