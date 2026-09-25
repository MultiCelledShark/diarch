package app.diarch.android.data

import android.content.ContentResolver
import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import kotlinx.coroutines.Dispatchers
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
        sessionStore.saveLogin(normalized, res.token, res.user.username, res.user.isAdmin)
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

    suspend fun importUri(uri: Uri, title: String?, authors: String?): ImportResponse =
        withContext(Dispatchers.IO) {
            val resolver = appContext.contentResolver
            val name = queryDisplayName(uri) ?: "upload.bin"
            val mime = resolver.getType(uri) ?: "application/octet-stream"
            val length = querySize(uri)
            // Stream the content URI. Audiobooks do not fit in the app heap, and
            // reading them on the main thread ANRs the process.
            val fileBody = ContentUriRequestBody(resolver, uri, mime.toMediaTypeOrNull(), length)
            val multipart = MultipartBody.Builder()
                .setType(MultipartBody.FORM)
                .addFormDataPart("file", name, fileBody)
            title?.takeIf { it.isNotBlank() }?.let { multipart.addFormDataPart("title", it) }
            authors?.takeIf { it.isNotBlank() }?.let { multipart.addFormDataPart("authors", it) }
            val request = Request.Builder()
                .url("${apiClient.baseUrl()}/api/library/import")
                .post(multipart.build())
                .build()
            apiClient.uploadClient().newCall(request).execute().use { response ->
                val text = response.body.string()
                if (!response.isSuccessful) {
                    throw IOException(text.ifBlank { "Import failed (${response.code})" })
                }
                importJson.decodeFromString(ImportResponse.serializer(), text)
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

    suspend fun listUsers(): List<User> = apiClient.api().listUsers()

    suspend fun listGrants(workId: String): List<WorkGrant> =
        apiClient.api().listGrants(workId)

    suspend fun grantAccess(workId: String, username: String) {
        val res = apiClient.api().addGrant(workId, GrantRequest(username = username))
        if (!res.isSuccessful) {
            val msg = res.errorBody()?.string()?.ifBlank { null }
                ?: "Grant failed (${res.code()})"
            throw IOException(msg)
        }
    }

    suspend fun revokeAccess(workId: String, userId: String) {
        val res = apiClient.api().revokeGrant(workId, userId)
        if (!res.isSuccessful) {
            val msg = res.errorBody()?.string()?.ifBlank { null }
                ?: "Revoke failed (${res.code()})"
            throw IOException(msg)
        }
    }

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
 */
private class ContentUriRequestBody(
    private val resolver: ContentResolver,
    private val uri: Uri,
    private val mime: MediaType?,
    private val length: Long,
) : RequestBody() {
    override fun contentType(): MediaType? = mime

    override fun contentLength(): Long = if (length > 0) length else -1L

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
