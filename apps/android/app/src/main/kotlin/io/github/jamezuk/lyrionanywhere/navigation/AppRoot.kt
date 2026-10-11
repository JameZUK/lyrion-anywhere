package io.github.jamezuk.lyrionanywhere.navigation

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import io.github.jamezuk.lyrionanywhere.AppContainer
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.LoadingState
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairScreen
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairViewModel
import io.github.jamezuk.lyrionanywhere.ui.onboarding.ServerScreen
import io.github.jamezuk.lyrionanywhere.ui.onboarding.ServerViewModel
import io.github.jamezuk.lyrionanywhere.ui.onboarding.SignInScreen
import io.github.jamezuk.lyrionanywhere.ui.onboarding.SignInViewModel
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable

@Serializable
private data object ServerRoute

@Serializable
private data object SignInRoute

@Serializable
private data object PairRoute

/** Chooses between signing in and the app, from the account state. */
@Composable
fun AppRoot(container: AppContainer) {
    val state by container.account.state.collectAsStateWithLifecycle()
    Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        when (val s = state) {
            AuthState.Starting -> LoadingState()
            is AuthState.Restoring -> Restoring()
            AuthState.NoServer -> Onboarding(container, signedOut = null)
            is AuthState.SignedOut -> Onboarding(container, signedOut = s)
            is AuthState.Unreachable -> Unreachable(container, s)
            is AuthState.SignedInState -> MainScreen(container, s)
        }
    }
}

@Composable
private fun Onboarding(container: AppContainer, signedOut: AuthState.SignedOut?) {
    val nav = rememberNavController()
    val scope = rememberCoroutineScope()
    NavHost(
        navController = nav,
        startDestination = if (signedOut == null) ServerRoute else SignInRoute,
        modifier = Modifier.safeDrawingPadding(),
    ) {
        composable<ServerRoute> {
            val vm = viewModel { ServerViewModel(container.account) }
            val ui by vm.state.collectAsStateWithLifecycle()
            ServerScreen(
                state = ui,
                onAddressChange = vm::onAddressChange,
                // Once the server is saved, the account state moves on to signing in.
                onSubmit = { vm.submit {} },
            )
        }
        composable<SignInRoute> {
            val vm = viewModel { SignInViewModel(container.account) }
            val ui by vm.state.collectAsStateWithLifecycle()
            SignInScreen(
                serverName = signedOut?.server?.name ?: "",
                state = ui,
                onUsernameChange = vm::onUsernameChange,
                onPasswordChange = vm::onPasswordChange,
                onCodeChange = vm::onCodeChange,
                onSubmit = vm::submit,
                onPair = { nav.navigate(PairRoute) },
                onChangeServer = {
                    scope.launch {
                        container.account.forgetServer()
                    }
                },
                revoked = signedOut?.revoked == true,
            )
        }
        composable<PairRoute> {
            val vm = viewModel { PairViewModel(container.account) }
            val ui by vm.state.collectAsStateWithLifecycle()
            val context = LocalContext.current
            var cameraPermitted by remember {
                mutableStateOf(
                    ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                        PackageManager.PERMISSION_GRANTED,
                )
            }
            val askCamera = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
                cameraPermitted = it
            }
            PairScreen(
                state = ui,
                cameraPermitted = cameraPermitted,
                onRequestCamera = { askCamera.launch(Manifest.permission.CAMERA) },
                onScanned = vm::onScanned,
                onCodeChange = vm::onCodeChange,
                onSubmit = vm::submit,
                onBack = { nav.popBackStack() },
            )
        }
    }
}

@Composable
private fun Restoring() {
    Column(
        Modifier.fillMaxSize(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        CircularProgressIndicator()
        Text(stringResource(R.string.restoring), Modifier.padding(top = 16.dp))
    }
}

@Composable
private fun Unreachable(container: AppContainer, state: AuthState.Unreachable) {
    val scope = rememberCoroutineScope()
    Box(Modifier.fillMaxSize().safeDrawingPadding().padding(24.dp), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(stringResource(R.string.unreachable_title), style = MaterialTheme.typography.headlineSmall)
            Text(
                stringResource(R.string.unreachable_body, state.server.name),
                textAlign = TextAlign.Center,
            )
            Button(onClick = { scope.launch { container.account.retry() } }) {
                Text(stringResource(R.string.action_retry))
            }
            TextButton(onClick = { scope.launch { container.account.signOut() } }) {
                Text(stringResource(R.string.action_sign_out))
            }
        }
    }
}
