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
    pub title: Option<String>,
    pub authors: Option<String>,
    pub isbn: Option<String>,
    pub subjects: Vec<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct EpubMeta {
    pub title: Option<String>,
    pub authors: Option<String>,
    pub isbn: Option<String>,
    pub subjects: Vec<String>,
    pub description: Option<String>,
}

/// Read Dublin Core fields from an EPUB's package OPF.
pub fn read_epub_metadata(epub: &Path) -> Result<EpubMeta> {
    let file = std::fs::File::open(epub)?;
    let mut archive = ZipArchive::new(file)?;
    let mut opf_path: Option<String> = None;
    if let Ok(mut container) = archive.by_name("META-INF/container.xml") {
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut container, &mut xml)?;
        if let Some(start) = xml.find("full-path=\"") {
            let rest = &xml[start + 11..];
            if let Some(end) = rest.find('"') {
                opf_path = Some(rest[..end].to_string());
            }
        }
    }
    let opf_name = {
        let mut found = opf_path;
        if found.is_none() {
            for i in 0..archive.len() {
                let name = archive.by_index(i)?.name().to_string();
                if name.ends_with(".opf") {
                    found = Some(name);
                    break;
                }
            }
        }
        found.unwrap_or_else(|| "content.opf".into())
    };
    let mut entry = archive
        .by_name(&opf_name)
        .map_err(|e| anyhow!("opf open {opf_name}: {e}"))?;
    let mut opf = String::new();
    std::io::Read::read_to_string(&mut entry, &mut opf)?;

    let mut authors = Vec::new();
    let mut rest = opf.as_str();
    while let Some(idx) = rest.find("<dc:creator") {
        if let Some(t) = tag_inner(&rest[idx..], "dc:creator") {
            if !t.trim().is_empty() {
                authors.push(t.trim().to_string());
            }
        }
        rest = &rest[idx + 11..];
    }

    let mut subjects = Vec::new();
    rest = opf.as_str();
    while let Some(idx) = rest.find("<dc:subject") {
        if let Some(t) = tag_inner(&rest[idx..], "dc:subject") {
            if !t.trim().is_empty() {
                subjects.push(t.trim().to_string());
            }
        }
        rest = &rest[idx + 11..];
    }

    let mut isbn = None;
    rest = opf.as_str();
    while let Some(idx) = rest.find("<dc:identifier") {
        if let Some(t) = tag_inner(&rest[idx..], "dc:identifier") {
            let cleaned = t
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
                .collect::<String>();
            if cleaned.len() == 13 || cleaned.len() == 10 {
                isbn = Some(cleaned.to_ascii_uppercase());
                break;
            }
            // urn:isbn:978...
            let lower = t.to_ascii_lowercase();
            if let Some(pos) = lower.find("isbn") {
                let digits: String = t[pos..]
                    .chars()
                    .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
                    .collect();
                if digits.len() == 13 || digits.len() == 10 {
                    isbn = Some(digits.to_ascii_uppercase());
                    break;
                }
            }
        }
        rest = &rest[idx + 14..];
    }

    Ok(EpubMeta {
        title: dc_text(&opf, "title"),
        authors: if authors.is_empty() {
            None
        } else {
            Some(authors.join(", "))
        },
        isbn,
        subjects,
        description: dc_text(&opf, "description").or_else(|| {
            // calibre sometimes uses <description xmlns=...>
            tag_inner(&opf, "description")
        }),
    })
}

fn dc_text(xml: &str, local: &str) -> Option<String> {
    tag_inner(xml, &format!("dc:{local}")).or_else(|| tag_inner(xml, local))
}

fn tag_inner(xml: &str, tag: &str) -> Option<String> {
    let open1 = format!("<{tag}");
    let open2 = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = if let Some(i) = xml.find(&open2) {
        i + open2.len()
    } else if let Some(i) = xml.find(&open1) {
        let after = xml[i..].find('>')? + i + 1;
        after
    } else {
        return None;
    };
    let end = xml[start..].find(&close)? + start;
    let text = xml[start..end]
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .trim()
        .to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
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
    let meta = read_epub_metadata(&epub_dest).unwrap_or_default();

    Ok(ImportResult {
        epub_path: Some(epub_dest),
        markdown_path: if md_dest.exists() {
            Some(md_dest)
        } else {
            None
        },
        cover_path: cover,
        needs_review: false,
        title: meta.title,
        authors: meta.authors,
        isbn: meta.isbn,
        subjects: meta.subjects,
        description: meta.description,
    })
}

fn missing_tool_msg(bin: &str) -> String {
    match bin {
        "pandoc" => "pandoc is required for PDF import. Install: apt install pandoc (Debian) or pacman -S pandoc (Arch)".into(),
        "ocrmypdf" => "ocrmypdf is required for PDF import. Install: apt install ocrmypdf tesseract-ocr tesseract-ocr-eng (Debian); on Arch: pacman -S tesseract tesseract-data-eng and `uv tool install ocrmypdf` (or AUR ocrmypdf)".into(),
        "pdftohtml" => "pdftohtml (poppler) is required for PDF→Markdown. Install: apt install poppler-utils (Debian) or pacman -S poppler (Arch)".into(),
        other => format!("{other} is required but was not found on PATH"),
    }
}

async fn ensure_on_path(bin: &str) -> Result<()> {
    // poppler tools accept -v (stderr) rather than --version
    let version_arg = if bin == "pdftohtml" { "-v" } else { "--version" };
    match Command::new(bin)
        .arg(version_arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
    {
        Ok(status) if status.success() => Ok(()),
        Ok(_) | Err(_) => {
            // Fallback: binary exists on PATH even if version flag is odd
            if which_bin(bin) {
                Ok(())
            } else {
                Err(anyhow!("{}", missing_tool_msg(bin)))
            }
        }
    }
}

fn which_bin(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .any(|p| std::path::Path::new(&p).join(bin).is_file())
        })
        .unwrap_or(false)
}

async fn ingest_pdf(library_root: &Path, work_id: Uuid, source: &Path) -> Result<ImportResult> {
    // Pandoc 3.x cannot read PDF; we OCR, then pdftohtml → pandoc HTML→Markdown.
    ensure_on_path("pandoc").await?;
    ensure_on_path("ocrmypdf").await?;
    ensure_on_path("pdftohtml").await?;

    let work_dir = library_root.join(work_id.to_string());
    tokio::fs::create_dir_all(&work_dir).await?;
    let quarantine = work_dir.join("import.pdf");
    tokio::fs::copy(source, &quarantine)
        .await
        .context("copy quarantine import.pdf")?;

    let ocr_pdf = work_dir.join("import.ocr.pdf");
    let ocr_status = Command::new("ocrmypdf")
        .arg("--skip-text")
        .arg(&quarantine)
        .arg(&ocr_pdf)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .map_err(|_| anyhow!("{}", missing_tool_msg("ocrmypdf")))?;

    if !ocr_status.success() {
        let _ = tokio::fs::remove_file(&ocr_pdf).await;
        return Err(anyhow!(
            "ocrmypdf failed OCR on PDF (exit {:?}). Ensure tesseract + language data are installed",
            ocr_status.code()
        ));
    }

    let html_path = work_dir.join("import.html");
    let html_status = Command::new("pdftohtml")
        .arg("-s")
        .arg("-i")
        .arg("-noframes")
        .arg(&ocr_pdf)
        .arg(&html_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .map_err(|_| anyhow!("{}", missing_tool_msg("pdftohtml")))?;

    if !html_status.success() {
        let _ = tokio::fs::remove_file(&ocr_pdf).await;
        let _ = tokio::fs::remove_file(&html_path).await;
        return Err(anyhow!(
            "pdftohtml failed converting PDF to HTML (exit {:?})",
            html_status.code()
        ));
    }

    // pdftohtml may write import.html or import-html.html depending on version
    let html_src = if html_path.exists() {
        html_path.clone()
    } else {
        let alt = work_dir.join("import-html.html");
        if alt.exists() {
            alt
        } else {
            // pick any .html just produced beside the OCR pdf
            let mut found = None;
            if let Ok(mut entries) = tokio::fs::read_dir(&work_dir).await {
                while let Ok(Some(e)) = entries.next_entry().await {
                    let p = e.path();
                    if p.extension().and_then(|x| x.to_str()) == Some("html") {
                        found = Some(p);
                        break;
                    }
                }
            }
            found.ok_or_else(|| anyhow!("pdftohtml produced no HTML output"))?
        }
    };

    let md_dest = work_markdown_path(library_root, work_id);
    let media = work_media_dir(library_root, work_id);
    tokio::fs::create_dir_all(&media).await?;
    let media_arg = format!("--extract-media={}", media.display());

    let status = Command::new("pandoc")
        .arg(&html_src)
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
        .map_err(|_| anyhow!("{}", missing_tool_msg("pandoc")))?;

    let _ = tokio::fs::remove_file(&ocr_pdf).await;
    let _ = tokio::fs::remove_file(&html_src).await;
    // pdftohtml may drop companion pngs next to html; leave media/ for extracted assets
    if let Ok(mut entries) = tokio::fs::read_dir(&work_dir).await {
        while let Ok(Some(e)) = entries.next_entry().await {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("import")
                && (name.ends_with(".png")
                    || name.ends_with(".jpg")
                    || name.ends_with(".html")
                    || name.ends_with(".xml"))
            {
                let _ = tokio::fs::remove_file(&p).await;
            }
        }
    }

    if !status.success() {
        return Err(anyhow!("pandoc failed converting PDF HTML intermediate to markdown"));
    }

    info!(?work_id, "PDF OCR'd and converted to markdown; awaiting review before EPUB export");

    Ok(ImportResult {
        epub_path: None,
        markdown_path: Some(md_dest),
        cover_path: None,
        needs_review: true,
        title: None,
        authors: None,
        isbn: None,
        subjects: vec![],
        description: None,
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
        title: None,
        authors: None,
        isbn: None,
        subjects: vec![],
        description: None,
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

/// Extract cover image from an EPUB into `dest` (typically `cover.jpg`).
pub async fn extract_epub_cover(epub: &Path, dest: &Path) -> Result<Option<PathBuf>> {
    tokio::task::spawn_blocking({
        let epub = epub.to_path_buf();
        let dest = dest.to_path_buf();
        move || extract_epub_cover_sync(&epub, &dest)
    })
    .await
    .context("cover extract task")?
}

/// Extract cover image from an EPUB into `dest` (typically `cover.jpg`).
/// Resolves OPF `<meta name="cover">` / `properties="cover-image"`, then filename heuristics.
pub fn extract_epub_cover_sync(epub: &Path, dest: &Path) -> Result<Option<PathBuf>> {
    let file = std::fs::File::open(epub)?;
    let mut archive = ZipArchive::new(file)?;

    let opf_path = find_opf_path(&mut archive);
    let mut cover_href: Option<String> = None;

    if let Some(ref opf_name) = opf_path {
        if let Ok(mut entry) = archive.by_name(opf_name) {
            let mut opf = String::new();
            std::io::Read::read_to_string(&mut entry, &mut opf)?;
            let opf_dir = Path::new(opf_name)
                .parent()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            cover_href = resolve_cover_href(&opf, &opf_dir);
        }
    }

    // Fallback: any path with "cover" in the name
    if cover_href.is_none() {
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
        cover_href = candidates.into_iter().next();
    }

    // Last resort: largest raster image (skip tiny logos)
    if cover_href.is_none() {
        let mut best: Option<(u64, String)> = None;
        for i in 0..archive.len() {
            let entry = archive.by_index(i)?;
            let name = entry.name().to_string();
            let lower = name.to_ascii_lowercase();
            if !(lower.ends_with(".jpg")
                || lower.ends_with(".jpeg")
                || lower.ends_with(".png")
                || lower.ends_with(".webp"))
            {
                continue;
            }
            let size = entry.size();
            if size < 20_000 {
                continue;
            }
            if best.as_ref().map(|(s, _)| size > *s).unwrap_or(true) {
                best = Some((size, name));
            }
        }
        cover_href = best.map(|(_, n)| n);
    }

    let Some(name) = cover_href else {
        return Ok(None);
    };

    // Re-open archive entry (borrow rules)
    drop(archive);
    let file = std::fs::File::open(epub)?;
    let mut archive = ZipArchive::new(file)?;
    let mut entry = archive
        .by_name(&name)
        .map_err(|e| anyhow!("cover entry {name}: {e}"))?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = std::fs::File::create(dest)?;
    std::io::copy(&mut entry, &mut out)?;
    Ok(Some(dest.to_path_buf()))
}

fn find_opf_path(archive: &mut ZipArchive<std::fs::File>) -> Option<String> {
    if let Ok(mut container) = archive.by_name("META-INF/container.xml") {
        let mut xml = String::new();
        if std::io::Read::read_to_string(&mut container, &mut xml).is_ok() {
            if let Some(start) = xml.find("full-path=\"") {
                let rest = &xml[start + 11..];
                if let Some(end) = rest.find('"') {
                    return Some(rest[..end].to_string());
                }
            }
        }
    }
    for i in 0..archive.len() {
        if let Ok(e) = archive.by_index(i) {
            let name = e.name().to_string();
            if name.ends_with(".opf") {
                return Some(name);
            }
        }
    }
    None
}

fn resolve_cover_href(opf: &str, opf_dir: &str) -> Option<String> {
    // <meta name="cover" content="id"/> or content before name
    let cover_id = {
        let lower = opf.to_ascii_lowercase();
        let mut id = None;
        if let Some(idx) = lower.find("name=\"cover\"") {
            let window = &opf[idx.saturating_sub(80)..(idx + 120).min(opf.len())];
            if let Some(c) = attr_value(window, "content") {
                id = Some(c);
            }
        }
        if id.is_none() {
            if let Some(idx) = lower.find("name='cover'") {
                let window = &opf[idx.saturating_sub(80)..(idx + 120).min(opf.len())];
                if let Some(c) = attr_value(window, "content") {
                    id = Some(c);
                }
            }
        }
        id
    };

    if let Some(id) = cover_id {
        if let Some(href) = find_manifest_href(opf, &id) {
            return Some(join_opf_href(opf_dir, &href));
        }
    }

    // EPUB3: properties="... cover-image ..."
    let lower = opf.to_ascii_lowercase();
    if let Some(idx) = lower.find("cover-image") {
        let start = opf[..idx].rfind('<').unwrap_or(0);
        let end = opf[idx..].find('>').map(|e| idx + e).unwrap_or(opf.len());
        let tag = &opf[start..end];
        if let Some(href) = attr_value(tag, "href") {
            return Some(join_opf_href(opf_dir, &href));
        }
    }
    None
}

fn find_manifest_href(opf: &str, id: &str) -> Option<String> {
    let needle = format!("id=\"{id}\"");
    let needle2 = format!("id='{id}'");
    let idx = opf.find(&needle).or_else(|| opf.find(&needle2))?;
    let start = opf[..idx].rfind('<')?;
    let end = opf[idx..].find('>').map(|e| idx + e)?;
    attr_value(&opf[start..end], "href")
}

fn attr_value(tag: &str, name: &str) -> Option<String> {
    for q in ['"', '\''] {
        let pat = format!("{name}={q}");
        if let Some(i) = tag.find(&pat) {
            let rest = &tag[i + pat.len()..];
            if let Some(end) = rest.find(q) {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

fn join_opf_href(opf_dir: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or(href);
    if opf_dir.is_empty() {
        href.to_string()
    } else {
        format!("{opf_dir}/{href}")
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_cover_from_opf_meta() {
        let opf = r#"
        <metadata>
          <meta content="main_cover_image" name="cover"/>
        </metadata>
        <manifest>
          <item href="images/title.jpg" id="main_cover_image" media-type="image/jpeg"/>
        </manifest>
        "#;
        assert_eq!(
            resolve_cover_href(opf, "OEBPS"),
            Some("OEBPS/images/title.jpg".into())
        );
    }

    #[test]
    fn extract_cover_from_covenant_epub_if_present() {
        let epub = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/library/daf8a61c-17b4-4e4b-be0e-c9cc4aaeaba1/book.epub");
        if !epub.exists() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("cover.jpg");
        let out = extract_epub_cover_sync(&epub, &dest).unwrap();
        assert!(out.is_some());
        assert!(dest.exists());
        assert!(dest.metadata().unwrap().len() > 50_000);
    }
}
