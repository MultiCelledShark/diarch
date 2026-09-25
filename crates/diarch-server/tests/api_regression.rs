use axum::body::Body;
use diarch_core::Config;
use diarch_server::{app, build_state, routes};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

async fn test_app() -> (tempfile::TempDir, axum::Router, Arc<diarch_server::state::AppState>) {
    // Avoid hanging auth tests on outbound StoryGraph / Cloudflare.
    // SAFETY: process-wide and only used by this test harness before the app starts.
    unsafe { std::env::set_var("DIARCH_STORYGRAPH_SKIP_LIVE_CHECK", "1") };
    let dir = tempdir().unwrap();
    let config = Config {
        listen: "127.0.0.1:0".into(),
        data_dir: dir.path().to_path_buf(),
        admin_username: "admin".into(),
        admin_password: "adminpass1234".into(),
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
    let token = login(&app, "admin", "adminpass1234").await;
    let (status, me, _) = json_req(&app, "GET", "/api/auth/me", Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(me["username"], "admin");
    assert_eq!(me["is_admin"], true);
}

#[tokio::test]
async fn wishlist_primary_code_and_taxonomy() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;
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
    let admin = login(&app, "admin", "adminpass1234").await;

    let (status, reader, _) = json_req(
        &app,
        "POST",
        "/api/users",
        Some(&admin),
        Some(json!({"username":"reader","password":"readerpass12","is_admin":false})),
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

    let reader_tok = login(&app, "reader", "readerpass12").await;
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

    // List / revoke grants (admin only). Reader cannot manage grants.
    let (status, grants, _) = json_req(
        &app,
        "GET",
        &format!("/api/works/{shared_id}/grants"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(grants.as_array().unwrap().len(), 1);
    assert_eq!(grants[0]["username"], "reader");

    let (status, _, _) = json_req(
        &app,
        "GET",
        &format!("/api/works/{shared_id}/grants"),
        Some(&reader_tok),
        None,
    )
    .await;
    assert_eq!(status, 403);

    // Readers cannot grant either.
    let (status, _, _) = json_req(
        &app,
        "POST",
        &format!("/api/works/{shared_id}/grants"),
        Some(&reader_tok),
        Some(json!({ "username": "reader" })),
    )
    .await;
    assert_eq!(status, 403);

    let (status, _, _) = json_req(
        &app,
        "DELETE",
        &format!("/api/works/{shared_id}/grants/{reader_id}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, 204);

    let (status, works, _) = json_req(&app, "GET", "/api/works", Some(&reader_tok), None).await;
    assert_eq!(status, 200);
    let titles: Vec<_> = works
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["title"].as_str().unwrap())
        .collect();
    assert!(!titles.contains(&"Shared"));

    // Admin still sees every work after revoke (unfettered access).
    let (status, _, _) =
        json_req(&app, "GET", &format!("/api/works/{shared_id}"), Some(&admin), None).await;
    assert_eq!(status, 200);
    let (status, _, _) =
        json_req(&app, "GET", &format!("/api/works/{secret_id}"), Some(&admin), None).await;
    assert_eq!(status, 200);

    // Granting to an admin is rejected — admins do not use work_grants.
    let (status, body, _) = json_req(
        &app,
        "POST",
        &format!("/api/works/{shared_id}/grants"),
        Some(&admin),
        Some(json!({ "username": "admin" })),
    )
    .await;
    assert_eq!(status, 400, "grant-to-admin should fail: {body}");

    // Process one idle job loop shouldn't panic
    routes::jobs::process_one(&state).await.unwrap();
}

#[tokio::test]
async fn review_edit_sets_storygraph_dirty_flag() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;
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
    let token = login(&app, "admin", "adminpass1234").await;
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
    let token = login(&app, "admin", "adminpass1234").await;
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
        Some(json!({
            "show_audio_gaps": false,
            "reader_infinite_scroll": true,
            "reader_typography": {
                "palette": "sepia",
                "font": "literata",
                "size": 115,
                "lineHeight": "loose",
                "measure": "narrow",
                "justify": true,
                "letterSpacing": "wide",
                "paragraphSpacing": "roomy",
                "indent": true,
                "hyphenate": true
            }
        })),
    )
    .await;
    assert_eq!(status, 204);
    let (status, settings, _) = json_req(&app, "GET", "/api/settings", Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(settings["show_audio_gaps"], false);
    assert_eq!(settings["reader_infinite_scroll"], true);
    assert_eq!(settings["reader_typography"]["palette"], "sepia");
    assert_eq!(settings["reader_typography"]["font"], "literata");
    assert_eq!(settings["reader_typography"]["size"], 115);
    assert_eq!(settings["reader_typography"]["lineHeight"], "loose");
    assert_eq!(settings["reader_typography"]["letterSpacing"], "wide");
    assert_eq!(settings["reader_typography"]["paragraphSpacing"], "roomy");
    assert_eq!(settings["reader_typography"]["indent"], true);
    assert_eq!(settings["reader_typography"]["hyphenate"], true);
    assert_eq!(settings["ui_theme"], "diarch");
    assert_eq!(settings["ui_theme_css"], "");

    let (status, _, _) = json_req(
        &app,
        "PUT",
        "/api/settings",
        Some(&token),
        Some(json!({
            "ui_theme": "Grove",
            "ui_theme_css": "@plugin \"daisyui/theme\" { name: \"grove\"; color-scheme: dark; --color-base-100: oklch(20% 0.02 140); --color-base-200: oklch(18% 0.02 140); --color-base-300: oklch(16% 0.02 140); --color-primary: #c4a35a; }"
        })),
    )
    .await;
    assert_eq!(status, 204);
    let (status, settings, _) = json_req(&app, "GET", "/api/settings", Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(settings["ui_theme"], "grove");
    let css = settings["ui_theme_css"].as_str().unwrap();
    assert!(css.contains("[data-theme=\"grove\"]"));
    assert!(css.contains("--color-primary: #c4a35a;"));
    assert!(!css.contains("@plugin"));

    let (status, _, _) = json_req(
        &app,
        "PUT",
        "/api/settings",
        Some(&token),
        Some(json!({ "ui_theme": "nord" })),
    )
    .await;
    assert_eq!(status, 204);
    let (status, settings, _) = json_req(&app, "GET", "/api/settings", Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(settings["ui_theme"], "nord");
    assert!(settings["ui_theme_css"].as_str().unwrap().contains("grove"));

    let (status, _, _) = json_req(
        &app,
        "PUT",
        "/api/settings",
        Some(&token),
        Some(json!({
            "ui_theme": "evil",
            "ui_theme_css": "--color-base-100: red; --color-base-200: url(https://x); --color-base-300: a; --color-primary: b;"
        })),
    )
    .await;
    assert_eq!(status, 204);
    let (status, settings, _) = json_req(&app, "GET", "/api/settings", Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(settings["ui_theme"], "evil");
    assert_eq!(settings["ui_theme_css"], "");

    // Unknown enum values are sanitized to defaults.
    let (status, _, _) = json_req(
        &app,
        "PUT",
        "/api/settings",
        Some(&token),
        Some(json!({
            "reader_typography": { "palette": "neon", "font": "comic", "size": 999 }
        })),
    )
    .await;
    assert_eq!(status, 204);
    let (status, settings, _) = json_req(&app, "GET", "/api/settings", Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(settings["reader_typography"]["palette"], "dark");
    assert_eq!(settings["reader_typography"]["font"], "serif");
    assert_eq!(settings["reader_typography"]["size"], 200);
}

#[tokio::test]
async fn markdown_import_job_and_confirm() {
    let (_dir, app, state) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;
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
    let token = login(&app, "admin", "adminpass1234").await;
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
    let token = login(&app, "admin", "adminpass1234").await;
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
    let token = login(&app, "admin", "adminpass1234").await;
    let (status, body, _) =
        json_req(&app, "POST", "/api/queue/needs_tts/export", Some(&token), None).await;
    assert_eq!(status, 200);
    assert!(body["todo"].as_str().unwrap().contains("ebook2audiobook"));
}

#[tokio::test]
async fn currently_reading_shelf_caps_at_three() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;
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

#[tokio::test]
async fn works_text_search_filters_by_q() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;

    let (status, _, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({
            "title": "The Left Hand of Darkness",
            "authors": "Ursula K. Le Guin",
            "isbn": "9780441478125"
        })),
    )
    .await;
    assert_eq!(status, 201);

    let (status, _, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({
            "title": "Snow Crash",
            "authors": "Neal Stephenson"
        })),
    )
    .await;
    assert_eq!(status, 201);

    let (status, list, _) =
        json_req(&app, "GET", "/api/works?q=le%20guin", Some(&token), None).await;
    assert_eq!(status, 200, "{list}");
    let arr = list.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["title"], "The Left Hand of Darkness");

    let (status, list, _) =
        json_req(&app, "GET", "/api/works?q=9780441478125", Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn reading_shelves_are_per_account() {
    let (_dir, app, _) = test_app().await;
    let admin_tok = login(&app, "admin", "adminpass1234").await;

    // Create a second reader account.
    let (status, reader, _) = json_req(
        &app,
        "POST",
        "/api/users",
        Some(&admin_tok),
        Some(json!({
            "username": "reader2",
            "password": "readerpass1234",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, 201, "{reader}");
    let reader_tok = login(&app, "reader2", "readerpass1234").await;

    // Admin creates a shared work and grants it to reader2.
    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&admin_tok),
        Some(json!({
            "title": "Shared Shelf Book",
            "authors": "A",
            "status": "reading"
        })),
    )
    .await;
    assert_eq!(status, 201, "{work}");
    let work_id = work["id"].as_str().unwrap();
    let (status, _, _) = json_req(
        &app,
        "POST",
        &format!("/api/works/{work_id}/grants"),
        Some(&admin_tok),
        Some(json!({ "username": "reader2" })),
    )
    .await;
    assert_eq!(status, 204);

    // Admin sees it on Currently Reading.
    let (status, list, _) =
        json_req(&app, "GET", "/api/works?status=reading", Some(&admin_tok), None).await;
    assert_eq!(status, 200);
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .any(|w| w["id"] == work_id),
        "admin should see the book on Currently Reading"
    );

    // Reader does not share that shelf — book starts unread for them.
    let (status, list, _) =
        json_req(&app, "GET", "/api/works?status=reading", Some(&reader_tok), None).await;
    assert_eq!(status, 200);
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .all(|w| w["id"] != work_id),
        "reader must not inherit admin's Currently Reading"
    );

    // Reader puts it on their own To Read without moving admin's shelf.
    let (status, updated, _) = json_req(
        &app,
        "PUT",
        &format!("/api/works/{work_id}"),
        Some(&reader_tok),
        Some(json!({ "status": "to_read" })),
    )
    .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["status"], "to_read");

    let (status, list, _) =
        json_req(&app, "GET", "/api/works?status=to_read", Some(&reader_tok), None).await;
    assert_eq!(status, 200);
    assert!(list.as_array().unwrap().iter().any(|w| w["id"] == work_id));

    let (status, list, _) =
        json_req(&app, "GET", "/api/works?status=reading", Some(&admin_tok), None).await;
    assert_eq!(status, 200);
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .any(|w| w["id"] == work_id),
        "admin's Currently Reading must be unchanged"
    );
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
    let token = login(&app, "admin", "adminpass1234").await;
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
    let token = login(&app, "admin", "adminpass1234").await;
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
async fn cover_candidate_approve_and_discard() {
    let (dir, app, state) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;
    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({"title":"Cover Stage","authors":"A"})),
    )
    .await;
    assert_eq!(status, 201, "{work}");
    let id = work["id"].as_str().unwrap();
    let wid: uuid::Uuid = id.parse().unwrap();

    // Without LocalAI, generate reports not configured.
    let (status, generated, _) = json_req(
        &app,
        "POST",
        &format!("/api/works/{id}/cover/generate"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(generated["ok"], false);
    assert!(generated["prompt"].as_str().unwrap().contains("Cover Stage"));

    // Stage a candidate on disk as LocalAI would.
    let candidate = state
        .config
        .work_dir(wid)
        .join(diarch_core::Config::cover_candidate_name());
    tokio::fs::create_dir_all(candidate.parent().unwrap())
        .await
        .unwrap();
    // Real JPEG magic bytes so apply_cover_bytes' format sniff accepts it.
    let mut fake_jpeg = vec![0xFFu8, 0xD8, 0xFF, 0xE0];
    fake_jpeg.extend_from_slice(b"fake-candidate-jpeg");
    tokio::fs::write(&candidate, &fake_jpeg)
        .await
        .unwrap();

    let (status, detail, _) =
        json_req(&app, "GET", &format!("/api/works/{id}"), Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(detail["has_cover_candidate"], true);
    assert_eq!(detail["work"]["needs_cover"], true);

    let (status, appr, _) = json_req(
        &app,
        "POST",
        &format!("/api/works/{id}/cover/approve"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{appr}");
    assert_eq!(appr["ok"], true);
    assert!(!candidate.exists());
    assert!(state.config.work_dir(wid).join("cover.jpg").exists());

    let (status, detail, _) =
        json_req(&app, "GET", &format!("/api/works/{id}"), Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(detail["has_cover_candidate"], false);
    assert_eq!(detail["work"]["needs_cover"], false);

    // Discard path
    tokio::fs::write(&candidate, b"another-candidate").await.unwrap();
    let (status, _, _) = json_req(
        &app,
        "POST",
        &format!("/api/works/{id}/cover/discard"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 204);
    assert!(!candidate.exists());

    // Reset to SVG placeholder from current (or overridden) title/authors.
    tokio::fs::write(state.config.work_dir(wid).join("cover.jpg"), b"keep-me")
        .await
        .unwrap();
    let (status, reset, _) = json_req(
        &app,
        "POST",
        &format!("/api/works/{id}/cover/placeholder"),
        Some(&token),
        Some(json!({"title":"Renamed Title","authors":"New Author"})),
    )
    .await;
    assert_eq!(status, 200, "{reset}");
    assert_eq!(reset["ok"], true);
    assert!(!state.config.work_dir(wid).join("cover.jpg").exists());
    let svg = tokio::fs::read_to_string(state.config.work_dir(wid).join("cover.svg"))
        .await
        .unwrap();
    assert!(svg.contains("Renamed Title"), "{svg}");
    assert!(svg.contains("New Author"), "{svg}");
    let (status, detail, _) =
        json_req(&app, "GET", &format!("/api/works/{id}"), Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(detail["work"]["needs_cover"], true);

    let _ = dir;
}

#[tokio::test]
async fn m4b_upload_attaches_and_streams_with_range() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;
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
    assert!(
        tr["message"].as_str().unwrap().contains("DIARCH_LOCALAI_URL"),
        "{tr}"
    );

    let (status, detail, _) =
        json_req(&app, "GET", &format!("/api/works/{id}"), Some(&token), None).await;
    assert_eq!(status, 200);
    assert!(detail.get("transcription_job").is_some());
}

#[tokio::test]
async fn aax_upload_without_key_fails_clearly() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;
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
    let token = login(&app, "admin", "adminpass1234").await;
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
    let token = login(&app, "admin", "adminpass1234").await;
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
async fn library_import_m4a_streams_to_book_m4b() {
    let (_dir, app, state) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;
    // Larger than a single multipart chunk so the handler must stream, not buffer.
    let mut fake = b"ftypM4A".to_vec();
    fake.extend(std::iter::repeat(0xABu8).take(256 * 1024));
    let boundary = "----libM4aBoundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"Hail Mary.m4a\"\r\nContent-Type: audio/mp4\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&fake);
    body.extend_from_slice(
        format!(
            "\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nProject Hail Mary\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"authors\"\r\n\r\nAndy Weir\r\n--{boundary}--\r\n"
        )
        .as_bytes(),
    );

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
    assert_eq!(res.status(), 201, "library m4a import");
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let j: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(j["kind"], "m4b");
    assert!(j["job_id"].is_null());
    assert_eq!(j["work"]["title"], "Project Hail Mary");
    assert_eq!(j["work"]["authors"], "Andy Weir");
    let id = j["work"]["id"].as_str().unwrap();
    let wid = uuid::Uuid::parse_str(id).unwrap();

    let stored = state.config.work_dir(wid).join("audio").join("book.m4b");
    let on_disk = tokio::fs::read(&stored).await.unwrap();
    assert_eq!(on_disk, fake);
    assert!(!state
        .config
        .work_dir(wid)
        .join("audio")
        .join(".book.m4b.partial")
        .exists());

    let (status, detail, _) =
        json_req(&app, "GET", &format!("/api/works/{id}"), Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(detail["work"]["needs_audio"], false);
    let audio = detail["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["kind"] == "audio")
        .expect("audio asset");
    assert_eq!(audio["bytes"].as_i64(), Some(fake.len() as i64));
    assert_eq!(audio["relative_path"], "audio/book.m4b");
}

#[tokio::test]
async fn library_import_aax_without_key_fails() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;
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
    let token = login(&app, "admin", "adminpass1234").await;
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

#[tokio::test]
async fn metadata_isbn_report_has_providers() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;
    // Nonsense ISBN that OL does not map (0000000000000 wrongly hits a real work).
    // Open Library miss; Google may be miss or error (429).
    let (status, body, _) = json_req(
        &app,
        "GET",
        "/api/metadata/isbn/1111111111111",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.get("hit").is_some() || body["hit"].is_null());
    let providers = body["providers"].as_array().expect("providers array");
    assert!(!providers.is_empty(), "{body}");
    let names: Vec<_> = providers
        .iter()
        .filter_map(|p| p["name"].as_str())
        .collect();
    assert!(names.contains(&"openlibrary"), "{names:?}");
    assert!(names.contains(&"googlebooks"), "{names:?}");
    assert!(names.contains(&"storygraph"), "{names:?}");
    for p in providers {
        let st = p["status"].as_str().unwrap_or("");
        assert!(
            matches!(st, "hit" | "miss" | "error"),
            "bad status {st} in {p}"
        );
        if st == "error" && p["name"] == "googlebooks" {
            let detail = p["detail"].as_str().unwrap_or("");
            assert!(
                detail.contains("rate limited")
                    || detail.contains("DIARCH_GOOGLE_BOOKS_KEY")
                    || detail.contains("forbidden")
                    || detail.contains("HTTP"),
                "{detail}"
            );
        }
        if st == "error" && p["name"] == "storygraph" {
            let detail = p["detail"].as_str().unwrap_or("");
            assert!(
                detail.contains("cookie not set")
                    || detail.contains("DIARCH_STORYGRAPH_COOKIE")
                    || detail.contains("Cloudflare")
                    || detail.contains("HTTP"),
                "{detail}"
            );
        }
    }
    assert!(body["hit"].is_null(), "expected no hit for fake ISBN: {body}");
}

#[tokio::test]
async fn integrations_probe_and_list() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;
    let (status, health, _) =
        json_req(&app, "POST", "/api/integrations/probe", Some(&token), None).await;
    assert_eq!(status, 200, "{health}");
    let rows = health.as_array().expect("health array");
    assert!(!rows.is_empty());
    let names: Vec<_> = rows
        .iter()
        .filter_map(|h| h["name"].as_str())
        .collect();
    for expected in ["pandoc", "storygraph", "remarkable", "ffmpeg", "pdftohtml"] {
        assert!(names.contains(&expected), "missing {expected} in {names:?}");
    }

    let (status, listed, _) =
        json_req(&app, "GET", "/api/integrations", Some(&token), None).await;
    assert_eq!(status, 200, "{listed}");
    assert!(listed.as_array().unwrap().len() >= rows.len());

    let (status, repair, _) = json_req(
        &app,
        "POST",
        "/api/integrations/pandoc/repair",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{repair}");
    assert_eq!(repair["ok"], true);
}

#[tokio::test]
async fn remarkable_status_and_auth_admin_only() {
    let (_dir, app, state) = test_app().await;
    let admin = login(&app, "admin", "adminpass1234").await;
    let (status, _, _) = json_req(
        &app,
        "POST",
        "/api/users",
        Some(&admin),
        Some(json!({"username":"reader1","password":"readerpass12","is_admin":false})),
    )
    .await;
    assert_eq!(status, 201);
    let reader = login(&app, "reader1", "readerpass12").await;

    let (status, st, _) =
        json_req(&app, "GET", "/api/remarkable/status", Some(&admin), None).await;
    assert_eq!(status, 200, "{st}");
    assert!(st["connect_url"].as_str().unwrap().contains("my.remarkable.com"));

    let (status, _, _) =
        json_req(&app, "GET", "/api/remarkable/status", Some(&reader), None).await;
    assert_eq!(status, 403);

    let (status, body, _) = json_req(
        &app,
        "POST",
        "/api/remarkable/auth",
        Some(&reader),
        Some(json!({"code":"short"})),
    )
    .await;
    assert_eq!(status, 403, "{body}");
    let _ = state;
}

#[tokio::test]
async fn remarkable_send_without_epub_reports_error() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;
    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({"title":"No Epub","authors":"A"})),
    )
    .await;
    assert_eq!(status, 201);
    let id = work["id"].as_str().unwrap();
    let (status, body, _) = json_req(
        &app,
        "POST",
        &format!("/api/works/{id}/remarkable"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], false);
    let err = body["error"].as_str().unwrap_or("");
    assert!(err.contains("EPUB") || err.contains("epub") || err.contains("rmapi"), "{err}");
}

#[tokio::test]
async fn clear_storygraph_flags_via_flags_endpoint() {
    let (_dir, app, state) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;
    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({"title":"Flagged","authors":"A"})),
    )
    .await;
    assert_eq!(status, 201);
    let id = work["id"].as_str().unwrap();
    let uuid = uuid::Uuid::parse_str(id).unwrap();
    let mut w = state.db.get_work(uuid).await.unwrap().unwrap();
    w.sg_review_dirty = true;
    w.sg_needs_add = true;
    w.sg_audio_only_remote = true;
    state.db.update_work(&w).await.unwrap();

    let (status, _, _) = json_req(
        &app,
        "POST",
        &format!("/api/works/{id}/flags/clear"),
        Some(&token),
        Some(json!({
            "flags": ["sg_review_dirty", "sg_needs_add", "sg_audio_only_remote"]
        })),
    )
    .await;
    assert_eq!(status, 204);

    let (status, detail, _) =
        json_req(&app, "GET", &format!("/api/works/{id}"), Some(&token), None).await;
    assert_eq!(status, 200, "{detail}");
    assert_eq!(detail["work"]["sg_review_dirty"], false);
    assert_eq!(detail["work"]["sg_needs_add"], false);
    assert_eq!(detail["work"]["sg_audio_only_remote"], false);
}

#[tokio::test]
async fn apply_meta_hit_sets_primary_from_subjects() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;
    let (status, work, _) = json_req(
        &app,
        "POST",
        "/api/works",
        Some(&token),
        Some(json!({"title":"Untitled","authors":""})),
    )
    .await;
    assert_eq!(status, 201, "{work}");
    let id = work["id"].as_str().unwrap();

    let (status, body, _) = json_req(
        &app,
        "POST",
        &format!("/api/works/{id}/apply-meta"),
        Some(&token),
        Some(json!({
            "title": "Dune",
            "authors": "Frank Herbert",
            "isbn": "9780441172719",
            "description": "Sandworms.",
            "subjects": ["Fiction", "Science Fiction"],
            "source": "googlebooks"
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["work"]["title"], "Dune");
    assert_eq!(body["work"]["authors"], "Frank Herbert");
    assert_eq!(body["work"]["isbn"], "9780441172719");
    assert_eq!(body["work"]["primary_code"], 8400);
    let codes = body["codes"].as_array().unwrap();
    assert!(codes.iter().any(|c| c == 8400));
    assert!(codes.iter().any(|c| c == 8000));
}

#[tokio::test]
async fn storygraph_sync_without_credentials_is_empty_ok() {
    let (_dir, app, _) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;
    let (status, body, _) =
        json_req(&app, "POST", "/api/storygraph/sync", Some(&token), None).await;
    // No username → empty pull, still a report.
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["sg_count"], 0);
    assert!(body["books"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn storygraph_status_and_auth_save_credentials() {
    let (_dir, app, state) = test_app().await;
    let token = login(&app, "admin", "adminpass1234").await;
    let (status, st, _) =
        json_req(&app, "GET", "/api/storygraph/status", Some(&token), None).await;
    assert_eq!(status, 200, "{st}");
    assert_eq!(st["username_set"], false);
    assert_eq!(st["cookie_set"], false);

    let (status, st, _) = json_req(
        &app,
        "POST",
        "/api/storygraph/auth",
        Some(&token),
        Some(json!({
            "username": "demo_user",
            "cookie": "remember_user_token=tokensecret99; cf_clearance=clearme1234",
            "user_agent": "Mozilla/5.0 TestAgent"
        })),
    )
    .await;
    assert_eq!(status, 200, "{st}");
    assert_eq!(st["username_set"], true);
    assert_eq!(st["cookie_set"], true);
    assert_eq!(st["remember_token"], true);
    assert_eq!(st["cf_clearance"], true);
    assert_eq!(st["verified"], true);
    assert_eq!(st["username"], "demo_user");
    let hint = st["cookie_hint"].as_str().unwrap_or("");
    assert!(hint.ends_with("et99"), "{hint}");
    // Never echo the full cookie in the JSON body.
    let body = st.to_string();
    assert!(!body.contains("tokensecret99"), "{body}");
    assert!(!body.contains("clearme1234"), "{body}");

    let path = state.config.data_dir.join("storygraph.conf");
    assert!(path.exists(), "expected credentials file at {}", path.display());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("demo_user"));
    assert!(text.contains("tokensecret99"));
    assert!(text.contains("cf_clearance=clearme1234"));
    assert!(text.contains("user_agent: Mozilla/5.0 TestAgent"));
}
