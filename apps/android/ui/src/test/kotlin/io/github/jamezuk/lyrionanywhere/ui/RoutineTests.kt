package io.github.jamezuk.lyrionanywhere.ui

import android.content.Context
import android.provider.Settings
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertContentDescriptionEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performSemanticsAction
import androidx.test.core.app.ApplicationProvider
import io.github.jamezuk.lyrionanywhere.ui.alarms.dayName
import io.github.jamezuk.lyrionanywhere.ui.alarms.timeText
import io.github.jamezuk.lyrionanywhere.ui.alarms.weekOrder
import org.junit.Before
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.AlarmChange
import io.github.jamezuk.lyrionanywhere.core.AlarmInfo
import io.github.jamezuk.lyrionanywhere.core.AlarmListing
import io.github.jamezuk.lyrionanywhere.core.AlarmSound
import io.github.jamezuk.lyrionanywhere.core.RatingStyle
import io.github.jamezuk.lyrionanywhere.core.TrackRating
import io.github.jamezuk.lyrionanywhere.data.AlarmsRepository
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.RatingsRepository
import io.github.jamezuk.lyrionanywhere.ui.alarms.AlarmsScreen
import io.github.jamezuk.lyrionanywhere.ui.alarms.AlarmsViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.RatingControl
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val P = "aa:00:00:00:00:02"

internal fun sampleAlarm(id: String, enabled: Boolean = true) = AlarmInfo(
    id = id,
    timeSecs = 25_200u,
    days = byteArrayOf(1, 2, 3, 4, 5),
    enabled = enabled,
    repeat = true,
    volume = 40u,
    shuffle = 0u,
    soundId = "0123456789abcdef",
    soundTitle = "Morning radio",
)

internal fun alarmListing(alarms: List<AlarmInfo>, enabled: Boolean = true) = AlarmListing(
    enabled = enabled,
    alarms = alarms,
    sounds = listOf(AlarmSound("current", "current", "Current queue"), AlarmSound("0123456789abcdef", "favourites", "Morning radio")),
    newId = null,
)

internal class FakeAlarms : AlarmsRepository {
    var current = alarmListing(listOf(sampleAlarm("1a2b3c4d")))
    val calls = mutableListOf<String>()
    var fail: AppError? = null

    private fun answer(call: String): ApiResult<AlarmListing> {
        calls += call
        return fail?.let { ApiResult.Err(it) } ?: ApiResult.Ok(current)
    }

    override suspend fun alarms(player: String) = answer("list $player")
    override suspend fun add(player: String, change: AlarmChange) =
        answer("add ${change.timeSecs} ${change.days?.toList()} ${change.soundId} ${change.enabled}")
    override suspend fun change(player: String, id: String, change: AlarmChange) = answer("change $id ${change.enabled}")
    override suspend fun delete(player: String, id: String): ApiResult<AlarmListing> {
        current = alarmListing(emptyList())
        return answer("delete $id")
    }
    override suspend fun setAll(player: String, enabled: Boolean): ApiResult<AlarmListing> {
        current = current.copy(enabled = enabled)
        return answer("all $enabled")
    }
}

private class FakeRatings(var rating: TrackRating? = null, var style: RatingStyle = RatingStyle.STARS) : RatingsRepository {
    val rated = mutableListOf<Int>()
    var fail = false
    override suspend fun style() = ApiResult.Ok(style)
    override suspend fun setStyle(style: RatingStyle): ApiResult<Unit> {
        this.style = style
        return ApiResult.Ok(Unit)
    }
    override suspend fun ratings(trackIds: List<Long>) = ApiResult.Ok(listOfNotNull(rating?.takeIf { it.trackId in trackIds }))
    override suspend fun rate(trackId: Long, value: Int): ApiResult<TrackRating?> {
        rated += value
        if (fail) return ApiResult.Err(AppError.Offline())
        val m = value.takeIf { it > 0 }?.toUInt()
        return ApiResult.Ok(TrackRating(trackId, m, rating?.shared, m ?: rating?.shared))
    }
    override suspend fun clear() = ApiResult.Ok(0L)
}

/** Rating control, Alarms screen (Wave 2 §1–2). */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class RoutineScreenTest {
    @get:Rule
    val compose = createComposeRule()

    /** Times follow the device's clock setting: the 24-hour clock unless a test says otherwise. */
    @Before
    fun clock24() = setClock24(true)

    @Test
    fun starsSetAndClearTheUsersRating() {
        val rated = mutableListOf<Int>()
        var rating by mutableStateOf<TrackRating?>(TrackRating(9, null, 60u, 60u))
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                RatingControl(rating, RatingStyle.STARS, failed = false) { v ->
                    rated += v
                    rating = TrackRating(9, v.takeIf { it > 0 }?.toUInt(), 60u, if (v > 0) v.toUInt() else 60u)
                }
            }
        }
        compose.onNodeWithTag("rating-star-4").performClick()
        compose.onNodeWithTag("rating-star-4").performClick() // your own rating: cleared
        assertEquals(listOf(80, 0), rated)
    }

    @Test
    fun aHeartTogglesAndAFailureIsShown() {
        val rated = mutableListOf<Int>()
        var loved by mutableStateOf(false)
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                RatingControl(TrackRating(9, if (loved) 100u else null, null, null), RatingStyle.HEART, failed = true) {
                    rated += it
                    loved = it > 0
                }
            }
        }
        compose.onNodeWithTag("rating-heart").assertIsOff().performClick()
        compose.onNodeWithTag("rating-heart").assertIsOn().performClick()
        assertEquals(listOf(100, 0), rated)
        compose.onNodeWithText("Couldn’t save the rating").assertExists()
    }

    @Test
    fun alarmsListSwitchAndDeleteAfterAsking() {
        val repo = FakeAlarms()
        val vm = AlarmsViewModel(repo, P)
        compose.setContent { LyrionTheme(dynamicColour = false) { AlarmsScreen(vm, "Kitchen") } }
        compose.onNodeWithText("07:00").assertExists()
        compose.onNodeWithText("Weekdays · Morning radio").assertExists()
        compose.onNodeWithTag("alarms-all").assertIsOn().performClick()
        compose.waitForIdle()
        assertEquals("all false", repo.calls.last())
        compose.onNodeWithTag("alarm-switch-1a2b3c4d").performClick()
        compose.waitForIdle()
        assertEquals("change 1a2b3c4d false", repo.calls.last())
        compose.onNodeWithTag("alarm-delete-1a2b3c4d").performClick()
        assertFalse(repo.calls.any { it.startsWith("delete") })
        compose.onNodeWithTag("alarm-delete-confirm").performClick()
        compose.waitForIdle()
        assertEquals("delete 1a2b3c4d", repo.calls.last())
        compose.onNodeWithText("No alarms yet.").assertExists()
    }

    @Test
    fun addingAnAlarmSendsTheChosenValues() {
        val repo = FakeAlarms()
        val vm = AlarmsViewModel(repo, P)
        compose.setContent { LyrionTheme(dynamicColour = false) { AlarmsScreen(vm, "Kitchen") } }
        compose.onNodeWithTag("alarm-add").performClick()
        compose.onNodeWithTag("alarm-day-6").performClick() // add Saturday
        compose.onNodeWithTag("alarm-day-1").performClick() // drop Monday
        compose.onNodeWithTag("alarm-save").performClick()
        compose.waitForIdle()
        assertEquals("add 25200 [2, 3, 4, 5, 6] current true", repo.calls.last())
    }

    @Test
    fun aRefusalIsShownWithRetry() {
        val repo = FakeAlarms().apply { fail = AppError.Forbidden("you may not control this player") }
        val vm = AlarmsViewModel(repo, P)
        compose.setContent { LyrionTheme(dynamicColour = false) { AlarmsScreen(vm, "Kitchen") } }
        compose.onNodeWithText("Try again").assertExists()
        compose.onNodeWithText("07:00").assertDoesNotExist()
    }

    @Test
    fun aFailedChangeIsShownAndTheListKept() {
        val repo = FakeAlarms()
        val vm = AlarmsViewModel(repo, P)
        compose.setContent { LyrionTheme(dynamicColour = false) { AlarmsScreen(vm, "Kitchen") } }
        repo.fail = AppError.Forbidden("you may not control this player")
        compose.onNodeWithTag("alarm-switch-1a2b3c4d").performClick()
        compose.waitForIdle()
        compose.onNodeWithTag("alarm-error").assertIsDisplayed()
        compose.onNodeWithText("Couldn't change the alarms", substring = true).assertExists()
        // The alarm is still listed, as the server still has it, and its switch unchanged.
        compose.onNodeWithTag("alarm-switch-1a2b3c4d").assertIsOn()
        compose.onNodeWithContentDescription("Close").performClick()
        compose.onNodeWithTag("alarm-error").assertDoesNotExist()
        // Adding and deleting say so too.
        compose.onNodeWithTag("alarm-delete-1a2b3c4d").performClick()
        compose.onNodeWithTag("alarm-delete-confirm").performClick()
        compose.waitForIdle()
        compose.onNodeWithTag("alarm-error").assertIsDisplayed()
        compose.onNodeWithText("07:00").assertExists()
    }

    @Test
    fun withoutThe24HourClockTimesHaveAmAndPm() {
        setClock24(false)
        val vm = AlarmsViewModel(FakeAlarms(), P)
        compose.setContent { LyrionTheme(dynamicColour = false) { AlarmsScreen(vm, "Kitchen") } }
        compose.onNodeWithText("7:00 AM", ignoreCase = true).assertExists()
        compose.onNodeWithText("07:00").assertDoesNotExist()
        compose.onNodeWithTag("alarm-1a2b3c4d").performClick()
        compose.onNodeWithTag("alarm-time").assertTextContains("7:00 AM", ignoreCase = true)
    }

    @Test
    fun theEditorKeepsItsChoicesAcrossRotation() {
        val vm = AlarmsViewModel(FakeAlarms(), P)
        val restore = StateRestorationTester(compose)
        restore.setContent { LyrionTheme(dynamicColour = false) { AlarmsScreen(vm, "Kitchen") } }
        compose.onNodeWithTag("alarm-add").performClick()
        compose.onNodeWithTag("alarm-day-6").performClick() // add Saturday
        compose.onNodeWithTag("alarm-day-1").performClick() // drop Monday
        compose.onNodeWithTag("alarm-volume").performSemanticsAction(SemanticsActions.SetProgress) { it(80f) }
        restore.emulateSavedInstanceStateRestore()
        compose.onNodeWithTag("alarm-day-6").assertIsSelected()
        compose.onNodeWithTag("alarm-day-1").assertIsNotSelected()
        compose.onNodeWithText("Volume 80").assertExists()
    }

    @Test
    fun theAlarmVolumeIsLabelled() {
        val vm = AlarmsViewModel(FakeAlarms(), P)
        compose.setContent { LyrionTheme(dynamicColour = false) { AlarmsScreen(vm, "Kitchen") } }
        compose.onNodeWithTag("alarm-1a2b3c4d").performClick()
        compose.onNodeWithTag("alarm-volume")
            .assertContentDescriptionEquals("Alarm volume")
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "40 per cent"))
    }

    @Test
    fun dayNamesAndOrderComeFromTheLanguage() {
        // English: Monday first, short names.
        assertEquals(listOf(1, 2, 3, 4, 5, 6, 0), weekOrder(java.util.Locale.UK))
        assertEquals("Mon", dayName(1, java.util.Locale.UK))
        // US weeks start on Sunday.
        assertEquals(0, weekOrder(java.util.Locale.US).first())
        // French names.
        assertEquals("lun.", dayName(1, java.util.Locale.FRANCE))
        assertEquals("dim.", dayName(0, java.util.Locale.FRANCE))
        assertEquals("07:00", timeText(25_200u, true, java.util.Locale.UK))
        assertEquals("19:30", timeText(70_200u, true, java.util.Locale.UK))
        assertEquals("7:30 PM", timeText(70_200u, false, java.util.Locale.US))
    }
}

/** Set the device's 12/24-hour clock (as Settings › Date & time does). */
internal fun setClock24(on: Boolean) {
    val context = ApplicationProvider.getApplicationContext<Context>()
    Settings.System.putString(context.contentResolver, Settings.System.TIME_12_24, if (on) "24" else "12")
}

/** The Alarms screen in another language: its day names come from the language. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "fr-rFR-w420dp-h1800dp")
class AlarmsInFrenchTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun dayNamesAreFrench() {
        setClock24(true)
        val repo = FakeAlarms().apply {
            current = alarmListing(listOf(sampleAlarm("1a2b3c4d").copy(days = byteArrayOf(1, 3))))
        }
        val vm = AlarmsViewModel(repo, P)
        compose.setContent { LyrionTheme(dynamicColour = false) { AlarmsScreen(vm, "Kitchen") } }
        compose.onNodeWithText("lun. mer.", substring = true).assertExists()
        compose.onNodeWithTag("alarm-add").performClick()
        compose.onNodeWithText("sam.").assertExists()
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class RatingsViewModelTest : MainDispatcherTest() {
    @Test
    fun theTracksRatingIsShownSetAndPutBackOnFailure() = runTest(dispatcher) {
        val players = FakePlayers()
        val repo = FakeRatings(TrackRating(1, null, 60u, 60u), RatingStyle.HEART)
        val vm = PlayerViewModel(players, FakeEvents(), FakeSettings(), ratings = repo, clock = { 0L })
        vm.start()
        assertEquals(RatingStyle.HEART, vm.state.value.ratingStyle)
        assertEquals(60u, vm.state.value.rating?.shared)
        vm.rate(84)
        assertEquals(listOf(80), repo.rated)
        assertEquals(80u, vm.state.value.rating?.mine)
        // Refused: put back, and said.
        repo.fail = true
        vm.rate(20)
        assertEquals(80u, vm.state.value.rating?.mine)
        assertTrue(vm.state.value.ratingFailed)
        // Another track: its own rating (none).
        players.statusResult = ApiResult.Ok(sampleStatus().copy(current = sampleTrack(2, 1)))
        vm.refresh()
        assertNull(vm.state.value.rating)
        assertFalse(vm.state.value.ratingFailed)
        vm.setRatingStyle(RatingStyle.STARS)
        assertEquals(RatingStyle.STARS, repo.style)
        vm.stop()
    }
}
