package app.diarch.android.data

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class User(
    val id: String,
    val username: String,
    @SerialName("is_admin") val isAdmin: Boolean = false,
    @SerialName("show_audio_gaps") val showAudioGaps: Boolean = false,
    @SerialName("created_at") val createdAt: String? = null,
)

@Serializable
data class LoginRequest(
    val username: String,
    val password: String,
)

@Serializable
data class LoginResponse(
    val token: String,
    val user: User,
)

@Serializable
data class Work(
    val id: String,
    val title: String,
    val authors: String = "",
    val isbn: String? = null,
    val description: String? = null,
    val status: String = "unread",
    @SerialName("primary_code") val primaryCode: Int? = null,
    @SerialName("year_list") val yearList: Int? = null,
    val rating: Double? = null,
    val review: String? = null,
    @SerialName("reading_direction") val readingDirection: String = "ltr",
    @SerialName("is_manga") val isManga: Boolean = false,
    @SerialName("needs_review") val needsReview: Boolean = false,
    @SerialName("needs_cover") val needsCover: Boolean = false,
    @SerialName("needs_tts") val needsTts: Boolean = false,
    @SerialName("needs_audio") val needsAudio: Boolean = false,
    @SerialName("needs_transcription") val needsTranscription: Boolean = false,
    @SerialName("sg_review_dirty") val sgReviewDirty: Boolean = false,
    @SerialName("sg_needs_add") val sgNeedsAdd: Boolean = false,
    @SerialName("sg_audio_only_remote") val sgAudioOnlyRemote: Boolean = false,
    @SerialName("sg_matched") val sgMatched: Boolean = false,
    @SerialName("sg_book_id") val sgBookId: String? = null,
    @SerialName("created_by") val createdBy: String? = null,
    @SerialName("created_at") val createdAt: String? = null,
    @SerialName("updated_at") val updatedAt: String? = null,
    @SerialName("has_epub") val hasEpub: Boolean = false,
    @SerialName("has_md") val hasMd: Boolean = false,
    @SerialName("has_audio") val hasAudio: Boolean = false,
)

@Serializable
data class WorkDetailResponse(
    val work: Work,
    val assets: List<WorkAsset> = emptyList(),
    val codes: List<Int> = emptyList(),
    @SerialName("has_epub") val hasEpub: Boolean = false,
    @SerialName("has_md") val hasMd: Boolean = false,
    @SerialName("has_audio") val hasAudio: Boolean = false,
    @SerialName("has_import_pdf") val hasImportPdf: Boolean = false,
    @SerialName("has_cover_candidate") val hasCoverCandidate: Boolean = false,
    @SerialName("has_transcript") val hasTranscript: Boolean = false,
)

@Serializable
data class WorkAsset(
    val id: String,
    @SerialName("work_id") val workId: String,
    val kind: String,
    @SerialName("relative_path") val relativePath: String,
    val mime: String? = null,
    val bytes: Long? = null,
    @SerialName("created_at") val createdAt: String? = null,
)

@Serializable
data class UpdateWorkRequest(
    val status: String? = null,
)

@Serializable
data class ReadingProgress(
    @SerialName("user_id") val userId: String? = null,
    @SerialName("work_id") val workId: String? = null,
    val mode: String,
    val position: String,
    val percent: Double,
    @SerialName("updated_at") val updatedAt: String? = null,
)

@Serializable
data class ProgressBody(
    val mode: String,
    val position: String,
    val percent: Double,
)

@Serializable
data class ReaderTypography(
    val palette: String = "dark",
    val font: String = "serif",
    val size: Int = 100,
    val lineHeight: String = "normal",
    val measure: String = "medium",
    val justify: Boolean = false,
    val letterSpacing: String = "normal",
    val paragraphSpacing: String = "normal",
    val indent: Boolean = false,
    val hyphenate: Boolean = false,
)

@Serializable
data class UserSettings(
    @SerialName("show_audio_gaps") val showAudioGaps: Boolean = false,
    @SerialName("reader_infinite_scroll") val readerInfiniteScroll: Boolean = false,
    @SerialName("reader_typography") val readerTypography: ReaderTypography = ReaderTypography(),
)

@Serializable
data class UpdateSettingsRequest(
    @SerialName("reader_infinite_scroll") val readerInfiniteScroll: Boolean? = null,
    @SerialName("reader_typography") val readerTypography: ReaderTypography? = null,
)

@Serializable
data class ImportResponse(
    val work: Work,
    @SerialName("job_id") val jobId: String? = null,
    val kind: String? = null,
)

@Serializable
data class AudioChapter(
    val index: Int,
    val title: String = "",
    val start: Double = 0.0,
    val end: Double? = null,
)

@Serializable
data class AudioChaptersResponse(
    val chapters: List<AudioChapter> = emptyList(),
)

@Serializable
data class WishlistRequest(
    val title: String? = null,
    val authors: String? = null,
    val isbn: String? = null,
)

@Serializable
data class WorkGrant(
    @SerialName("user_id") val userId: String,
    val username: String,
)

@Serializable
data class GrantRequest(
    val username: String? = null,
    @SerialName("user_id") val userId: String? = null,
)

enum class Shelf(val apiStatus: String, val label: String) {
    Reading("reading", "Currently Reading"),
    ToRead("to_read", "To Read"),
    Library("unread", "Library"),
    Wishlist("wishlist", "Wishlist"),
    Finished("read", "Finished"),
}
