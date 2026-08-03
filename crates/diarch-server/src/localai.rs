//! LocalAI / Hermes helpers: cover image generation and audiobook transcription.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tracing::warn;

use crate::state::AppState;

/// HTTP client for slow LocalAI jobs (image gen / ASR).
pub fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .connect_timeout(Duration::from_secs(20))
        .build()
        .context("build LocalAI HTTP client")
}

fn base_url(state: &AppState) -> Result<&str> {
    state
        .config
        .localai_url
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("DIARCH_LOCALAI_URL not set"))
}

/// Probe LocalAI: prefer `/v1/models`, fall back to `/readyz`.
pub async fn probe(state: &AppState) -> (String, Option<String>, bool) {
    let Some(base) = state.config.localai_url.as_ref() else {
        return (
            "degraded".into(),
            Some("DIARCH_LOCALAI_URL not set".into()),
            false,
        );
    };
    let base = base.trim_end_matches('/');
    let http = match client() {
        Ok(c) => c,
        Err(e) => return ("broken".into(), Some(e.to_string()), false),
    };

    let models_url = format!("{base}/v1/models");
    match http.get(&models_url).send().await {
        Ok(r) if r.status().is_success() => {
            let hint = match r.json::<Value>().await {
                Ok(v) => {
                    let data = v.get("data").and_then(|d| d.as_array());
                    let n = data.map(|a| a.len()).unwrap_or(0);
                    let ids: Vec<&str> = data
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| m.get("id").and_then(|i| i.as_str()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let img = state
                        .config
                        .localai_image_model
                        .as_deref()
                        .unwrap_or("?");
                    let asr = state
                        .config
                        .localai_transcribe_model
                        .as_deref()
                        .unwrap_or("?");
                    let img_ok = img == "?" || ids.iter().any(|id| *id == img || id.contains(img));
                    let asr_ok = asr == "?" || ids.iter().any(|id| *id == asr || id.contains(asr));
                    let mut parts = vec![format!("{n} models"), format!("image={img}"), format!("asr={asr}")];
                    if !img_ok {
                        parts.push(format!("missing image model '{img}'"));
                    }
                    if !asr_ok {
                        parts.push(format!("missing ASR model '{asr}'"));
                    }
                    let detail = parts.join(" · ");
                    if img_ok && asr_ok {
                        Some(detail)
                    } else {
                        // Still reachable; mark degraded via status below by returning detail
                        // with a sentinel prefix checked by caller — return as Ok with note.
                        Some(detail)
                    }
                }
                Err(_) => None,
            };
            let missing = hint
                .as_deref()
                .map(|h| h.contains("missing "))
                .unwrap_or(false);
            if missing {
                return ("degraded".into(), hint, true);
            }
            return ("ok".into(), hint, true);
        }
        Ok(r) => {
            let status = r.status();
            // Fall through to readyz
            warn!(%status, "LocalAI /v1/models not ok; trying /readyz");
        }
        Err(e) => {
            return (
                "broken".into(),
                Some(format!("unreachable: {e}")),
                false,
            );
        }
    }

    let readyz = format!("{base}/readyz");
    match http.get(&readyz).send().await {
        Ok(r) if r.status().is_success() => (
            "ok".into(),
            Some("readyz ok (/v1/models unavailable)".into()),
            true,
        ),
        Ok(r) => (
            "degraded".into(),
            Some(format!("HTTP {} from /readyz", r.status())),
            false,
        ),
        Err(e) => ("broken".into(), Some(e.to_string()), false),
    }
}

pub async fn generate_cover_bytes(state: &AppState, prompt: &str) -> Result<Vec<u8>> {
    // Optional Hermes agent shim.
    if let Some(hermes) = state.config.hermes_url.as_ref() {
        let http = client()?;
        let body = serde_json::json!({
            "task": "generate_book_cover",
            "prompt": prompt
        });
        let resp = http
            .post(format!("{}/cover", hermes.trim_end_matches('/')))
            .json(&body)
            .send()
            .await
            .context("Hermes cover request")?;
        if resp.status().is_success() {
            return Ok(resp.bytes().await?.to_vec());
        }
        warn!(status = %resp.status(), "Hermes cover failed; falling back to LocalAI");
    }

    let base = base_url(state)?;
    let model = state
        .config
        .localai_image_model
        .as_deref()
        .unwrap_or("flux.2-klein-4b");
    let size = state
        .config
        .localai_image_size
        .as_deref()
        .unwrap_or("512x512");
    let http = client()?;
    let url = format!("{}/v1/images/generations", base.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "size": size,
        "n": 1,
        "response_format": "b64_json"
    });
    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("LocalAI image generation")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let hint = backend_oom_hint(&http, base, model).await;
        bail!(
            "LocalAI image gen HTTP {status}: {}{}",
            truncate(&text, 280),
            hint
        );
    }
    let v: Value = resp.json().await.context("parse LocalAI image response")?;
    if let Some(b64) = v.pointer("/data/0/b64_json").and_then(|x| x.as_str()) {
        use base64::Engine as _;
        return base64::engine::general_purpose::STANDARD
            .decode(b64)
            .context("decode b64_json cover");
    }
    if let Some(img_url) = v.pointer("/data/0/url").and_then(|x| x.as_str()) {
        let img = http
            .get(img_url)
            .send()
            .await
            .context("fetch generated cover URL")?;
        if !img.status().is_success() {
            bail!("cover URL fetch HTTP {}", img.status());
        }
        return Ok(img.bytes().await?.to_vec());
    }
    bail!("LocalAI image response missing b64_json/url");
}

/// Pull recent LocalAI backend logs and, if VRAM/OOM shows up, return a short hint.
async fn backend_oom_hint(http: &reqwest::Client, base: &str, model: &str) -> String {
    let url = format!(
        "{}/api/backend-logs/{}",
        base.trim_end_matches('/'),
        model
    );
    let Ok(resp) = http.get(&url).send().await else {
        return String::new();
    };
    if !resp.status().is_success() {
        return String::new();
    }
    let Ok(v) = resp.json::<Value>().await else {
        return String::new();
    };
    let blob = v.to_string().to_lowercase();
    if blob.contains("out of memory")
        || blob.contains("cudamalloc failed")
        || blob.contains("failed to allocate")
    {
        return " — LocalAI GPU out of VRAM (unload other models in the LocalAI UI, or set DIARCH_LOCALAI_IMAGE_MODEL=sd-1.5-ggml / DIARCH_LOCALAI_IMAGE_SIZE=256x384)".into();
    }
    if blob.contains("eof") || blob.contains("connection refused") {
        return " — LocalAI image backend crashed; check GPU VRAM and retry after unloading other models".into();
    }
    String::new()
}

const TRANSCRIPT_CHUNK_SECS: f64 = 600.0; // 10 minutes

/// Marker placed at the top of markdown produced from ASR so we can safely refresh it.
pub const TRANSCRIPT_MD_MARKER: &str = "<!-- diarch:source=transcript -->";

/// Format raw ASR text as Markdown for `book.md`.
pub fn transcript_to_markdown(title: &str, authors: &str, body: &str) -> String {
    let mut out = String::new();
    out.push_str(TRANSCRIPT_MD_MARKER);
    out.push('\n');
    out.push_str(&format!("# {}\n\n", title.trim()));
    if !authors.trim().is_empty() {
        out.push_str(&format!("*By {}*\n\n", authors.trim()));
    }
    out.push_str("*Transcribed from audiobook.*\n\n---\n\n");
    let cleaned = body
        .replace('\r', "")
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n");
    let mut prev_blank = false;
    for line in cleaned.lines() {
        let blank = line.is_empty();
        if blank && prev_blank {
            continue;
        }
        out.push_str(line);
        out.push('\n');
        prev_blank = blank;
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Write `book.md` from `transcript.txt` when missing or previously transcript-sourced.
pub async fn install_transcript_as_markdown(
    work_dir: &Path,
    title: &str,
    authors: &str,
) -> Result<bool> {
    let transcript = work_dir.join("transcript.txt");
    if !transcript.exists() {
        return Ok(false);
    }
    let md_path = work_dir.join("book.md");
    if md_path.exists() {
        let existing = tokio::fs::read_to_string(&md_path).await.unwrap_or_default();
        if !existing.contains(TRANSCRIPT_MD_MARKER) {
            return Ok(false);
        }
    }
    let body = tokio::fs::read_to_string(&transcript).await?;
    let md = transcript_to_markdown(title, authors, &body);
    tokio::fs::write(&md_path, md.as_bytes()).await?;
    Ok(true)
}

/// Transcribe `book.m4b` via LocalAI, writing `transcript.txt` under the work dir.
/// Optional `job_id` receives live `transcribing chunk N/M` detail updates.
pub async fn transcribe_audiobook(
    state: &AppState,
    work_dir: &Path,
    job_id: Option<uuid::Uuid>,
) -> Result<String> {
    let base = base_url(state)?;
    let model = state
        .config
        .localai_transcribe_model
        .as_deref()
        .unwrap_or("whisper-base");
    let m4b = work_dir.join("audio").join(crate::audio::BOOK_M4B);
    if !m4b.exists() {
        bail!("book.m4b missing");
    }
    let duration = audio_duration_secs(&m4b).await?;
    let total_chunks = ((duration / TRANSCRIPT_CHUNK_SECS).ceil() as usize).max(1);
    let http = client()?;
    let base = base.trim_end_matches('/');

    let tmp = work_dir.join(".transcribe_tmp");
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    tokio::fs::create_dir_all(&tmp).await?;

    let mut parts: Vec<String> = Vec::new();
    let mut start = 0.0_f64;
    let mut idx = 0_usize;
    while start < duration {
        if let Some(jid) = job_id {
            let _ = state
                .db
                .update_job(
                    jid,
                    "running",
                    Some(&format!("transcribing chunk {}/{}", idx + 1, total_chunks)),
                )
                .await;
        }
        let chunk = tmp.join(format!("chunk_{idx:04}.mp3"));
        extract_chunk(&m4b, start, TRANSCRIPT_CHUNK_SECS, &chunk).await?;
        let text = transcribe_file(&http, base, model, &chunk).await?;
        if !text.trim().is_empty() {
            parts.push(text.trim().to_string());
        }
        let _ = tokio::fs::remove_file(&chunk).await;
        start += TRANSCRIPT_CHUNK_SECS;
        idx += 1;
    }

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    let joined = parts.join("\n\n");
    let dest = work_dir.join("transcript.txt");
    tokio::fs::write(&dest, &joined).await?;
    Ok(format!(
        "transcribed {:.0}s in {idx} chunk(s) → transcript.txt ({} chars)",
        duration,
        joined.len()
    ))
}

async fn audio_duration_secs(path: &Path) -> Result<f64> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("ffprobe duration")?;
    if !out.status.success() {
        bail!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    s.parse::<f64>()
        .with_context(|| format!("parse duration `{s}`"))
}

async fn extract_chunk(src: &Path, start: f64, len: f64, dest: &Path) -> Result<()> {
    let out = Command::new("ffmpeg")
        .args([
            "-y",
            "-ss",
            &format!("{start:.3}"),
            "-t",
            &format!("{len:.3}"),
            "-i",
        ])
        .arg(src)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-b:a", "64k"])
        .arg(dest)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("ffmpeg chunk extract")?;
    if !out.status.success() {
        bail!(
            "ffmpeg chunk failed: {}",
            truncate(&String::from_utf8_lossy(&out.stderr), 400)
        );
    }
    if !dest.exists() {
        bail!("ffmpeg produced no chunk at {}", dest.display());
    }
    Ok(())
}

async fn transcribe_file(
    http: &reqwest::Client,
    base: &str,
    model: &str,
    path: &Path,
) -> Result<String> {
    let bytes = tokio::fs::read(path).await?;
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("chunk.mp3")
        .to_string();
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(filename)
        .mime_str("audio/mpeg")
        .context("chunk mime")?;
    let form = reqwest::multipart::Form::new()
        .text("model", model.to_string())
        .part("file", part);
    let url = format!(
        "{}/v1/audio/transcriptions",
        base.trim_end_matches('/')
    );
    let resp = http
        .post(&url)
        .multipart(form)
        .send()
        .await
        .context("LocalAI transcription request")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let hint = asr_backend_hint(http, base, model).await;
        bail!(
            "transcription HTTP {status}: {}{}",
            truncate(&text, 280),
            hint
        );
    }
    let v: Value = resp.json().await.context("parse transcription JSON")?;
    if let Some(t) = v.get("text").and_then(|x| x.as_str()) {
        return Ok(t.to_string());
    }
    // Some backends return plain string
    if let Some(t) = v.as_str() {
        return Ok(t.to_string());
    }
    Ok(String::new())
}

/// Pull ASR backend logs and return an actionable hint when the loader is broken.
async fn asr_backend_hint(http: &reqwest::Client, base: &str, model: &str) -> String {
    let url = format!(
        "{}/api/backend-logs/{}",
        base.trim_end_matches('/'),
        model
    );
    let Ok(resp) = http.get(&url).send().await else {
        return String::new();
    };
    if !resp.status().is_success() {
        return String::new();
    }
    let Ok(v) = resp.json::<Value>().await else {
        return String::new();
    };
    let blob = v.to_string();
    let lower = blob.to_lowercase();
    if lower.contains("getpwuid") || lower.contains("uid not found") {
        return " — LocalAI NeMo/PyTorch can’t resolve container UID (TrueNAS often runs as uid 568). In the LocalAI app env set USER=localai and HOME=/tmp (and ideally TORCHINDUCTOR_CACHE_DIR=/tmp/torch), then restart. Or install whisper-base and set DIARCH_LOCALAI_TRANSCRIBE_MODEL=whisper-base".into();
    }
    if lower.contains("cannot open shared object") || lower.contains("no such file or directory") {
        return " — LocalAI ASR backend binary/library missing; install a whisper-* model from the gallery and point DIARCH_LOCALAI_TRANSCRIBE_MODEL at it".into();
    }
    if lower.contains("out of memory") || lower.contains("cudamalloc failed") {
        return " — LocalAI GPU out of VRAM; unload other models before transcribing".into();
    }
    if lower.contains("grpc service not ready") || lower.contains("not ready") {
        return " — LocalAI ASR backend failed to start; check backend logs for that model in the LocalAI UI".into();
    }
    String::new()
}

fn truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        chars[..max].iter().collect::<String>() + "…"
    }
}

#[allow(dead_code)]
pub fn transcript_path(work_dir: &Path) -> PathBuf {
    work_dir.join("transcript.txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_markdown_includes_marker_and_title() {
        let md = transcript_to_markdown("Lolcows", "Author", "Hello world.\n\nSecond para.");
        assert!(md.starts_with(TRANSCRIPT_MD_MARKER));
        assert!(md.contains("# Lolcows"));
        assert!(md.contains("*By Author*"));
        assert!(md.contains("Hello world."));
        assert!(md.contains("Second para."));
    }
}
