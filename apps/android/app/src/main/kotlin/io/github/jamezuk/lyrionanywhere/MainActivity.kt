package io.github.jamezuk.lyrionanywhere

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import io.github.jamezuk.lyrionanywhere.navigation.AppRoot
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val container = (application as LyrionApp).container
        setContent {
            LyrionTheme {
                AppRoot(container)
            }
        }
    }
}
