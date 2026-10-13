package io.github.jamezuk.lyrionanywhere.navigation

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.Settings
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.core.app.ActivityCompat
import androidx.lifecycle.compose.LifecycleResumeEffect
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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import io.github.jamezuk.lyrionanywhere.AppContainer
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.LoadingState
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairScreen
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairViewModel
import io.github.jamezuk.lyrionanywhere.ui.onboarding.RefusedLinkDialog
import io.github.jamezuk.lyrionanywhere.ui.onboarding.ServerScreen
import io.github.jamezuk.lyrionanywhere.ui.onboarding.ServerViewModel
import io.github.jamezuk.lyrionanywhere.ui.onboarding.SignInScreen
import io.github.jamezuk.lyrionanywhere.ui.onboarding.SignInViewModel
import io.github.jamezuk.lyrionanywhere.ui.onboarding.SwitchAccountDialog
import io.github.jamezuk.lyrionanywhere.ui.onboarding.UnreachableScreen
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable

@Serializable
private data object ServerRoute

@Serializable
private data object SignInRoute

@Serializable
private data object PairRoute

/**
 * Chooses between signing in, the app, and offline mode (the server can't be reached: the
 * downloaded library, played on this phone), from the account state and the network.
 */
@Composable
fun AppRoot(container: AppContainer) {
    val state by container.account.state.collectAsStateWithLifecycle()
    val offline by container.offlineMode.offline.collectAsStateWithLifecycle()
    val downloads by container.offline.state.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        val s = state
        val server = when (s) {
            is AuthState.Unreachable -> s.server
            is AuthState.Restoring -> s.server
            is AuthState.SignedInState -> s.server
            else -> null
        }
        // Offline with downloads to show (whose they are is known): offline mode.
        if (offline && server != null && downloads.owner != null) {
            OfflineApp(
                container,
                bannerText = if (s is AuthState.SignedInState) {
                    stringResource(R.string.offline_banner_signed_in)
                } else {
                    stringResource(R.string.offline_banner, server.name)
                },
                onRetry = { scope.launch { container.account.retry() } },
                onSignOut = if (s is AuthState.SignedInState) {
                    null
                } else {
                    { remove ->
                        scope.launch {
                            container.offline.signOut(remove)
                            container.account.signOut()
                        }
                    }
                },
            )
            return@Surface
        }
        when (s) {
            AuthState.Starting -> LoadingState()
            is AuthState.Restoring -> Restoring()
            AuthState.NoServer -> Onboarding(container, signedOut = null)
            is AuthState.SignedOut -> Onboarding(container, signedOut = s)
            is AuthState.Unreachable -> {
                // Signing out goes the way Settings' does: asked first, plays sent and
                // downloads closed (kept unless the user removes them).
                var leaving by remember { mutableStateOf(false) }
                UnreachableScreen(
                    serverName = s.server.name,
                    detail = s.detail,
                    downloadedTracks = downloads.tracks.size,
                    downloadedBytes = downloads.usedBytes,
                    onRetry = { scope.launch { container.account.retry() } },
                    onSignOut = { remove ->
                        if (!leaving) {
                            leaving = true
                            scope.launch {
                                try {
                                    container.offline.signOut(remove)
                                    container.account.signOut()
                                } finally {
                                    leaving = false
                                }
                            }
                        }
                    },
                )
            }
            is AuthState.SignedInState -> MainScreen(container, s)
        }
    }
    PairingLinkPrompts(container, state)
}

/**
 * A pairing link the app was opened with, while a sign-in is kept: ask before signing out
 * (the onboarding screens then show the link and ask again before pairing). A link that
 * isn't valid: say it was ignored.
 */
@Composable
private fun PairingLinkPrompts(container: AppContainer, state: AuthState) {
    val links = container.pairingLinks
    val pending by links.pending.collectAsStateWithLifecycle()
    val refused by links.refused.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    if (refused) RefusedLinkDialog(onDismiss = links::acknowledgeRefused)
    val link = pending ?: return
    val current = when (state) {
        is AuthState.SignedInState -> state.server
        is AuthState.Unreachable -> state.server
        else -> return
    }
    val target = remember(link) {
        link.server?.let { (container.account.checkServerAddress(it) as? ApiResult.Ok)?.value?.name }
    }
    var leaving by remember(link) { mutableStateOf(false) }
    SwitchAccountDialog(
        current = current.name,
        target = target,
        onContinue = {
            if (!leaving) {
                leaving = true
                scope.launch {
                    // As Settings' sign-out: downloads kept, plays sent first.
                    container.offline.signOut(removeDownloads = false)
                    container.account.signOut()
                }
            }
        },
        onCancel = links::dismiss,
    )
}

@Composable
private fun Onboarding(container: AppContainer, signedOut: AuthState.SignedOut?) {
    val nav = rememberNavController()
    val scope = rememberCoroutineScope()
    // A pairing link the app was opened with goes to the pairing screen, to be confirmed.
    val pendingLink by container.pairingLinks.pending.collectAsStateWithLifecycle()
    LaunchedEffect(pendingLink) {
        if (pendingLink != null) nav.navigate(PairRoute) { launchSingleTop = true }
    }
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
                // A QR code names the server too.
                onScan = { nav.navigate(PairRoute) { launchSingleTop = true } },
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
            // Refused with "don't ask again" (or by policy): Android won't show the prompt,
            // so the screen explains and offers the app's settings.
            var cameraBlocked by rememberSaveable { mutableStateOf(false) }
            val askCamera = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
                cameraPermitted = granted
                val activity = context.findActivity()
                cameraBlocked = !granted && activity != null &&
                    !ActivityCompat.shouldShowRequestPermissionRationale(activity, Manifest.permission.CAMERA)
            }
            // Back from the settings: the camera may be allowed now.
            LifecycleResumeEffect(Unit) {
                if (ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
                    cameraPermitted = true
                    cameraBlocked = false
                }
                onPauseOrDispose {}
            }
            // A link waiting: shown here for the user to confirm (taken once).
            LaunchedEffect(pendingLink) {
                container.pairingLinks.take()?.let(vm::offer)
            }
            PairScreen(
                state = ui,
                cameraPermitted = cameraPermitted,
                onRequestCamera = { askCamera.launch(Manifest.permission.CAMERA) },
                onScanned = vm::onScanned,
                onCodeChange = vm::onCodeChange,
                onSubmit = vm::submit,
                onBack = { nav.popBackStack() },
                onConfirm = vm::confirm,
                onCancel = vm::cancel,
                backLabel = stringResource(
                    if (signedOut == null) R.string.action_back else R.string.action_back_to_sign_in,
                ),
                cameraBlocked = cameraBlocked,
                onOpenSettings = {
                    val settings = Intent(
                        Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                        Uri.fromParts("package", context.packageName, null),
                    )
                    // No settings app (a locked-down profile) must not crash the app.
                    runCatching { context.startActivity(settings) }
                },
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

/** The activity behind a Compose context (null if there isn't one). */
private tailrec fun Context.findActivity(): Activity? = when (this) {
    is Activity -> this
    is ContextWrapper -> baseContext.findActivity()
    else -> null
}

