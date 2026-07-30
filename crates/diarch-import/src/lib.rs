use anyhow::{anyhow, Context, Result};
use diarch_core::{work_cover_path, work_markdown_path, work_media_dir};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tracing::{info, warn};
use uuid::Uuid;
use zip::ZipArchive;

#[derive(Debug, Clone)]
pub struct ImportResult {
    pub epub_path: Option<PathBuf>,
    pub markdown_path: Option<PathBuf>,
    pub cover_path: Option<PathBuf>,
    pub needs_review: bool,
}

/// Convert or store an uploaded file into the work directory.
pub async fn ingest_file(
    library_root: &Path,
    work_id: Uuid,
    source_path: &Path,
    original_name: &str,
) -> Result<ImportResult> {
    let work_dir = library_root.join(work_id.to_string());
    tokio::fs::create_dir_all(&work_dir).await?;
    tokio::fs::create_dir_all(work_media_dir(library_root, work_id)).await?;

    let ext = Path::new(original_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "epub" => ingest_epub(library_root, work_id, source_path).await,
        "pdf" => ingest_pdf(library_root, work_id, source_path).await,
        "md" | "markdown" => ingest_markdown(library_root, work_id, source_path).await,
        other => Err(anyhow!("unsupported import format: {other}")),
    }
}

async fn ingest_epub(library_root: &Path, work_id: Uuid, source: &Path) -> Result<ImportResult> {
    let epub_dest = library_root.join(work_id.to_string()).join("book.epub");
    tokio::fs::copy(source, &epub_dest).await?;

    let md_dest = work_markdown_path(library_root, work_id);
    let media = work_media_dir(library_root, work_id);
    let media_arg = format!("--extract-media={}", media.display());

    let status = Command::new("pandoc")
        .arg(&epub_dest)
        .arg("-t")
        .arg("markdown")
        .arg("-o")
        .arg(&md_dest)
        .arg(&media_arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .context("run pandoc epub→md")?;

    if !status.success() {
        warn!(?work_id, "pandoc epub→md failed; keeping epub only");
    }

    let cover = extract_epub_cover(&epub_dest, &work_cover_path(library_root, work_id)).await?;

    Ok(ImportResult {
        epub_path: Some(epub_dest),
        markdown_path: if md_dest.exists() {
            Some(md_dest)
        } else {
            None
        },
        cover_path: cover,
        needs_review: false,
    })
}

async fn ingest_pdf(library_root: &Path, work_id: Uuid, source: &Path) -> Result<ImportResult> {
    let md_dest = work_markdown_path(library_root, work_id);
    let media = work_media_dir(library_root, work_id);
    let media_arg = format!("--extract-media={}", media.display());

    let status = Command::new("pandoc")
        .arg(source)
        .arg("-t")
        .arg("markdown")
        .arg("-o")
        .arg(&md_dest)
        .arg(&media_arg)
        .stdin(Stdio::null())
        .status()
        .await
        .context("run pandoc pdf→md")?;

    if !status.success() {
        return Err(anyhow!("pandoc failed converting PDF to markdown"));
    }

    info!(?work_id, "PDF converted to markdown; awaiting review before EPUB export");

    Ok(ImportResult {
        epub_path: None,
        markdown_path: Some(md_dest),
        cover_path: None,
        needs_review: true,
    })
}

async fn ingest_markdown(library_root: &Path, work_id: Uuid, source: &Path) -> Result<ImportResult> {
    let md_dest = work_markdown_path(library_root, work_id);
    tokio::fs::copy(source, &md_dest).await?;
    Ok(ImportResult {
        epub_path: None,
        markdown_path: Some(md_dest),
        cover_path: None,
        needs_review: true,
    })
}

/// After markdown is confirmed, export EPUB and drop any leftover PDF.
pub async fn confirm_markdown_to_epub(library_root: &Path, work_id: Uuid) -> Result<PathBuf> {
    let md = work_markdown_path(library_root, work_id);
    if !md.exists() {
        return Err(anyhow!("no markdown to confirm"));
    }
    let epub = library_root.join(work_id.to_string()).join("book.epub");
    let media = work_media_dir(library_root, work_id);
    let cover = work_cover_path(library_root, work_id);

    let mut cmd = Command::new("pandoc");
    cmd.arg(&md)
        .arg("-o")
        .arg(&epub)
        .arg("--to=epub3")
        .stdin(Stdio::null());
    if media.exists() {
        // resource path for images referenced from md
        cmd.arg(format!("--resource-path={}", media.display()));
    }
    if cover.exists() {
        cmd.arg(format!("--epub-cover-image={}", cover.display()));
    }

    let status = cmd.status().await.context("pandoc md→epub")?;
    if !status.success() {
        return Err(anyhow!("pandoc md→epub failed"));
    }

    // Remove any quarantine PDF if present
    let pdf = library_root.join(work_id.to_string()).join("import.pdf");
    let _ = tokio::fs::remove_file(pdf).await;

    Ok(epub)
}

async fn extract_epub_cover(epub: &Path, dest: &Path) -> Result<Option<PathBuf>> {
    let file = std::fs::File::open(epub)?;
    let mut archive = ZipArchive::new(file)?;
    let mut candidates: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        let name = archive.by_index(i)?.name().to_string();
        let lower = name.to_ascii_lowercase();
        if lower.contains("cover")
            && (lower.ends_with(".jpg")
                || lower.ends_with(".jpeg")
                || lower.ends_with(".png")
                || lower.ends_with(".webp"))
        {
            candidates.push(name);
        }
    }
    candidates.sort_by_key(|n| n.len());
    if let Some(name) = candidates.into_iter().next() {
        let mut entry = archive.by_name(&name)?;
        let mut out = std::fs::File::create(dest)?;
        std::io::copy(&mut entry, &mut out)?;
        return Ok(Some(dest.to_path_buf()));
    }
    Ok(None)
}

pub fn zip_markdown_bundle(library_root: &Path, work_id: Uuid, out: &Path) -> Result<()> {
    let md = work_markdown_path(library_root, work_id);
    let media = work_media_dir(library_root, work_id);
    let file = std::fs::File::create(out)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    if md.exists() {
        zip.start_file("book.md", opts)?;
        std::io::copy(&mut std::fs::File::open(&md)?, &mut zip)?;
    }
    if media.exists() {
        for entry in walkdir_simple(&media)? {
            let rel = entry.strip_prefix(&media).unwrap();
            let name = format!("media/{}", rel.display());
            zip.start_file(name, opts)?;
            std::io::copy(&mut std::fs::File::open(&entry)?, &mut zip)?;
        }
    }
    zip.finish()?;
    Ok(())
}

fn walkdir_simple(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    fn rec(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let p = e.path();
            if p.is_dir() {
                rec(&p, out)?;
            } else {
                out.push(p);
            }
        }
        Ok(())
    }
    rec(dir, &mut out)?;
    Ok(out)
}

/// Export EPUBs marked needs_tts into a queue folder for a desktop watcher.
/// TODO(ebook2audiobook): implement folder watcher in the ebook2audiobook project
/// that consumes `queue/needs_tts/*.epub` and drops M4A into `queue/incoming_audio/`.
pub async fn export_needs_tts_queue(library_root: &Path, queue_dir: &Path, work_ids: &[Uuid]) -> Result<usize> {
    tokio::fs::create_dir_all(queue_dir).await?;
    let mut n = 0;
    for id in work_ids {
        let src = library_root.join(id.to_string()).join("book.epub");
        if src.exists() {
            let dest = queue_dir.join(format!("{id}.epub"));
            tokio::fs::copy(&src, &dest).await?;
            n += 1;
        }
    }
    Ok(n)
}
