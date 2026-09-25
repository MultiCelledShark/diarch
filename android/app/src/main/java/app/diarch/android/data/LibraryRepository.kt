package app.diarch.android.data

import android.content.ContentResolver
import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import okhttp3.MediaType
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.MultipartBody
import okhttp3.Request
import okhttp3.RequestBody
import okio.BufferedSink
import retrofit2.HttpException
import java.io.IOException

class LibraryRepository(
    private val apiClient: ApiClient,
    private val sessionStore: SessionStore,
    private val appContext: Context,
    private val offlineStore: OfflineStore,
) {
    suspend fun login(baseUrl: String, username: String, password: String): User {
        val normalized = SessionStore.normalizeBaseUrl(baseUrl)
        apiClient.updateSession(normalized, "")
        val res = apiClient.api().login(LoginRequest(username, password))
        apiClient.updateSession(normalized, res.token)
        sessionStore.saveLogin(normalized, res.token, res.user.username)
        return res.user
    }

    suspend fun logout() {
        runCatching { apiClient.api().logout() }
        sessionStore.clearToken()
        apiClient.updateSession(apiClient.baseUrl(), "")
    }

    data class WorksLoad(
        val works: List<Work>,
        val offline: Boolean = false,
    )

    suspend fun listWorks(shelf: Shelf, query: String? = null): WorksLoad {
        val q = query?.trim()?.takeIf { it.isNotEmpty() }
        return try {
            // When searching, ignore shelf status so matches aren't hidden on other shelves.
            val status = if (q != null) null else shelf.apiStatus
            WorksLoad(apiClient.api().listWorks(status = status, q = q))
        } catch (e: Exception) {
            val downloaded = offlineStore.listDownloaded().map { it.toWork() }
            if (downloaded.isEmpty()) throw e
            val needle = q?.lowercase()
            val filtered = downloaded.filter { work ->
                val matchesQuery = needle == null ||
                    work.title.lowercase().contains(needle) ||
                    work.authors.lowercase().contains(needle) ||
                    work.isbn?.lowercase()?.contains(needle) == true
                if (!matchesQuery) return@filter false
                if (q != null) true
                else work.status == shelf.apiStatus || shelf == Shelf.Library
            }
            WorksLoad(filtered, offline = true)
        }
    }

    suspend fun getWork(id: String): WorkDetailResponse {
        return try {
            apiClient.api().getWork(id)
        } catch (e: Exception) {
            val manifest = offlineStore.readManifest(id) ?: throw e
            val avail = offlineStore.availability(id)
            WorkDetailResponse(
                work = manifest.toWork(),
                hasEpub = avail.hasEpub,
                hasMd = avail.hasMd,
                hasAudio = avail.hasAudio,
            )
        }
    }

    suspend fun setStatus(id: String, status: String): Work {
        try {
            val work = apiClient.api().updateWork(id, UpdateWorkRequest(status = status))
            offlineStore.updateManifestStatus(id, status)
            return work
        } catch (e: HttpException) {
            if (e.code() == 409) {
                val msg = e.response()?.errorBody()?.string()?.ifBlank { null }
                    ?: "That shelf is full. Move a book off it first."
                throw ShelfFullException(msg)
            }
            throw e
        } catch (e: Exception) {
            // Server unreachable: still allow shelf moves on offline copies.
            offlineStore.updateManifestStatus(id, status)
                ?: throw e
            return offlineStore.readManifest(id)?.toWork() ?: throw e
        }
    }

    /**
     * Upload a library file (and optional cover) then wait for any async import
     * job. Mirrors the web client's poll-before-open behaviour so EPUB/PDF/MD
     * imports are not shown as "No EPUB" while the job is still running.
     */
    suspend fun importUri(
        uri: Uri,
        title: String?,
        authors: String?,
        coverUri: Uri? = null,
        onStatus: (String) -> Unit = {},
    ): ImportResponse =
        withContext(Dispatchers.IO) {
            val resolver = appContext.contentResolver
            val mime = resolver.getType(uri) ?: "application/octet-stream"
            val name = uploadFileName(uri, mime)
            val length = querySize(uri)
            // Stream the content URI. Audiobooks do not fit in the app heap, and
            // reading them on the main thread ANRs the process.
            val fileBody = ContentUriRequestBody(resolver, uri, mime.toMediaTypeOrNull(), length)
            val multipart = MultipartBody.Builder()
                .setType(MultipartBody.FORM)
                .addFormDataPart("file", name, fileBody)
            title?.takeIf { it.isNotBlank() }?.let { multipart.addFormDataPart("title", it) }
            authors?.takeIf { it.isNotBlank() }?.let { multipart.addFormDataPart("authors", it) }
            if (coverUri != null) {
                appendCoverPart(multipart, coverUri)
            }
            onStatus("Uploading…")
            val request = Request.Builder()
                .url("${apiClient.baseUrl()}/api/library/import")
                .post(multipart.build())
                .build()
            val imported = apiClient.uploadClient().newCall(request).execute().use { response ->
                val text = response.body.string()
                if (!response.isSuccessful) {
                    throw IOException(text.ifBlank { "Import failed (${response.code})" })
                }
                importJson.decodeFromString(ImportResponse.serializer(), text)
            }
            val jobId = imported.jobId
            if (jobId.isNullOrBlank()) {
                onStatus("Imported")
                return@withContext imported
            }
            onStatus("Importing…")
            pollImportJob(jobId, onStatus)
            imported
        }

    /** Attach / replace cover art on an existing work (`POST /api/works/{id}/cover`). */
    suspend fun uploadCover(workId: String, uri: Uri) =
        withContext(Dispatchers.IO) {
            val multipart = MultipartBody.Builder().setType(MultipartBody.FORM)
            appendCoverPart(multipart, uri)
            val request = Request.Builder()
                .url("${apiClient.baseUrl()}/api/works/$workId/cover")
                .post(multipart.build())
                .build()
            apiClient.uploadClient().newCall(request).execute().use { response ->
                if (!response.isSuccessful) {
                    val text = response.body.string()
                    throw IOException(text.ifBlank { "Cover upload failed (${response.code})" })
                }
            }
        }

    private suspend fun pollImportJob(jobId: String, onStatus: (String) -> Unit) {
        // Web polls every 500ms for up to ~60s; allow longer for PDF/OCR on LAN.
        repeat(240) {
            delay(500)
            val job = try {
                apiClient.api().getJob(jobId)
            } catch (e: Exception) {
                throw IOException(e.message ?: "Could not check import status")
            }
            when (job.status) {
                "done" -> {
                    onStatus("Imported")
                    return
                }
                "failed" -> {
                    val detail = job.detail?.takeIf { it.isNotBlank() } ?: "unknown error"
                    throw IOException("Import failed: $detail")
                }
                else -> onStatus("Importing…")
            }
        }
        throw IOException("Timed out waiting for import job")
    }

    private fun appendCoverPart(multipart: MultipartBody.Builder, uri: Uri) {
        val resolver = appContext.contentResolver
        val mime = resolver.getType(uri) ?: "image/jpeg"
        val name = uploadFileName(uri, mime)
        val length = querySize(uri)
        val body = ContentUriRequestBody(resolver, uri, mime.toMediaTypeOrNull(), length)
        multipart.addFormDataPart("cover", name, body)
    }

    /**
     * Prefer the provider display name, but force a known extension from MIME
     * when Android returns `upload.bin` / extensionless names. The server also
     * infers format, but sending a correct name keeps both sides aligned.
     */
    private fun uploadFileName(uri: Uri, mime: String): String {
        val raw = queryDisplayName(uri)?.substringAfterLast('/')?.takeIf { it.isNotBlank() }
            ?: "upload.bin"
        if (hasKnownImportExtension(raw) || hasKnownImageExtension(raw)) {
            return raw
        }
        val ext = extensionForMime(mime) ?: return raw
        val stem = raw.substringBeforeLast('.').ifBlank { "upload" }
        return "$stem.$ext"
    }

    private fun hasKnownImportExtension(name: String): Boolean {
        val lower = name.lowercase()
        return lower.endsWith(".epub") ||
            lower.endsWith(".pdf") ||
            lower.endsWith(".md") ||
            lower.endsWith(".markdown") ||
            lower.endsWith(".m4b") ||
            lower.endsWith(".m4a") ||
            lower.endsWith(".aax")
    }

    private fun hasKnownImageExtension(name: String): Boolean {
        val lower = name.lowercase()
        return lower.endsWith(".jpg") ||
            lower.endsWith(".jpeg") ||
            lower.endsWith(".png") ||
            lower.endsWith(".webp")
    }

    private fun extensionForMime(mime: String): String? {
        val base = mime.substringBefore(';').trim().lowercase()
        return when (base) {
            "application/epub+zip" -> "epub"
            "application/pdf" -> "pdf"
            "text/markdown", "text/x-markdown" -> "md"
            "audio/m4b", "audio/x-m4b" -> "m4b"
            "audio/m4a", "audio/x-m4a", "audio/mp4", "audio/aac" -> "m4a"
            "audio/vnd.audible.aax", "audio/aax" -> "aax"
            "image/jpeg" -> "jpg"
            "image/png" -> "png"
            "image/webp" -> "webp"
            else -> null
        }
    }

    suspend fun getSettings(): UserSettings =
        runCatching { apiClient.api().getSettings() }.getOrElse {
            UserSettings()
        }

    suspend fun putSettings(body: UpdateSettingsRequest) {
        runCatching { apiClient.api().putSettings(body) }
    }

    suspend fun putProgress(id: String, mode: String, position: String, percent: Double) {
        offlineStore.saveLocalProgress(id, mode, position, percent)
        runCatching {
            apiClient.api().putProgress(id, ProgressBody(mode, position, percent))
        }
    }

    suspend fun getProgress(id: String, mode: String): ReadingProgress? {
        val local = offlineStore.readLocalProgress(id, mode)
        // When a local copy exists, don't block the reader on a dead/slow server.
        val remote = if (offlineStore.isDownloaded(id)) {
            kotlinx.coroutines.withTimeoutOrNull(3_000) {
                runCatching { apiClient.api().getProgress(id, mode) }.getOrNull()
            }
        } else {
            runCatching { apiClient.api().getProgress(id, mode) }.getOrNull()
        }
        if (remote == null) return local
        if (local == null) return remote
        val remoteMs = parseUpdatedAtMillis(remote.updatedAt)
        val localMs = parseUpdatedAtMillis(local.updatedAt)
        return if (localMs != null && (remoteMs == null || localMs > remoteMs)) local else remote
    }

    private fun parseUpdatedAtMillis(value: String?): Long? {
        if (value.isNullOrBlank()) return null
        return runCatching { java.time.Instant.parse(value).toEpochMilli() }.getOrNull()
    }

    fun coverUrl(workId: String, updatedAt: String? = null): String =
        apiClient.coverUrl(workId, updatedAt)

    fun authHeader(): String? = apiClient.authHeader()

    fun baseUrl(): String = apiClient.baseUrl()

    fun audioUrl(workId: String, filename: String = "book.m4b"): String =
        "${apiClient.baseUrl()}/api/works/$workId/audio/$filename"

    suspend fun audioChapters(workId: String): List<AudioChapter> =
        runCatching { apiClient.api().audioChapters(workId).chapters }.getOrDefault(emptyList())

    suspend fun addWishlist(title: String?, authors: String?, isbn: String?): Work =
        apiClient.api().addWishlist(
            WishlistRequest(
                title = title?.takeIf { it.isNotBlank() },
                authors = authors?.takeIf { it.isNotBlank() },
                isbn = isbn?.takeIf { it.isNotBlank() },
            ),
        )

    private fun queryDisplayName(uri: Uri): String? {
        val cursor = appContext.contentResolver.query(uri, null, null, null, null) ?: return null
        cursor.use {
            val idx = it.getColumnIndex(OpenableColumns.DISPLAY_NAME)
            if (idx >= 0 && it.moveToFirst()) return it.getString(idx)
        }
        return null
    }

    private fun querySize(uri: Uri): Long {
        val cursor = appContext.contentResolver.query(
            uri,
            arrayOf(OpenableColumns.SIZE),
            null,
            null,
            null,
        ) ?: return -1L
        cursor.use {
            val idx = it.getColumnIndex(OpenableColumns.SIZE)
            if (idx >= 0 && it.moveToFirst() && !it.isNull(idx)) return it.getLong(idx)
        }
        return -1L
    }

    private companion object {
        val importJson = Json {
            ignoreUnknownKeys = true
            isLenient = true
        }
    }
}

/**
 * Reads a content URI straight into the request sink. [writeTo] may run more
 * than once (retry), so each call reopens the stream instead of buffering.
 *
 * Always advertises an unknown length: some content providers report a wrong
 * [OpenableColumns.SIZE], and a mismatched Content-Length aborts the multipart
 * body mid-upload (the symptom behind failed mobile EPUB imports).
 */
private class ContentUriRequestBody(
    private val resolver: ContentResolver,
    private val uri: Uri,
    private val mime: MediaType?,
    @Suppress("UNUSED_PARAMETER") private val length: Long,
) : RequestBody() {
    override fun contentType(): MediaType? = mime

    override fun contentLength(): Long = -1L

    override fun writeTo(sink: BufferedSink) {
        val input = resolver.openInputStream(uri)
            ?: throw IOException("Could not read selected file")
        input.use { stream ->
            val buf = ByteArray(64 * 1024)
            while (true) {
                val n = stream.read(buf)
                if (n < 0) break
                if (n > 0) sink.write(buf, 0, n)
            }
        }
    }
}

class ShelfFullException(message: String) : Exception(message)
