package app.diarch.android.ui.update

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.diarch.android.data.AppUpdate
import app.diarch.android.data.UpdateChecker
import kotlinx.coroutines.launch
import java.io.File

private sealed interface UpdateUiState {
    data object Idle : UpdateUiState
    data object Checking : UpdateUiState
    data class Available(val update: AppUpdate) : UpdateUiState
    data class Downloading(val update: AppUpdate, val progress: Float) : UpdateUiState
    data class Ready(val update: AppUpdate, val apk: File) : UpdateUiState
    data class Error(val message: String) : UpdateUiState
    data object UpToDate : UpdateUiState
}

/**
 * Silent startup check against Forgejo releases (skips tags the user dismissed).
 * Manual checks go through [UpdateCheckTrigger.requestCheck].
 */
@Composable
fun UpdatePromptHost(
    checker: UpdateChecker,
    checkOnStart: Boolean = true,
) {
    val scope = rememberCoroutineScope()
    var state by remember { mutableStateOf<UpdateUiState>(UpdateUiState.Idle) }
    var pendingInstall by remember { mutableStateOf<File?>(null) }

    val installPermissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.StartActivityForResult(),
    ) {
        val apk = pendingInstall
        pendingInstall = null
        if (apk != null && checker.canInstallPackages()) {
            runCatching { checker.installApk(apk) }
                .onFailure { state = UpdateUiState.Error(it.message ?: "Install failed") }
        }
    }

    fun launchInstall(apk: File) {
        if (!checker.canInstallPackages()) {
            pendingInstall = apk
            installPermissionLauncher.launch(checker.installPermissionSettingsIntent())
            return
        }
        runCatching { checker.installApk(apk) }
            .onFailure { state = UpdateUiState.Error(it.message ?: "Install failed") }
    }

    fun startDownload(update: AppUpdate) {
        state = UpdateUiState.Downloading(update, 0f)
        scope.launch {
            try {
                val apk = checker.downloadApk(update) { downloaded, total ->
                    val p = if (total > 0) downloaded.toFloat() / total.toFloat() else 0f
                    state = UpdateUiState.Downloading(update, p.coerceIn(0f, 1f))
                }
                state = UpdateUiState.Ready(update, apk)
            } catch (e: Exception) {
                state = UpdateUiState.Error(e.message ?: "Download failed")
            }
        }
    }

    fun runCheck(manual: Boolean) {
        state = UpdateUiState.Checking
        scope.launch {
            try {
                val update = checker.checkForUpdate(includeDismissed = manual)
                state = when {
                    update != null -> UpdateUiState.Available(update)
                    manual -> UpdateUiState.UpToDate
                    else -> UpdateUiState.Idle
                }
            } catch (e: Exception) {
                state = if (manual) {
                    UpdateUiState.Error(e.message ?: "Update check failed")
                } else {
                    UpdateUiState.Idle
                }
            }
        }
    }

    LaunchedEffect(Unit) {
        if (checkOnStart) runCheck(manual = false)
    }

    DisposableEffect(Unit) {
        UpdateCheckTrigger.bind { runCheck(manual = true) }
        onDispose { UpdateCheckTrigger.clear() }
    }

    when (val s = state) {
        is UpdateUiState.Available -> {
            AlertDialog(
                onDismissRequest = {
                    checker.dismiss(s.update.tagName)
                    state = UpdateUiState.Idle
                },
                title = { Text("Update available") },
                text = {
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(max = 360.dp)
                            .verticalScroll(rememberScrollState()),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Text(
                            "Diarch ${s.update.versionLabel} is available " +
                                "(you have ${checker.localVersionName()}).",
                        )
                        if (s.update.notes.isNotBlank()) {
                            Text(s.update.notes)
                        }
                    }
                },
                confirmButton = {
                    TextButton(onClick = { startDownload(s.update) }) {
                        Text("Download")
                    }
                },
                dismissButton = {
                    TextButton(onClick = {
                        checker.dismiss(s.update.tagName)
                        state = UpdateUiState.Idle
                    }) {
                        Text("Later")
                    }
                },
            )
        }
        is UpdateUiState.Downloading -> {
            AlertDialog(
                onDismissRequest = {},
                title = { Text("Downloading update") },
                text = {
                    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        Text(s.update.apkName)
                        if (s.progress > 0f) {
                            LinearProgressIndicator(
                                progress = { s.progress },
                                modifier = Modifier.fillMaxWidth(),
                            )
                            Text("${(s.progress * 100).toInt()}%")
                        } else {
                            LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                        }
                    }
                },
                confirmButton = {},
            )
        }
        is UpdateUiState.Ready -> {
            AlertDialog(
                onDismissRequest = { state = UpdateUiState.Idle },
                title = { Text("Ready to install") },
                text = {
                    Text(
                        "Diarch ${s.update.versionLabel} downloaded. " +
                            "Android will ask you to confirm the install.",
                    )
                },
                confirmButton = {
                    TextButton(onClick = { launchInstall(s.apk) }) {
                        Text("Install")
                    }
                },
                dismissButton = {
                    TextButton(onClick = { state = UpdateUiState.Idle }) {
                        Text("Cancel")
                    }
                },
            )
        }
        is UpdateUiState.Error -> {
            AlertDialog(
                onDismissRequest = { state = UpdateUiState.Idle },
                title = { Text("Update check failed") },
                text = { Text(s.message) },
                confirmButton = {
                    TextButton(onClick = { state = UpdateUiState.Idle }) {
                        Text("OK")
                    }
                },
            )
        }
        is UpdateUiState.UpToDate -> {
            AlertDialog(
                onDismissRequest = { state = UpdateUiState.Idle },
                title = { Text("You're up to date") },
                text = {
                    Text("Diarch ${checker.localVersionName()} is the latest release.")
                },
                confirmButton = {
                    TextButton(onClick = { state = UpdateUiState.Idle }) {
                        Text("OK")
                    }
                },
            )
        }
        UpdateUiState.Checking, UpdateUiState.Idle -> Unit
    }
}

/** Lets shelves request a manual update check without prop-drilling. */
object UpdateCheckTrigger {
    @Volatile
    private var runner: (() -> Unit)? = null

    fun bind(block: () -> Unit) {
        runner = block
    }

    fun clear() {
        runner = null
    }

    fun requestCheck() {
        runner?.invoke()
    }
}
