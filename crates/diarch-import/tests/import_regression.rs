use diarch_import::{export_needs_tts_queue, ingest_file, zip_markdown_bundle, TtsQueueItem};
use std::path::Path;
use tempfile::tempdir;
use uuid::Uuid;

#[tokio::test]
async fn rejects_unsupported_format() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("x.docx");
    tokio::fs::write(&src, b"nope").await.unwrap();
    let err = ingest_file(dir.path(), Uuid::new_v4(), &src, "x.docx")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unsupported"));
}

#[tokio::test]
async fn markdown_import_sets_needs_review() {
    let dir = tempdir().unwrap();
    let lib = dir.path().join("library");
    tokio::fs::create_dir_all(&lib).await.unwrap();
    let src = dir.path().join("book.md");
    tokio::fs::write(&src, b"# Hello\n\nWorld\n").await.unwrap();
    let id = Uuid::new_v4();
    let result = ingest_file(&lib, id, &src, "book.md").await.unwrap();
    assert!(result.needs_review);
    assert!(result.markdown_path.unwrap().exists());
    assert!(result.epub_path.is_none());
}

#[tokio::test]
async fn zip_markdown_bundle_includes_md_and_media() {
    let dir = tempdir().unwrap();
    let lib = dir.path().join("library");
    let id = Uuid::new_v4();
    let work = lib.join(id.to_string());
    tokio::fs::create_dir_all(work.join("media")).await.unwrap();
    tokio::fs::write(work.join("book.md"), b"# Title\n").await.unwrap();
    tokio::fs::write(work.join("media/img.png"), b"PNG").await.unwrap();
    let out = dir.path().join("out.zip");
    zip_markdown_bundle(&lib, id, &out).unwrap();
    assert!(out.metadata().unwrap().len() > 20);

    let file = std::fs::File::open(&out).unwrap();
    let mut zip = zip::ZipArchive::new(file).unwrap();
    let names: Vec<_> = (0..zip.len()).map(|i| zip.by_index(i).unwrap().name().to_string()).collect();
    assert!(names.iter().any(|n| n == "book.md"));
    assert!(names.iter().any(|n| n.starts_with("media/")));
}

#[tokio::test]
async fn export_needs_tts_queue_copies_epubs() {
    let dir = tempdir().unwrap();
    let lib = dir.path().join("library");
    let queue = dir.path().join("queue");
    let id = Uuid::new_v4();
    tokio::fs::create_dir_all(lib.join(id.to_string())).await.unwrap();
    tokio::fs::write(lib.join(id.to_string()).join("book.epub"), b"PK").await.unwrap();
    let files = export_needs_tts_queue(
        &lib,
        &queue,
        &[TtsQueueItem {
            work_id: id,
            title: "A Covenant of Ice".into(),
            authors: "Author".into(),
        }],
    )
    .await
    .unwrap();
    assert_eq!(files.len(), 1);
    assert!(queue
        .join(format!("A Covenant of Ice--{}.epub", id.as_simple()))
        .exists());
    assert!(queue.join("manifest.json").exists());
}

#[tokio::test]
async fn pandoc_markdown_to_epub_when_available() {
    if tokio::process::Command::new("pandoc")
        .arg("--version")
        .output()
        .await
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("skipping: pandoc not available");
        return;
    }
    let dir = tempdir().unwrap();
    let lib = dir.path().join("library");
    let id = Uuid::new_v4();
    let src = dir.path().join("book.md");
    tokio::fs::write(&src, b"# Chapter\n\nHello Diarch.\n").await.unwrap();
    let imported = ingest_file(&lib, id, &src, "book.md").await.unwrap();
    assert!(imported.needs_review);
    let epub = diarch_import::confirm_markdown_to_epub(
        &lib,
        id,
        Some("Hello Title"),
        Some("Ada Lovelace"),
    )
    .await
    .unwrap();
    assert!(epub.exists());
    assert!(epub.metadata().unwrap().len() > 100);
    let meta = diarch_import::read_epub_metadata(&epub).unwrap();
    assert_eq!(meta.title.as_deref(), Some("Hello Title"));
}

fn tools_available() -> bool {
    let pandoc = std::process::Command::new("pandoc")
        .arg("--version")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let ocr = std::process::Command::new("ocrmypdf")
        .arg("--version")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let pdftohtml = std::path::Path::new("/usr/bin/pdftohtml").is_file()
        || std::env::var_os("PATH")
            .map(|paths| {
                std::env::split_paths(&paths)
                    .any(|p| p.join("pdftohtml").is_file())
            })
            .unwrap_or(false);
    pandoc && ocr && pdftohtml
}

/// PDF fixture generated with ghostscript (pandoc 3 cannot emit PDF without LaTeX).
fn write_sample_pdf(path: &std::path::Path) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures_sample.pdf");
    if fixture.exists() {
        std::fs::copy(&fixture, path).unwrap();
        return;
    }
    // Fallback: invoke gs if fixture missing
    let ps = path.with_extension("ps");
    std::fs::write(
        &ps,
        b"%!PS\n/Helvetica findfont 24 scalefont setfont\n72 720 moveto\n(Hello Diarch) show\nshowpage\n",
    )
    .unwrap();
    let status = std::process::Command::new("gs")
        .args(["-q", "-sDEVICE=pdfwrite", "-o"])
        .arg(path)
        .arg(&ps)
        .status()
        .expect("ghostscript");
    assert!(status.success());
    let _ = std::fs::remove_file(ps);
}

#[tokio::test]
async fn pdf_ingest_quarantines_and_makes_markdown() {
    if !tools_available() {
        eprintln!("skipping: pandoc, ocrmypdf, or pdftohtml not available");
        return;
    }
    let dir = tempdir().unwrap();
    let lib = dir.path().join("library");
    let id = Uuid::new_v4();
    let src = dir.path().join("sample.pdf");
    write_sample_pdf(&src);
    let result = ingest_file(&lib, id, &src, "My_Sample_Book.pdf")
        .await
        .expect("pdf ingest");
    assert!(result.needs_review);
    assert!(result.epub_path.is_none());
    let quarantine = lib.join(id.to_string()).join("import.pdf");
    assert!(quarantine.exists(), "import.pdf quarantine missing");
    let md = result.markdown_path.expect("markdown path");
    assert!(md.exists());
    let text = tokio::fs::read_to_string(&md).await.unwrap();
    assert!(
        text.to_ascii_lowercase().contains("hello") || text.to_ascii_lowercase().contains("diarch"),
        "unexpected md: {text}"
    );

    // confirm deletes quarantine PDF
    let epub = diarch_import::confirm_markdown_to_epub(&lib, id, Some("PDF Book"), None)
        .await
        .expect("confirm");
    assert!(epub.exists());
    assert!(!quarantine.exists(), "import.pdf should be removed on confirm");
}
