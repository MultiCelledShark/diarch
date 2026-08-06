package app.diarch.android.ui.shelves

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Logout
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.DoneAll
import androidx.compose.material.icons.filled.LibraryBooks
import androidx.compose.material.icons.filled.MenuBook
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Schedule
import androidx.compose.material.icons.filled.SystemUpdate
import androidx.compose.material.icons.filled.FavoriteBorder
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.diarch.android.BuildConfig
import app.diarch.android.DiarchApp
import app.diarch.android.data.Shelf
import app.diarch.android.data.Work
import app.diarch.android.ui.update.UpdateCheckTrigger
import coil.compose.AsyncImage
import coil.request.ImageRequest
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ShelvesScreen(
    onOpenWork: (String) -> Unit,
    onImport: () -> Unit,
    onWishlistAdd: () -> Unit,
    onLogout: () -> Unit,
) {
    var shelf by remember { mutableStateOf(Shelf.Reading) }
    var works by remember { mutableStateOf<List<Work>>(emptyList()) }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var menuOpen by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val repo = DiarchApp.instance.repository

    fun refresh() {
        scope.launch {
            loading = true
            error = null
            try {
                val result = repo.listWorks(shelf)
                works = result.works
                if (result.offline) {
                    error = "Showing offline copies (server unreachable)"
                }
            } catch (e: Exception) {
                error = e.message ?: "Failed to load"
            } finally {
                loading = false
            }
        }
    }

    LaunchedEffect(shelf) { refresh() }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(shelf.label) },
                actions = {
                    IconButton(onClick = { menuOpen = true }) {
                        Icon(Icons.Default.MoreVert, contentDescription = "Menu")
                    }
                    DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                        DropdownMenuItem(
                            text = { Text("Check for updates") },
                            leadingIcon = { Icon(Icons.Default.SystemUpdate, null) },
                            onClick = {
                                menuOpen = false
                                UpdateCheckTrigger.requestCheck()
                            },
                        )
                        DropdownMenuItem(
                            text = { Text("v${BuildConfig.VERSION_NAME}") },
                            enabled = false,
                            onClick = {},
                        )
                        DropdownMenuItem(
                            text = { Text("Log out") },
                            leadingIcon = { Icon(Icons.AutoMirrored.Filled.Logout, null) },
                            onClick = {
                                menuOpen = false
                                scope.launch {
                                    repo.logout()
                                    onLogout()
                                }
                            },
                        )
                    }
                },
            )
        },
        bottomBar = {
            NavigationBar {
                Shelf.entries.forEach { s ->
                    NavigationBarItem(
                        selected = shelf == s,
                        onClick = { shelf = s },
                        icon = {
                            Icon(
                                when (s) {
                                    Shelf.Reading -> Icons.Default.MenuBook
                                    Shelf.ToRead -> Icons.Default.Schedule
                                    Shelf.Library -> Icons.Default.LibraryBooks
                                    Shelf.Wishlist -> Icons.Default.FavoriteBorder
                                    Shelf.Finished -> Icons.Default.DoneAll
                                },
                                contentDescription = s.label,
                            )
                        },
                        label = { Text(s.label.split(' ').first()) },
                    )
                }
            }
        },
        floatingActionButton = {
            FloatingActionButton(
                onClick = {
                    if (shelf == Shelf.Wishlist) onWishlistAdd() else onImport()
                },
            ) {
                Icon(
                    Icons.Default.Add,
                    contentDescription = if (shelf == Shelf.Wishlist) "Add to wishlist" else "Import",
                )
            }
        },
    ) { padding ->
        PullToRefreshBox(
            isRefreshing = loading,
            onRefresh = { refresh() },
            modifier = Modifier
                .fillMaxSize()
                .padding(padding),
        ) {
            when {
                error != null && works.isEmpty() -> {
                    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                        Text(error!!, color = MaterialTheme.colorScheme.error)
                    }
                }
                works.isEmpty() && !loading -> {
                    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                        Text(
                            when (shelf) {
                                Shelf.Reading -> "Nothing in progress"
                                Shelf.ToRead -> "To Read is empty"
                                Shelf.Library -> "Library is empty — import a file"
                                Shelf.Wishlist -> "Wishlist is empty — add a title or scan an ISBN"
                                Shelf.Finished -> "No finished books yet"
                            },
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                else -> {
                    LazyColumn(
                        contentPadding = PaddingValues(16.dp),
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        if (error != null) {
                            item {
                                Text(
                                    error!!,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    style = MaterialTheme.typography.bodyMedium,
                                )
                            }
                        }
                        items(works, key = { it.id }) { work ->
                            WorkRow(work = work, onClick = { onOpenWork(work.id) })
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun WorkRow(work: Work, onClick: () -> Unit) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val offlineCover = DiarchApp.instance.offlineStore.localCoverUri(work.id)
    val coverUrl = DiarchApp.instance.repository.coverUrl(work.id, work.updatedAt)
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        AsyncImage(
            model = ImageRequest.Builder(context)
                .data(offlineCover ?: coverUrl)
                .crossfade(true)
                .build(),
            contentDescription = null,
            contentScale = ContentScale.Crop,
            modifier = Modifier
                .width(56.dp)
                .aspectRatio(2f / 3f),
        )
        Spacer(modifier = Modifier.width(12.dp))
        Column(modifier = Modifier.weight(1f)) {
            Text(
                work.title,
                style = MaterialTheme.typography.titleLarge,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            if (work.authors.isNotBlank()) {
                Text(
                    work.authors,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Spacer(modifier = Modifier.height(4.dp))
            Text(
                buildString {
                    if (work.hasEpub) append("EPUB")
                    if (work.hasMd) {
                        if (isNotEmpty()) append(" · ")
                        append("MD")
                    }
                    if (work.hasAudio) {
                        if (isNotEmpty()) append(" · ")
                        append("Audio")
                    }
                    if (isEmpty()) append("No files yet")
                    if (DiarchApp.instance.offlineStore.isDownloaded(work.id)) {
                        if (isNotEmpty()) append(" · ")
                        append("Offline")
                    }
                },
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
