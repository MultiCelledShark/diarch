package app.diarch.android.data

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.MultipartBody
import okhttp3.RequestBody.Companion.toRequestBody
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

    suspend fun listWorks(shelf: Shelf): List<Work> {
        return try {
            apiClient.api().listWorks(shelf.apiStatus)
        } catch (e: Exception) {
            offlineStore.listDownloaded()
                .map { it.toWork() }
                .filter { it.status == shelf.apiStatus || shelf == Shelf.Library }
                .ifEmpty { throw e }
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
            return apiClient.api().updateWork(id, UpdateWorkRequest(status = status))
        } catch (e: HttpException) {
            if (e.code() == 409) {
                val msg = e.response()?.errorBody()?.string()?.ifBlank { null }
                    ?: "That shelf is full. Move a book off it first."
                throw ShelfFullException(msg)
            }
            throw e
        }
    }

    suspend fun importUri(uri: Uri, title: String?, authors: String?): ImportResponse {
        val resolver = appContext.contentResolver
        val name = queryDisplayName(uri) ?: "upload.bin"
        val mime = resolver.getType(uri) ?: "application/octet-stream"
        val bytes = resolver.openInputStream(uri)?.use { it.readBytes() }
            ?: throw IOException("Could not read selected file")
        val body = bytes.toRequestBody(mime.toMediaTypeOrNull())
        val part = MultipartBody.Part.createFormData("file", name, body)
        val titleBody = title?.takeIf { it.isNotBlank() }?.toRequestBody("text/plain".toMediaTypeOrNull())
        val authorsBody = authors?.takeIf { it.isNotBlank() }?.toRequestBody("text/plain".toMediaTypeOrNull())
        return apiClient.api().importFile(part, titleBody, authorsBody)
    }

    suspend fun getSettings(): UserSettings = apiClient.api().getSettings()

    suspend fun putSettings(body: UpdateSettingsRequest) {
        apiClient.api().putSettings(body)
    }

    suspend fun putProgress(id: String, mode: String, position: String, percent: Double) {
        offlineStore.saveLocalProgress(id, mode, position, percent)
        runCatching {
            apiClient.api().putProgress(id, ProgressBody(mode, position, percent))
        }
    }

    suspend fun getProgress(id: String, mode: String): ReadingProgress? {
        val remote = runCatching { apiClient.api().getProgress(id, mode) }.getOrNull()
        if (remote != null) return remote
        return offlineStore.readLocalProgress(id, mode)
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
}

class ShelfFullException(message: String) : Exception(message)
