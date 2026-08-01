//! Audiobook helpers: Audible AAX → M4B (ffmpeg) and chapter listing (ffprobe).
//!
//! Conversion flags match ~/Projects/audible2m4b README (copy codecs, preserve chapters).

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// Canonical filename under `{work}/audio/`.
pub const BOOK_M4B: &str = "book.m4b";

#[derive(Debug, Clone, Serialize)]
pub struct AudioChapter {
    pub index: usize,
    pub title: String,
    pub start: f64,
    pub end: Option<f64>,
}

pub fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn ffprobe_available() -> bool {
    std::process::Command::new("ffprobe")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Tail of ffmpeg stderr for errors (skip progress spam; prefer end over header dump).
fn ffmpeg_error_snippet(stderr: &str, activation_bytes: &str) -> String {
    let scrubbed = stderr.replace(activation_bytes, "[redacted]");
    let lines: Vec<&str> = scrubbed
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("frame=") || t.starts_with("size=") || t.is_empty())
        })
        .collect();
    let joined = if lines.len() > 40 {
        lines[lines.len().saturating_sub(40)..].join("\n")
    } else {
        lines.join("\n")
    };
    let chars: Vec<char> = joined.chars().collect();
    if chars.len() <= 900 {
        joined
    } else {
        chars[chars.len() - 900..].iter().collect()
    }
}

fn free_bytes_for(path: &Path) -> Option<u64> {
    let check = if path.exists() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| Path::new(".").to_path_buf())
    };
    let out = std::process::Command::new("df")
        .args(["-B1", "--output=avail"])
        .arg(&check)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse::<u64>().ok())
        .next_back()
}

/// Convert Audible AAX → DRM-free M4B (stream copy, chapters preserved).
pub async fn aax_to_m4b(src: &Path, dest: &Path, activation_bytes: &str) -> Result<()> {
    if activation_bytes.is_empty() {
        bail!("DIARCH_AUDIBLE_KEY is not set (Audible activation bytes required)");
    }
    if !ffmpeg_available() {
        bail!("ffmpeg not found on PATH (required for AAX → M4B)");
    }
    if !src.exists() {
        bail!("AAX source missing: {}", src.display());
    }
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let src_len = tokio::fs::metadata(src).await?.len();
    // faststart rewrites the file: need ~2× output (~same as AAX) free, plus headroom.
    let need = src_len.saturating_mul(2).saturating_add(64 * 1024 * 1024);
    if let Some(avail) = free_bytes_for(dest.parent().unwrap_or(dest)) {
        if avail < need {
            bail!(
                "not enough free disk for AAX→M4B (need ~{} MiB, have {} MiB). Free space and retry.",
                need / (1024 * 1024),
                avail / (1024 * 1024)
            );
        }
    }

    // Temp must keep a .m4b extension so ffmpeg can pick the ipod/mp4 muxer
    // (`book.m4b.partial` fails with "Unable to choose an output format").
    let tmp = {
        let stem = dest
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("book");
        dest.with_file_name(format!("{stem}.partial.m4b"))
    };
    let _ = tokio::fs::remove_file(&tmp).await;

    let output = Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-y")
        .arg("-activation_bytes")
        .arg(activation_bytes)
        .arg("-i")
        .arg(src)
        .args([
            "-map",
            "0:a",
            "-map",
            "0:v?",
            "-map_metadata",
            "0",
            "-map_chapters",
            "0",
            "-c",
            "copy",
            "-movflags",
            "faststart",
            "-f",
            "ipod",
        ])
        .arg(&tmp)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("failed to spawn ffmpeg")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let _ = tokio::fs::remove_file(&tmp).await;
        let snippet = ffmpeg_error_snippet(&err, activation_bytes);
        let disk_hint = free_bytes_for(dest.parent().unwrap_or(dest))
            .map(|a| format!("; free disk {} MiB", a / (1024 * 1024)))
            .unwrap_or_default();
        bail!(
            "ffmpeg AAX→M4B failed (exit {:?}){}: {}",
            output.status.code(),
            disk_hint,
            snippet
        );
    }

    tokio::fs::rename(&tmp, dest)
        .await
        .with_context(|| format!("rename {} → {}", tmp.display(), dest.display()))?;
    Ok(())
}

/// List chapters from an M4B via ffprobe. Empty if no chapters or ffprobe missing.
pub async fn ffprobe_chapters(path: &Path) -> Result<Vec<AudioChapter>> {
    if !path.exists() {
        return Ok(vec![]);
    }
    if !ffprobe_available() {
        return Ok(vec![]);
    }
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_chapters",
            "-show_entries",
            "format=duration",
        ])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("failed to spawn ffprobe")?;

    if !output.status.success() {
        return Ok(vec![]);
    }
    let v: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    let duration = v
        .pointer("/format/duration")
        .and_then(|d| d.as_str())
        .and_then(|s| s.parse::<f64>().ok());
    let Some(arr) = v.get("chapters").and_then(|c| c.as_array()) else {
        return Ok(vec![]);
    };
    let mut chapters = Vec::with_capacity(arr.len());
    for (i, ch) in arr.iter().enumerate() {
        let start = ch
            .get("start_time")
            .and_then(|s| s.as_str())
            .and_then(|s| s.parse().ok())
            .or_else(|| ch.get("start").and_then(|s| s.as_f64()))
            .unwrap_or(0.0);
        let end = if i + 1 < arr.len() {
            arr[i + 1]
                .get("start_time")
                .and_then(|s| s.as_str())
                .and_then(|s| s.parse().ok())
                .or_else(|| arr[i + 1].get("start").and_then(|s| s.as_f64()))
        } else {
            duration
        };
        let title = ch
            .pointer("/tags/title")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("Chapter {}", i + 1));
        chapters.push(AudioChapter {
            index: i + 1,
            title,
            start,
            end,
        });
    }
    Ok(chapters)
}

pub fn require_activation_bytes(key: Option<&str>) -> Result<&str> {
    key.filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("DIARCH_AUDIBLE_KEY is not set (Audible activation bytes required)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn book_m4b_name() {
        assert_eq!(BOOK_M4B, "book.m4b");
    }

    #[test]
    fn partial_temp_keeps_m4b_extension() {
        let dest = Path::new("/tmp/audio/book.m4b");
        let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap();
        let tmp = dest.with_file_name(format!("{stem}.partial.m4b"));
        assert_eq!(tmp.extension().and_then(|e| e.to_str()), Some("m4b"));
        assert_ne!(tmp, dest);
    }

    #[test]
    fn error_snippet_prefers_tail() {
        let mut s = String::from("header checksum line\n");
        for i in 0..60 {
            s.push_str(&format!("Chapter detail {i}\n"));
        }
        s.push_str("No space left on device\n");
        let snip = ffmpeg_error_snippet(&s, "deadbeef");
        assert!(snip.contains("No space left on device"), "{snip}");
        assert!(!snip.contains("header checksum"));
    }
}
