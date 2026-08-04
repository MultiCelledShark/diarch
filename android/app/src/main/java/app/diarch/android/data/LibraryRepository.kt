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

    suspend fun listWorks(shelf: Shelf): List<Work> =
        apiClient.api().listWorks(shelf.apiStatus)

    suspend fun getWork(id: String): WorkDetailResponse =
        apiClient.api().getWork(id)

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

    suspend fun getProgress(id: String, mode: String): ReadingProgress? =
        runCatching { apiClient.api().getProgress(id, mode) }.getOrNull()

    suspend fun putProgress(id: String, mode: String, position: String, percent: Double) {
        runCatching {
            apiClient.api().putProgress(id, ProgressBody(mode, position, percent))
        }
    }

    fun coverUrl(workId: String, updatedAt: String? = null): String =
        apiClient.coverUrl(workId, updatedAt)

    fun authHeader(): String? = apiClient.authHeader()

    fun baseUrl(): String = apiClient.baseUrl()

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
