pub mod jobs;

use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::Utc;
use diarch_core::taxonomy::suggest_primary;
use diarch_core::{
    infer_manga, AssetKind, ReadingProgress, ReadingStatus, Work, WorkAsset,
};
use rust_embed::Embed;
use serde::{Deserialize, Serialize};
use std::path::Path as FsPath;
use std::sync::Arc;
use uuid::Uuid;

use crate::auth::{AdminUser, AuthUser};
use crate::metadata;
use crate::state::AppState;

#[derive(Embed)]
#[folder = "../../web/dist/"]
#[prefix = ""]
struct Assets;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
        .route("/api/users", get(list_users).post(create_user))
        .route("/api/taxonomy", get(list_taxonomy))
        .route("/api/works", get(list_works).post(create_work))
        .route("/api/library/import", post(library_import))
        .route("/api/works/{id}", get(get_work).put(update_work))
        .route("/api/works/{id}/codes", put(set_codes))
        .route("/api/works/{id}/grants", get(list_grants).post(add_grant))
        .route("/api/works/{id}/grants/{user_id}", axum::routing::delete(revoke_grant))
        .route("/api/works/{id}/import", post(import_file))
        .route("/api/works/{id}/confirm", post(confirm_review))
        .route("/api/works/{id}/download/epub", get(download_epub))
        .route("/api/works/{id}/download/markdown", get(download_md))
        .route("/api/works/{id}/cover", get(get_cover).post(upload_cover))
        .route("/api/works/{id}/cover/generate", post(generate_cover))
        .route("/api/works/{id}/cover/prompt", get(cover_prompt))
        .route("/api/works/{id}/audio", post(upload_audio))
        .route("/api/works/{id}/audio/{filename}", get(stream_audio))
        .route("/api/works/{id}/progress", get(get_progress).put(put_progress))
        .route("/api/works/{id}/content/markdown", get(get_markdown))
        .route("/api/works/{id}/content/epub", get(get_epub_file))
        .route("/api/works/{id}/remarkable", post(send_remarkable))
        .route("/api/works/{id}/flags/clear", post(clear_flags))
        .route("/api/works/{id}/refresh-metadata", post(refresh_metadata))
        .route("/api/wishlist", post(wishlist_add))
        .route("/api/metadata/isbn/{isbn}", get(meta_isbn))
        .route("/api/metadata/search", get(meta_search))
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/jobs", get(list_jobs))
        .route("/api/jobs/{id}", get(get_job))
        .route("/api/integrations", get(list_integrations))
        .route("/api/integrations/{name}/repair", post(repair_integration))
        .route("/api/storygraph/sync", post(sg_sync))
        .route("/api/queue/needs_tts/export", post(export_tts_queue))
        .route("/", get(index))
        .route("/assets/{*path}", get(static_asset))
        .layer(DefaultBodyLimit::max(512 * 1024 * 1024))
}

async fn health(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "ok": true,
        "service": "diarch",
        "data_dir": state.config.data_dir,
    }))
}

#[derive(Deserialize)]
struct LoginReq {
    username: String,
    password: String,
}

async fn login(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<LoginReq>,
) -> Result<(CookieJar, Json<serde_json::Value>), StatusCode> {
    let hash = state
        .db
        .get_password_hash(&body.username)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if !diarch_db::Db::verify_password(&body.password, &hash).unwrap_or(false) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let user = state
        .db
        .get_user_by_username(&body.username)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let token = state
        .db
        .create_session(user.id, 30)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let cookie = Cookie::build(("diarch_session", token.clone()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .build();
    Ok((
        jar.add(cookie),
        Json(serde_json::json!({ "token": token, "user": user })),
    ))
}

async fn logout(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> Result<(CookieJar, StatusCode), StatusCode> {
    if let Some(c) = jar.get("diarch_session") {
        let _ = state.db.delete_session(c.value()).await;
    }
    Ok((jar.remove(Cookie::from("diarch_session")), StatusCode::NO_CONTENT))
}

async fn me(AuthUser(user): AuthUser) -> Json<User> {
    Json(user)
}

// re-export User in scope
use diarch_core::User;

async fn list_users(AdminUser(_): AdminUser, State(state): State<Arc<AppState>>) -> Result<Json<Vec<User>>, StatusCode> {
    state
        .db
        .list_users()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[derive(Deserialize)]
struct CreateUserReq {
    username: String,
    password: String,
    is_admin: Option<bool>,
}

async fn create_user(
    AdminUser(_): AdminUser,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateUserReq>,
) -> Result<(StatusCode, Json<User>), StatusCode> {
    let u = state
        .db
        .create_user(&body.username, &body.password, body.is_admin.unwrap_or(false))
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok((StatusCode::CREATED, Json(u)))
}

async fn list_taxonomy(
    AuthUser(_): AuthUser,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<diarch_core::TaxonomyNode>>, StatusCode> {
    state
        .db
        .list_taxonomy()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[derive(Deserialize)]
struct ListQuery {
    status: Option<String>,
    attention: Option<String>,
}

async fn list_works(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<Work>>, StatusCode> {
    let mut works = state
        .db
        .list_works_for_user(&user, q.status.as_deref(), q.attention.as_deref())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !user.show_audio_gaps || !state.config.show_audio_gaps {
        // still return works; UI hides soft badges via settings — filter only if attention=needs_audio
    }
    let _ = &mut works;
    Ok(Json(works))
}

#[derive(Deserialize)]
struct CreateWorkReq {
    title: String,
    authors: Option<String>,
    isbn: Option<String>,
    description: Option<String>,
    status: Option<String>,
    codes: Option<Vec<i32>>,
    primary_code: Option<i32>,
    year_list: Option<i32>,
    is_manga: Option<bool>,
    wishlist: Option<bool>,
}

async fn create_work(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateWorkReq>,
) -> Result<(StatusCode, Json<Work>), (StatusCode, String)> {
    let now = Utc::now();
    let codes = body.codes.clone().unwrap_or_default();
    let primary = body
        .primary_code
        .or_else(|| suggest_primary(&codes));
    let is_manga = infer_manga(primary, &codes, body.is_manga.unwrap_or(false));
    let status = if body.wishlist.unwrap_or(false) {
        ReadingStatus::Wishlist
    } else {
        ReadingStatus::parse(body.status.as_deref().unwrap_or("unread"))
            .unwrap_or(ReadingStatus::Unread)
    };
    if let Some(cap) = status.shelf_cap() {
        let n = state
            .db
            .count_accessible_by_status(&user, status.as_str(), None)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db error".into()))?;
        if n >= cap {
            let label = match status {
                ReadingStatus::Reading => "Currently Reading",
                ReadingStatus::ToRead => "To Read",
                _ => status.as_str(),
            };
            return Err((
                StatusCode::CONFLICT,
                format!("{label} is full (max {cap}). Move a book off that shelf first."),
            ));
        }
    }
    let work = Work {
        id: Uuid::new_v4(),
        title: body.title,
        authors: body.authors.unwrap_or_default(),
        isbn: body.isbn,
        description: body.description,
        status,
        primary_code: primary,
        year_list: body.year_list,
        rating: None,
        review: None,
        reading_direction: if is_manga { "rtl".into() } else { "ltr".into() },
        is_manga,
        needs_review: false,
        needs_cover: true,
        needs_tts: false,
        needs_audio: false,
        needs_transcription: false,
        sg_review_dirty: false,
        sg_needs_add: false,
        sg_audio_only_remote: false,
        sg_matched: false,
        sg_book_id: None,
        created_by: Some(user.id),
        created_at: now,
        updated_at: now,
    };
    state
        .db
        .create_work(&work, &codes, Some(user.id))
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db error".into()))?;
    let dir = state.config.work_dir(work.id);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "fs error".into()))?;
    // placeholder cover
    let svg = metadata::placeholder_cover_svg(&work.title, &work.authors);
    let _ = tokio::fs::write(dir.join("cover.svg"), svg).await;
    Ok((StatusCode::CREATED, Json(work)))
}

async fn get_work(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state
        .db
        .user_can_access(&user, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let work = state
        .db
        .get_work(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let codes = state.db.work_codes(id).await.unwrap_or_default();
    let assets = state.db.list_assets(id).await.unwrap_or_default();
    Ok(Json(serde_json::json!({
        "work": work,
        "codes": codes,
        "assets": assets,
    })))
}

#[derive(Deserialize)]
struct UpdateWorkReq {
    title: Option<String>,
    authors: Option<String>,
    isbn: Option<String>,
    description: Option<String>,
    status: Option<String>,
    primary_code: Option<i32>,
    year_list: Option<i32>,
    rating: Option<f64>,
    review: Option<String>,
    reading_direction: Option<String>,
    is_manga: Option<bool>,
    needs_tts: Option<bool>,
    needs_audio: Option<bool>,
    needs_cover: Option<bool>,
    needs_transcription: Option<bool>,
    clear_sg_review_dirty: Option<bool>,
    clear_sg_needs_add: Option<bool>,
    clear_sg_audio_only_remote: Option<bool>,
}

async fn update_work(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateWorkReq>,
) -> Result<Json<Work>, (StatusCode, String)> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err((StatusCode::FORBIDDEN, "forbidden".into()));
    }
    let mut work = state
        .db
        .get_work(id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db error".into()))?
        .ok_or((StatusCode::NOT_FOUND, "not found".into()))?;
    if let Some(t) = body.title {
        work.title = t;
    }
    if let Some(a) = body.authors {
        work.authors = a;
    }
    if let Some(i) = body.isbn {
        work.isbn = Some(i);
    }
    if let Some(d) = body.description {
        work.description = Some(d);
    }
    if let Some(s) = body.status.as_deref().and_then(ReadingStatus::parse) {
        if s != work.status {
            if let Some(cap) = s.shelf_cap() {
                let n = state
                    .db
                    .count_accessible_by_status(&user, s.as_str(), Some(id))
                    .await
                    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db error".into()))?;
                if n >= cap {
                    let label = match s {
                        ReadingStatus::Reading => "Currently Reading",
                        ReadingStatus::ToRead => "To Read",
                        _ => s.as_str(),
                    };
                    return Err((
                        StatusCode::CONFLICT,
                        format!("{label} is full (max {cap}). Move a book off that shelf first."),
                    ));
                }
            }
        }
        work.status = s;
    }
    if let Some(p) = body.primary_code {
        work.primary_code = Some(p);
    }
    if let Some(y) = body.year_list {
        work.year_list = Some(y);
    }
    let mut review_changed = false;
    if let Some(r) = body.rating {
        if work.rating != Some(r) {
            review_changed = true;
        }
        work.rating = Some(r);
    }
    if let Some(r) = body.review {
        if work.review.as_deref() != Some(r.as_str()) {
            review_changed = true;
        }
        work.review = Some(r);
    }
    if review_changed {
        work.sg_review_dirty = true;
    }
    if let Some(d) = body.reading_direction {
        work.reading_direction = d;
    }
    if let Some(m) = body.is_manga {
        work.is_manga = m;
        if m {
            work.reading_direction = "rtl".into();
        }
    }
    if let Some(v) = body.needs_tts {
        work.needs_tts = v;
    }
    if let Some(v) = body.needs_audio {
        work.needs_audio = v;
    }
    if let Some(v) = body.needs_cover {
        work.needs_cover = v;
    }
    if let Some(v) = body.needs_transcription {
        work.needs_transcription = v;
    }
    if body.clear_sg_review_dirty.unwrap_or(false) {
        work.sg_review_dirty = false;
    }
    if body.clear_sg_needs_add.unwrap_or(false) {
        work.sg_needs_add = false;
    }
    if body.clear_sg_audio_only_remote.unwrap_or(false) {
        work.sg_audio_only_remote = false;
    }
    work.updated_at = Utc::now();
    state
        .db
        .update_work(&work)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db error".into()))?;
    Ok(Json(work))
}

#[derive(Deserialize)]
struct CodesReq {
    codes: Vec<i32>,
    primary_code: Option<i32>,
}

async fn set_codes(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<CodesReq>,
) -> Result<StatusCode, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    state
        .db
        .set_work_codes(id, &body.codes)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(mut work) = state.db.get_work(id).await.ok().flatten() {
        work.primary_code = body.primary_code.or_else(|| suggest_primary(&body.codes));
        work.is_manga = infer_manga(work.primary_code, &body.codes, work.is_manga);
        if work.is_manga {
            work.reading_direction = "rtl".into();
        }
        let _ = state.db.update_work(&work).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn list_grants(
    AdminUser(_): AdminUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<Uuid>>, StatusCode> {
    state
        .db
        .list_grants(id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[derive(Deserialize)]
struct GrantReq {
    user_id: Option<Uuid>,
    username: Option<String>,
}

async fn add_grant(
    AdminUser(_): AdminUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<GrantReq>,
) -> Result<StatusCode, StatusCode> {
    let user_id = if let Some(uid) = body.user_id {
        uid
    } else if let Some(name) = body.username.as_deref() {
        state
            .db
            .get_user_by_username(name)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?
            .id
    } else {
        return Err(StatusCode::BAD_REQUEST);
    };
    state
        .db
        .grant_work(user_id, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_grant(
    AdminUser(_): AdminUser,
    State(state): State<Arc<AppState>>,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, StatusCode> {
    state
        .db
        .revoke_work(user_id, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn import_file(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let mut saved: Option<(String, std::path::PathBuf)> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    {
        let name = field.file_name().unwrap_or("upload.bin").to_string();
        let data = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
        let tmp = state.config.data_dir.join("imports").join(format!("{id}-{name}"));
        tokio::fs::write(&tmp, &data)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        saved = Some((name, tmp));
    }
    let (name, path) = saved.ok_or(StatusCode::BAD_REQUEST)?;
    let job = state
        .db
        .create_job("import", Some(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    // stash path in job detail
    let _ = state
        .db
        .update_job(job.id, "pending", Some(&format!("{name}|{}", path.display())))
        .await;
    Ok(Json(serde_json::json!({ "job_id": job.id })))
}

/// Create a work and queue file import in one step (primary Library → Import EPUB flow).
async fn library_import(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let mut saved: Option<(String, Vec<u8>)> = None;
    let mut primary_code: Option<i32> = None;
    let mut title_override: Option<String> = None;
    let mut authors_override: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    {
        let field_name = field.name().unwrap_or("").to_string();
        if field_name == "file" || field.file_name().is_some() {
            let name = field.file_name().unwrap_or("book.epub").to_string();
            let data = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
            saved = Some((name, data.to_vec()));
        } else if field_name == "primary_code" {
            let text = field.text().await.unwrap_or_default();
            primary_code = text.trim().parse().ok();
        } else if field_name == "title" {
            let text = field.text().await.unwrap_or_default();
            if !text.trim().is_empty() {
                title_override = Some(text.trim().to_string());
            }
        } else if field_name == "authors" {
            let text = field.text().await.unwrap_or_default();
            if !text.trim().is_empty() {
                authors_override = Some(text.trim().to_string());
            }
        }
    }

    let (name, data) = saved.ok_or(StatusCode::BAD_REQUEST)?;
    let stem = FsPath::new(&name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .replace('_', " ")
        .replace('-', " ");

    // Peek EPUB metadata before creating the work when possible
    let tmp_peek = state
        .config
        .data_dir
        .join("imports")
        .join(format!("peek-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(state.config.data_dir.join("imports"))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    tokio::fs::write(&tmp_peek, &data)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let meta = if name.to_ascii_lowercase().ends_with(".epub") {
        diarch_import::read_epub_metadata(&tmp_peek).unwrap_or_default()
    } else {
        diarch_import::EpubMeta::default()
    };

    let hint = crate::metadata::taxonomy_from_metadata(
        &state,
        meta.isbn.as_deref(),
        &meta.subjects,
    )
    .await;

    let now = Utc::now();
    let mut codes = hint.codes.clone();
    if let Some(p) = primary_code {
        if !codes.contains(&p) {
            codes.push(p);
        }
    }
    let primary = primary_code
        .or(hint.primary)
        .or_else(|| diarch_core::taxonomy::suggest_primary(&codes));
    let is_manga = diarch_core::infer_manga(primary, &codes, false);
    let work = Work {
        id: Uuid::new_v4(),
        title: title_override
            .or(meta.title)
            .or(hint.title)
            .unwrap_or(stem),
        authors: authors_override
            .or(meta.authors)
            .or(hint.authors)
            .unwrap_or_default(),
        isbn: meta.isbn.or(hint.isbn),
        description: meta.description.or(hint.description),
        status: ReadingStatus::Unread,
        primary_code: primary,
        year_list: None,
        rating: None,
        review: None,
        reading_direction: if is_manga {
            "rtl".into()
        } else {
            "ltr".into()
        },
        is_manga,
        needs_review: false,
        needs_cover: true,
        needs_tts: false,
        needs_audio: false,
        needs_transcription: false,
        sg_review_dirty: false,
        sg_needs_add: false,
        sg_audio_only_remote: false,
        sg_matched: false,
        sg_book_id: None,
        created_by: Some(user.id),
        created_at: now,
        updated_at: now,
    };
    state
        .db
        .create_work(&work, &codes, Some(user.id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    tokio::fs::create_dir_all(state.config.work_dir(work.id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let svg = crate::metadata::placeholder_cover_svg(&work.title, &work.authors);
    let _ = tokio::fs::write(state.config.work_dir(work.id).join("cover.svg"), svg).await;

    let dest = state
        .config
        .data_dir
        .join("imports")
        .join(format!("{}-{}", work.id, name));
    tokio::fs::write(&dest, &data)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let _ = tokio::fs::remove_file(&tmp_peek).await;

    let job = state
        .db
        .create_job("import", Some(work.id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let _ = state
        .db
        .update_job(
            job.id,
            "pending",
            Some(&format!("{name}|{}", dest.display())),
        )
        .await;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "work": work,
            "job_id": job.id,
        })),
    ))
}

async fn confirm_review(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let job = state
        .db
        .create_job("confirm_epub", Some(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "job_id": job.id })))
}

async fn download_epub(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let path = state.config.work_dir(id).join("book.epub");
    let data = tokio::fs::read(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/epub+zip"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"book.epub\"",
            ),
        ],
        data,
    )
        .into_response())
}

async fn download_md(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let out = state.config.data_dir.join("imports").join(format!("{id}-md.zip"));
    diarch_import::zip_markdown_bundle(&state.config.library_dir(), id, &out)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let data = tokio::fs::read(&out)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"book-md.zip\"",
            ),
        ],
        data,
    )
        .into_response())
}

async fn get_cover(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let dir = state.config.work_dir(id);
    let jpg = dir.join("cover.jpg");
    if !jpg.exists() {
        let epub = dir.join("book.epub");
        if epub.exists() {
            if let Ok(Some(_)) = diarch_import::extract_epub_cover(&epub, &jpg).await {
                if let Ok(Some(mut w)) = state.db.get_work(id).await {
                    w.needs_cover = false;
                    w.updated_at = Utc::now();
                    let _ = state.db.update_work(&w).await;
                }
            }
        }
    }
    for name in ["cover.jpg", "cover.png", "cover.webp", "cover.svg"] {
        let p = dir.join(name);
        if p.exists() {
            let data = tokio::fs::read(&p)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let ct = match name {
                "cover.svg" => "image/svg+xml",
                "cover.png" => "image/png",
                "cover.webp" => "image/webp",
                _ => "image/jpeg",
            };
            return Ok((
                [
                    (header::CONTENT_TYPE, ct),
                    (header::CACHE_CONTROL, "private, max-age=300"),
                ],
                data,
            )
                .into_response());
        }
    }
    Err(StatusCode::NOT_FOUND)
}

async fn upload_cover(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<StatusCode, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    while let Some(field) = multipart.next_field().await.map_err(|_| StatusCode::BAD_REQUEST)? {
        let data = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
        let dest = state.config.work_dir(id).join("cover.jpg");
        tokio::fs::write(&dest, &data)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Some(mut w) = state.db.get_work(id).await.ok().flatten() {
            w.needs_cover = false;
            let _ = state.db.update_work(&w).await;
        }
        let asset = WorkAsset {
            id: Uuid::new_v4(),
            work_id: id,
            kind: AssetKind::Cover,
            relative_path: "cover.jpg".into(),
            mime: Some("image/jpeg".into()),
            bytes: Some(data.len() as i64),
            created_at: Utc::now(),
        };
        let _ = state.db.add_asset(&asset).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn generate_cover(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let work = state
        .db
        .get_work(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let genre = work
        .primary_code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "general".into());
    let prompt = metadata::cover_prompt(&work.title, &work.authors, &genre);
    match metadata::generate_cover_localai(&state, &prompt)
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?
    {
        Some(bytes) => {
            let dest = state.config.work_dir(id).join("cover.jpg");
            tokio::fs::write(&dest, &bytes)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            if let Some(mut w) = state.db.get_work(id).await.ok().flatten() {
                w.needs_cover = false;
                let _ = state.db.update_work(&w).await;
            }
            Ok(Json(serde_json::json!({ "ok": true, "prompt": prompt })))
        }
        None => Ok(Json(serde_json::json!({
            "ok": false,
            "prompt": prompt,
            "message": "LocalAI/Hermes unavailable; use prompt manually"
        }))),
    }
}

async fn cover_prompt(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let work = state
        .db
        .get_work(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let genre = work
        .primary_code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "general".into());
    Ok(Json(serde_json::json!({
        "prompt": metadata::cover_prompt(&work.title, &work.authors, &genre)
    })))
}

async fn upload_audio(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let audio_dir = state.config.work_dir(id).join("audio");
    tokio::fs::create_dir_all(&audio_dir)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut filename = String::from("book.m4a");
    while let Some(field) = multipart.next_field().await.map_err(|_| StatusCode::BAD_REQUEST)? {
        filename = field.file_name().unwrap_or("book.m4a").to_string();
        let data = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
        let dest = audio_dir.join(&filename);
        tokio::fs::write(&dest, &data)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let asset = WorkAsset {
            id: Uuid::new_v4(),
            work_id: id,
            kind: AssetKind::Audio,
            relative_path: format!("audio/{filename}"),
            mime: Some("audio/mp4".into()),
            bytes: Some(data.len() as i64),
            created_at: Utc::now(),
        };
        let _ = state.db.add_asset(&asset).await;
        if let Some(mut w) = state.db.get_work(id).await.ok().flatten() {
            w.needs_audio = false;
            w.sg_audio_only_remote = false;
            let _ = state.db.update_work(&w).await;
        }
    }
    Ok(Json(serde_json::json!({ "filename": filename })))
}

async fn stream_audio(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(Uuid, String)>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let path = state.config.work_dir(id).join("audio").join(&filename);
    let data = tokio::fs::read(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let len = data.len() as u64;
    if let Some(range) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        if let Some(r) = range.strip_prefix("bytes=") {
            let mut parts = r.split('-');
            let start: u64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
            let end: u64 = parts
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(len - 1)
                .min(len - 1);
            let slice = data[start as usize..=end as usize].to_vec();
            return Ok((
                StatusCode::PARTIAL_CONTENT,
                [
                    (header::CONTENT_TYPE, "audio/mp4".into()),
                    (
                        header::CONTENT_RANGE,
                        format!("bytes {start}-{end}/{len}"),
                    ),
                    (header::ACCEPT_RANGES, "bytes".into()),
                    (header::CONTENT_LENGTH, slice.len().to_string()),
                ],
                slice,
            )
                .into_response());
        }
    }
    Ok((
        [
            (header::CONTENT_TYPE, "audio/mp4"),
            (header::ACCEPT_RANGES, "bytes"),
        ],
        data,
    )
        .into_response())
}

#[derive(Deserialize)]
struct ProgressQuery {
    mode: Option<String>,
}

async fn get_progress(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<ProgressQuery>,
) -> Result<Json<Option<ReadingProgress>>, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let mode = q.mode.as_deref().unwrap_or("epub");
    state
        .db
        .get_progress(user.id, id, mode)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[derive(Deserialize)]
struct ProgressBody {
    mode: Option<String>,
    position: String,
    percent: f64,
}

async fn put_progress(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<ProgressBody>,
) -> Result<StatusCode, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let p = ReadingProgress {
        user_id: user.id,
        work_id: id,
        mode: body.mode.unwrap_or_else(|| "epub".into()),
        position: body.position,
        percent: body.percent,
        updated_at: Utc::now(),
    };
    state
        .db
        .upsert_progress(&p)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_markdown(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let path = state.config.work_dir(id).join("book.md");
    let data = tokio::fs::read(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(([(header::CONTENT_TYPE, "text/markdown; charset=utf-8")], data).into_response())
}

async fn get_epub_file(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let path = state.config.work_dir(id).join("book.epub");
    let data = tokio::fs::read(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/epub+zip"),
            (
                header::CONTENT_DISPOSITION,
                "inline; filename=\"book.epub\"",
            ),
        ],
        data,
    )
        .into_response())
}

async fn send_remarkable(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    match crate::remarkable::send_epub(&state, id).await {
        Ok(()) => Ok(Json(serde_json::json!({ "ok": true }))),
        Err(e) => Ok(Json(serde_json::json!({ "ok": false, "error": e.to_string() }))),
    }
}

async fn refresh_metadata(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Work>, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let mut work = state
        .db
        .get_work(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let epub = state.config.work_dir(id).join("book.epub");
    let meta = if epub.exists() {
        diarch_import::read_epub_metadata(&epub).unwrap_or_default()
    } else {
        diarch_import::EpubMeta::default()
    };

    let isbn = work.isbn.clone().or(meta.isbn.clone());
    let hint = metadata::taxonomy_from_metadata(&state, isbn.as_deref(), &meta.subjects).await;

    if work.isbn.is_none() {
        work.isbn = meta.isbn.or(hint.isbn.clone());
    }
    if work.description.is_none() {
        work.description = meta.description.or(hint.description.clone());
    }
    if work.authors.is_empty() {
        if let Some(a) = meta.authors.or(hint.authors.clone()) {
            work.authors = a;
        }
    }
    if (work.title.is_empty() || work.title == "Untitled") && meta.title.is_some() {
        work.title = meta.title.unwrap();
    }

    let mut codes = state.db.work_codes(id).await.unwrap_or_default();
    for c in &hint.codes {
        if !codes.contains(c) {
            codes.push(*c);
        }
    }
    if !codes.is_empty() {
        state
            .db
            .set_work_codes(id, &codes)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    work.primary_code =
        diarch_core::taxonomy::prefer_primary(work.primary_code, hint.primary);
    work.is_manga = infer_manga(work.primary_code, &codes, work.is_manga);
    if work.is_manga {
        work.reading_direction = "rtl".into();
    }
    work.updated_at = Utc::now();
    state
        .db
        .update_work(&work)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(work))
}

#[derive(Deserialize)]
struct ClearFlags {
    flags: Vec<String>,
}

async fn clear_flags(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<ClearFlags>,
) -> Result<StatusCode, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let mut work = state
        .db
        .get_work(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    for f in body.flags {
        match f.as_str() {
            "sg_review_dirty" => work.sg_review_dirty = false,
            "sg_needs_add" => work.sg_needs_add = false,
            "sg_audio_only_remote" => work.sg_audio_only_remote = false,
            "needs_tts" => work.needs_tts = false,
            "needs_audio" => work.needs_audio = false,
            "needs_cover" => work.needs_cover = false,
            "needs_review" => work.needs_review = false,
            _ => {}
        }
    }
    state.db.update_work(&work).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct WishlistReq {
    title: Option<String>,
    authors: Option<String>,
    isbn: Option<String>,
    codes: Option<Vec<i32>>,
    primary_code: Option<i32>,
}

async fn wishlist_add(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Json(body): Json<WishlistReq>,
) -> Result<(StatusCode, Json<Work>), (StatusCode, String)> {
    let mut title = body.title.unwrap_or_default();
    let mut authors = body.authors.unwrap_or_default();
    let mut isbn = body.isbn.clone();
    let mut description = None;

    if let Some(ref isbn_v) = body.isbn {
        if let Ok(Some(hit)) = metadata::lookup_isbn(&state, isbn_v).await {
            if title.is_empty() {
                title = hit.title;
            }
            if authors.is_empty() {
                authors = hit.authors;
            }
            isbn = hit.isbn.or(isbn);
            description = hit.description;
        }
    } else if !title.is_empty() {
        let author_ref = if authors.is_empty() {
            None
        } else {
            Some(authors.as_str())
        };
        if let Ok(hits) = metadata::search_title(&state, &title, author_ref).await {
            if let Some(hit) = hits.into_iter().next() {
                if authors.is_empty() {
                    authors = hit.authors;
                }
                isbn = hit.isbn.or(isbn);
                description = hit.description;
            }
        }
    }
    if title.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "title or isbn required".into()));
    }

    create_work(
        AuthUser(user),
        State(state),
        Json(CreateWorkReq {
            title,
            authors: Some(authors),
            isbn,
            description,
            status: Some("wishlist".into()),
            codes: body.codes,
            primary_code: body.primary_code,
            year_list: None,
            is_manga: None,
            wishlist: Some(true),
        }),
    )
    .await
}

async fn meta_isbn(
    AuthUser(_): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(isbn): Path<String>,
) -> Result<Json<Option<metadata::MetaHit>>, StatusCode> {
    metadata::lookup_isbn(&state, &isbn)
        .await
        .map(Json)
        .map_err(|_| StatusCode::BAD_GATEWAY)
}

#[derive(Deserialize)]
struct SearchQ {
    title: String,
    author: Option<String>,
}

async fn meta_search(
    AuthUser(_): AuthUser,
    State(state): State<Arc<AppState>>,
    Query(q): Query<SearchQ>,
) -> Result<Json<Vec<metadata::MetaHit>>, StatusCode> {
    metadata::search_title(&state, &q.title, q.author.as_deref())
        .await
        .map(Json)
        .map_err(|_| StatusCode::BAD_GATEWAY)
}

#[derive(Serialize)]
struct Settings {
    show_audio_gaps: bool,
    reader_infinite_scroll: bool,
}

async fn get_settings(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Settings>, StatusCode> {
    let scroll = state
        .db
        .get_reader_infinite_scroll(user.id)
        .await
        .unwrap_or(false);
    Ok(Json(Settings {
        show_audio_gaps: user.show_audio_gaps && state.config.show_audio_gaps,
        reader_infinite_scroll: scroll,
    }))
}

#[derive(Deserialize)]
struct SettingsReq {
    show_audio_gaps: Option<bool>,
    reader_infinite_scroll: Option<bool>,
}

async fn put_settings(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Json(body): Json<SettingsReq>,
) -> Result<StatusCode, StatusCode> {
    if let Some(v) = body.show_audio_gaps {
        state
            .db
            .set_user_show_audio_gaps(user.id, v)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    if let Some(v) = body.reader_infinite_scroll {
        state
            .db
            .set_reader_infinite_scroll(user.id, v)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn list_jobs(
    AdminUser(_): AdminUser,
    State(_state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "message": "use GET /api/jobs/{id}" }))
}

async fn get_job(
    AuthUser(_): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<diarch_core::Job>, StatusCode> {
    state
        .db
        .get_job(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn list_integrations(
    AdminUser(_): AdminUser,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<diarch_core::IntegrationHealth>>, StatusCode> {
    state
        .db
        .list_integration_health()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn repair_integration(
    AdminUser(_): AdminUser,
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Json<serde_json::Value> {
    match crate::fixer::repair(&state, &name).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e })),
    }
}

async fn sg_sync(
    AdminUser(_): AdminUser,
    State(state): State<Arc<AppState>>,
) -> Result<Json<crate::storygraph::SyncReport>, StatusCode> {
    crate::storygraph::sync_flags(&state)
        .await
        .map(Json)
        .map_err(|_| StatusCode::BAD_GATEWAY)
}

async fn export_tts_queue(
    AdminUser(_): AdminUser,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let works = state
        .db
        .works_needing_tts()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let ids: Vec<_> = works.iter().map(|w| w.id).collect();
    let n = diarch_import::export_needs_tts_queue(
        &state.config.library_dir(),
        &state.config.data_dir.join("queue/needs_tts"),
        &ids,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({
        "exported": n,
        "todo": "Desktop watcher for ebook2audiobook should consume queue/needs_tts — see docs/TODO-ebook2audiobook-watcher.md"
    })))
}

async fn index() -> Response {
    match Assets::get("index.html") {
        Some(f) => Html(String::from_utf8_lossy(&f.data).to_string()).into_response(),
        None => Html(include_str!("../../../../web/public/index.html")).into_response(),
    }
}

async fn static_asset(Path(path): Path<String>) -> Result<Response, StatusCode> {
    let path = path.trim_start_matches('/');
    let file = Assets::get(path)
        .or_else(|| Assets::get(&format!("assets/{path}")))
        .ok_or(StatusCode::NOT_FOUND)?;
    let ct = mime_guess(path);
    Ok(([(header::CONTENT_TYPE, ct)], file.data.to_vec()).into_response())
}

fn mime_guess(path: &str) -> &'static str {
    if path.ends_with(".js") {
        "application/javascript"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".html") {
        "text/html"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
}
