use diarch_import::{export_needs_tts_queue, ingest_file, zip_markdown_bundle};
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
    let n = export_needs_tts_queue(&lib, &queue, &[id]).await.unwrap();
    assert_eq!(n, 1);
    assert!(queue.join(format!("{id}.epub")).exists());
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
    let epub = diarch_import::confirm_markdown_to_epub(&lib, id).await.unwrap();
    assert!(epub.exists());
    assert!(epub.metadata().unwrap().len() > 100);
}
