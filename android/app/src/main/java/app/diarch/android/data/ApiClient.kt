package app.diarch.android.data

import com.jakewharton.retrofit2.converter.kotlinx.serialization.asConverterFactory
import kotlinx.serialization.json.Json
import okhttp3.Interceptor
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.logging.HttpLoggingInterceptor
import retrofit2.Retrofit
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

class ApiClient {
    private val tokenRef = AtomicReference("")
    private val baseUrlRef = AtomicReference("")

    private val json = Json {
        ignoreUnknownKeys = true
        isLenient = true
        encodeDefaults = false
    }

    private val authInterceptor = Interceptor { chain ->
        val token = tokenRef.get()
        val req = if (token.isNotBlank()) {
            chain.request().newBuilder()
                .header("Authorization", "Bearer $token")
                .build()
        } else {
            chain.request()
        }
        chain.proceed(req)
    }

    private val logging = HttpLoggingInterceptor().apply {
        level = HttpLoggingInterceptor.Level.BASIC
    }

    private val okHttp = OkHttpClient.Builder()
        .addInterceptor(authInterceptor)
        .addInterceptor(logging)
        .connectTimeout(60, TimeUnit.SECONDS)
        .readTimeout(120, TimeUnit.SECONDS)
        .writeTimeout(120, TimeUnit.SECONDS)
        .build()

    @Volatile
    private var api: DiarchApi? = null

    fun updateSession(baseUrl: String, token: String) {
        tokenRef.set(token)
        val normalized = SessionStore.normalizeBaseUrl(baseUrl).trimEnd('/') + "/"
        if (baseUrlRef.get() != normalized) {
            baseUrlRef.set(normalized)
            api = buildApi(normalized)
        } else if (api == null && normalized.isNotBlank() && normalized != "/") {
            api = buildApi(normalized)
        }
    }

    fun api(): DiarchApi {
        return api ?: error("Server URL not configured")
    }

    /** Shared OkHttp client (already attaches the Authorization header). Used by the
     * reader WebView's shouldInterceptRequest to proxy content requests with auth. */
    fun httpClient(): OkHttpClient = okHttp

    fun coverUrl(workId: String, cacheBust: String? = null): String {
        val base = baseUrlRef.get().trimEnd('/')
        val bust = cacheBust?.let { "?v=$it" }.orEmpty()
        return "$base/api/works/$workId/cover$bust"
    }

    fun authHeader(): String? {
        val token = tokenRef.get()
        return if (token.isBlank()) null else "Bearer $token"
    }

    fun baseUrl(): String = baseUrlRef.get().trimEnd('/')

    private fun buildApi(baseUrl: String): DiarchApi {
        return Retrofit.Builder()
            .baseUrl(baseUrl)
            .client(okHttp)
            .addConverterFactory(json.asConverterFactory("application/json".toMediaType()))
            .build()
            .create(DiarchApi::class.java)
    }
}
