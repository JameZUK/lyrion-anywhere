package io.github.jamezuk.lyrionanywhere.ui.onboarding

import io.github.jamezuk.lyrionanywhere.core.PairingInvite
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.getAndUpdate

/**
 * A pairing link the app was opened with (`lyrion-anywhere://pair?server=…&code=…`, e.g.
 * tapped in a message), held until the user decides.
 *
 * Holding it is all that happens: a link can only fill in the pairing screen. It is read by
 * the same strict check as a scanned QR code ([read], the Rust `parse_pairing_invite`), and
 * nothing signs in, signs out or changes server until the user confirms there. Anything
 * that isn't a valid pairing link is dropped (and the user told), never half-used.
 */
class PairingLinks(private val read: (String) -> ApiResult<PairingInvite>) {
    private val _pending = MutableStateFlow<PairingInvite?>(null)

    /** The link waiting for the user, if any. */
    val pending: StateFlow<PairingInvite?> = _pending.asStateFlow()

    private val _refused = MutableStateFlow(false)

    /** The last link offered wasn't a valid pairing link (until [acknowledgeRefused]). */
    val refused: StateFlow<Boolean> = _refused.asStateFlow()

    /**
     * Offer the link the app was opened with. Only `lyrion-anywhere://pair` links count: a
     * bare code, another scheme or host, or anything the strict check refuses is dropped.
     * A newer valid link replaces one still waiting. Returns whether it was kept.
     */
    fun offer(link: String?): Boolean {
        val text = link?.trim()
        if (text.isNullOrEmpty() || text.length > MAX_LINK || !text.startsWith(PREFIX, ignoreCase = true)) {
            return refuse()
        }
        return when (val r = read(text)) {
            is ApiResult.Ok -> {
                _pending.value = r.value
                _refused.value = false
                true
            }
            is ApiResult.Err -> refuse()
        }
    }

    /** Take the waiting link (once) to show it on the pairing screen. */
    fun take(): PairingInvite? = _pending.getAndUpdate { null }

    /** The user declined the link. */
    fun dismiss() {
        _pending.value = null
    }

    fun acknowledgeRefused() {
        _refused.value = false
    }

    private fun refuse(): Boolean {
        _refused.value = true
        return false
    }

    companion object {
        /** The only links accepted (docs/PROTOCOL.md, "Pairing links"). */
        const val PREFIX = "lyrion-anywhere://pair"

        /** Longest link considered, as in the Rust check. */
        const val MAX_LINK = 2048
    }
}
