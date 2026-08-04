package app.diarch.android.ui.reader

import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.SkipNext
import androidx.compose.material.icons.filled.SkipPrevious
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.datasource.DefaultHttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.ProgressiveMediaSource
import app.diarch.android.data.AudioChapter
import kotlinx.coroutines.delay
import kotlin.math.abs

@Composable
fun AudioPlayerBar(
    streamUrl: String,
    authHeader: String?,
    chapters: List<AudioChapter>,
    initialPositionSec: Double?,
    onChapterChanged: (AudioChapter) -> Unit,
    onProgress: (positionSec: Double, percent: Double, chapter: AudioChapter?) -> Unit,
    localUri: android.net.Uri? = null,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    var playing by remember { mutableStateOf(false) }
    var positionMs by remember { mutableLongStateOf(0L) }
    var durationMs by remember { mutableLongStateOf(0L) }
    var chapterIndex by remember { mutableIntStateOf(-1) }
    var showChapters by remember { mutableStateOf(false) }
    var scrub by remember { mutableFloatStateOf(0f) }
    var scrubbing by remember { mutableStateOf(false) }
    var speed by remember { mutableFloatStateOf(1f) }

    val player = rememberAudioPlayer(context, streamUrl, authHeader, initialPositionSec, localUri)

    DisposableEffect(player) {
        val listener = object : Player.Listener {
            override fun onIsPlayingChanged(isPlaying: Boolean) {
                playing = isPlaying
            }

            override fun onPlaybackStateChanged(playbackState: Int) {
                if (playbackState == Player.STATE_READY) {
                    durationMs = player.duration.coerceAtLeast(0L)
                }
            }
        }
        player.addListener(listener)
        onDispose {
            player.removeListener(listener)
            player.release()
        }
    }

    LaunchedEffect(player, chapters) {
        while (true) {
            positionMs = player.currentPosition.coerceAtLeast(0L)
            durationMs = player.duration.coerceAtLeast(0L)
            if (!scrubbing && durationMs > 0) {
                scrub = positionMs.toFloat() / durationMs.toFloat()
            }
            val posSec = positionMs / 1000.0
            val idx = chapterIndexFor(chapters, posSec)
            if (idx >= 0 && idx != chapterIndex) {
                chapterIndex = idx
                onChapterChanged(chapters[idx])
            }
            val chapter = chapters.getOrNull(chapterIndex)
            val percent = if (durationMs > 0) (positionMs.toDouble() / durationMs) * 100.0 else 0.0
            onProgress(posSec, percent, chapter)
            delay(500)
        }
    }

    LaunchedEffect(speed) {
        player.setPlaybackSpeed(speed)
    }

    Column(
        modifier = modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .padding(horizontal = 12.dp, vertical = 8.dp),
    ) {
        val chapterLabel = chapters.getOrNull(chapterIndex)?.title?.takeIf { it.isNotBlank() }
            ?: if (chapters.isEmpty()) "Audiobook" else "Chapter ${chapterIndex + 1}"
        Text(
            chapterLabel,
            style = MaterialTheme.typography.labelLarge,
            maxLines = 1,
        )
        Spacer(modifier = Modifier.height(4.dp))
        Slider(
            value = scrub.coerceIn(0f, 1f),
            onValueChange = {
                scrubbing = true
                scrub = it
            },
            onValueChangeFinished = {
                scrubbing = false
                if (durationMs > 0) {
                    player.seekTo((scrub * durationMs).toLong())
                }
            },
            modifier = Modifier.fillMaxWidth(),
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(formatTime(positionMs), style = MaterialTheme.typography.bodyMedium)
            Row(verticalAlignment = Alignment.CenterVertically) {
                IconButton(
                    onClick = {
                        if (chapters.isNotEmpty() && chapterIndex > 0) {
                            seekToChapter(player, chapters[chapterIndex - 1])
                        } else {
                            player.seekTo((player.currentPosition - 15_000).coerceAtLeast(0))
                        }
                    },
                ) {
                    Icon(Icons.Default.SkipPrevious, contentDescription = "Previous chapter")
                }
                IconButton(onClick = {
                    if (player.isPlaying) player.pause() else player.play()
                }) {
                    Icon(
                        if (playing) Icons.Default.Pause else Icons.Default.PlayArrow,
                        contentDescription = if (playing) "Pause" else "Play",
                    )
                }
                IconButton(
                    onClick = {
                        if (chapters.isNotEmpty() && chapterIndex < chapters.lastIndex) {
                            seekToChapter(player, chapters[chapterIndex + 1])
                        } else {
                            player.seekTo(player.currentPosition + 30_000)
                        }
                    },
                ) {
                    Icon(Icons.Default.SkipNext, contentDescription = "Next chapter")
                }
            }
            Text(formatTime(durationMs), style = MaterialTheme.typography.bodyMedium)
        }
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            TextButton(onClick = {
                speed = when {
                    speed < 1f -> 1f
                    speed < 1.25f -> 1.25f
                    speed < 1.5f -> 1.5f
                    speed < 2f -> 2f
                    else -> 0.75f
                }
            }) {
                Text("${speed}x")
            }
            if (chapters.isNotEmpty()) {
                TextButton(onClick = { showChapters = !showChapters }) {
                    Text(if (showChapters) "Hide chapters" else "Chapters (${chapters.size})")
                }
            }
        }
        if (showChapters && chapters.isNotEmpty()) {
            LazyColumn(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(160.dp),
            ) {
                itemsIndexed(chapters) { index, chapter ->
                    val selected = index == chapterIndex
                    Text(
                        text = buildString {
                            append(chapter.title.ifBlank { "Chapter ${chapter.index}" })
                            append("  ·  ")
                            append(formatTime((chapter.start * 1000).toLong()))
                        },
                        color = if (selected) {
                            MaterialTheme.colorScheme.primary
                        } else {
                            MaterialTheme.colorScheme.onSurface
                        },
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable { seekToChapter(player, chapter) }
                            .padding(vertical = 8.dp, horizontal = 4.dp),
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
            }
        }
    }
}

@Composable
private fun rememberAudioPlayer(
    context: Context,
    streamUrl: String,
    authHeader: String?,
    initialPositionSec: Double?,
    localUri: android.net.Uri?,
): ExoPlayer {
    return remember(streamUrl, authHeader, localUri) {
        ExoPlayer.Builder(context).build().apply {
            if (localUri != null) {
                setMediaItem(MediaItem.fromUri(localUri))
            } else {
                val headers = HashMap<String, String>()
                if (!authHeader.isNullOrBlank()) {
                    headers["Authorization"] = authHeader
                }
                val httpFactory = DefaultHttpDataSource.Factory()
                    .setAllowCrossProtocolRedirects(true)
                    .setDefaultRequestProperties(headers)
                val source = ProgressiveMediaSource.Factory(httpFactory)
                    .createMediaSource(MediaItem.fromUri(streamUrl))
                setMediaSource(source)
            }
            prepare()
            playWhenReady = false
            if (initialPositionSec != null && initialPositionSec > 0) {
                seekTo((initialPositionSec * 1000).toLong())
            }
        }
    }
}

private fun seekToChapter(player: ExoPlayer, chapter: AudioChapter) {
    player.seekTo((chapter.start * 1000).toLong().coerceAtLeast(0L))
    player.play()
}

private fun chapterIndexFor(chapters: List<AudioChapter>, positionSec: Double): Int {
    if (chapters.isEmpty()) return -1
    for (i in chapters.indices.reversed()) {
        if (positionSec + 0.05 >= chapters[i].start) return i
    }
    return 0
}

private fun formatTime(ms: Long): String {
    if (ms <= 0) return "0:00"
    val totalSec = ms / 1000
    val h = totalSec / 3600
    val m = (totalSec % 3600) / 60
    val s = totalSec % 60
    return if (h > 0) "%d:%02d:%02d".format(h, m, s) else "%d:%02d".format(m, s)
}

/** Prefer nearest chapter when restoring from a saved audio position. */
fun nearestChapter(chapters: List<AudioChapter>, positionSec: Double): AudioChapter? {
    if (chapters.isEmpty()) return null
    return chapters.minByOrNull { abs(it.start - positionSec) }
}
