package io.github.jamezuk.lyrionanywhere.navigation

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.toRoute
import io.github.jamezuk.lyrionanywhere.data.offline.ConnectionUi
import io.github.jamezuk.lyrionanywhere.ui.components.ConnectionFrame
import io.github.jamezuk.lyrionanywhere.ui.components.connectionMessage
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.serialization.Serializable

/*
 * A stand-in for AppRoot and MainScreen (which need the whole app, Rust included): the same
 * switch between "signing in", offline mode and the signed-in app, with the signed-in app
 * kept in [AppPlaces] exactly as AppRoot keeps MainScreen, and the same connection line.
 */

@Serializable data object TestHome

@Serializable data class TestAlbum(val id: Long)

@Serializable data object TestDownloads

enum class Phase { SIGNING_IN, MAIN, OFFLINE }

/** What the host shows; held outside the composition, as the app's flows are. */
class HostState {
    var phase by mutableStateOf(Phase.MAIN)
    var connection by mutableStateOf(ConnectionUi())
    var account by mutableStateOf("main|https://music.example.com/|1")
}

const val ALBUM_LIST = "album-list"
const val TRACKS = 200

@Composable
fun PlacesHost(h: HostState) {
    val places = rememberAppPlaces()
    when (h.phase) {
        Phase.SIGNING_IN -> Text("Signing in…")
        Phase.OFFLINE -> Text("Downloaded")
        Phase.MAIN -> places.Keep(h.account) { TestMain(h.connection) }
    }
}

@Composable
private fun TestMain(connection: ConnectionUi) {
    val nav = rememberNavController()
    ConnectionFrame(
        connectionMessage(connection, playingHere = false),
        onDownloads = { nav.navigate(TestDownloads) { launchSingleTop = true } },
        Modifier.fillMaxSize(),
    ) {
        NavHost(nav, startDestination = TestHome) {
            composable<TestHome> {
                Column {
                    Text("Home")
                    Button(onClick = { nav.navigate(TestAlbum(7)) }) { Text("Open album") }
                }
            }
            composable<TestAlbum> { entry ->
                val id = entry.toRoute<TestAlbum>().id
                Column {
                    Text("Album $id")
                    LazyColumn(state = rememberLazyListState(), modifier = Modifier.testTag(ALBUM_LIST)) {
                        items(TRACKS) { Text("Track $it") }
                    }
                }
            }
            composable<TestDownloads> { Text("Downloads screen") }
        }
    }
}

/** An activity showing [PlacesHost], for recreating (configuration change, process death). */
class PlacesActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { LyrionTheme(dynamicColour = false) { PlacesHost(state) } }
    }

    companion object {
        /** Survives the activity, as the app's state does across a recreation. */
        var state = HostState()
    }
}
