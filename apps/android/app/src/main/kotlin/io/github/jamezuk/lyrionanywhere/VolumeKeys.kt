package io.github.jamezuk.lyrionanywhere

import android.view.KeyEvent

/**
 * The hardware volume keys while the app is in front. With a remote player selected, the
 * signed-in screen sets [handler] to step that player's volume (as other LMS controllers
 * do); otherwise, or when the handler declines (this phone is the player), the keys are
 * left to the system, which sets this phone's own volume.
 */
class VolumeKeys {
    /** Steps the selected player's volume up (true) or down; returns whether it did. */
    @Volatile
    var handler: ((up: Boolean) -> Boolean)? = null

    /** The volume key whose press was used, so its release is used too. */
    private var consumed: Int? = null

    /** A key went down: true if it was used here (the system mustn't change the phone's volume). */
    fun onKeyDown(keyCode: Int): Boolean {
        val up = when (keyCode) {
            KeyEvent.KEYCODE_VOLUME_UP -> true
            KeyEvent.KEYCODE_VOLUME_DOWN -> false
            else -> return false
        }
        val used = handler?.invoke(up) == true
        consumed = if (used) keyCode else null
        return used
    }

    /** A key came up: used here only if its press was. */
    fun onKeyUp(keyCode: Int): Boolean {
        if (keyCode != KeyEvent.KEYCODE_VOLUME_UP && keyCode != KeyEvent.KEYCODE_VOLUME_DOWN) return false
        val used = consumed == keyCode
        if (used) consumed = null
        return used
    }
}
