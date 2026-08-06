package app.diarch.android.data

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import app.diarch.android.BuildConfig
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import okhttp3.OkHttpClient
import okhttp3.Request
import java.io.File
import java.io.IOException
import java.util.concurrent.TimeUnit

@Serializable
data class ForgejoRelease(
    val id: Long = 0,
    @SerialName("tag_name") val tagName: String = "",
    val name: String = "",
    val body: String = "",
    val draft: Boolean = false,
    val prerelease: Boolean = false,
    val assets: List<ForgejoAsset> = emptyList(),
)

@Serializable
data class ForgejoAsset(
    val id: Long = 0,
    val name: String = "",
    val size: Long = 0,
    @SerialName("browser_download_url") val browserDownloadUrl: String = "",
)

data class AppUpdate(
    val tagName: String,
    val title: String,
    val notes: String,
    val versionLabel: String,
    val remoteVersionCode: Int?,
    val apkUrl: String,
    val apkName: String,
)

/**
 * Checks Forgejo/Gitea `/api/v1/repos/{owner}/{repo}/releases/latest` for a newer Android APK
 * and downloads/installs it via [FileProvider].
 *
 * Release convention (see android/README.md):
 * - Tag: `android-v{versionName}` (e.g. `android-v0.2.0`) or `v{versionName}`
 * - Body may include `versionCode: N` for unambiguous comparison
 * - Attach an `*.apk` asset (prefer name containing `arm64`)
 */
class UpdateChecker(private val context: Context) {
    private val json = Json {
        ignoreUnknownKeys = true
        isLenient = true
    }

    private val http = OkHttpClient.Builder()
        .connectTimeout(8, TimeUnit.SECONDS)
        .readTimeout(120, TimeUnit.SECONDS)
        .writeTimeout(60, TimeUnit.SECONDS)
        .build()

    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private val baseUrl: String
        get() = BuildConfig.FORGEJO_BASE_URL.trimEnd('/')

    private val releasesApiUrl: String
        get() = "$baseUrl/api/v1/repos/${BuildConfig.FORGEJO_OWNER}/${BuildConfig.FORGEJO_REPO}/releases/latest"

    fun localVersionName(): String = BuildConfig.VERSION_NAME

    fun localVersionCode(): Int = BuildConfig.VERSION_CODE

    fun wasDismissed(tagName: String): Boolean =
        prefs.getString(KEY_DISMISSED_TAG, null) == tagName

    fun dismiss(tagName: String) {
        prefs.edit().putString(KEY_DISMISSED_TAG, tagName).apply()
    }

    suspend fun checkForUpdate(includeDismissed: Boolean = false): AppUpdate? =
        withContext(Dispatchers.IO) {
            val release = fetchLatestRelease() ?: return@withContext null
            if (release.draft || release.prerelease) return@withContext null
            if (!isNewer(release)) return@withContext null
            if (!includeDismissed && wasDismissed(release.tagName)) return@withContext null
            val asset = pickApkAsset(release.assets) ?: return@withContext null
            val url = absolutize(asset.browserDownloadUrl)
            AppUpdate(
                tagName = release.tagName,
                title = release.name.ifBlank { release.tagName },
                notes = release.body.trim(),
                versionLabel = displayVersion(release),
                remoteVersionCode = parseVersionCode(release),
                apkUrl = url,
                apkName = asset.name.ifBlank { "diarch-update.apk" },
            )
        }

    suspend fun downloadApk(
        update: AppUpdate,
        onProgress: (downloaded: Long, total: Long) -> Unit = { _, _ -> },
    ): File = withContext(Dispatchers.IO) {
        val dir = File(context.cacheDir, "updates").also { it.mkdirs() }
        val dest = File(dir, "diarch-update.apk")
        val tmp = File(dir, "diarch-update.apk.part")
        val req = Request.Builder().url(update.apkUrl).get().build()
        http.newCall(req).execute().use { resp ->
            if (!resp.isSuccessful) {
                throw IOException("Download failed (HTTP ${resp.code})")
            }
            val body = resp.body ?: throw IOException("Empty APK response")
            val total = body.contentLength()
            body.byteStream().use { input ->
                tmp.outputStream().use { output ->
                    val buf = ByteArray(64 * 1024)
                    var readTotal = 0L
                    while (true) {
                        val n = input.read(buf)
                        if (n <= 0) break
                        output.write(buf, 0, n)
                        readTotal += n
                        onProgress(readTotal, total)
                    }
                }
            }
        }
        if (dest.exists()) dest.delete()
        if (!tmp.renameTo(dest)) {
            tmp.copyTo(dest, overwrite = true)
            tmp.delete()
        }
        dest
    }

    fun canInstallPackages(): Boolean =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            context.packageManager.canRequestPackageInstalls()
        } else {
            true
        }

    fun installPermissionSettingsIntent(): Intent =
        Intent(
            Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
            Uri.parse("package:${context.packageName}"),
        )

    fun installApk(apk: File) {
        val uri = FileProvider.getUriForFile(
            context,
            "${context.packageName}.fileprovider",
            apk,
        )
        val intent = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(uri, "application/vnd.android.package-archive")
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        // Grant to resolvers so the system package installer can read the APK.
        val resInfo = context.packageManager.queryIntentActivities(
            intent,
            PackageManager.MATCH_DEFAULT_ONLY,
        )
        for (info in resInfo) {
            context.grantUriPermission(
                info.activityInfo.packageName,
                uri,
                Intent.FLAG_GRANT_READ_URI_PERMISSION,
            )
        }
        context.startActivity(intent)
    }

    private fun fetchLatestRelease(): ForgejoRelease? {
        val req = Request.Builder()
            .url(releasesApiUrl)
            .header("Accept", "application/json")
            .get()
            .build()
        http.newCall(req).execute().use { resp ->
            if (resp.code == 404) return null
            if (!resp.isSuccessful) {
                throw IOException("Update check failed (HTTP ${resp.code})")
            }
            val body = resp.body?.string() ?: return null
            return json.decodeFromString(ForgejoRelease.serializer(), body)
        }
    }

    private fun isNewer(release: ForgejoRelease): Boolean {
        val remoteCode = parseVersionCode(release)
        if (remoteCode != null) {
            return remoteCode > localVersionCode()
        }
        val remoteName = parseVersionName(release.tagName) ?: parseVersionName(release.name)
            ?: return false
        return compareSemver(remoteName, localVersionName()) > 0
    }

    private fun displayVersion(release: ForgejoRelease): String {
        val code = parseVersionCode(release)
        val name = parseVersionName(release.tagName)
            ?: parseVersionName(release.name)
            ?: release.tagName
        return if (code != null) "$name ($code)" else name
    }

    private fun absolutize(url: String): String {
        if (url.startsWith("http://") || url.startsWith("https://")) return url
        return baseUrl.trimEnd('/') + "/" + url.trimStart('/')
    }

    companion object {
        private const val PREFS = "diarch_updates"
        private const val KEY_DISMISSED_TAG = "dismissed_tag"

        fun pickApkAsset(assets: List<ForgejoAsset>): ForgejoAsset? {
            val apks = assets.filter { it.name.endsWith(".apk", ignoreCase = true) }
            if (apks.isEmpty()) return null
            return apks.firstOrNull { it.name.contains("arm64", ignoreCase = true) }
                ?: apks.firstOrNull { it.name.contains("diarch", ignoreCase = true) }
                ?: apks.first()
        }

        /** `versionCode: 12` anywhere in the release body. */
        fun parseVersionCode(release: ForgejoRelease): Int? {
            val bodyMatch = Regex("""(?im)^\s*versionCode\s*[:=]\s*(\d+)\s*$""")
                .find(release.body)
                ?.groupValues
                ?.getOrNull(1)
                ?.toIntOrNull()
            if (bodyMatch != null) return bodyMatch
            // Tags like android-code-12 or android-12 (integer only after prefix)
            val tag = release.tagName.trim()
            Regex("""(?i)^android-(?:code-)?(\d+)$""").find(tag)?.groupValues?.getOrNull(1)
                ?.toIntOrNull()
                ?.let { return it }
            return null
        }

        fun parseVersionName(raw: String): String? {
            val s = raw.trim()
            if (s.isEmpty()) return null
            val m = Regex(
                """(?i)(?:^|[\s/])(?:android-)?v?(\d+\.\d+\.\d+)(?:$|[^\d.])""",
            ).find(s)
            if (m != null) return m.groupValues[1]
            // Bare semver tag
            if (Regex("""^\d+\.\d+\.\d+$""").matches(s)) return s
            if (Regex("""^v\d+\.\d+\.\d+$""", RegexOption.IGNORE_CASE).matches(s)) {
                return s.drop(1)
            }
            return null
        }

        /** Positive if a > b. */
        fun compareSemver(a: String, b: String): Int {
            fun parts(v: String) = v.split('.').map { it.toIntOrNull() ?: 0 }
            val pa = parts(a)
            val pb = parts(b)
            val n = maxOf(pa.size, pb.size)
            for (i in 0 until n) {
                val da = pa.getOrElse(i) { 0 }
                val db = pb.getOrElse(i) { 0 }
                if (da != db) return da.compareTo(db)
            }
            return 0
        }
    }
}
