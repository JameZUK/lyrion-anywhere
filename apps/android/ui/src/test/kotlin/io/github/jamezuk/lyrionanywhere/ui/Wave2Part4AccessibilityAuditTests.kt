package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.data.AudioOutput
import io.github.jamezuk.lyrionanywhere.data.CarAutoOpen
import io.github.jamezuk.lyrionanywhere.data.CarModeSetting
import io.github.jamezuk.lyrionanywhere.data.NowPlayingPrefs
import io.github.jamezuk.lyrionanywhere.data.OutputClass
import io.github.jamezuk.lyrionanywhere.data.OutputEq
import io.github.jamezuk.lyrionanywhere.data.SoundSettings
import io.github.jamezuk.lyrionanywhere.data.VisualStyle
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerState
import io.github.jamezuk.lyrionanywhere.ui.car.CarListsUi
import io.github.jamezuk.lyrionanywhere.ui.car.CarModeScreen
import io.github.jamezuk.lyrionanywhere.ui.car.CarModeSettingView
import io.github.jamezuk.lyrionanywhere.ui.components.HapticsSetting
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineNowPlayingActions
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineNowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflinePlayerUi
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.DynamicColourSetting
import io.github.jamezuk.lyrionanywhere.ui.settings.EqCompareButton
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundActions
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundUiState
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * Wave 2 part 4, audited as TalkBack sees it ([assertAccessible]): car mode in each view and
 * on a wide window, its setting (with the refusal note and the device picker), the Sound
 * screen with per-output EQ (curve, A/B, bindings), Now playing with the EQ chip and the car
 * button, the offline Now playing with and without its visualiser, and the device settings.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class Wave2Part4AccessibilityAuditTests {
    @get:Rule
    val compose = createComposeRule()

    private fun audit(content: @Composable () -> Unit) {
        compose.setContent { LyrionTheme(dynamicColour = false) { content() } }
        compose.assertAccessible()
    }

    private fun car(lists: CarListsUi = carLists) = audit { CarModeScreen(auditPlayerState(), lists, clock = { 0L }, actions = carActions()) }

    @Test
    fun carModePlaying() = car()

    @Test
    fun carModeFavourites() {
        car()
        compose.onNodeWithTag("car-favourites").performClick()
        compose.assertAccessible()
    }

    @Test
    fun carModeStations() {
        car()
        compose.onNodeWithTag("car-stations").performClick()
        compose.assertAccessible()
    }

    @Test
    fun carModeWithNothingToList() {
        car(CarListsUi(loading = false))
        compose.onNodeWithTag("car-favourites").performClick()
        compose.assertAccessible()
    }

    @Test
    @Config(sdk = [35], qualifiers = "w900dp-h420dp")
    fun carModeOnAWideScreen() = car()

    @Test
    fun carModeSettingOff() = audit { CarModeSettingView(CarModeSetting(), {}, FakeCarAccess()) }

    @Test
    fun carModeSettingWithTheRefusalNote() {
        audit { CarModeSettingView(CarModeSetting(), {}, FakeCarAccess(permitted = false, answer = false)) }
        compose.onNodeWithTag("car-auto-ANY_CAR").performClick()
        compose.onNodeWithTag("car-auto-refused").assertExists()
        compose.assertAccessible()
    }

    @Test
    fun carModeDevicePicker() {
        audit { CarModeSettingView(CarModeSetting(CarAutoOpen.THIS_DEVICE, "Example Car Kit", "0A:1B:2C:3D:4E:5F"), {}, FakeCarAccess(permitted = true)) }
        compose.onNodeWithTag("car-auto-THIS_DEVICE").performClick()
        compose.onNodeWithTag("car-pick-list").assertExists()
        compose.assertAccessible()
    }

    private val noSound = object : SoundActions {}

    @Test
    fun soundWithPerOutputEq() = audit {
        SoundScreen(
            SoundUiState(
                sound = SoundSettings(
                    eqEnabled = true,
                    eq = mainEq,
                    outputEq = listOf(
                        OutputEq(OutputClass.BLUETOOTH, "Example Headphones", mainEq.copy(name = "For headphones")),
                        OutputEq(OutputClass.SPEAKER, "", null),
                    ),
                ),
                output = AudioOutput(OutputClass.BLUETOOTH, "Example Headphones"),
            ),
            noSound,
        )
    }

    @Test
    fun soundWithNothingPlayingHere() = audit {
        SoundScreen(SoundUiState(sound = SoundSettings(eqEnabled = true, eq = mainEq)), noSound)
    }

    @Test
    fun theComparisonBypassed() = audit { EqCompareButton(bypass = true, enabled = true) {} }

    @Test
    fun nowPlayingWithTheEqChipAndTheCarButton() {
        val base = phonePlayingState()
        audit {
            NowPlayingScreen(
                base.copy(local = base.local.copy(signalPath = signalPath(), sound = SoundSettings(eqEnabled = true, eq = mainEq))),
                clock = { 0L },
                actions = nowPlayingActions().copy(onCarMode = {}),
            )
        }
    }

    private fun offline(prefs: NowPlayingPrefs) = OfflinePlayerUi(
        OfflinePlayerState(status = dlStatus(), tracks = listOf(dlTrack(1), dlTrack(2)).associateBy { it.key }),
        at = 0,
        nowPlaying = prefs,
    )

    @Test
    fun offlineNowPlayingWithTheVisualiserOff() = audit {
        OfflineNowPlayingScreen(offline(NowPlayingPrefs()), clock = { 0L }, actions = OfflineNowPlayingActions(), visual = FakeVisualSource())
    }

    /** Each style in turn, in one composition. */
    @Test
    fun offlineNowPlayingWithTheVisualiserInEachStyle() {
        val current = androidx.compose.runtime.mutableStateOf(VisualStyle.BARS)
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                OfflineNowPlayingScreen(
                    offline(NowPlayingPrefs(visualiser = true, visualStyle = current.value)),
                    clock = { 0L },
                    actions = OfflineNowPlayingActions(),
                    visual = FakeVisualSource(),
                )
            }
        }
        for (s in VisualStyle.entries) {
            compose.runOnUiThread { current.value = s }
            compose.assertAccessible()
        }
    }

    @Test
    fun deviceSettings() = audit {
        androidx.compose.foundation.layout.Column {
            HapticsSetting(true, {}, systemOn = true)
            HapticsSetting(false, {}, systemOn = false)
            DynamicColourSetting(true, {}, available = true)
            DynamicColourSetting(true, {}, available = false)
        }
    }
}
