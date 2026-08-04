package app.diarch.android.ui.detail

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import app.diarch.android.DiarchApp
import app.diarch.android.data.Shelf
import app.diarch.android.data.ShelfFullException
import app.diarch.android.data.Work
import app.diarch.android.data.WorkDetailResponse
import coil.compose.AsyncImage
import coil.request.ImageRequest
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WorkDetailScreen(
    workId: String,
    onBack: () -> Unit,
    onRead: (WorkDetailResponse) -> Unit,
) {
    var detail by remember { mutableStateOf<WorkDetailResponse?>(null) }
    var loading by remember { mutableStateOf(true) }
    var moveMenu by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    val repo = DiarchApp.instance.repository
    val context = LocalContext.current

    fun reload() {
        scope.launch {
            loading = true
            try {
                detail = repo.getWork(workId)
            } catch (e: Exception) {
                snackbar.showSnackbar(e.message ?: "Failed to load")
            } finally {
                loading = false
            }
        }
    }

    LaunchedEffect(workId) { reload() }

    val work = detail?.work
    val hasEpub = detail?.resolvedHasEpub == true
    val hasMd = detail?.resolvedHasMd == true

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(work?.title ?: "Work") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        if (loading && detail == null) {
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(padding),
                verticalArrangement = Arrangement.Center,
            ) {
                CircularProgressIndicator(modifier = Modifier.padding(24.dp))
            }
            return@Scaffold
        }
        if (work == null) {
            Text("Not found", modifier = Modifier.padding(padding).padding(24.dp))
            return@Scaffold
        }

        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
        ) {
            Row(modifier = Modifier.fillMaxWidth()) {
                AsyncImage(
                    model = ImageRequest.Builder(context)
                        .data(repo.coverUrl(work.id, work.updatedAt))
                        .crossfade(true)
                        .build(),
                    contentDescription = null,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier
                        .width(120.dp)
                        .aspectRatio(2f / 3f),
                )
                Spacer(modifier = Modifier.width(16.dp))
                Column(modifier = Modifier.weight(1f)) {
                    Text(work.title, style = MaterialTheme.typography.headlineMedium)
                    if (work.authors.isNotBlank()) {
                        Spacer(modifier = Modifier.height(8.dp))
                        Text(work.authors, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(
                        "Shelf: ${shelfLabel(work.status)}",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    Spacer(modifier = Modifier.height(4.dp))
                    Text(
                        buildString {
                            append(if (hasEpub) "EPUB ready" else "No EPUB")
                            append(" · ")
                            append(if (hasMd) "Markdown ready" else "No Markdown")
                        },
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            Spacer(modifier = Modifier.height(24.dp))

            if (hasEpub || hasMd) {
                Button(
                    onClick = { detail?.let(onRead) },
                    modifier = Modifier.fillMaxWidth(),
                    enabled = !busy,
                ) {
                    Text("Read")
                }
                Spacer(modifier = Modifier.height(8.dp))
            }

            OutlinedButton(
                onClick = { moveMenu = true },
                modifier = Modifier.fillMaxWidth(),
                enabled = !busy,
            ) {
                Text("Move to shelf…")
            }
            DropdownMenu(expanded = moveMenu, onDismissRequest = { moveMenu = false }) {
                Shelf.entries.forEach { shelf ->
                    DropdownMenuItem(
                        text = { Text(shelf.label) },
                        onClick = {
                            moveMenu = false
                            if (work.status == shelf.apiStatus) return@DropdownMenuItem
                            busy = true
                            scope.launch {
                                try {
                                    val updated = repo.setStatus(work.id, shelf.apiStatus)
                                    detail = detail?.copy(work = mergeWork(work, updated))
                                    snackbar.showSnackbar("Moved to ${shelf.label}")
                                } catch (e: ShelfFullException) {
                                    snackbar.showSnackbar(e.message ?: "Shelf full")
                                } catch (e: Exception) {
                                    snackbar.showSnackbar(e.message ?: "Move failed")
                                } finally {
                                    busy = false
                                }
                            }
                        },
                    )
                }
            }

            work.description?.takeIf { it.isNotBlank() }?.let { desc ->
                Spacer(modifier = Modifier.height(24.dp))
                Text("Description", style = MaterialTheme.typography.titleLarge)
                Spacer(modifier = Modifier.height(8.dp))
                Text(desc, style = MaterialTheme.typography.bodyLarge)
            }
        }
    }
}

private fun shelfLabel(status: String): String =
    Shelf.entries.find { it.apiStatus == status }?.label ?: status

private fun mergeWork(listItem: Work, updated: Work): Work =
    updated.copy(
        hasEpub = listItem.hasEpub,
        hasMd = listItem.hasMd,
        hasAudio = listItem.hasAudio,
    )

private val WorkDetailResponse.resolvedHasEpub: Boolean
    get() = hasEpub || assets.any { it.kind == "epub" } || work.hasEpub

private val WorkDetailResponse.resolvedHasMd: Boolean
    get() = hasMd || assets.any { it.kind == "markdown" } || work.hasMd
