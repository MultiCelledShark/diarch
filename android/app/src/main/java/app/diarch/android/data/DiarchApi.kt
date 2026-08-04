package app.diarch.android.data

import okhttp3.MultipartBody
import okhttp3.RequestBody
import okhttp3.ResponseBody
import retrofit2.Response
import retrofit2.http.Body
import retrofit2.http.GET
import retrofit2.http.Multipart
import retrofit2.http.POST
import retrofit2.http.PUT
import retrofit2.http.Part
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
    suspend fun listWorks(@Query("status") status: String? = null): List<Work>

    @GET("api/works/{id}")
    suspend fun getWork(@Path("id") id: String): WorkDetailResponse

    @PUT("api/works/{id}")
    suspend fun updateWork(@Path("id") id: String, @Body body: UpdateWorkRequest): Work

    @Multipart
    @POST("api/library/import")
    suspend fun importFile(
        @Part file: MultipartBody.Part,
        @Part("title") title: RequestBody? = null,
        @Part("authors") authors: RequestBody? = null,
    ): ImportResponse

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
}
