package app.diarch.android.data

import android.content.Context
import android.net.Uri
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import okhttp3.OkHttpClient
import okhttp3.Request
import java.io.File
import java.io.IOException
import java.time.Instant

@Serializable
data class OfflineManifest(
    val workId: String,
    val title: String,
    val authors: String = "",
    val status: String = "unread",
    val readingDirection: String = "ltr",
    val isManga: Boolean = false,
    val updatedAt: String? = null,
    val hasEpub: Boolean = false,
    val hasMd: Boolean = false,
    val hasAudio: Boolean = false,
    val hasCover: Boolean = false,
    val audioFile: String? = null,
    val savedAt: Long = System.currentTimeMillis(),
)

/** Locally persisted reading progress, timestamped so it can be compared against the
 * server's `updated_at` to decide which copy is newer (see LibraryRepository.getProgress). */
@Serializable
data class LocalProgress(
    val mode: String,
    val position: String,
    val percent: Double,
    val savedAt: Long = System.currentTimeMillis(),
)

data class OfflineAvailability(
    val workId: String,
    val hasEpub: Boolean,
    val hasMd: Boolean,
    val hasAudio: Boolean,
    val hasCover: Boolean,
    val manifest: OfflineManifest?,
) {
    val isAvailable: Boolean get() = hasEpub || hasMd || hasAudio
}

class OfflineStore(
    private val context: Context,
    private val apiClient: ApiClient,
) {
    private val json = Json {
        ignoreUnknownKeys = true
        encodeDefaults = true
    }

    private val root: File
        get() = File(context.filesDir, "offline").also { it.mkdirs() }

    fun workDir(workId: String): File =
        File(root, workId).also { it.mkdirs() }

    fun epubFile(workId: String) = File(workDir(workId), "book.epub")
    fun mdFile(workId: String) = File(workDir(workId), "book.md")
    fun coverFile(workId: String) = File(workDir(workId), "cover.jpg")
    fun chaptersFile(workId: String) = File(workDir(workId), "chapters.json")
    fun manifestFile(workId: String) = File(workDir(workId), "meta.json")

    fun audioFile(workId: String, name: String = "book.m4b") =
        File(workDir(workId), name)

    fun availability(workId: String): OfflineAvailability {
        val manifest = readManifest(workId)
        return OfflineAvailability(
            workId = workId,
            hasEpub = epubFile(workId).isFile,
            hasMd = mdFile(workId).isFile,
            hasAudio = audioFile(workId).isFile ||
                (manifest?.audioFile?.let { File(workDir(workId), it).isFile } == true),
            hasCover = coverFile(workId).isFile,
            manifest = manifest,
        )
    }

    fun isDownloaded(workId: String): Boolean = availability(workId).isAvailable

    fun listDownloaded(): List<OfflineManifest> =
        root.listFiles()
            ?.filter { it.isDirectory }
            ?.mapNotNull { readManifest(it.name) }
            ?.filter { availability(it.workId).isAvailable }
            ?.sortedBy { it.title.lowercase() }
            .orEmpty()

    fun readManifest(workId: String): OfflineManifest? {
        val f = manifestFile(workId)
        if (!f.isFile) return null
        return runCatching { json.decodeFromString<OfflineManifest>(f.readText()) }.getOrNull()
    }

    fun localCoverUri(workId: String): Uri? {
        val f = coverFile(workId)
        return if (f.isFile) Uri.fromFile(f) else null
    }

    fun localAudioUri(workId: String): Uri? {
        val manifest = readManifest(workId)
        val name = manifest?.audioFile ?: "book.m4b"
        val f = audioFile(workId, name)
        return if (f.isFile) Uri.fromFile(f) else null
    }

    fun readMarkdownText(workId: String): String? {
        val f = mdFile(workId)
        return if (f.isFile) f.readText() else null
    }

    fun readChaptersJson(workId: String): String? {
        val f = chaptersFile(workId)
        return if (f.isFile) f.readText() else null
    }

    fun progressFile(workId: String, mode: String) =
        File(workDir(workId), "progress-$mode.json")

    fun saveLocalProgress(workId: String, mode: String, position: String, percent: Double) {
        val body = json.encodeToString(
            LocalProgress(mode = mode, position = position, percent = percent),
        )
        progressFile(workId, mode).writeText(body)
    }

    fun readLocalProgress(workId: String, mode: String): ReadingProgress? {
        val f = progressFile(workId, mode)
        if (!f.isFile) return null
        return runCatching {
            val body = json.decodeFromString<LocalProgress>(f.readText())
            ReadingProgress(
                mode = body.mode,
                position = body.position,
                percent = body.percent,
                updatedAt = Instant.ofEpochMilli(body.savedAt).toString(),
            )
        }.getOrNull()
    }

    suspend fun download(
        detail: WorkDetailResponse,
        onProgress: (String) -> Unit = {},
    ): OfflineManifest = withContext(Dispatchers.IO) {
        val work = detail.work
        val workId = work.id
        val dir = workDir(workId)
        val hasEpub = detail.hasEpub || detail.assets.any { it.kind == "epub" } || work.hasEpub
        val hasMd = detail.hasMd || detail.assets.any { it.kind == "markdown" } || work.hasMd
        val audioAsset = detail.assets.firstOrNull { it.kind == "audio" }
        val hasAudio = detail.hasAudio || audioAsset != null || work.hasAudio
        val audioName = audioAsset?.relativePath?.substringAfterLast('/')?.ifBlank { null } ?: "book.m4b"

        if (!hasEpub && !hasMd && !hasAudio) {
            throw IOException("Nothing to download for this work")
        }

        val client = downloadClient()

        if (hasEpub) {
            onProgress("Downloading EPUB…")
            downloadTo(client, "${apiClient.baseUrl()}/api/works/$workId/content/epub", epubFile(workId))
        }
        if (hasMd) {
            onProgress("Downloading markdown…")
            downloadTo(client, "${apiClient.baseUrl()}/api/works/$workId/content/markdown", mdFile(workId))
        }
        if (hasAudio) {
            onProgress("Downloading audiobook…")
            downloadTo(
                client,
                "${apiClient.baseUrl()}/api/works/$workId/audio/$audioName",
                audioFile(workId, audioName),
            )
            runCatching {
                onProgress("Saving chapters…")
                val chaptersBody = apiClient.api().audioChapters(workId)
                chaptersFile(workId).writeText(json.encodeToString(chaptersBody))
            }
        }

        onProgress("Saving cover…")
        runCatching {
            downloadTo(client, apiClient.coverUrl(workId, work.updatedAt), coverFile(workId))
        }

        val manifest = OfflineManifest(
            workId = workId,
            title = work.title,
            authors = work.authors,
            status = work.status,
            readingDirection = work.readingDirection,
            isManga = work.isManga,
            updatedAt = work.updatedAt,
            hasEpub = epubFile(workId).isFile,
            hasMd = mdFile(workId).isFile,
            hasAudio = audioFile(workId, audioName).isFile,
            hasCover = coverFile(workId).isFile,
            audioFile = audioName.takeIf { audioFile(workId, audioName).isFile },
        )
        manifestFile(workId).writeText(json.encodeToString(manifest))
        onProgress("Saved offline")
        manifest
    }

    fun remove(workId: String) {
        val dir = File(root, workId)
        if (dir.exists()) dir.deleteRecursively()
    }

    private fun downloadClient(): OkHttpClient {
        return OkHttpClient.Builder()
            .addInterceptor { chain ->
                val token = apiClient.authHeader()
                val req = if (token != null) {
                    chain.request().newBuilder().header("Authorization", token).build()
                } else {
                    chain.request()
                }
                chain.proceed(req)
            }
            .connectTimeout(60, java.util.concurrent.TimeUnit.SECONDS)
            .readTimeout(300, java.util.concurrent.TimeUnit.SECONDS)
            .build()
    }

    private fun downloadTo(client: OkHttpClient, url: String, dest: File) {
        val req = Request.Builder().url(url).get().build()
        client.newCall(req).execute().use { resp ->
            if (!resp.isSuccessful) {
                throw IOException("Download failed (${resp.code}) for ${dest.name}")
            }
            val body = resp.body ?: throw IOException("Empty body for ${dest.name}")
            dest.parentFile?.mkdirs()
            val tmp = File(dest.parentFile, "${dest.name}.part")
            body.byteStream().use { input ->
                tmp.outputStream().use { output -> input.copyTo(output) }
            }
            if (!tmp.renameTo(dest)) {
                tmp.copyTo(dest, overwrite = true)
                tmp.delete()
            }
        }
    }
}

fun OfflineManifest.toWork(): Work =
    Work(
        id = workId,
        title = title,
        authors = authors,
        status = status,
        readingDirection = readingDirection,
        isManga = isManga,
        updatedAt = updatedAt,
        hasEpub = hasEpub,
        hasMd = hasMd,
        hasAudio = hasAudio,
    )
