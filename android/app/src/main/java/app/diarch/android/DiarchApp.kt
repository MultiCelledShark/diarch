package app.diarch.android

import android.app.Application
import app.diarch.android.data.ApiClient
import app.diarch.android.data.LibraryRepository
import app.diarch.android.data.OfflineStore
import app.diarch.android.data.SessionStore
import app.diarch.android.data.UpdateChecker
import coil3.ImageLoader
import coil3.PlatformContext
import coil3.SingletonImageLoader
import coil3.network.okhttp.OkHttpNetworkFetcherFactory
import okhttp3.OkHttpClient

class DiarchApp : Application(), SingletonImageLoader.Factory {
    lateinit var sessionStore: SessionStore
        private set
    lateinit var apiClient: ApiClient
        private set
    lateinit var repository: LibraryRepository
        private set
    lateinit var offlineStore: OfflineStore
        private set
    lateinit var updateChecker: UpdateChecker
        private set

    override fun onCreate() {
        super.onCreate()
        instance = this
        sessionStore = SessionStore(this)
        apiClient = ApiClient()
        offlineStore = OfflineStore(this, apiClient)
        repository = LibraryRepository(apiClient, sessionStore, this, offlineStore)
        updateChecker = UpdateChecker(this)
    }

    override fun newImageLoader(context: PlatformContext): ImageLoader {
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
        return ImageLoader.Builder(context)
            .components {
                add(OkHttpNetworkFetcherFactory(callFactory = client))
            }
            .build()
    }

    companion object {
        lateinit var instance: DiarchApp
            private set
    }
}
