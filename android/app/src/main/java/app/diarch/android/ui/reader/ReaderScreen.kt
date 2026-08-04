package app.diarch.android.ui.reader

import android.annotation.SuppressLint
import android.net.Uri
import android.view.ViewGroup
import android.webkit.JavascriptInterface
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
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
import app.diarch.android.data.AudioChapter
import app.diarch.android.data.ReaderTypography
import app.diarch.android.data.UpdateSettingsRequest
import app.diarch.android.data.WorkDetailResponse
import kotlinx.coroutines.DelicateCoroutinesApi
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.GlobalScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.Request
import org.json.JSONObject
import java.io.FileInputStream
import java.io.IOException

data class TocItem(val label: String, val href: String)

/** Virtual origin the reader WebView is pointed at for offline EPUBs; served entirely by
 * [ReaderWebView]'s shouldInterceptRequest from the on-disk file, never touching the network. */
private const val OFFLINE_EPUB_ORIGIN = "https://diarch.offline/epub"

/** Matches the online content endpoints so the WebView can proxy them with an
 * Authorization header from [app.diarch.android.data.ApiClient] without exposing the
 * token to JS. */
private val CONTENT_PATH_REGEX = Regex("^/api/works/[^/]+/content/[^/]+/?$")

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ReaderScreen(
    detail: WorkDetailResponse,
    onClose: () -> Unit,
) {
    val work = detail.work
    val offline = DiarchApp.instance.offlineStore
    val offlineAvail = remember(work.id) { offline.availability(work.id) }
    val hasEpub =
        detail.assets.any { it.kind == "epub" } || detail.hasEpub || work.hasEpub || offlineAvail.hasEpub
    val hasMd =
        detail.assets.any { it.kind == "markdown" } || detail.hasMd || work.hasMd || offlineAvail.hasMd
    val hasAudio =
        detail.assets.any { it.kind == "audio" } || detail.hasAudio || work.hasAudio || offlineAvail.hasAudio
    val audioAsset = detail.assets.firstOrNull { it.kind == "audio" }
    val audioFile = offlineAvail.manifest?.audioFile
        ?: audioAsset?.relativePath?.substringAfterLast('/')?.ifBlank { null }
        ?: "book.m4b"
    val localAudioUri = remember(work.id, hasAudio) { offline.localAudioUri(work.id) }
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
    var chapters by remember { mutableStateOf<List<AudioChapter>>(emptyList()) }
    var audioStartSec by remember { mutableStateOf<Double?>(null) }
    var audioReady by remember { mutableStateOf(false) }
    var lastSyncedChapter by remember { mutableStateOf<Int?>(null) }
    var lastAudioSaveMs by remember { mutableStateOf(0L) }
    var lastProgressSaveMs by remember { mutableStateOf(0L) }
    var pendingProgress by remember { mutableStateOf<Triple<String, String, Double>?>(null) }

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
        if (!hasEpub && !hasMd) return
        if (useMd && !hasMd) return
        if (!useMd && !hasEpub) return
        infiniteScroll = useMd
        scope.launch {
            val mode = if (useMd) "markdown" else "epub"
            val progress = withContext(Dispatchers.IO) { repo.getProgress(work.id, mode) }
            val opts = JSONObject()
                .put("mode", mode)
                .put("forceJustify", useMd)
            if (progress != null) {
                opts.put(
                    "progress",
                    JSONObject()
                        .put("position", progress.position)
                        .put("percent", progress.percent),
                )
            } else {
                opts.put("progress", JSONObject.NULL)
            }
            if (useMd) {
                val localMd = withContext(Dispatchers.IO) { offline.readMarkdownText(work.id) }
                if (localMd != null) {
                    opts.put("localMarkdown", localMd)
                    opts.put("offline", true)
                }
            } else {
                val hasLocalEpub = withContext(Dispatchers.IO) { offline.epubFile(work.id).isFile }
                if (hasLocalEpub) {
                    // Served by the WebView's shouldInterceptRequest — no base64 round-trip.
                    opts.put("localEpubUrl", "$OFFLINE_EPUB_ORIGIN/${work.id}")
                    opts.put("offline", true)
                }
            }
            eval("DiarchReader.open($opts);")
            runCatching {
                repo.putSettings(UpdateSettingsRequest(readerInfiniteScroll = useMd))
            }
        }
    }

    fun syncTextToChapter(chapter: AudioChapter) {
        if (chapter.index == lastSyncedChapter) return
        lastSyncedChapter = chapter.index
        val title = chapter.title.ifBlank { return }
        eval("DiarchReader.goChapterTitle(${JSONObject.quote(title)});")
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
        if (hasAudio) {
            chapters = withContext(Dispatchers.IO) {
                val remote = runCatching { repo.audioChapters(work.id) }.getOrDefault(emptyList())
                if (remote.isNotEmpty()) {
                    remote
                } else {
                    val local = offline.readChaptersJson(work.id) ?: return@withContext emptyList()
                    runCatching {
                        json.decodeFromString(
                            app.diarch.android.data.AudioChaptersResponse.serializer(),
                            local,
                        ).chapters
                    }.getOrDefault(emptyList())
                }
            }
            val audioProg = withContext(Dispatchers.IO) { repo.getProgress(work.id, "audio") }
            audioStartSec = audioProg?.position?.toDoubleOrNull()
            audioReady = true
        }
    }

    LaunchedEffect(bridgeReady, webView) {
        if (!bridgeReady || webView == null || opened) return@LaunchedEffect
        opened = true
        // No token is passed to the WebView: content requests are authenticated
        // natively via shouldInterceptRequest (see ReaderWebView below).
        val base = repo.baseUrl()
        val rtl = work.readingDirection == "rtl" || work.isManga
        eval(
            """
            DiarchReader.init({
              workId: ${JSONObject.quote(work.id)},
              baseUrl: ${JSONObject.quote(base)},
              rtl: $rtl,
              typography: ${typoJson(typography)}
            });
            """.trimIndent(),
        )
        if (hasEpub || hasMd) {
            openMode(infiniteScroll)
        }
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
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding),
        ) {
            Box(
                modifier = Modifier
                    .weight(1f)
                    .fillMaxWidth(),
            ) {
                if (hasEpub || hasMd) {
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
                                    "chapterSync" -> {
                                        val matched = obj["matched"]?.jsonPrimitive?.booleanOrNull == true
                                        val label = obj["label"]?.jsonPrimitive?.contentOrNull
                                        if (matched && !label.isNullOrBlank()) {
                                            progressText = "Synced · $label"
                                        }
                                    }
                                    "progress" -> {
                                        val mode = obj["mode"]?.jsonPrimitive?.contentOrNull ?: return@ReaderWebView
                                        val position = obj["position"]?.jsonPrimitive?.contentOrNull ?: ""
                                        val percent = obj["percent"]?.jsonPrimitive?.doubleOrNull ?: 0.0
                                        progressText = "${percent.toInt()}% read"
                                        pendingProgress = Triple(mode, position, percent)
                                        val now = System.currentTimeMillis()
                                        if (now - lastProgressSaveMs < 500) return@ReaderWebView
                                        lastProgressSaveMs = now
                                        pendingProgress = null
                                        scope.launch {
                                            repo.putProgress(work.id, mode, position, percent)
                                        }
                                    }
                                }
                            } catch (_: Exception) {
                            }
                        },
                    )
                } else {
                    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                        Text(
                            "Audiobook only — use the player below",
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
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
            if (hasAudio && audioReady) {
                AudioPlayerBar(
                    streamUrl = repo.audioUrl(work.id, audioFile),
                    authHeader = repo.authHeader(),
                    chapters = chapters,
                    initialPositionSec = audioStartSec,
                    localUri = localAudioUri,
                    onChapterChanged = { chapter -> syncTextToChapter(chapter) },
                    onProgress = { posSec, percent, chapter ->
                        val now = System.currentTimeMillis()
                        if (now - lastAudioSaveMs < 2000) return@AudioPlayerBar
                        lastAudioSaveMs = now
                        scope.launch {
                            repo.putProgress(work.id, "audio", posSec.toString(), percent)
                        }
                        if (chapter != null && (hasEpub || hasMd)) {
                            // keep subtitle fresh while listening
                            if (progressText.isBlank() || progressText.startsWith("Synced") || progressText.contains("audio")) {
                                progressText = "${percent.toInt()}% audio · ${chapter.title.ifBlank { "ch ${chapter.index}" }}"
                            }
                        }
                    },
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
            // Flush any progress update that was still waiting out the throttle window,
            // mirroring the audio player's lastAudioSaveMs pattern. rememberCoroutineScope
            // is cancelled around the same time as this callback, so use a detached
            // best-effort scope rather than `scope.launch`.
            pendingProgress?.let { (mode, position, percent) ->
                @OptIn(DelicateCoroutinesApi::class)
                GlobalScope.launch(Dispatchers.IO) {
                    runCatching { repo.putProgress(work.id, mode, position, percent) }
                }
            }
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

                    override fun shouldInterceptRequest(
                        view: WebView?,
                        request: WebResourceRequest?,
                    ): WebResourceResponse? {
                        val url = request?.url ?: return super.shouldInterceptRequest(view, request)
                        offlineEpubResponse(url)?.let { return it }
                        contentProxyResponse(url)?.let { return it }
                        return super.shouldInterceptRequest(view, request)
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

/** Serves an offline EPUB straight off disk for `OFFLINE_EPUB_ORIGIN/{workId}` requests. */
private fun offlineEpubResponse(url: Uri): WebResourceResponse? {
    if (!url.toString().startsWith(OFFLINE_EPUB_ORIGIN)) return null
    val workId = url.lastPathSegment
    if (workId.isNullOrBlank()) {
        return WebResourceResponse("application/epub+zip", "utf-8", 400, "Bad Request", emptyMap(), null)
    }
    val file = DiarchApp.instance.offlineStore.epubFile(workId)
    if (!file.isFile) {
        return WebResourceResponse("application/epub+zip", "utf-8", 404, "Not Found", emptyMap(), null)
    }
    return WebResourceResponse("application/epub+zip", null, FileInputStream(file))
}

/** Proxies `/api/works/{id}/content/{kind}` requests through the app's authenticated OkHttp
 * client, so the WebView never needs the bearer token exposed to JS. */
private fun contentProxyResponse(url: Uri): WebResourceResponse? {
    val apiClient = DiarchApp.instance.apiClient
    val baseUri = runCatching { Uri.parse(apiClient.baseUrl()) }.getOrNull() ?: return null
    if (url.scheme != baseUri.scheme || !url.host.equals(baseUri.host, ignoreCase = true)) return null
    val path = url.path ?: return null
    if (!CONTENT_PATH_REGEX.matches(path)) return null
    return try {
        val request = Request.Builder().url(url.toString()).build()
        val response = apiClient.httpClient().newCall(request).execute()
        val body = response.body
        if (body == null) {
            response.close()
            return WebResourceResponse("text/plain", "utf-8", 502, "Bad Gateway", emptyMap(), null)
        }
        val mediaType = body.contentType()
        val mime = mediaType?.let { "${it.type}/${it.subtype}" } ?: "application/octet-stream"
        val charset = mediaType?.charset()?.name()
        val reason = response.message.ifBlank { "OK" }
        WebResourceResponse(mime, charset, response.code, reason, emptyMap(), body.byteStream())
    } catch (e: IOException) {
        WebResourceResponse("text/plain", "utf-8", 502, "Bad Gateway", emptyMap(), null)
    }
}
