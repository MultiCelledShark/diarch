package app.diarch.android

import android.app.Application
import app.diarch.android.data.ApiClient
import app.diarch.android.data.LibraryRepository
import app.diarch.android.data.OfflineStore
import app.diarch.android.data.SessionStore
import coil.ImageLoader
import coil.ImageLoaderFactory
import coil.request.ImageRequest
import okhttp3.OkHttpClient

class DiarchApp : Application(), ImageLoaderFactory {
    lateinit var sessionStore: SessionStore
        private set
    lateinit var apiClient: ApiClient
        private set
    lateinit var repository: LibraryRepository
        private set
    lateinit var offlineStore: OfflineStore
        private set

    override fun onCreate() {
        super.onCreate()
        instance = this
        sessionStore = SessionStore(this)
        apiClient = ApiClient()
        offlineStore = OfflineStore(this, apiClient)
        repository = LibraryRepository(apiClient, sessionStore, this, offlineStore)
    }

    override fun newImageLoader(): ImageLoader {
        val client = OkHttpClient.Builder()
            .addInterceptor { chain ->
                val token = apiClient.authHeader()
                val req = if (token != null) {
                    chain.request().newBuilder().header("Authorization", token).build()
                } else {
                    chain.request()
                }
                chain.proceed(req)
            }
            .build()
        return ImageLoader.Builder(this)
            .okHttpClient(client)
            .build()
    }

    companion object {
        lateinit var instance: DiarchApp
            private set
    }
}
