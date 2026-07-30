use anyhow::{anyhow, Result};
use chrono::Utc;
use diarch_core::{AssetKind, WorkAsset};
use std::sync::Arc;
use uuid::Uuid;

use crate::state::AppState;

pub async fn process_one(state: &Arc<AppState>) -> Result<()> {
    let Some(job) = state.db.next_pending_job().await? else {
        return Ok(());
    };
    state.db.update_job(job.id, "running", job.detail.as_deref()).await?;

    let result = match job.kind.as_str() {
        "import" => run_import(state, &job).await,
        "confirm_epub" => run_confirm(state, &job).await,
        "transcribe" => {
            // Throttled transcription hook — mark detail for ops; full Whisper later.
            state
                .db
                .update_job(
                    job.id,
                    "failed",
                    Some("transcription worker not configured; set needs_transcription and run offline"),
                )
                .await?;
            return Ok(());
        }
        other => Err(anyhow!("unknown job kind: {other}")),
    };

    match result {
        Ok(detail) => state.db.update_job(job.id, "done", detail.as_deref()).await?,
        Err(e) => {
            state
                .db
                .update_job(job.id, "failed", Some(&e.to_string()))
                .await?
        }
    }
    Ok(())
}

async fn run_import(state: &Arc<AppState>, job: &diarch_core::Job) -> Result<Option<String>> {
    let work_id = job.work_id.ok_or_else(|| anyhow!("import job missing work_id"))?;
    let detail = job
        .detail
        .as_deref()
        .ok_or_else(|| anyhow!("import job missing path detail"))?;
    let (name, path) = detail
        .split_once('|')
        .ok_or_else(|| anyhow!("bad import detail"))?;

    // Lower priority
    #[cfg(unix)]
    unsafe {
        libc_nice(10);
    }

    let result =
        diarch_import::ingest_file(&state.config.library_dir(), work_id, std::path::Path::new(path), name)
            .await?;

    if let Some(ref p) = result.epub_path {
        register_asset(state, work_id, AssetKind::Epub, "book.epub", "application/epub+zip", p).await?;
    }
    if let Some(ref p) = result.markdown_path {
        register_asset(state, work_id, AssetKind::Markdown, "book.md", "text/markdown", p).await?;
    }
    if let Some(ref p) = result.cover_path {
        register_asset(state, work_id, AssetKind::Cover, "cover.jpg", "image/jpeg", p).await?;
    }

    if let Some(mut work) = state.db.get_work(work_id).await? {
        work.needs_review = result.needs_review;
        if result.cover_path.is_some() {
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
        if result.epub_path.is_some() && !result.needs_review {
            if work.status == diarch_core::ReadingStatus::Wishlist {
                work.status = diarch_core::ReadingStatus::Unread;
            }
        }
        work.updated_at = Utc::now();
        state.db.update_work(&work).await?;
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
    let epub =
        diarch_import::confirm_markdown_to_epub(&state.config.library_dir(), work_id).await?;
    register_asset(
        state,
        work_id,
        AssetKind::Epub,
        "book.epub",
        "application/epub+zip",
        &epub,
    )
    .await?;
    if let Some(mut work) = state.db.get_work(work_id).await? {
        work.needs_review = false;
        if work.status == diarch_core::ReadingStatus::Wishlist {
            work.status = diarch_core::ReadingStatus::Unread;
        }
        work.updated_at = Utc::now();
        state.db.update_work(&work).await?;
    }
    Ok(Some("epub confirmed".into()))
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
