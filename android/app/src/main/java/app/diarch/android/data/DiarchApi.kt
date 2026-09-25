package app.diarch.android.data

import okhttp3.ResponseBody
import retrofit2.Response
import retrofit2.http.Body
import retrofit2.http.GET
import retrofit2.http.POST
import retrofit2.http.PUT
import retrofit2.http.Path
import retrofit2.http.Query
import retrofit2.http.Streaming

interface DiarchApi {
    @POST("api/auth/login")
    suspend fun login(@Body body: LoginRequest): LoginResponse

    @POST("api/auth/logout")
    suspend fun logout(): Response<Unit>

    @GET("api/auth/me")
    suspend fun me(): User

    @GET("api/works")
    suspend fun listWorks(
        @Query("status") status: String? = null,
        @Query("q") q: String? = null,
    ): List<Work>

    @GET("api/works/{id}")
    suspend fun getWork(@Path("id") id: String): WorkDetailResponse

    @PUT("api/works/{id}")
    suspend fun updateWork(@Path("id") id: String, @Body body: UpdateWorkRequest): Work

    @GET("api/settings")
    suspend fun getSettings(): UserSettings

    @PUT("api/settings")
    suspend fun putSettings(@Body body: UpdateSettingsRequest): Response<Unit>

    @GET("api/works/{id}/progress")
    suspend fun getProgress(
        @Path("id") id: String,
        @Query("mode") mode: String,
    ): ReadingProgress?

    @PUT("api/works/{id}/progress")
    suspend fun putProgress(@Path("id") id: String, @Body body: ProgressBody): Response<Unit>

    @Streaming
    @GET("api/works/{id}/content/epub")
    suspend fun downloadEpub(@Path("id") id: String): ResponseBody

    @Streaming
    @GET("api/works/{id}/content/markdown")
    suspend fun downloadMarkdown(@Path("id") id: String): ResponseBody

    @GET("api/works/{id}/audio/chapters")
    suspend fun audioChapters(@Path("id") id: String): AudioChaptersResponse

    @GET("api/jobs/{id}")
    suspend fun getJob(@Path("id") id: String): JobStatus

    @POST("api/wishlist")
    suspend fun addWishlist(@Body body: WishlistRequest): Work
}
