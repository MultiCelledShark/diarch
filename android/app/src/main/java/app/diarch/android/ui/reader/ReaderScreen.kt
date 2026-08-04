package app.diarch.android.ui.reader

import android.annotation.SuppressLint
import android.view.ViewGroup
import android.webkit.JavascriptInterface
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.NavigateBefore
import androidx.compose.material.icons.automirrored.filled.NavigateNext
import androidx.compose.material.icons.filled.FormatSize
import androidx.compose.material.icons.filled.List
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.SwapVert
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import app.diarch.android.DiarchApp
import app.diarch.android.data.ReaderTypography
import app.diarch.android.data.UpdateSettingsRequest
import app.diarch.android.data.WorkDetailResponse
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.json.JSONObject

data class TocItem(val label: String, val href: String)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ReaderScreen(
    detail: WorkDetailResponse,
    onClose: () -> Unit,
) {
    val work = detail.work
    val hasEpub = detail.assets.any { it.kind == "epub" } || detail.hasEpub || work.hasEpub
    val hasMd = detail.assets.any { it.kind == "markdown" } || detail.hasMd || work.hasMd
    val repo = DiarchApp.instance.repository
    val scope = rememberCoroutineScope()
    val json = remember { Json { ignoreUnknownKeys = true } }

    var webView by remember { mutableStateOf<WebView?>(null) }
    var bridgeReady by remember { mutableStateOf(false) }
    var infiniteScroll by remember { mutableStateOf(false) }
    var typography by remember { mutableStateOf(ReaderTypography(justify = false)) }
    var progressText by remember { mutableStateOf("") }
    var toc by remember { mutableStateOf<List<TocItem>>(emptyList()) }
    var showToc by remember { mutableStateOf(false) }
    var showTypo by remember { mutableStateOf(false) }
    var menuOpen by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var opened by remember { mutableStateOf(false) }

    fun eval(script: String) {
        webView?.evaluateJavascript(script, null)
    }

    fun typoJson(t: ReaderTypography): String =
        JSONObject()
            .put("palette", t.palette)
            .put("font", t.font)
            .put("size", t.size)
            .put("lineHeight", t.lineHeight)
            .put("measure", t.measure)
            .put("justify", t.justify)
            .put("letterSpacing", t.letterSpacing)
            .put("paragraphSpacing", t.paragraphSpacing)
            .put("indent", t.indent)
            .put("hyphenate", t.hyphenate)
            .toString()

    fun openMode(useMd: Boolean) {
        if (useMd && !hasMd) return
        if (!useMd && !hasEpub) return
        infiniteScroll = useMd
        scope.launch {
            val mode = if (useMd) "markdown" else "epub"
            val progress = withContext(Dispatchers.IO) { repo.getProgress(work.id, mode) }
            val progressJson = if (progress != null) {
                JSONObject()
                    .put("position", progress.position)
                    .put("percent", progress.percent)
                    .toString()
            } else {
                "null"
            }
            val forceJustify = if (useMd) "true" else "false"
            eval(
                """
                DiarchReader.open({
                  mode: '$mode',
                  forceJustify: $forceJustify,
                  progress: $progressJson
                });
                """.trimIndent(),
            )
            runCatching {
                repo.putSettings(UpdateSettingsRequest(readerInfiniteScroll = useMd))
            }
        }
    }

    LaunchedEffect(Unit) {
        try {
            val settings = repo.getSettings()
            typography = settings.readerTypography
            val preferScroll = settings.readerInfiniteScroll && hasMd
            infiniteScroll = preferScroll || (!hasEpub && hasMd)
        } catch (_: Exception) {
            infiniteScroll = !hasEpub && hasMd
        }
    }

    LaunchedEffect(bridgeReady, webView) {
        if (!bridgeReady || webView == null || opened) return@LaunchedEffect
        opened = true
        val token = repo.authHeader()?.removePrefix("Bearer ") ?: ""
        val base = repo.baseUrl()
        val rtl = work.readingDirection == "rtl" || work.isManga
        eval(
            """
            DiarchReader.init({
              workId: ${JSONObject.quote(work.id)},
              baseUrl: ${JSONObject.quote(base)},
              token: ${JSONObject.quote(token)},
              rtl: $rtl,
              typography: ${typoJson(typography)}
            });
            """.trimIndent(),
        )
        openMode(infiniteScroll)
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(work.title, maxLines = 1)
                        if (progressText.isNotBlank()) {
                            Text(
                                progressText,
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                },
                navigationIcon = {
                    IconButton(onClick = onClose) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Close")
                    }
                },
                actions = {
                    if (!infiniteScroll && hasEpub) {
                        IconButton(onClick = { eval("DiarchReader.prev();") }) {
                            Icon(Icons.AutoMirrored.Filled.NavigateBefore, contentDescription = "Previous")
                        }
                        IconButton(onClick = { eval("DiarchReader.next();") }) {
                            Icon(Icons.AutoMirrored.Filled.NavigateNext, contentDescription = "Next")
                        }
                    }
                    IconButton(onClick = { menuOpen = true }) {
                        Icon(Icons.Default.MoreVert, contentDescription = "Menu")
                    }
                    DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                        if (hasMd && hasEpub) {
                            DropdownMenuItem(
                                text = {
                                    Text(
                                        if (infiniteScroll) "Paginated EPUB"
                                        else "Infinite scroll (markdown)",
                                    )
                                },
                                leadingIcon = { Icon(Icons.Default.SwapVert, null) },
                                onClick = {
                                    menuOpen = false
                                    openMode(!infiniteScroll)
                                },
                            )
                        } else if (hasMd && !hasEpub) {
                            DropdownMenuItem(
                                text = { Text("Infinite scroll") },
                                onClick = {
                                    menuOpen = false
                                    openMode(true)
                                },
                            )
                        }
                        DropdownMenuItem(
                            text = { Text("Contents") },
                            leadingIcon = { Icon(Icons.Default.List, null) },
                            onClick = {
                                menuOpen = false
                                showToc = true
                            },
                        )
                        DropdownMenuItem(
                            text = { Text("Typography") },
                            leadingIcon = { Icon(Icons.Default.FormatSize, null) },
                            onClick = {
                                menuOpen = false
                                showTypo = true
                            },
                        )
                    }
                },
            )
        },
    ) { padding ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding),
        ) {
            ReaderWebView(
                onCreated = { webView = it },
                onMessage = { raw ->
                    try {
                        val obj = json.parseToJsonElement(raw).jsonObject
                        when (obj["type"]?.jsonPrimitive?.contentOrNull) {
                            "bridgeReady" -> bridgeReady = true
                            "ready" -> error = null
                            "error" -> error = obj["message"]?.jsonPrimitive?.contentOrNull
                            "toc" -> {
                                val items = obj["items"]?.jsonArray.orEmpty().mapNotNull { el ->
                                    val o = el.jsonObject
                                    val label = o["label"]?.jsonPrimitive?.contentOrNull ?: return@mapNotNull null
                                    val href = o["href"]?.jsonPrimitive?.contentOrNull ?: return@mapNotNull null
                                    TocItem(label, href)
                                }
                                toc = items
                            }
                            "progress" -> {
                                val mode = obj["mode"]?.jsonPrimitive?.contentOrNull ?: return@ReaderWebView
                                val position = obj["position"]?.jsonPrimitive?.contentOrNull ?: ""
                                val percent = obj["percent"]?.jsonPrimitive?.doubleOrNull ?: 0.0
                                progressText = "${percent.toInt()}% read"
                                scope.launch {
                                    repo.putProgress(work.id, mode, position, percent)
                                }
                            }
                        }
                    } catch (_: Exception) {
                    }
                },
            )
            if (error != null) {
                Text(
                    error!!,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier
                        .align(Alignment.BottomCenter)
                        .padding(16.dp),
                )
            }
        }
    }

    if (showToc) {
        ModalBottomSheet(
            onDismissRequest = { showToc = false },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(16.dp)
                    .verticalScroll(rememberScrollState()),
            ) {
                Text("Contents", style = MaterialTheme.typography.titleLarge)
                Spacer(modifier = Modifier.height(8.dp))
                if (toc.isEmpty()) {
                    Text("No table of contents", color = MaterialTheme.colorScheme.onSurfaceVariant)
                } else {
                    toc.forEach { item ->
                        TextButton(
                            onClick = {
                                eval("DiarchReader.goToc(${JSONObject.quote(item.href)});")
                                showToc = false
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Text(item.label, modifier = Modifier.fillMaxWidth())
                        }
                    }
                }
                Spacer(modifier = Modifier.height(24.dp))
            }
        }
    }

    if (showTypo) {
        TypographySheet(
            typography = typography,
            onChange = { next ->
                typography = next
                eval("DiarchReader.setTypography(${typoJson(next)});")
                scope.launch {
                    runCatching {
                        repo.putSettings(UpdateSettingsRequest(readerTypography = next))
                    }
                }
            },
            onDismiss = { showTypo = false },
        )
    }

    DisposableEffect(Unit) {
        onDispose {
            webView?.destroy()
            webView = null
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun TypographySheet(
    typography: ReaderTypography,
    onChange: (ReaderTypography) -> Unit,
    onDismiss: () -> Unit,
) {
    var size by remember(typography.size) { mutableFloatStateOf(typography.size.toFloat()) }
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text("Typography", style = MaterialTheme.typography.titleLarge)
            Text("Palette")
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                listOf("dark", "sepia", "light", "paper", "night", "contrast").forEach { p ->
                    TextButton(onClick = { onChange(typography.copy(palette = p)) }) {
                        Text(p)
                    }
                }
            }
            Text("Font")
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                listOf("serif", "literata", "sans", "dyslexia", "mono").forEach { f ->
                    TextButton(onClick = { onChange(typography.copy(font = f)) }) {
                        Text(f)
                    }
                }
            }
            Text("Size ${size.toInt()}%")
            Slider(
                value = size,
                onValueChange = { size = it },
                onValueChangeFinished = {
                    val stepped = ((size / 5).toInt() * 5).coerceIn(80, 200)
                    size = stepped.toFloat()
                    onChange(typography.copy(size = stepped))
                },
                valueRange = 80f..200f,
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(
                    checked = typography.justify,
                    onCheckedChange = { onChange(typography.copy(justify = it)) },
                )
                Text("Justify")
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(
                    checked = typography.indent,
                    onCheckedChange = { onChange(typography.copy(indent = it)) },
                )
                Text("First-line indent")
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(
                    checked = typography.hyphenate,
                    onCheckedChange = { onChange(typography.copy(hyphenate = it)) },
                )
                Text("Hyphenate")
            }
            Spacer(modifier = Modifier.height(24.dp))
        }
    }
}

@SuppressLint("SetJavaScriptEnabled")
@Composable
private fun ReaderWebView(
    onCreated: (WebView) -> Unit,
    onMessage: (String) -> Unit,
) {
    AndroidView(
        factory = { context ->
            WebView(context).apply {
                layoutParams = ViewGroup.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT,
                )
                settings.javaScriptEnabled = true
                settings.domStorageEnabled = true
                settings.allowFileAccess = true
                settings.mixedContentMode = WebSettings.MIXED_CONTENT_ALWAYS_ALLOW
                settings.cacheMode = WebSettings.LOAD_DEFAULT
                addJavascriptInterface(
                    object {
                        @JavascriptInterface
                        fun postMessage(message: String) {
                            (context as? android.app.Activity)?.runOnUiThread {
                                onMessage(message)
                            } ?: android.os.Handler(android.os.Looper.getMainLooper()).post {
                                onMessage(message)
                            }
                        }
                    },
                    "DiarchBridge",
                )
                webViewClient = object : WebViewClient() {
                    override fun onPageFinished(view: WebView?, url: String?) {
                        // bridge posts bridgeReady itself
                    }
                }
                loadUrl("file:///android_asset/reader/reader.html")
                onCreated(this)
            }
        },
        modifier = Modifier.fillMaxSize(),
        update = { /* keep */ },
    )
}
