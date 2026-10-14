package io.github.jamezuk.lyrionanywhere.ui.offline

import android.content.res.Resources
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.Button
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.data.offline.ContinueOnState
import io.github.jamezuk.lyrionanywhere.data.offline.HandOffResult
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.player.errorWithDetail

/** A player the downloads queue can go to; [id] null is this phone (started if need be). */
data class ContinueChoice(val id: String?, val name: String)

/** The key a choice is remembered by. */
private val ContinueChoice.tag: String get() = id ?: PHONE

private const val PHONE = "phone"

/**
 * Where the queue can go: this phone (when it can be a player), then the other players this
 * account may control, as "Play on…" lists them (the gateway lists only those).
 */
fun continueChoices(players: List<Player>, phoneId: String?, phoneName: String, phoneAvailable: Boolean): List<ContinueChoice> =
    buildList {
        if (phoneAvailable) add(ContinueChoice(null, phoneName))
        players.filter { it.id != phoneId }.forEach { add(ContinueChoice(it.id, it.name.ifBlank { it.id })) }
    }

/** The player offered first: this phone when it was the one in use, else the last one used, else this phone, else the first. */
fun defaultContinueChoice(choices: List<ContinueChoice>, lastUsed: String?, phoneId: String?): ContinueChoice? =
    choices.firstOrNull { it.id == null && lastUsed != null && lastUsed == phoneId }
        ?: choices.firstOrNull { it.id != null && it.id == lastUsed }
        ?: choices.firstOrNull { it.id == null }
        ?: choices.firstOrNull()

/** What to say about a finished hand-off (null: nothing). */
fun continueOnResultText(res: Resources, result: HandOffResult?, target: String): String? = when (result) {
    null -> null
    is HandOffResult.Moved -> res.getString(R.string.continue_on_done, target) +
        if (result.skipped > 0) " " + res.getQuantityString(R.plurals.continue_on_skipped, result.skipped, result.skipped) else ""
    HandOffResult.Empty -> res.getString(R.string.continue_on_empty)
    HandOffResult.NoneResolved -> res.getString(R.string.continue_on_none, target)
    HandOffResult.Changed -> res.getString(R.string.continue_on_changed, target)
    is HandOffResult.Failed -> res.getString(R.string.continue_on_failed, target) + "\n" + errorWithDetail(res, result.error)
}

/**
 * "Back online. Continue on <player>?" (SPEC §8.4): non-blocking, above the current screen.
 * Never acts by itself; the choice of player is the user's. Shows the last result too.
 */
@Composable
fun ContinueOnPrompt(
    state: ContinueOnState,
    choices: List<ContinueChoice>,
    default: ContinueChoice?,
    onContinue: (ContinueChoice) -> Unit,
    onNotNow: () -> Unit,
    onDismissResult: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colours = MaterialTheme.colorScheme
    val res = LocalResources.current
    val result = continueOnResultText(res, state.result, state.resultTarget)
    if (result != null && !state.offered) {
        ResultLine(result, state.result is HandOffResult.Moved, onDismissResult, modifier)
        return
    }
    val tracks = state.tracks ?: return
    if (choices.isEmpty()) return
    var picked by rememberSaveable { mutableStateOf<String?>(null) }
    val chosen = choices.firstOrNull { it.tag == picked } ?: default ?: choices.first()
    val busy = state.busy
    Column(
        modifier
            .fillMaxWidth()
            .background(colours.secondaryContainer)
            .padding(horizontal = 16.dp, vertical = 8.dp)
            .testTag("continue-on")
            .semantics { liveRegion = LiveRegionMode.Polite },
    ) {
        Text(
            stringResource(R.string.continue_on_title, chosen.name),
            style = MaterialTheme.typography.titleSmall,
            color = colours.onSecondaryContainer,
            modifier = Modifier.semantics { heading() },
        )
        val detail = pluralStringResource(R.plurals.continue_on_detail, tracks, tracks)
        Text(
            if (state.playing) stringResource(R.string.continue_on_playing, detail) else detail,
            style = MaterialTheme.typography.bodySmall,
            color = colours.onSecondaryContainer,
        )
        if (choices.size > 1) {
            Text(
                stringResource(R.string.continue_on_choose),
                style = MaterialTheme.typography.labelMedium,
                color = colours.onSecondaryContainer,
                modifier = Modifier.padding(top = 4.dp),
            )
            Column(Modifier.selectableGroup()) {
                for (c in choices) {
                    val selected = c == chosen
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier
                            .fillMaxWidth()
                            .selectable(selected = selected, enabled = busy == null, role = Role.RadioButton) { picked = c.tag }
                            .testTag("continue-on-choice-${c.tag}"),
                    ) {
                        RadioButton(selected = selected, onClick = null, enabled = busy == null)
                        Text(c.name, color = colours.onSecondaryContainer, modifier = Modifier.padding(start = 8.dp))
                    }
                }
            }
        }
        if (busy != null) {
            Text(stringResource(R.string.continue_on_busy, busy), color = colours.onSecondaryContainer)
            LinearProgressIndicator(Modifier.fillMaxWidth().padding(vertical = 4.dp))
        }
        result?.let { Text(it, color = colours.onSecondaryContainer, modifier = Modifier.padding(vertical = 4.dp)) }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Button(onClick = { onContinue(chosen) }, enabled = busy == null) { Text(stringResource(R.string.continue_on_go)) }
            TextButton(onClick = onNotNow, enabled = busy == null) { Text(stringResource(R.string.continue_on_not_now)) }
        }
    }
}

@Composable
private fun ResultLine(text: String, ok: Boolean, onDismiss: () -> Unit, modifier: Modifier) {
    val colours = MaterialTheme.colorScheme
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = modifier
            .fillMaxWidth()
            .background(if (ok) colours.secondaryContainer else colours.errorContainer)
            .padding(start = 16.dp, end = 4.dp)
            .testTag("continue-on-result")
            .semantics { liveRegion = LiveRegionMode.Polite },
    ) {
        Text(
            text,
            style = MaterialTheme.typography.bodyMedium,
            color = if (ok) colours.onSecondaryContainer else colours.onErrorContainer,
            modifier = Modifier.weight(1f).padding(vertical = 10.dp),
        )
        TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_dismiss)) }
    }
}
