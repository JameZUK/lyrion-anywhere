package io.github.jamezuk.lyrionanywhere.ui.components

import androidx.annotation.StringRes
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.QueueMusic
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material.icons.outlined.LibraryMusic
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material3.IconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import io.github.jamezuk.lyrionanywhere.ui.R

/**
 * The app's top-level destinations, the same four on Android and the web, in this order:
 * Home, Library, Search and Queue. Now playing is the mini player above them (tap or swipe
 * up to open); Settings is the gear at the top right of Home; Downloads is a Library chip.
 * Phones show them as a bottom bar, larger windows as a rail or drawer (the same four).
 */
enum class MainTab(@StringRes val label: Int, val icon: ImageVector) {
    HOME(R.string.nav_home, Icons.Outlined.Home),
    LIBRARY(R.string.nav_library, Icons.Outlined.LibraryMusic),
    SEARCH(R.string.nav_search, Icons.Outlined.Search),
    QUEUE(R.string.nav_queue, Icons.AutoMirrored.Outlined.QueueMusic),
    ;

    /** Its test tag, `tab-home` and so on. */
    val tag: String get() = "tab-" + name.lowercase()
}

/** Settings: the gear at the top right of Home (its only entry point). */
@Composable
fun HomeSettingsButton(onClick: () -> Unit, modifier: Modifier = Modifier) {
    IconButton(onClick = onClick, modifier = modifier.testTag("home-settings")) {
        Icon(Icons.Outlined.Settings, contentDescription = stringResource(R.string.title_settings))
    }
}

/** The four tabs as a bottom bar (what phones show; also used to check them in tests). */
@Composable
fun MainTabBar(selected: MainTab?, onSelect: (MainTab) -> Unit, modifier: Modifier = Modifier) {
    NavigationBar(modifier) {
        MainTab.entries.forEach { tab ->
            NavigationBarItem(
                selected = tab == selected,
                onClick = { onSelect(tab) },
                icon = { Icon(tab.icon, contentDescription = null) },
                label = { Text(stringResource(tab.label)) },
                modifier = Modifier.testTag(tab.tag),
            )
        }
    }
}
