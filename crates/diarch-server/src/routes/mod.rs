pub mod jobs;

use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::Utc;
use diarch_core::taxonomy::suggest_primary;
use diarch_core::{
    infer_manga, AssetKind, ReadingProgress, ReadingStatus, User, Work, WorkAsset,
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
        .route(
            "/api/works/{id}",
            get(get_work).put(update_work).delete(delete_work),
        )
        .route("/api/works/{id}/codes", put(set_codes))
        .route("/api/works/{id}/apply-meta", post(apply_meta_hit))
        .route("/api/works/{id}/grants", get(list_grants).post(add_grant))
        .route("/api/works/{id}/grants/{user_id}", delete(revoke_grant))
        .route("/api/works/{id}/import", post(import_file))
        .route("/api/works/{id}/confirm", post(confirm_review))
        .route("/api/works/{id}/download/epub", get(download_epub))
        .route("/api/works/{id}/download/markdown", get(download_md))
        .route("/api/works/{id}/cover", get(get_cover).post(upload_cover))
        .route("/api/works/{id}/cover/fetch", post(fetch_cover))
        .route("/api/works/{id}/cover/generate", post(generate_cover))
        .route("/api/works/{id}/cover/placeholder", post(reset_cover_placeholder))
        .route("/api/works/{id}/cover/prompt", get(cover_prompt))
        .route("/api/works/{id}/cover/candidate", get(get_cover_candidate))
        .route("/api/works/{id}/cover/approve", post(approve_cover_candidate))
        .route("/api/works/{id}/cover/discard", post(discard_cover_candidate))
        .route("/api/works/{id}/audio", post(upload_audio))
        .route("/api/works/{id}/audio/chapters", get(audio_chapters))
        .route("/api/works/{id}/transcribe", post(request_transcribe))
        .route("/api/works/{id}/transcript", get(get_transcript))
        .route("/api/works/{id}/audio/{filename}", get(stream_audio))
        .route("/api/works/{id}/progress", get(get_progress).put(put_progress))
        .route(
            "/api/works/{id}/content/markdown",
            get(get_markdown).put(put_markdown),
        )
        .route("/api/works/{id}/content/pdf", get(get_pdf_file))
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
        .route("/api/integrations/probe", post(probe_integrations))
        .route("/api/integrations/{name}/repair", post(repair_integration))
        .route("/api/remarkable/status", get(remarkable_status))
        .route("/api/remarkable/auth", post(remarkable_auth))
        .route("/api/storygraph/sync", post(sg_sync))
        .route("/api/storygraph/status", get(sg_status))
        .route("/api/storygraph/auth", post(sg_auth))
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
    year_list: Option<i32>,
}

#[derive(Serialize)]
struct WorkListItem {
    #[serde(flatten)]
    work: Work,
    has_epub: bool,
    has_md: bool,
    has_audio: bool,
}

async fn list_works(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<WorkListItem>>, StatusCode> {
    let works = state
        .db
        .list_works_for_user_filtered(
            &user,
            q.status.as_deref(),
            q.attention.as_deref(),
            q.year_list,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let ids: Vec<_> = works.iter().map(|w| w.id).collect();
    let flags = state
        .db
        .asset_flags_for_works(&ids)
        .await
        .unwrap_or_default();
    let items = works
        .into_iter()
        .map(|work| {
            let (has_epub, has_md, has_audio) = flags.get(&work.id).copied().unwrap_or((false, false, false));
            WorkListItem {
                work,
                has_epub,
                has_md,
                has_audio,
            }
        })
        .collect();
    Ok(Json(items))
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
    let has_import_pdf = state.config.work_dir(id).join("import.pdf").exists();
    let has_cover_candidate = state
        .config
        .work_dir(id)
        .join(diarch_core::Config::cover_candidate_name())
        .exists();
    let has_transcript = state.config.work_dir(id).join("transcript.txt").exists();
    let transcription_job = state
        .db
        .latest_job_for_work(id, "transcribe")
        .await
        .ok()
        .flatten();
    Ok(Json(serde_json::json!({
        "work": work,
        "codes": codes,
        "assets": assets,
        "has_import_pdf": has_import_pdf,
        "has_cover_candidate": has_cover_candidate,
        "has_transcript": has_transcript,
        "transcription_job": transcription_job,
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
    /// When true, clear `year_list` (null). Takes precedence over `year_list`.
    clear_year_list: Option<bool>,
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

async fn delete_work(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err((StatusCode::FORBIDDEN, "forbidden".into()));
    }
    let work = state
        .db
        .get_work(id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db error".into()))?
        .ok_or((StatusCode::NOT_FOUND, "not found".into()))?;
    if !diarch_db::Db::user_can_delete(&user, &work) {
        return Err((
            StatusCode::FORBIDDEN,
            "only the creator or an admin can delete this work".into(),
        ));
    }
    let work_dir = state.config.work_dir(id);
    let removed = state
        .db
        .delete_work(id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "delete failed".into()))?;
    if !removed {
        return Err((StatusCode::NOT_FOUND, "not found".into()));
    }
    if work_dir.exists() {
        let _ = tokio::fs::remove_dir_all(&work_dir).await;
    }
    // Quarantine uploads named `{work_id}-…`
    let imports = state.config.data_dir.join("imports");
    if let Ok(mut rd) = tokio::fs::read_dir(&imports).await {
        let prefix = format!("{id}-");
        while let Ok(Some(entry)) = rd.next_entry().await {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(&prefix)
            {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
    }
    Ok(StatusCode::NO_CONTENT)
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
    if body.clear_year_list.unwrap_or(false) {
        work.year_list = None;
    } else if let Some(y) = body.year_list {
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

/// Apply a catalog MetaHit onto a work (fields + subject→taxonomy mapping).
async fn apply_meta_hit(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(mut hit): Json<metadata::MetaHit>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let mut work = state
        .db
        .get_work(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    metadata::attach_taxonomy(&mut hit);

    if !hit.title.is_empty() {
        work.title = hit.title.clone();
    }
    if !hit.authors.is_empty() {
        work.authors = hit.authors.clone();
    }
    if let Some(isbn) = hit.isbn.clone().filter(|s| !s.is_empty()) {
        work.isbn = Some(isbn);
    }
    if let Some(desc) = hit.description.clone().filter(|s| !s.is_empty()) {
        work.description = Some(desc);
    }

    let mut codes = state.db.work_codes(id).await.unwrap_or_default();
    for c in &hit.codes {
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
        diarch_core::taxonomy::prefer_primary(work.primary_code, hit.primary_code);
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

    Ok(Json(serde_json::json!({
        "work": work,
        "codes": codes,
        "applied_primary": hit.primary_code,
        "applied_subjects": hit.subjects,
        "source": hit.source,
    })))
}

async fn list_grants(
    AdminUser(_): AdminUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let rows = state
        .db
        .list_grants_detailed(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        rows.into_iter()
            .map(|(user_id, username)| {
                serde_json::json!({
                    "user_id": user_id,
                    "username": username,
                })
            })
            .collect(),
    ))
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

/// Create a work and queue file import in one step (Library → Import).
/// Accepts ebook (.epub/.pdf/.md) or audiobook (.m4b/.aax).
async fn library_import(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let mut saved: Option<(String, Vec<u8>)> = None;
    let mut primary_code: Option<i32> = None;
    let mut title_override: Option<String> = None;
    let mut authors_override: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad multipart".into()))?
    {
        let field_name = field.name().unwrap_or("").to_string();
        if field_name == "file" || field.file_name().is_some() {
            let name = field.file_name().unwrap_or("book.epub").to_string();
            let data = field
                .bytes()
                .await
                .map_err(|_| (StatusCode::BAD_REQUEST, "bad file body".into()))?;
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

    let (name, data) = saved.ok_or((StatusCode::BAD_REQUEST, "file required".into()))?;
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".mp3") {
        return Err((
            StatusCode::BAD_REQUEST,
            "MP3 is not supported; use .m4b or Audible .aax".into(),
        ));
    }
    if lower.ends_with(".aax") || lower.ends_with(".m4b") || lower.ends_with(".m4a") {
        return library_import_audio(
            &state,
            &user,
            name,
            data,
            title_override,
            authors_override,
            primary_code,
        )
        .await;
    }

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
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "imports dir failed".into(),
            )
        })?;
    tokio::fs::write(&tmp_peek, &data)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "peek write failed".into()))?;
    let meta = if lower.ends_with(".epub") {
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
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "create work failed".into(),
            )
        })?;
    tokio::fs::create_dir_all(state.config.work_dir(work.id))
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "mkdir work failed".into()))?;
    let svg = crate::metadata::placeholder_cover_svg(&work.title, &work.authors);
    let _ = tokio::fs::write(state.config.work_dir(work.id).join("cover.svg"), svg).await;

    let dest = state
        .config
        .data_dir
        .join("imports")
        .join(format!("{}-{}", work.id, name));
    tokio::fs::write(&dest, &data)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "store import failed".into()))?;
    let _ = tokio::fs::remove_file(&tmp_peek).await;

    let job = state
        .db
        .create_job("import", Some(work.id))
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "job create failed".into()))?;
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
            "kind": "import",
        })),
    ))
}

/// Library Import for audiobook-only files: create work + attach M4B or queue AAX convert.
async fn library_import_audio(
    state: &Arc<AppState>,
    user: &User,
    name: String,
    data: Vec<u8>,
    title_override: Option<String>,
    authors_override: Option<String>,
    primary_code: Option<i32>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let lower = name.to_ascii_lowercase();
    let stem = FsPath::new(&name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .replace('_', " ")
        .replace('-', " ");

    let is_aax = lower.ends_with(".aax");
    if is_aax {
        if state.config.audible_key.as_deref().unwrap_or("").is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "DIARCH_AUDIBLE_KEY is not set; cannot convert AAX".into(),
            ));
        }
        if !crate::audio::ffmpeg_available() {
            return Err((
                StatusCode::BAD_REQUEST,
                "ffmpeg not found on PATH; required for AAX → M4B".into(),
            ));
        }
    }

    let now = Utc::now();
    let codes = primary_code.map(|p| vec![p]).unwrap_or_default();
    let primary = primary_code.or_else(|| diarch_core::taxonomy::suggest_primary(&codes));
    let work = Work {
        id: Uuid::new_v4(),
        title: title_override.unwrap_or(stem),
        authors: authors_override.unwrap_or_default(),
        isbn: None,
        description: None,
        status: ReadingStatus::Unread,
        primary_code: primary,
        year_list: None,
        rating: None,
        review: None,
        reading_direction: "ltr".into(),
        is_manga: false,
        needs_review: false,
        needs_cover: true,
        needs_tts: false,
        needs_audio: is_aax, // cleared when convert finishes
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
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "create work failed".into(),
            )
        })?;
    let work_dir = state.config.work_dir(work.id);
    tokio::fs::create_dir_all(&work_dir)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "mkdir work failed".into()))?;
    let svg = crate::metadata::placeholder_cover_svg(&work.title, &work.authors);
    let _ = tokio::fs::write(work_dir.join("cover.svg"), svg).await;

    if is_aax {
        let aax_path = work_dir.join("import.aax");
        tokio::fs::write(&aax_path, &data)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "failed to store AAX".into()))?;
        let job = state
            .db
            .create_job("aax_to_m4b", Some(work.id))
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "job create failed".into()))?;
        let _ = state
            .db
            .update_job(job.id, "pending", Some("import.aax"))
            .await;
        return Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({
                "work": work,
                "job_id": job.id,
                "kind": "aax_to_m4b",
                "message": "Converting AAX → M4B",
            })),
        ));
    }

    // .m4b / .m4a → attach immediately as book.m4b
    let audio_dir = work_dir.join("audio");
    tokio::fs::create_dir_all(&audio_dir)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "mkdir audio failed".into()))?;
    let dest = audio_dir.join(crate::audio::BOOK_M4B);
    tokio::fs::write(&dest, &data)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "write m4b failed".into()))?;
    let asset = WorkAsset {
        id: Uuid::new_v4(),
        work_id: work.id,
        kind: AssetKind::Audio,
        relative_path: format!("audio/{}", crate::audio::BOOK_M4B),
        mime: Some("audio/mp4".into()),
        bytes: Some(data.len() as i64),
        created_at: Utc::now(),
    };
    let _ = state.db.add_asset(&asset).await;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "work": work,
            "job_id": serde_json::Value::Null,
            "kind": "m4b",
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

async fn work_title(state: &AppState, id: Uuid) -> String {
    state
        .db
        .get_work(id)
        .await
        .ok()
        .flatten()
        .map(|w| w.title)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "Untitled".into())
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
    let title = work_title(&state, id).await;
    let disp = diarch_core::content_disposition_attachment(&title, "epub");
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/epub+zip")
        .header(header::CONTENT_DISPOSITION, disp)
        .body(axum::body::Body::from(data))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()))
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
    let title = work_title(&state, id).await;
    let disp = diarch_core::content_disposition_attachment(&title, "md.zip");
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_DISPOSITION, disp)
        .body(axum::body::Body::from(data))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()))
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
        apply_cover_bytes(&state, id, &data)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn apply_cover_bytes(state: &AppState, id: Uuid, data: &[u8]) -> anyhow::Result<()> {
    let dest = state.config.work_dir(id).join("cover.jpg");
    tokio::fs::write(&dest, data).await?;
    if let Some(mut w) = state.db.get_work(id).await.ok().flatten() {
        w.needs_cover = false;
        w.updated_at = Utc::now();
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
    Ok(())
}

async fn fetch_cover(
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
    let Some(isbn) = work.isbn.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(Json(serde_json::json!({
            "ok": false,
            "message": "No ISBN on this work — add an ISBN first"
        })));
    };
    match metadata::fetch_remote_cover(&state, isbn)
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?
    {
        Some(bytes) => {
            apply_cover_bytes(&state, id, &bytes)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(Json(serde_json::json!({ "ok": true })))
        }
        None => Ok(Json(serde_json::json!({
            "ok": false,
            "message": "No remote cover found for this ISBN"
        }))),
    }
}

/// Remove raster covers / candidates and write a fresh SVG placeholder from
/// current (or optionally provided) title + authors. Sets `needs_cover`.
async fn reset_cover_placeholder(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<PlaceholderCoverReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let mut work = state
        .db
        .get_work(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let title = req
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(work.title.as_str());
    let authors = req
        .authors
        .as_deref()
        .map(str::trim)
        .unwrap_or(work.authors.as_str());

    let dir = state.config.work_dir(id);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    for name in [
        "cover.jpg",
        "cover.png",
        "cover.webp",
        diarch_core::Config::cover_candidate_name(),
    ] {
        let p = dir.join(name);
        if p.exists() {
            let _ = tokio::fs::remove_file(&p).await;
        }
    }
    let svg = metadata::placeholder_cover_svg(title, authors);
    let svg_path = dir.join("cover.svg");
    tokio::fs::write(&svg_path, svg.as_bytes())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    work.needs_cover = true;
    work.updated_at = Utc::now();
    state
        .db
        .update_work(&work)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({
        "ok": true,
        "needs_cover": true,
        "path": "cover.svg"
    })))
}

#[derive(Default, Deserialize)]
struct PlaceholderCoverReq {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    authors: Option<String>,
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
    if state.config.localai_url.is_none() && state.config.hermes_url.is_none() {
        return Ok(Json(serde_json::json!({
            "ok": false,
            "prompt": prompt,
            "message": "LocalAI not configured — set DIARCH_LOCALAI_URL"
        })));
    }
    match metadata::generate_cover_localai(&state, &prompt).await {
        Ok(Some(bytes)) => {
            let dest = state
                .config
                .work_dir(id)
                .join(diarch_core::Config::cover_candidate_name());
            tokio::fs::write(&dest, &bytes)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            // Staged: do not touch cover.jpg or needs_cover until approve.
            Ok(Json(serde_json::json!({
                "ok": true,
                "staged": true,
                "prompt": prompt,
                "bytes": bytes.len()
            })))
        }
        Ok(None) => Ok(Json(serde_json::json!({
            "ok": false,
            "prompt": prompt,
            "message": "LocalAI unavailable; use Fetch cover or Upload"
        }))),
        Err(e) => Ok(Json(serde_json::json!({
            "ok": false,
            "prompt": prompt,
            "message": e.to_string()
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

async fn get_cover_candidate(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let p = state
        .config
        .work_dir(id)
        .join(diarch_core::Config::cover_candidate_name());
    let data = tokio::fs::read(&p)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "private, no-store"),
        ],
        data,
    )
        .into_response())
}

async fn approve_cover_candidate(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let dir = state.config.work_dir(id);
    let candidate = dir.join(diarch_core::Config::cover_candidate_name());
    let data = tokio::fs::read(&candidate)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    apply_cover_bytes(&state, id, &data)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let _ = tokio::fs::remove_file(&candidate).await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn discard_cover_candidate(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let candidate = state
        .config
        .work_dir(id)
        .join(diarch_core::Config::cover_candidate_name());
    let _ = tokio::fs::remove_file(&candidate).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn upload_audio(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err((StatusCode::FORBIDDEN, "forbidden".into()));
    }
    if state.db.get_work(id).await.ok().flatten().is_none() {
        return Err((StatusCode::NOT_FOUND, "work not found".into()));
    }

    let mut saved: Option<(String, Vec<u8>)> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad multipart".into()))?
    {
        let name = field
            .file_name()
            .unwrap_or("book.m4b")
            .to_string();
        let data = field
            .bytes()
            .await
            .map_err(|_| (StatusCode::BAD_REQUEST, "bad file body".into()))?;
        saved = Some((name, data.to_vec()));
    }
    let (orig_name, data) = saved.ok_or((StatusCode::BAD_REQUEST, "file required".into()))?;
    let lower = orig_name.to_ascii_lowercase();
    let work_dir = state.config.work_dir(id);

    if lower.ends_with(".mp3") {
        return Err((
            StatusCode::BAD_REQUEST,
            "MP3 is not supported; upload .m4b or Audible .aax".into(),
        ));
    }

    if lower.ends_with(".aax") {
        if state.config.audible_key.as_deref().unwrap_or("").is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "DIARCH_AUDIBLE_KEY is not set; cannot convert AAX".into(),
            ));
        }
        if !crate::audio::ffmpeg_available() {
            return Err((
                StatusCode::BAD_REQUEST,
                "ffmpeg not found on PATH; required for AAX → M4B".into(),
            ));
        }
        let aax_path = work_dir.join("import.aax");
        tokio::fs::write(&aax_path, &data)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "failed to store AAX".into()))?;
        let job = state
            .db
            .create_job("aax_to_m4b", Some(id))
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "job create failed".into()))?;
        let _ = state
            .db
            .update_job(job.id, "pending", Some("import.aax"))
            .await;
        return Ok(Json(serde_json::json!({
            "job_id": job.id,
            "kind": "aax_to_m4b",
            "message": "Converting AAX → M4B"
        })));
    }

    if !(lower.ends_with(".m4b") || lower.ends_with(".m4a")) {
        return Err((
            StatusCode::BAD_REQUEST,
            "expected .m4b (preferred) or .aax; .m4a accepted as book.m4b".into(),
        ));
    }

    let audio_dir = work_dir.join("audio");
    tokio::fs::create_dir_all(&audio_dir)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "mkdir audio failed".into()))?;
    let filename = crate::audio::BOOK_M4B.to_string();
    let dest = audio_dir.join(&filename);
    tokio::fs::write(&dest, &data)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "write m4b failed".into()))?;
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
        w.updated_at = Utc::now();
        let _ = state.db.update_work(&w).await;
    }
    Ok(Json(serde_json::json!({
        "filename": filename,
        "kind": "m4b"
    })))
}

async fn audio_chapters(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let path = state
        .config
        .work_dir(id)
        .join("audio")
        .join(crate::audio::BOOK_M4B);
    let chapters = crate::audio::ffprobe_chapters(&path)
        .await
        .unwrap_or_default();
    Ok(Json(serde_json::json!({ "chapters": chapters })))
}

async fn request_transcribe(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let Some(mut work) = state.db.get_work(id).await.ok().flatten() else {
        return Err(StatusCode::NOT_FOUND);
    };
    if state.config.localai_url.is_none() && state.config.hermes_url.is_none() {
        return Ok(Json(serde_json::json!({
            "ok": false,
            "ready": false,
            "message": "LocalAI not configured — set DIARCH_LOCALAI_URL"
        })));
    }
    let m4b = state
        .config
        .work_dir(id)
        .join("audio")
        .join(crate::audio::BOOK_M4B);
    if !m4b.exists() {
        return Ok(Json(serde_json::json!({
            "ok": false,
            "ready": true,
            "message": "No book.m4b to transcribe — upload an audiobook first"
        })));
    }
    work.needs_transcription = true;
    work.updated_at = Utc::now();
    let _ = state.db.update_work(&work).await;
    let job = state
        .db
        .create_job("transcribe", Some(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "ready": true,
        "job_id": job.id,
        "message": "Transcription queued (chunked via LocalAI; may take a while)"
    })))
}

async fn get_transcript(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let path = state.config.work_dir(id).join("transcript.txt");
    let data = tokio::fs::read(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let title = work_title(&state, id).await;
    let disp = diarch_core::content_disposition_attachment(&title, "transcript.txt");
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CONTENT_DISPOSITION, disp)
        .body(axum::body::Body::from(data))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()))
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
    // Path traversal guard
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return Err(StatusCode::BAD_REQUEST);
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
                .unwrap_or(len.saturating_sub(1))
                .min(len.saturating_sub(1));
            if start > end || len == 0 {
                return Err(StatusCode::RANGE_NOT_SATISFIABLE);
            }
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

async fn put_markdown(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    body: String,
) -> Result<StatusCode, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let dir = state.config.work_dir(id);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let path = dir.join("book.md");
    tokio::fs::write(&path, body.as_bytes())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Do not clear needs_review — confirm_epub owns that transition.
    if let Some(mut work) = state.db.get_work(id).await.ok().flatten() {
        work.updated_at = Utc::now();
        let _ = state.db.update_work(&work).await;
    }
    let assets = state.db.list_assets(id).await.unwrap_or_default();
    if !assets.iter().any(|a| a.kind == AssetKind::Markdown) {
        let meta = tokio::fs::metadata(&path)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let asset = WorkAsset {
            id: Uuid::new_v4(),
            work_id: id,
            kind: AssetKind::Markdown,
            relative_path: "book.md".into(),
            mime: Some("text/markdown".into()),
            bytes: Some(meta.len() as i64),
            created_at: Utc::now(),
        };
        let _ = state.db.add_asset(&asset).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn get_pdf_file(
    AuthUser(user): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    if !state.db.user_can_access(&user, id).await.unwrap_or(false) {
        return Err(StatusCode::FORBIDDEN);
    }
    let path = state.config.work_dir(id).join("import.pdf");
    let data = tokio::fs::read(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/pdf"),
            (
                header::CONTENT_DISPOSITION,
                "inline; filename=\"import.pdf\"",
            ),
        ],
        data,
    )
        .into_response())
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
    } else if let Some(t) = hint.title {
        if work.title.is_empty() || work.title == "Untitled" {
            work.title = t;
        }
    }

    // Prefer StoryGraph book page when already matched.
    if let Some(sg_id) = work.sg_book_id.clone() {
        if work.description.is_none()
            || work.isbn.is_none()
            || work.authors.is_empty()
            || work.title.is_empty()
            || work.title == "Untitled"
        {
            if let Ok(Some(hit)) = crate::storygraph::fetch_book_metadata(&state, &sg_id).await {
                if work.isbn.is_none() {
                    work.isbn = hit.isbn;
                }
                if work.authors.is_empty() && !hit.authors.is_empty() {
                    work.authors = hit.authors;
                }
                if work.description.is_none() {
                    work.description = hit.description;
                }
                if (work.title.is_empty() || work.title == "Untitled") && !hit.title.is_empty() {
                    work.title = hit.title;
                }
            }
        }
    }

    // Title+author search (Open Library → Google → LoC → StoryGraph) when ISBN path left gaps.
    let need_search = work.isbn.is_none()
        || work.authors.is_empty()
        || work.description.is_none()
        || work.title.is_empty()
        || work.title == "Untitled";
    if need_search && !work.title.is_empty() && work.title != "Untitled" {
        let author_ref = if work.authors.is_empty() {
            None
        } else {
            Some(work.authors.as_str())
        };
        if let Ok(report) = metadata::search_title(&state, &work.title, author_ref).await {
            if let Some(hit) = report.hits.into_iter().next() {
                if work.isbn.is_none() {
                    work.isbn = hit.isbn;
                }
                if work.authors.is_empty() && !hit.authors.is_empty() {
                    work.authors = hit.authors;
                }
                if work.description.is_none() {
                    work.description = hit.description;
                }
                if (work.title.is_empty() || work.title == "Untitled") && !hit.title.is_empty() {
                    work.title = hit.title;
                }
            }
        }
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

    // Best-effort remote cover when still on placeholder.
    if work.needs_cover {
        if let Some(isbn_v) = work.isbn.as_deref().filter(|s| !s.is_empty()) {
            if let Ok(Some(bytes)) = metadata::fetch_remote_cover(&state, isbn_v).await {
                let _ = apply_cover_bytes(&state, id, &bytes).await;
                if let Ok(Some(updated)) = state.db.get_work(id).await {
                    return Ok(Json(updated));
                }
            }
        }
    }

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
            "needs_transcription" => work.needs_transcription = false,
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
        if let Ok(report) = metadata::search_title(&state, &title, author_ref).await {
            if let Some(hit) = report.hits.into_iter().next() {
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

    let (status, Json(work)) = create_work(
        AuthUser(user),
        State(state.clone()),
        Json(CreateWorkReq {
            title,
            authors: Some(authors),
            isbn: isbn.clone(),
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
    .map_err(|(s, e)| (s, e))?;

    if let Some(isbn_v) = work.isbn.as_deref().filter(|s| !s.is_empty()) {
        if let Ok(Some(bytes)) = metadata::fetch_remote_cover(&state, isbn_v).await {
            let _ = apply_cover_bytes(&state, work.id, &bytes).await;
            if let Ok(Some(updated)) = state.db.get_work(work.id).await {
                return Ok((status, Json(updated)));
            }
        }
    }

    Ok((status, Json(work)))
}

async fn meta_isbn(
    AuthUser(_): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(isbn): Path<String>,
) -> Result<Json<metadata::LookupReport>, StatusCode> {
    metadata::lookup_isbn_report(&state, &isbn)
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
) -> Result<Json<metadata::SearchReport>, StatusCode> {
    metadata::search_title(&state, &q.title, q.author.as_deref())
        .await
        .map(Json)
        .map_err(|_| StatusCode::BAD_GATEWAY)
}

#[derive(Serialize)]
struct Settings {
    show_audio_gaps: bool,
    reader_infinite_scroll: bool,
    reader_typography: diarch_core::ReaderTypography,
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
    let typo_json = state
        .db
        .get_reader_typography_json(user.id)
        .await
        .unwrap_or_else(|_| "{}".into());
    Ok(Json(Settings {
        show_audio_gaps: user.show_audio_gaps && state.config.show_audio_gaps,
        reader_infinite_scroll: scroll,
        reader_typography: diarch_core::ReaderTypography::from_json_str(&typo_json),
    }))
}

#[derive(Deserialize)]
struct SettingsReq {
    show_audio_gaps: Option<bool>,
    reader_infinite_scroll: Option<bool>,
    reader_typography: Option<diarch_core::ReaderTypography>,
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
    if let Some(typo) = body.reader_typography {
        let json = typo
            .sanitize()
            .to_json_string()
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        state
            .db
            .set_reader_typography_json(user.id, &json)
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
    AuthUser(_): AuthUser,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<diarch_core::IntegrationHealth>>, StatusCode> {
    state
        .db
        .list_integration_health()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn probe_integrations(
    AuthUser(_): AuthUser,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<diarch_core::IntegrationHealth>>, StatusCode> {
    crate::fixer::probe_all(&state).await;
    state
        .db
        .list_integration_health()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn repair_integration(
    AuthUser(_): AuthUser,
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Json<serde_json::Value> {
    match crate::fixer::repair(&state, &name).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e })),
    }
}

async fn remarkable_status(
    AuthUser(_): AuthUser,
) -> Json<crate::remarkable::RemarkableStatus> {
    Json(crate::remarkable::status())
}

#[derive(Deserialize)]
struct RemarkableAuthReq {
    code: String,
}

async fn remarkable_auth(
    AuthUser(_): AuthUser,
    State(state): State<Arc<AppState>>,
    Json(body): Json<RemarkableAuthReq>,
) -> Result<Json<crate::remarkable::RemarkableStatus>, (StatusCode, String)> {
    crate::remarkable::authenticate(&state, &body.code)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn sg_status(
    AuthUser(_): AuthUser,
    State(state): State<Arc<AppState>>,
) -> Json<crate::storygraph::StoryGraphStatus> {
    Json(crate::storygraph::status(&state))
}

#[derive(Deserialize)]
struct SgAuthReq {
    username: String,
    cookie: String,
    /// Optional; must match the browser that minted `cf_clearance`.
    user_agent: Option<String>,
}

async fn sg_auth(
    AuthUser(_): AuthUser,
    State(state): State<Arc<AppState>>,
    Json(body): Json<SgAuthReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut st = crate::storygraph::save_credentials(
        &state,
        &body.username,
        &body.cookie,
        body.user_agent.as_deref(),
    )
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let skip_live = std::env::var("DIARCH_STORYGRAPH_SKIP_LIVE_CHECK")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);

    let verify = if skip_live {
        Ok(())
    } else {
        match tokio::time::timeout(
            std::time::Duration::from_secs(15),
            crate::storygraph::verify_access(&state),
        )
        .await
        {
            Ok(r) => r,
            Err(_) => Err(
                "StoryGraph live check timed out — Cloudflare may be blocking this host".into(),
            ),
        }
    };

    match &verify {
        Ok(()) => {
            let _ = state
                .db
                .set_integration_health("storygraph", "ok", None, true)
                .await;
            st.detail = Some(if skip_live {
                "configured (live check skipped)".into()
            } else {
                "configured · live check ok".into()
            });
        }
        Err(e) => {
            let short: String = e.chars().take(200).collect();
            let _ = state
                .db
                .set_integration_health("storygraph", "broken", Some(&short), false)
                .await;
            st.detail = Some(format!("configured · live check failed: {e}"));
        }
    }

    Ok(Json(serde_json::json!({
        "username_set": st.username_set,
        "cookie_set": st.cookie_set,
        "remember_token": st.remember_token,
        "cf_clearance": st.cf_clearance,
        "username": st.username,
        "cookie_hint": st.cookie_hint,
        "detail": st.detail,
        "config_path": st.config_path,
        "verified": verify.is_ok(),
        "verify_error": verify.err(),
    })))
}

async fn sg_sync(
    AuthUser(_): AuthUser,
    State(state): State<Arc<AppState>>,
) -> Result<Json<crate::storygraph::SyncReport>, (StatusCode, String)> {
    crate::storygraph::sync_flags(&state)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))
}

async fn export_tts_queue(
    AuthUser(_): AuthUser,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let works = state
        .db
        .works_needing_tts()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let items: Vec<_> = works
        .iter()
        .map(|w| diarch_import::TtsQueueItem {
            work_id: w.id,
            title: w.title.clone(),
            authors: w.authors.clone(),
        })
        .collect();
    let files = diarch_import::export_needs_tts_queue(
        &state.config.library_dir(),
        &state.config.data_dir.join("queue/needs_tts"),
        &items,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({
        "exported": files.len(),
        "files": files,
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
