package io.github.jamezuk.lyrionanywhere.ui.docs

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.filled.Battery5Bar
import androidx.compose.material.icons.filled.SignalCellular4Bar
import androidx.compose.material.icons.filled.Wifi
import androidx.compose.material3.BottomSheetDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.MainTab
import io.github.jamezuk.lyrionanywhere.ui.components.MainTabBar
import io.github.jamezuk.lyrionanywhere.ui.player.MiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState

/** The status bar and gesture bar heights the screenshots give the app (as insets). */
const val STATUS_BAR_DP = 28
const val NAV_BAR_DP = 20

/**
 * A phone's screen: the app edge to edge, with a plausible status bar (time, signal, Wi-Fi,
 * battery) and gesture handle drawn over it. The app itself is given matching window insets
 * by the test, so its bars keep clear of these as on a real phone.
 */
@Composable
fun DeviceFrame(darkIcons: Boolean? = null, content: @Composable () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    val dark = darkIcons ?: (scheme.background.luminance() > 0.5f)
    val ink = if (dark) Color(0xFF1B1B1F) else Color(0xFFF2F2F2)
    Box(Modifier.fillMaxSize().background(scheme.background)) {
        // As the app's Scaffold does: the background and its content colour.
        Surface(Modifier.fillMaxSize(), color = scheme.background) { content() }
        Row(
            Modifier.fillMaxWidth().height(STATUS_BAR_DP.dp).padding(horizontal = 20.dp).align(Alignment.TopCenter),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("10:42", color = ink, fontSize = 14.sp, fontWeight = FontWeight.Medium)
            Box(Modifier.weight(1f))
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Filled.Wifi, contentDescription = null, tint = ink, modifier = Modifier.size(16.dp))
                Icon(Icons.Filled.SignalCellular4Bar, contentDescription = null, tint = ink, modifier = Modifier.size(15.dp))
                Icon(Icons.Filled.Battery5Bar, contentDescription = null, tint = ink, modifier = Modifier.size(16.dp))
            }
        }
        Box(
            Modifier.fillMaxWidth().height(NAV_BAR_DP.dp).align(Alignment.BottomCenter),
            contentAlignment = Alignment.Center,
        ) {
            Box(Modifier.width(108.dp).height(4.dp).background(ink.copy(alpha = 0.8f), RoundedCornerShape(2.dp)))
        }
    }
}

/** A screen with its own top bar, as the app's (`MainScreen.kt`, `Screen`). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DocScreen(title: String, back: Boolean = false, actions: @Composable () -> Unit = {}, content: @Composable () -> Unit) {
    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            navigationIcon = {
                if (back) {
                    IconButton(onClick = {}) {
                        Icon(Icons.AutoMirrored.Outlined.ArrowBack, contentDescription = stringResource(R.string.action_back))
                    }
                }
            },
            actions = { actions() },
        )
        content()
    }
}

/**
 * The app's main frame on a phone: the screen, the mini player (unless [mini] is null) and
 * the four tabs, as `MainScreen.kt` lays them out. With neither a tab nor a mini player it is
 * Now playing, which has the whole screen (no tabs, Wave 3.5), clear of the gesture bar.
 */
@Composable
fun PhoneShell(tab: MainTab?, mini: PlayerUiState?, content: @Composable () -> Unit) {
    if (tab == null && mini == null) {
        Box(Modifier.fillMaxSize().navigationBarsPadding()) { content() }
        return
    }
    Scaffold(
        contentWindowInsets = WindowInsets(0),
        bottomBar = {
            Column {
                if (mini != null) MiniPlayer(mini, onOpen = {}, onTogglePlay = {}, onNext = {})
                MainTabBar(tab, onSelect = {})
            }
        },
    ) { padding ->
        Box(Modifier.padding(padding)) { content() }
    }
}

/** The app's main frame on a large screen: a navigation rail beside the screen. */
@Composable
fun TabletShell(tab: MainTab?, content: @Composable () -> Unit) {
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().padding(start = RAIL_DP.dp).navigationBarsPadding()) { content() }
        NavigationRail(Modifier.width(RAIL_DP.dp).fillMaxHeight(), windowInsets = WindowInsets.statusBars) {
            MainTab.entries.forEach { t ->
                NavigationRailItem(
                    selected = t == tab,
                    onClick = {},
                    icon = { Icon(t.icon, contentDescription = null) },
                    label = { Text(stringResource(t.label)) },
                )
            }
        }
    }
}

const val RAIL_DP = 80

/** A modal bottom sheet, open over [behind] (drawn as Material 3 draws one). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SheetOver(behind: @Composable () -> Unit, sheet: @Composable () -> Unit) {
    Box(Modifier.fillMaxSize()) {
        behind()
        Box(Modifier.fillMaxSize().background(BottomSheetDefaults.ScrimColor))
        Surface(
            modifier = Modifier.fillMaxWidth().align(Alignment.BottomCenter).padding(top = 72.dp),
            shape = BottomSheetDefaults.ExpandedShape,
            color = BottomSheetDefaults.ContainerColor,
            tonalElevation = BottomSheetDefaults.Elevation,
        ) {
            Column(Modifier.fillMaxWidth().navigationBarsPadding(), horizontalAlignment = Alignment.CenterHorizontally) {
                BottomSheetDefaults.DragHandle()
                sheet()
            }
        }
    }
}
