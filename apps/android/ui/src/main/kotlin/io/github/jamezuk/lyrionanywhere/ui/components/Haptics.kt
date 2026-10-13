package io.github.jamezuk.lyrionanywhere.ui.components

import android.content.Context
import android.provider.Settings
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Vibration
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import io.github.jamezuk.lyrionanywhere.ui.R
import kotlin.math.floor

/*
 * Haptics (Wave 2 §5.4): light ticks on a skip by swipe, every 10 % while scrubbing, volume
 * steps, picking up and dropping in a drag, setting a rating and setting a sleep timer.
 *
 * One switch, on by default. Every tick goes through Compose's haptic feedback, which asks
 * the view for it: Android then also follows the phone's own touch-feedback setting.
 */

/** The app's haptics, behind its switch: [enabled] off means no tick at all. */
class SwitchedHaptics(private val enabled: () -> Boolean, private val inner: HapticFeedback) : HapticFeedback {
    override fun performHapticFeedback(hapticFeedbackType: HapticFeedbackType) {
        if (enabled()) inner.performHapticFeedback(hapticFeedbackType)
    }
}

/** Every tick in [content] follows the app's Haptics switch. */
@Composable
fun ProvideHaptics(enabled: Boolean, content: @Composable () -> Unit) {
    val inner = LocalHapticFeedback.current
    val on by rememberUpdatedState(enabled)
    val switched = remember(inner) { SwitchedHaptics({ on }, inner) }
    CompositionLocalProvider(LocalHapticFeedback provides switched, content = content)
}

/** When a moving value deserves a tick. Pure, tested on the JVM. */
object HapticSteps {
    /** Scrubbing ticks at every tenth of the track. */
    const val SCRUB = 0.1f

    /** The volume ticks every 5 %. */
    const val VOLUME = 5f

    /** Whether going from [from] to [to] crossed a multiple of [step] (either way). */
    fun crossed(from: Float, to: Float, step: Float): Boolean {
        if (!from.isFinite() || !to.isFinite() || step <= 0f || !step.isFinite()) return false
        return floor(from / step) != floor(to / step)
    }

    /** What each kind of moment feels like: all light. */
    val tick = HapticFeedbackType.SegmentTick
    val pickUp = HapticFeedbackType.GestureThresholdActivate
    val drop = HapticFeedbackType.GestureEnd
    val confirm = HapticFeedbackType.ContextClick
}

/** The phone's own touch-feedback setting (on unless switched off there). */
fun systemTouchFeedback(context: Context): Boolean = try {
    Settings.System.getInt(context.contentResolver, Settings.System.HAPTIC_FEEDBACK_ENABLED, 1) != 0
} catch (_: Exception) {
    true
}

/** Settings: the one Haptics switch. Says when the phone's own setting keeps ticks off. */
@Composable
fun HapticsSetting(on: Boolean, onChange: (Boolean) -> Unit, systemOn: Boolean = systemTouchFeedback(LocalContext.current)) {
    Column {
        ListItem(
            leadingContent = { Icon(Icons.Outlined.Vibration, contentDescription = null) },
            headlineContent = { Text(stringResource(R.string.settings_haptics)) },
            supportingContent = {
                Text(
                    stringResource(
                        if (systemOn) R.string.settings_haptics_detail else R.string.settings_haptics_system_off,
                    ),
                )
            },
            trailingContent = { Switch(checked = on, onCheckedChange = null) },
            modifier = Modifier
                .toggleable(value = on, role = Role.Switch, onValueChange = onChange)
                .testTag("haptics-setting"),
        )
    }
}
