use axum::body::Body;
use diarch_core::Config;
use diarch_server::{app, build_state, routes};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

async fn test_app() -> (tempfile::TempDir, axum::Router, Arc<diarch_server::state::AppState>) {
    let dir = tempdir().unwrap();
    let config = Config {
        listen: "127.0.0.1:0".into(),
        data_dir: dir.path().to_path_buf(),
        admin_username: "admin".into(),
        admin_password: "adminpass".into(),
        ..Config::default()
    };
    let state = build_state(config).await.unwrap();
    let router = app(state.clone());
    (dir, router, state)
}

async fn json_req(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> (u16, Value, Option<String>) {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(c) = cookie {
        builder = builder.header("cookie", format!("diarch_session={c}"));
    }
    let req = if let Some(b) = body {
        builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&b).unwrap()))
            .unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status().as_u16();
    let set_cookie = res
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            s.split(';')
                .next()
                .and_then(|p| p.strip_prefix("diarch_session="))
                .map(|t| t.to_string())
        });
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let val = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::String(
            String::from_utf8_lossy(&bytes).to_string(),
        ))
    };
    (status, val, set_cookie)
}

async fn login(app: &axum::Router, user: &str, pass: &str) -> String {
    let (status, body, cookie) = json_req(
        app,
        "POST",
        "/api/auth/login",
        None,
        Some(json!({"username": user, "password": pass})),
    )
    .await;
    assert_eq!(status, 200, "login failed: {body}");
    cookie.or_else(|| body.get("token").and_then(|t| t.as_str()).map(|s| s.to_string()))
        .expect("session cookie or token")
}

#[tokio::test]
async fn health_is_public() {
    let (_dir, app, _) = test_app().await;
    let (status, body, _) = json_req(&app, "GET", "/api/health", None, None).await;
    assert_eq!(status, 200);
    assert_eq!(body["ok"], true);
    assert_eq!(body["service"], "diarch");
}

#[tokio::test]
async fn auth_me_requires_session() {
    let (_dir, app, _) = test_app().await;
    let (status, _, _) = json_req(&app, "GET", "/api/auth/me", None, None).await;
    assert_eq!(status, 401);
    let token = login(&app, "admin", "adminpass").await;
    let (status, me, _) = json_req(&app, "GET", "/api/auth/me", Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(me["username"], "admin");
    assert_eq!(me["is_admin"], true);
}

#[tokio::test]
async fn wishlist_primary_code_and_taxonomy() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let (status, tax, _) = json_req(&app, "GET", "/api/taxonomy", Some(&token), None).await;
    assert_eq!(status, 200);
    assert!(tax.as_array().unwrap().len() > 50);

    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({
            "title": "The Eye of the World",
            "authors": "Robert Jordan",
            "codes": [8201, 8940],
            "primary_code": 8201,
            "wishlist": true
        })),
    )
    .await;
    assert_eq!(status, 201);
    assert_eq!(work["status"], "wishlist");
    assert_eq!(work["primary_code"], 8201);

    let id = work["id"].as_str().unwrap();
    let (status, detail, _) =
        json_req(&app, "GET", &format!("/api/works/{id}"), Some(&token), None).await;
    assert_eq!(status, 200);
    let codes = detail["codes"].as_array().unwrap();
    assert!(codes.iter().any(|c| c == 8201));
    assert!(codes.iter().any(|c| c == 8940));
}

#[tokio::test]
async fn acl_regression_grant_required() {
    let (_dir, app, state) = test_app().await;
    let admin = login(&app, "admin", "adminpass").await;

    let (status, reader, _) = json_req(
        &app,
        "POST",
        "/api/users",
        Some(&admin),
        Some(json!({"username":"reader","password":"reader","is_admin":false})),
    )
    .await;
    assert_eq!(status, 201);
    let reader_id = reader["id"].as_str().unwrap().to_string();

    let (status, secret, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&admin),
        Some(json!({"title":"Secret","authors":"A"})),
    )
    .await;
    assert_eq!(status, 201);
    let secret_id = secret["id"].as_str().unwrap().to_string();

    let (status, shared, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&admin),
        Some(json!({"title":"Shared","authors":"B","wishlist":true,"primary_code":8201,"codes":[8201]})),
    )
    .await;
    assert_eq!(status, 201);
    let shared_id = shared["id"].as_str().unwrap().to_string();

    let (status, _, _) = json_req(
        &app,
        "POST",
        &format!("/api/works/{shared_id}/grants"),
        Some(&admin),
        Some(json!({ "username": "reader" })),
    )
    .await;
    assert_eq!(status, 204);
    let _ = reader_id;

    let reader_tok = login(&app, "reader", "reader").await;
    let (status, works, _) = json_req(&app, "GET", "/api/works", Some(&reader_tok), None).await;
    assert_eq!(status, 200);
    let titles: Vec<_> = works
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"Shared"));
    assert!(!titles.contains(&"Secret"));

    let (status, _, _) =
        json_req(&app, "GET", &format!("/api/works/{secret_id}"), Some(&reader_tok), None).await;
    assert_eq!(status, 403);

    // Process one idle job loop shouldn't panic
    routes::jobs::process_one(&state).await.unwrap();
}

#[tokio::test]
async fn review_edit_sets_storygraph_dirty_flag() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({"title":"Book","authors":"X"})),
    )
    .await;
    assert_eq!(status, 201);
    let id = work["id"].as_str().unwrap();
    let (status, updated, _) = json_req(
        &app,
        "PUT",
        &format!("/api/works/{id}"),
        Some(&token),
        Some(json!({"rating": 4.5, "review": "Loved it"})),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(updated["sg_review_dirty"], true);

    let (status, cleared, _) = json_req(
        &app,
        "PUT",
        &format!("/api/works/{id}"),
        Some(&token),
        Some(json!({"clear_sg_review_dirty": true})),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(cleared["sg_review_dirty"], false);
}

#[tokio::test]
async fn manga_code_sets_rtl_direction() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({
            "title": "Manga Title",
            "authors": "Author",
            "codes": [8920],
            "primary_code": 8920
        })),
    )
    .await;
    assert_eq!(status, 201);
    assert_eq!(work["is_manga"], true);
    assert_eq!(work["reading_direction"], "rtl");
}

#[tokio::test]
async fn progress_and_settings_persist() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({"title":"P","authors":"A"})),
    )
    .await;
    assert_eq!(status, 201);
    let id = work["id"].as_str().unwrap();

    let (status, _, _) = json_req(
        &app,
        "PUT",
        &format!("/api/works/{id}/progress"),
        Some(&token),
        Some(json!({"mode":"epub","position":"cfi-1","percent":33.0})),
    )
    .await;
    assert_eq!(status, 204);

    let (status, prog, _) = json_req(
        &app,
        "GET",
        &format!("/api/works/{id}/progress?mode=epub"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(prog["percent"], 33.0);

    let (status, _, _) = json_req(
        &app,
        "PUT",
        "/api/settings",
        Some(&token),
        Some(json!({"show_audio_gaps": false, "reader_infinite_scroll": true})),
    )
    .await;
    assert_eq!(status, 204);
    let (status, settings, _) = json_req(&app, "GET", "/api/settings", Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(settings["show_audio_gaps"], false);
    assert_eq!(settings["reader_infinite_scroll"], true);
}

#[tokio::test]
async fn markdown_import_job_and_confirm() {
    let (_dir, app, state) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({"title":"MD","authors":"A"})),
    )
    .await;
    assert_eq!(status, 201);
    let id = work["id"].as_str().unwrap().to_string();
    let work_uuid = uuid::Uuid::parse_str(&id).unwrap();

    // Stage import like the multipart handler does
    let imports = state.config.data_dir.join("imports");
    tokio::fs::create_dir_all(&imports).await.unwrap();
    let path = imports.join(format!("{id}-book.md"));
    tokio::fs::write(&path, b"# Hello\n\nRegression test.\n")
        .await
        .unwrap();
    let job = state
        .db
        .create_job("import", Some(work_uuid))
        .await
        .unwrap();
    state
        .db
        .update_job(
            job.id,
            "pending",
            Some(&format!("book.md|{}", path.display())),
        )
        .await
        .unwrap();

    routes::jobs::process_one(&state).await.unwrap();
    let detail = json_req(&app, "GET", &format!("/api/works/{id}"), Some(&token), None)
        .await
        .1;
    assert_eq!(detail["work"]["needs_review"], true);

    if tokio::process::Command::new("pandoc")
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        let (status, confirm, _) =
            json_req(&app, "POST", &format!("/api/works/{id}/confirm"), Some(&token), None).await;
        assert_eq!(status, 200);
        let job_id = confirm["job_id"].as_str().unwrap();
        // drain jobs
        for _ in 0..5 {
            routes::jobs::process_one(&state).await.unwrap();
            let j = state
                .db
                .get_job(uuid::Uuid::parse_str(job_id).unwrap())
                .await
                .unwrap()
                .unwrap();
            if j.status == "done" || j.status == "failed" {
                assert_eq!(j.status, "done", "{:?}", j.detail);
                break;
            }
        }
        let detail = json_req(&app, "GET", &format!("/api/works/{id}"), Some(&token), None)
            .await
            .1;
        assert_eq!(detail["work"]["needs_review"], false);
        let kinds: Vec<_> = detail["assets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["kind"].as_str().unwrap())
            .collect();
        assert!(kinds.contains(&"epub"));
        assert!(kinds.contains(&"markdown"));
    }
}

#[tokio::test]
async fn cover_prompt_and_placeholder_helpers() {
    let prompt = diarch_server::metadata::cover_prompt("Dune", "Herbert", "8401");
    assert!(prompt.contains("Dune"));
    assert!(prompt.contains("8401"));
    let svg = diarch_server::metadata::placeholder_cover_svg("Title & Co", "Author <X>");
    assert!(svg.contains("Title &amp; Co"));
    assert!(svg.contains("Author &lt;X&gt;"));
}

#[tokio::test]
async fn attention_filter_endpoint() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({"title":"NeedsTTS","authors":"A"})),
    )
    .await;
    assert_eq!(status, 201);
    let id = work["id"].as_str().unwrap();
    let (status, _, _) = json_req(
        &app,
        "PUT",
        &format!("/api/works/{id}"),
        Some(&token),
        Some(json!({"needs_tts": true})),
    )
    .await;
    assert_eq!(status, 200);
    let (status, list, _) =
        json_req(&app, "GET", "/api/works?attention=needs_tts", Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn library_import_epub_creates_work_and_assets() {
    let dir = tempfile::tempdir().unwrap();
    // Minimal EPUB zip
    let epub_path = dir.path().join("sample.epub");
    {
        let file = std::fs::File::create(&epub_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("mimetype", opts).unwrap();
        use std::io::Write;
        zip.write_all(b"application/epub+zip").unwrap();
        zip.start_file("META-INF/container.xml", opts).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?><container><rootfiles><rootfile full-path="content.opf"/></rootfiles></container>"#,
        )
        .unwrap();
        zip.start_file("content.opf", opts).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?>
<package>
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Phase One Sample</dc:title>
    <dc:creator>Test Author</dc:creator>
  </metadata>
  <manifest><item id="c1" href="chap.html" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="c1"/></spine>
</package>"#,
        )
        .unwrap();
        zip.start_file("chap.html", opts).unwrap();
        zip.write_all(b"<html><body><p>Hello Diarch.</p></body></html>")
            .unwrap();
        zip.finish().unwrap();
    }

    let (_td, app, state) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let bytes = std::fs::read(&epub_path).unwrap();
    let boundary = "----diarchboundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"sample.epub\"\r\nContent-Type: application/epub+zip\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&bytes);
    body.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"primary_code\"\r\n\r\n8201\r\n--{boundary}--\r\n").as_bytes());

    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/api/library/import")
        .header("cookie", format!("diarch_session={token}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), 201);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let j: Value = serde_json::from_slice(&bytes).unwrap();
    let job_id = j["job_id"].as_str().unwrap();
    let work_id = j["work"]["id"].as_str().unwrap();
    assert_eq!(j["work"]["title"], "Phase One Sample");
    assert_eq!(j["work"]["authors"], "Test Author");
    assert_eq!(j["work"]["primary_code"], 8201);

    for _ in 0..40 {
        routes::jobs::process_one(&state).await.unwrap();
        let job = state
            .db
            .get_job(uuid::Uuid::parse_str(job_id).unwrap())
            .await
            .unwrap()
            .unwrap();
        if job.status == "done" || job.status == "failed" {
            assert_eq!(job.status, "done", "{:?}", job.detail);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let (status, detail, _) =
        json_req(&app, "GET", &format!("/api/works/{work_id}"), Some(&token), None).await;
    assert_eq!(status, 200);
    let kinds: Vec<_> = detail["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"epub"), "{kinds:?}");
}

#[tokio::test]
async fn tts_queue_export_documents_todo() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let (status, body, _) =
        json_req(&app, "POST", "/api/queue/needs_tts/export", Some(&token), None).await;
    assert_eq!(status, 200);
    assert!(body["todo"].as_str().unwrap().contains("ebook2audiobook"));
}

#[tokio::test]
async fn currently_reading_shelf_caps_at_three() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let mut ids = Vec::new();
    for i in 0..3 {
        let (status, body, _) = json_req(
            &app,
            "POST",
            "/api/works",
            Some(&token),
            Some(json!({
                "title": format!("Reading {i}"),
                "authors": "A",
                "status": "reading"
            })),
        )
        .await;
        assert_eq!(status, 201, "{body}");
        ids.push(body["id"].as_str().unwrap().to_string());
    }
    let (status, body, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({
            "title": "Reading overflow",
            "authors": "A",
            "status": "reading"
        })),
    )
    .await;
    assert_eq!(status, 409, "{body}");

    let (status, body, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({ "title": "Parked", "authors": "A" })),
    )
    .await;
    assert_eq!(status, 201);
    let id = body["id"].as_str().unwrap();
    let (status, body, _) = json_req(
        &app,
        "PUT",
        &format!("/api/works/{id}"),
        Some(&token),
        Some(json!({ "status": "reading" })),
    )
    .await;
    assert_eq!(status, 409, "{body}");

    let (status, list, _) =
        json_req(&app, "GET", "/api/works?status=reading", Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(list.as_array().unwrap().len(), 3);
}

async fn raw_req(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    content_type: &str,
    body: Vec<u8>,
) -> (u16, Vec<u8>) {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(c) = cookie {
        builder = builder.header("cookie", format!("diarch_session={c}"));
    }
    let req = builder
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec())
}

#[tokio::test]
async fn put_markdown_keeps_needs_review() {
    let (_dir, app, state) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({"title":"ReviewMe","authors":"A"})),
    )
    .await;
    assert_eq!(status, 201);
    let id = work["id"].as_str().unwrap().to_string();
    let work_uuid = uuid::Uuid::parse_str(&id).unwrap();

    let imports = state.config.data_dir.join("imports");
    tokio::fs::create_dir_all(&imports).await.unwrap();
    let path = imports.join(format!("{id}-book.md"));
    tokio::fs::write(&path, b"# Draft\n").await.unwrap();
    let job = state
        .db
        .create_job("import", Some(work_uuid))
        .await
        .unwrap();
    state
        .db
        .update_job(
            job.id,
            "pending",
            Some(&format!("book.md|{}", path.display())),
        )
        .await
        .unwrap();
    routes::jobs::process_one(&state).await.unwrap();

    let (status, detail, _) =
        json_req(&app, "GET", &format!("/api/works/{id}"), Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(detail["work"]["needs_review"], true);
    assert_eq!(detail["has_import_pdf"], false);

    let (status, _) = raw_req(
        &app,
        "PUT",
        &format!("/api/works/{id}/content/markdown"),
        Some(&token),
        "text/markdown; charset=utf-8",
        b"# Edited\n\nStill reviewing.\n".to_vec(),
    )
    .await;
    assert_eq!(status, 204);

    let (status, body) = raw_req(
        &app,
        "GET",
        &format!("/api/works/{id}/content/markdown"),
        Some(&token),
        "text/plain",
        vec![],
    )
    .await;
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body).contains("Still reviewing"));

    let (status, detail, _) =
        json_req(&app, "GET", &format!("/api/works/{id}"), Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(detail["work"]["needs_review"], true);
}

#[tokio::test]
async fn pdf_import_put_md_confirm_api() {
    let pandoc_ok = tokio::process::Command::new("pandoc")
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    let ocr_ok = tokio::process::Command::new("ocrmypdf")
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    let pdftohtml_ok = std::path::Path::new("/usr/bin/pdftohtml").is_file()
        || std::env::var_os("PATH")
            .map(|paths| {
                std::env::split_paths(&paths)
                    .any(|p| p.join("pdftohtml").is_file())
            })
            .unwrap_or(false);
    if !pandoc_ok || !ocr_ok || !pdftohtml_ok {
        eprintln!("skipping: pandoc, ocrmypdf, or pdftohtml not available");
        return;
    }

    let (_dir, app, state) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({"title":"PdfBook","authors":""})),
    )
    .await;
    assert_eq!(status, 201);
    let id = work["id"].as_str().unwrap().to_string();
    let work_uuid = uuid::Uuid::parse_str(&id).unwrap();

    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../diarch-import/tests/fixtures_sample.pdf");
    let pdf_bytes = if fixture.exists() {
        tokio::fs::read(&fixture).await.unwrap()
    } else {
        // last-resort handcrafted (may fail OCR/html path)
        b"%PDF-1.4\n1 0 obj<< /Type /Catalog /Pages 2 0 R >>endobj\ntrailer<< /Root 1 0 R >>\n%%EOF\n"
            .to_vec()
    };
    let imports = state.config.data_dir.join("imports");
    tokio::fs::create_dir_all(&imports).await.unwrap();
    let path = imports.join(format!("{id}-book.pdf"));
    tokio::fs::write(&path, pdf_bytes).await.unwrap();
    let job = state
        .db
        .create_job("import", Some(work_uuid))
        .await
        .unwrap();
    state
        .db
        .update_job(
            job.id,
            "pending",
            Some(&format!("book.pdf|{}", path.display())),
        )
        .await
        .unwrap();
    routes::jobs::process_one(&state).await.unwrap();

    let done = state.db.get_job(job.id).await.unwrap().unwrap();
    assert_eq!(done.status, "done", "{:?}", done.detail);

    let quarantine = state.config.work_dir(work_uuid).join("import.pdf");
    assert!(quarantine.exists());

    let (status, detail, _) =
        json_req(&app, "GET", &format!("/api/works/{id}"), Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(detail["work"]["needs_review"], true);
    assert_eq!(detail["has_import_pdf"], true);

    let (status, pdf_body) = raw_req(
        &app,
        "GET",
        &format!("/api/works/{id}/content/pdf"),
        Some(&token),
        "application/octet-stream",
        vec![],
    )
    .await;
    assert_eq!(status, 200);
    assert!(pdf_body.starts_with(b"%PDF"));

    let (status, _) = raw_req(
        &app,
        "PUT",
        &format!("/api/works/{id}/content/markdown"),
        Some(&token),
        "text/markdown; charset=utf-8",
        b"# PdfBook\n\nReviewed body.\n".to_vec(),
    )
    .await;
    assert_eq!(status, 204);

    let (status, confirm, _) =
        json_req(&app, "POST", &format!("/api/works/{id}/confirm"), Some(&token), None).await;
    assert_eq!(status, 200);
    let job_id = confirm["job_id"].as_str().unwrap();
    for _ in 0..10 {
        routes::jobs::process_one(&state).await.unwrap();
        let j = state
            .db
            .get_job(uuid::Uuid::parse_str(job_id).unwrap())
            .await
            .unwrap()
            .unwrap();
        if j.status == "done" || j.status == "failed" {
            assert_eq!(j.status, "done", "{:?}", j.detail);
            break;
        }
    }

    assert!(!quarantine.exists(), "import.pdf removed on confirm");
    let (status, detail, _) =
        json_req(&app, "GET", &format!("/api/works/{id}"), Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(detail["work"]["needs_review"], false);
    assert_eq!(detail["has_import_pdf"], false);
    let kinds: Vec<_> = detail["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"epub"));
    assert!(kinds.contains(&"markdown"));
}

#[tokio::test]
async fn m4b_upload_attaches_and_streams_with_range() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({"title":"Audio Book","authors":"A","needs_audio":true})),
    )
    .await;
    assert_eq!(status, 201, "{work}");
    let id = work["id"].as_str().unwrap();

    // Minimal fake "m4b" payload (not a real media file; attach path only).
    let fake = b"ftypM4B fake audiobook bytes for test!!!!";
    let boundary = "----audioBoundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"book.m4b\"\r\nContent-Type: audio/mp4\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(fake);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let req = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/api/works/{id}/audio"))
        .header("cookie", format!("diarch_session={token}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), 200, "upload");
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let j: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(j["filename"], "book.m4b");

    let (status, detail, _) =
        json_req(&app, "GET", &format!("/api/works/{id}"), Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(detail["work"]["needs_audio"], false);
    let kinds: Vec<_> = detail["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"audio"), "{kinds:?}");

    let req = axum::http::Request::builder()
        .method("GET")
        .uri(format!("/api/works/{id}/audio/book.m4b"))
        .header("cookie", format!("diarch_session={token}"))
        .header("range", "bytes=0-9")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), 206);
    assert_eq!(
        res.headers().get("content-type").and_then(|v| v.to_str().ok()),
        Some("audio/mp4")
    );
    let slice = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&slice[..], &fake[..10]);

    let (status, ch, _) = json_req(
        &app,
        "GET",
        &format!("/api/works/{id}/audio/chapters"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert!(ch["chapters"].as_array().unwrap().is_empty());

    let (status, tr, _) = json_req(
        &app,
        "POST",
        &format!("/api/works/{id}/transcribe"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(tr["ready"], false);
    assert!(tr["message"].as_str().unwrap().contains("Phase 8"));
}

#[tokio::test]
async fn aax_upload_without_key_fails_clearly() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({"title":"AAX Book","authors":"A"})),
    )
    .await;
    assert_eq!(status, 201, "{work}");
    let id = work["id"].as_str().unwrap();

    let boundary = "----aaxBoundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"book.aax\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(b"not-a-real-aax");
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let req = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/api/works/{id}/audio"))
        .header("cookie", format!("diarch_session={token}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), 400);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let msg = String::from_utf8_lossy(&bytes);
    assert!(
        msg.contains("DIARCH_AUDIBLE_KEY") || msg.contains("activation"),
        "{msg}"
    );
}

#[tokio::test]
async fn mp3_audio_upload_rejected() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({"title":"MP3 Book","authors":"A"})),
    )
    .await;
    assert_eq!(status, 201);
    let id = work["id"].as_str().unwrap();

    let boundary = "----mp3Boundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"book.mp3\"\r\nContent-Type: audio/mpeg\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(b"ID3fake");
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let req = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/api/works/{id}/audio"))
        .header("cookie", format!("diarch_session={token}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), 400);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let msg = String::from_utf8_lossy(&bytes);
    assert!(msg.to_ascii_lowercase().contains("mp3"), "{msg}");
}

#[tokio::test]
async fn library_import_m4b_creates_work_with_audio() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let fake = b"ftypM4B library import bytes!!!!";
    let boundary = "----libAudioBoundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"HailMary.m4b\"\r\nContent-Type: audio/mp4\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(fake);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/api/library/import")
        .header("cookie", format!("diarch_session={token}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), 201, "library m4b import");
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let j: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(j["kind"], "m4b");
    assert!(j["job_id"].is_null());
    assert_eq!(j["work"]["title"], "HailMary");
    let id = j["work"]["id"].as_str().unwrap();

    let (status, detail, _) =
        json_req(&app, "GET", &format!("/api/works/{id}"), Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(detail["work"]["needs_audio"], false);
    let kinds: Vec<_> = detail["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"audio"), "{kinds:?}");
}

#[tokio::test]
async fn library_import_aax_without_key_fails() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let boundary = "----libAaxBoundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"book.aax\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(b"not-a-real-aax");
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/api/library/import")
        .header("cookie", format!("diarch_session={token}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), 400);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let msg = String::from_utf8_lossy(&bytes);
    assert!(
        msg.contains("DIARCH_AUDIBLE_KEY") || msg.contains("activation"),
        "{msg}"
    );
}

#[tokio::test]
async fn delete_work_removes_row_and_files() {
    let (dir, app, state) = test_app().await;
    let token = login(&app, "admin", "adminpass").await;
    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({"title":"Delete Me","authors":"A"})),
    )
    .await;
    assert_eq!(status, 201, "{work}");
    let id = work["id"].as_str().unwrap();
    let work_dir = state.config.data_dir.join("library").join(id);
    tokio::fs::create_dir_all(&work_dir).await.unwrap();
    tokio::fs::write(work_dir.join("book.epub"), b"epub").await.unwrap();
    assert!(work_dir.join("book.epub").exists());

    let (status, body, _) =
        json_req(&app, "DELETE", &format!("/api/works/{id}"), Some(&token), None).await;
    assert_eq!(status, 204, "{body}");
    assert!(!work_dir.exists());

    let (status, _, _) =
        json_req(&app, "GET", &format!("/api/works/{id}"), Some(&token), None).await;
    assert_eq!(status, 404);
    let _ = dir;
}
