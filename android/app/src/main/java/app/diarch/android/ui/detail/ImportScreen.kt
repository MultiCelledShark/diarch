package app.diarch.android.ui.detail

import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.diarch.android.DiarchApp
import kotlinx.coroutines.launch
import retrofit2.HttpException

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ImportScreen(
    onBack: () -> Unit,
    onImported: (String) -> Unit,
) {
    var uri by remember { mutableStateOf<Uri?>(null) }
    var fileName by remember { mutableStateOf<String?>(null) }
    var title by remember { mutableStateOf("") }
    var authors by remember { mutableStateOf("") }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val repo = DiarchApp.instance.repository

    val picker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocument(),
    ) { picked ->
        uri = picked
        fileName = picked?.lastPathSegment
        error = null
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Import") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(16.dp),
            verticalArrangement = Arrangement.Top,
        ) {
            Text(
                "Drop an EPUB, PDF, Markdown, or audiobook (M4B/M4A/AAX). Conversion runs on the server.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(16.dp))
            OutlinedButton(
                onClick = {
                    picker.launch(
                        arrayOf(
                            "application/epub+zip",
                            "application/pdf",
                            "text/markdown",
                            "text/plain",
                            "audio/*",
                            "application/octet-stream",
                            "*/*",
                        ),
                    )
                },
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(if (uri == null) "Choose file" else "Choose another file")
            }
            if (fileName != null) {
                Spacer(Modifier.height(8.dp))
                Text(fileName!!, style = MaterialTheme.typography.bodyLarge)
            }
            Spacer(Modifier.height(16.dp))
            OutlinedTextField(
                value = title,
                onValueChange = { title = it },
                label = { Text("Title (optional)") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(Modifier.height(12.dp))
            OutlinedTextField(
                value = authors,
                onValueChange = { authors = it },
                label = { Text("Authors (optional)") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            if (error != null) {
                Spacer(Modifier.height(12.dp))
                Text(error!!, color = MaterialTheme.colorScheme.error)
            }
            Spacer(Modifier.height(24.dp))
            Button(
                onClick = {
                    val picked = uri ?: return@Button
                    loading = true
                    error = null
                    scope.launch {
                        try {
                            val res = repo.importUri(
                                picked,
                                title.takeIf { it.isNotBlank() },
                                authors.takeIf { it.isNotBlank() },
                            )
                            onImported(res.work.id)
                        } catch (e: HttpException) {
                            error = e.response()?.errorBody()?.string()
                                ?: "Import failed (${e.code()})"
                        } catch (e: Exception) {
                            error = e.message ?: "Import failed"
                        } finally {
                            loading = false
                        }
                    }
                },
                enabled = uri != null && !loading,
                modifier = Modifier.fillMaxWidth(),
            ) {
                if (loading) CircularProgressIndicator(
                    modifier = Modifier.height(20.dp),
                    strokeWidth = 2.dp,
                    color = MaterialTheme.colorScheme.onPrimary,
                ) else Text("Upload")
            }
        }
    }
}
