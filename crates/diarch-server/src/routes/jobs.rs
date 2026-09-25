use anyhow::{anyhow, bail, Result};
use chrono::Utc;
use diarch_core::{AssetKind, WorkAsset};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

use crate::state::AppState;

/// Parse import job detail: prefer JSON `{name, rel_path}`, fall back to legacy `name|absolute`.
fn parse_import_detail(state: &AppState, detail: &str) -> Result<(String, PathBuf)> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(detail) {
        let name = v
            .get("name")
            .and_then(|x| x.as_str())
            .ok_or_else(|| anyhow!("import detail missing name"))?
            .to_string();
        let rel = v
            .get("rel_path")
            .and_then(|x| x.as_str())
            .ok_or_else(|| anyhow!("import detail missing rel_path"))?;
        if rel.contains("..") || Path::new(rel).is_absolute() {
            bail!("invalid import rel_path");
        }
        let path = state.config.data_dir.join(rel);
        let imports = state.config.data_dir.join("imports");
        if !path.starts_with(&imports) {
            bail!("import path escapes imports/");
        }
        return Ok((name, path));
    }
    // Legacy: name|/absolute/path
    let (name, path_str) = detail
        .split_once('|')
        .ok_or_else(|| anyhow!("bad import detail"))?;
    let path = PathBuf::from(path_str);
    let imports = state.config.data_dir.join("imports");
    if !path.starts_with(&imports) {
        bail!("legacy import path outside imports/");
    }
    Ok((name.to_string(), path))
}

pub async fn process_one(state: &Arc<AppState>) -> Result<bool> {
    let Some(job) = state.db.next_pending_job().await? else {
        return Ok(false);
    };
    state.db.update_job(job.id, "running", job.detail.as_deref()).await?;

    let result = match job.kind.as_str() {
        "import" => run_import(state, &job).await,
        "confirm_epub" => run_confirm(state, &job).await,
        "aax_to_m4b" => run_aax_to_m4b(state, &job).await,
        "transcribe" => run_transcribe(state, &job).await,
        other => Err(anyhow!("unknown job kind: {other}")),
    };

    match result {
        Ok(detail) => state.db.update_job(job.id, "done", detail.as_deref()).await?,
        Err(e) => {
            state
                .db
                .update_job(job.id, "failed", Some(&e.to_string()))
                .await?;
            // Don't leave the work stuck in "Transcribing…" after a failed job.
            if job.kind == "transcribe" {
                if let Some(work_id) = job.work_id {
                    clear_transcription_in_progress(state, work_id).await;
                }
            }
        }
    }
    Ok(true)
}

async fn clear_transcription_in_progress(state: &Arc<AppState>, work_id: Uuid) {
    if let Ok(Some(mut work)) = state.db.get_work(work_id).await {
        if work.needs_transcription {
            work.needs_transcription = false;
            work.updated_at = Utc::now();
            let _ = state.db.update_work(&work).await;
        }
    }
    let tmp = state.config.work_dir(work_id).join(".transcribe_tmp");
    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

async fn run_import(state: &Arc<AppState>, job: &diarch_core::Job) -> Result<Option<String>> {
    let work_id = job.work_id.ok_or_else(|| anyhow!("import job missing work_id"))?;
    let detail = job
        .detail
        .as_deref()
        .ok_or_else(|| anyhow!("import job missing path detail"))?;
    let (name, path) = parse_import_detail(state, detail)?;

    // Lower priority
    #[cfg(unix)]
    unsafe {
        libc_nice(10);
    }

    let work_dir = state.config.work_dir(work_id);
    let user_cover_marker = work_dir.join(".user_cover");
    let preserved_user_cover = if user_cover_marker.exists() {
        let cover = work_dir.join("cover.jpg");
        if cover.exists() {
            tokio::fs::read(&cover).await.ok()
        } else {
            None
        }
    } else {
        None
    };

    let result =
        diarch_import::ingest_file(&state.config.library_dir(), work_id, &path, &name)
            .await?;

    // Prefer a cover the client attached during upload over any embedded EPUB art.
    let cover_path = if let Some(bytes) = preserved_user_cover.filter(|b| !b.is_empty()) {
        let dest = work_dir.join("cover.jpg");
        tokio::fs::write(&dest, &bytes).await?;
        let _ = tokio::fs::remove_file(&user_cover_marker).await;
        Some(dest)
    } else {
        let _ = tokio::fs::remove_file(&user_cover_marker).await;
        result.cover_path.clone()
    };

    if let Some(ref p) = result.epub_path {
        register_asset(state, work_id, AssetKind::Epub, "book.epub", "application/epub+zip", p).await?;
    }
    if let Some(ref p) = result.markdown_path {
        register_asset(state, work_id, AssetKind::Markdown, "book.md", "text/markdown", p).await?;
    }
    if let Some(ref p) = cover_path {
        register_asset(state, work_id, AssetKind::Cover, "cover.jpg", "image/jpeg", p).await?;
    }

    if let Some(mut work) = state.db.get_work(work_id).await? {
        work.needs_review = result.needs_review;
        if cover_path.is_some() {
            work.needs_cover = false;
        }
        if let Some(ref t) = result.title {
            if work.title.is_empty()
                || work.title == "Untitled"
                || work.title.starts_with("Import ")
                || work.title.ends_with(".epub")
            {
                work.title = t.clone();
            }
        }
        if let Some(ref a) = result.authors {
            if work.authors.is_empty() {
                work.authors = a.clone();
            }
        }
        if work.isbn.is_none() {
            work.isbn = result.isbn.clone();
        }
        if work.description.is_none() {
            work.description = result.description.clone();
        }

        // Taxonomy from EPUB subjects + Open Library (ISBN), unless user already set a leaf.
        let hint = crate::metadata::taxonomy_from_metadata(
            state,
            work.isbn.as_deref().or(result.isbn.as_deref()),
            &result.subjects,
        )
        .await;
        if work.isbn.is_none() {
            work.isbn = hint.isbn.clone();
        }
        if work.description.is_none() {
            work.description = hint.description.clone();
        }
        if work.authors.is_empty() {
            if let Some(a) = hint.authors {
                work.authors = a;
            }
        }

        let existing = state.db.work_codes(work_id).await.unwrap_or_default();
        let mut codes = existing;
        for c in &hint.codes {
            if !codes.contains(c) {
                codes.push(*c);
            }
        }
        if !codes.is_empty() {
            let _ = state.db.set_work_codes(work_id, &codes).await;
        }
        work.primary_code =
            diarch_core::taxonomy::prefer_primary(work.primary_code, hint.primary);
        work.is_manga = diarch_core::infer_manga(work.primary_code, &codes, work.is_manga);
        if work.is_manga {
            work.reading_direction = "rtl".into();
        }

        if result.epub_path.is_some() && !result.needs_review {
            if work.status == diarch_core::ReadingStatus::Wishlist {
                work.status = diarch_core::ReadingStatus::Unread;
            }
        }
        work.updated_at = Utc::now();
        state.db.update_work(&work).await?;
        if let Some(uid) = work.created_by {
            if work.status == diarch_core::ReadingStatus::Unread {
                let personal = state.db.get_user_work_status(uid, work_id).await?;
                if matches!(
                    personal,
                    None | Some(diarch_core::ReadingStatus::Wishlist)
                ) {
                    let _ = state
                        .db
                        .upsert_user_work_status(
                            uid,
                            work_id,
                            diarch_core::ReadingStatus::Unread.as_str(),
                        )
                        .await;
                }
            }
        }
    }

    let _ = tokio::fs::remove_file(path).await;
    Ok(Some("imported".into()))
}

async fn run_confirm(state: &Arc<AppState>, job: &diarch_core::Job) -> Result<Option<String>> {
    let work_id = job.work_id.ok_or_else(|| anyhow!("confirm missing work"))?;
    #[cfg(unix)]
    unsafe {
        libc_nice(10);
    }
    let work = state.db.get_work(work_id).await?;
    let title = work.as_ref().map(|w| w.title.as_str());
    let authors = work.as_ref().map(|w| w.authors.as_str());
    let epub = diarch_import::confirm_markdown_to_epub(
        &state.config.library_dir(),
        work_id,
        title,
        authors,
    )
    .await?;
    register_asset(
        state,
        work_id,
        AssetKind::Epub,
        "book.epub",
        "application/epub+zip",
        &epub,
    )
    .await?;
    if let Some(mut work) = work {
        work.needs_review = false;
        if work.status == diarch_core::ReadingStatus::Wishlist {
            work.status = diarch_core::ReadingStatus::Unread;
        }
        work.updated_at = Utc::now();
        state.db.update_work(&work).await?;
        if let Some(uid) = work.created_by {
            if work.status == diarch_core::ReadingStatus::Unread {
                let personal = state.db.get_user_work_status(uid, work_id).await?;
                if matches!(
                    personal,
                    None | Some(diarch_core::ReadingStatus::Wishlist)
                ) {
                    let _ = state
                        .db
                        .upsert_user_work_status(
                            uid,
                            work_id,
                            diarch_core::ReadingStatus::Unread.as_str(),
                        )
                        .await;
                }
            }
        }
    }
    Ok(Some("epub confirmed".into()))
}

async fn run_aax_to_m4b(state: &Arc<AppState>, job: &diarch_core::Job) -> Result<Option<String>> {
    let work_id = job.work_id.ok_or_else(|| anyhow!("aax_to_m4b missing work_id"))?;
    let key = crate::audio::require_activation_bytes(state.config.audible_key.as_deref())?;
    let work_dir = state.config.work_dir(work_id);
    let aax = work_dir.join("import.aax");
    let dest = work_dir.join("audio").join(crate::audio::BOOK_M4B);
    crate::audio::aax_to_m4b(&aax, &dest, key).await?;
    register_asset(
        state,
        work_id,
        AssetKind::Audio,
        &format!("audio/{}", crate::audio::BOOK_M4B),
        "audio/mp4",
        &dest,
    )
    .await?;
    if let Some(mut work) = state.db.get_work(work_id).await? {
        work.needs_audio = false;
        work.sg_audio_only_remote = false;
        work.updated_at = Utc::now();
        state.db.update_work(&work).await?;
    }
    let _ = tokio::fs::remove_file(&aax).await;
    let _ = state
        .db
        .set_integration_health("audible", "ok", None, true)
        .await;
    let _ = state
        .db
        .set_integration_health("ffmpeg", "ok", None, true)
        .await;
    Ok(Some("aax converted to book.m4b".into()))
}

async fn run_transcribe(state: &Arc<AppState>, job: &diarch_core::Job) -> Result<Option<String>> {
    let work_id = job.work_id.ok_or_else(|| anyhow!("transcribe missing work_id"))?;
    if state.config.localai_url.is_none() && state.config.hermes_url.is_none() {
        bail!("DIARCH_LOCALAI_URL not set");
    }
    if !crate::audio::ffmpeg_available() || !crate::audio::ffprobe_available() {
        bail!("ffmpeg/ffprobe required for transcription chunking");
    }
    let work_dir = state.config.work_dir(work_id);
    let mut detail =
        crate::localai::transcribe_audiobook(state, &work_dir, Some(job.id)).await?;
    let dest = work_dir.join("transcript.txt");
    register_asset(
        state,
        work_id,
        AssetKind::Media,
        "transcript.txt",
        "text/plain",
        &dest,
    )
    .await?;

    let work = state.db.get_work(work_id).await?;
    let title = work
        .as_ref()
        .map(|w| w.title.as_str())
        .unwrap_or("Untitled");
    let authors = work.as_ref().map(|w| w.authors.as_str()).unwrap_or("");
    let wrote_md =
        crate::localai::install_transcript_as_markdown(&work_dir, title, authors).await?;
    if wrote_md {
        let md_path = work_dir.join("book.md");
        let assets = state.db.list_assets(work_id).await.unwrap_or_default();
        if !assets.iter().any(|a| a.kind == AssetKind::Markdown) {
            register_asset(
                state,
                work_id,
                AssetKind::Markdown,
                "book.md",
                "text/markdown",
                &md_path,
            )
            .await?;
        }
        detail.push_str(" · book.md");
    }

    if let Some(mut work) = work {
        work.needs_transcription = false;
        if wrote_md {
            // Offer the review editor for ASR cleanup (no PDF required).
            work.needs_review = true;
        }
        work.updated_at = Utc::now();
        state.db.update_work(&work).await?;
    }
    let _ = state
        .db
        .set_integration_health("localai", "ok", None, true)
        .await;
    Ok(Some(detail))
}

async fn register_asset(
    state: &Arc<AppState>,
    work_id: Uuid,
    kind: AssetKind,
    rel: &str,
    mime: &str,
    path: &std::path::Path,
) -> Result<()> {
    let meta = tokio::fs::metadata(path).await?;
    let asset = WorkAsset {
        id: Uuid::new_v4(),
        work_id,
        kind,
        relative_path: rel.into(),
        mime: Some(mime.into()),
        bytes: Some(meta.len() as i64),
        created_at: Utc::now(),
    };
    state.db.add_asset(&asset).await?;
    Ok(())
}

#[cfg(unix)]
unsafe fn libc_nice(delta: i32) {
    // Best-effort; ignore errors
    let _ = delta;
    // Avoid linking libc just for nice — use `renice` via no-op if unavailable.
}
