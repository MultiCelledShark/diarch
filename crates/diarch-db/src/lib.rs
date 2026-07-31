use anyhow::{anyhow, Context, Result};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use chrono::{Duration, Utc};
use diarch_core::taxonomy::{flatten, SeedNode};
use diarch_core::{
    AssetKind, IntegrationHealth, Job, ReadingProgress, ReadingStatus, TaxonomyNode, User, Work,
    WorkAsset,
};
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::str::FromStr;
use uuid::Uuid;

pub struct Db {
    pool: SqlitePool,
}

impl Db {
    pub async fn connect(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let url = format!("sqlite://{}?mode=rwc", db_path.display());
        let options = SqliteConnectOptions::from_str(&url)?
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        let db = Self { pool };
        db.migrate().await?;
        Ok(db)
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    async fn migrate(&self) -> Result<()> {
        let sql = include_str!("migrations/001_init.sql");
        sqlx::raw_sql(sql).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn seed_taxonomy(&self, seed_json: &str) -> Result<usize> {
        let nodes: Vec<SeedNode> = serde_json::from_str(seed_json)?;
        let flat = flatten(&nodes, None);
        for (code, name, parent) in &flat {
            sqlx::query(
                "INSERT OR IGNORE INTO taxonomy (code, name, parent_code) VALUES (?, ?, ?)",
            )
            .bind(code)
            .bind(name)
            .bind(parent)
            .execute(&self.pool)
            .await?;
        }
        Ok(flat.len())
    }

    pub async fn ensure_admin(&self, username: &str, password: &str) -> Result<User> {
        if let Some(u) = self.get_user_by_username(username).await? {
            return Ok(u);
        }
        self.create_user(username, password, true).await
    }

    pub fn hash_password(password: &str) -> Result<String> {
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| anyhow!("hash: {e}"))?
            .to_string();
        Ok(hash)
    }

    pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
        let parsed = PasswordHash::new(hash).map_err(|e| anyhow!("parse hash: {e}"))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    }

    pub async fn create_user(&self, username: &str, password: &str, is_admin: bool) -> Result<User> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let hash = Self::hash_password(password)?;
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, is_admin, show_audio_gaps, created_at)
             VALUES (?, ?, ?, ?, 1, ?)",
        )
        .bind(id.to_string())
        .bind(username)
        .bind(hash)
        .bind(is_admin as i64)
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "INSERT INTO user_settings (user_id, reader_infinite_scroll) VALUES (?, 0)",
        )
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;
        self.get_user(id).await?.ok_or_else(|| anyhow!("user missing"))
    }

    pub async fn list_users(&self) -> Result<Vec<User>> {
        let rows = sqlx::query(
            "SELECT id, username, is_admin, show_audio_gaps, created_at FROM users ORDER BY username",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().filter_map(|r| row_user(&r).ok()).collect())
    }

    pub async fn get_user(&self, id: Uuid) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id, username, is_admin, show_audio_gaps, created_at FROM users WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| row_user(&r)).transpose()?)
    }

    pub async fn get_user_by_username(&self, username: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id, username, is_admin, show_audio_gaps, created_at FROM users WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| row_user(&r)).transpose()?)
    }

    pub async fn get_password_hash(&self, username: &str) -> Result<Option<String>> {
        let row = sqlx::query("SELECT password_hash FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<String, _>("password_hash")))
    }

    pub async fn set_user_show_audio_gaps(&self, user_id: Uuid, show: bool) -> Result<()> {
        sqlx::query("UPDATE users SET show_audio_gaps = ? WHERE id = ?")
            .bind(show as i64)
            .bind(user_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn create_session(&self, user_id: Uuid, days: i64) -> Result<String> {
        let mut raw = [0u8; 32];
        OsRng.fill_bytes(&mut raw);
        let token = hex::encode(raw);
        let now = Utc::now();
        let exp = now + Duration::days(days);
        sqlx::query(
            "INSERT INTO sessions (token, user_id, expires_at, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&token)
        .bind(user_id.to_string())
        .bind(exp.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(token)
    }

    pub async fn user_for_session(&self, token: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT u.id, u.username, u.is_admin, u.show_audio_gaps, u.created_at
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token = ? AND s.expires_at > ?",
        )
        .bind(token)
        .bind(Utc::now().to_rfc3339())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| row_user(&r)).transpose()?)
    }

    pub async fn delete_session(&self, token: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE token = ?")
            .bind(token)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_taxonomy(&self) -> Result<Vec<TaxonomyNode>> {
        let rows = sqlx::query("SELECT code, name, parent_code FROM taxonomy ORDER BY code")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| TaxonomyNode {
                code: r.get("code"),
                name: r.get("name"),
                parent_code: r.get("parent_code"),
            })
            .collect())
    }

    pub async fn create_work(&self, work: &Work, codes: &[i32], grant_user: Option<Uuid>) -> Result<()> {
        sqlx::query(
            "INSERT INTO works (
                id, title, authors, isbn, description, status, primary_code, year_list,
                rating, review, reading_direction, is_manga, needs_review, needs_cover,
                needs_tts, needs_audio, needs_transcription, sg_review_dirty, sg_needs_add,
                sg_audio_only_remote, sg_matched, sg_book_id, created_by, created_at, updated_at
             ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(work.id.to_string())
        .bind(&work.title)
        .bind(&work.authors)
        .bind(&work.isbn)
        .bind(&work.description)
        .bind(work.status.as_str())
        .bind(work.primary_code)
        .bind(work.year_list)
        .bind(work.rating)
        .bind(&work.review)
        .bind(&work.reading_direction)
        .bind(work.is_manga as i64)
        .bind(work.needs_review as i64)
        .bind(work.needs_cover as i64)
        .bind(work.needs_tts as i64)
        .bind(work.needs_audio as i64)
        .bind(work.needs_transcription as i64)
        .bind(work.sg_review_dirty as i64)
        .bind(work.sg_needs_add as i64)
        .bind(work.sg_audio_only_remote as i64)
        .bind(work.sg_matched as i64)
        .bind(&work.sg_book_id)
        .bind(work.created_by.map(|u| u.to_string()))
        .bind(work.created_at.to_rfc3339())
        .bind(work.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        for code in codes {
            sqlx::query("INSERT OR IGNORE INTO work_codes (work_id, code) VALUES (?, ?)")
                .bind(work.id.to_string())
                .bind(code)
                .execute(&self.pool)
                .await?;
        }
        if let Some(uid) = grant_user {
            self.grant_work(uid, work.id).await?;
        }
        Ok(())
    }

    pub async fn update_work(&self, work: &Work) -> Result<()> {
        sqlx::query(
            "UPDATE works SET title=?, authors=?, isbn=?, description=?, status=?, primary_code=?,
             year_list=?, rating=?, review=?, reading_direction=?, is_manga=?, needs_review=?,
             needs_cover=?, needs_tts=?, needs_audio=?, needs_transcription=?, sg_review_dirty=?,
             sg_needs_add=?, sg_audio_only_remote=?, sg_matched=?, sg_book_id=?, updated_at=?
             WHERE id=?",
        )
        .bind(&work.title)
        .bind(&work.authors)
        .bind(&work.isbn)
        .bind(&work.description)
        .bind(work.status.as_str())
        .bind(work.primary_code)
        .bind(work.year_list)
        .bind(work.rating)
        .bind(&work.review)
        .bind(&work.reading_direction)
        .bind(work.is_manga as i64)
        .bind(work.needs_review as i64)
        .bind(work.needs_cover as i64)
        .bind(work.needs_tts as i64)
        .bind(work.needs_audio as i64)
        .bind(work.needs_transcription as i64)
        .bind(work.sg_review_dirty as i64)
        .bind(work.sg_needs_add as i64)
        .bind(work.sg_audio_only_remote as i64)
        .bind(work.sg_matched as i64)
        .bind(&work.sg_book_id)
        .bind(Utc::now().to_rfc3339())
        .bind(work.id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_work_codes(&self, work_id: Uuid, codes: &[i32]) -> Result<()> {
        sqlx::query("DELETE FROM work_codes WHERE work_id = ?")
            .bind(work_id.to_string())
            .execute(&self.pool)
            .await?;
        for code in codes {
            sqlx::query("INSERT INTO work_codes (work_id, code) VALUES (?, ?)")
                .bind(work_id.to_string())
                .bind(code)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    pub async fn work_codes(&self, work_id: Uuid) -> Result<Vec<i32>> {
        let rows = sqlx::query("SELECT code FROM work_codes WHERE work_id = ? ORDER BY code")
            .bind(work_id.to_string())
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(|r| r.get("code")).collect())
    }

    pub async fn get_work(&self, id: Uuid) -> Result<Option<Work>> {
        let row = sqlx::query("SELECT * FROM works WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| row_work(&r)).transpose()?)
    }

    pub async fn user_can_access(&self, user: &User, work_id: Uuid) -> Result<bool> {
        if user.is_admin {
            return Ok(true);
        }
        let row = sqlx::query(
            "SELECT 1 AS ok FROM work_grants WHERE user_id = ? AND work_id = ?
             UNION
             SELECT 1 FROM works WHERE id = ? AND created_by = ?",
        )
        .bind(user.id.to_string())
        .bind(work_id.to_string())
        .bind(work_id.to_string())
        .bind(user.id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    pub async fn list_works_for_user(
        &self,
        user: &User,
        status: Option<&str>,
        attention: Option<&str>,
    ) -> Result<Vec<Work>> {
        let rows = if user.is_admin {
            sqlx::query("SELECT * FROM works ORDER BY updated_at DESC")
                .fetch_all(&self.pool)
                .await?
        } else {
            sqlx::query(
                "SELECT DISTINCT w.* FROM works w
                 LEFT JOIN work_grants g ON g.work_id = w.id
                 WHERE g.user_id = ? OR w.created_by = ?
                 ORDER BY w.updated_at DESC",
            )
            .bind(user.id.to_string())
            .bind(user.id.to_string())
            .fetch_all(&self.pool)
            .await?
        };
        let mut works: Vec<Work> = rows
            .into_iter()
            .filter_map(|r| row_work(&r).ok())
            .collect();
        if let Some(st) = status {
            works.retain(|w| w.status.as_str() == st);
        }
        if let Some(att) = attention {
            works.retain(|w| match att {
                "needs_review" => w.needs_review,
                "needs_cover" => w.needs_cover,
                "needs_tts" => w.needs_tts,
                "needs_audio" => w.needs_audio,
                "sg_review_dirty" => w.sg_review_dirty,
                "sg_needs_add" => w.sg_needs_add,
                "sg_audio_only_remote" => w.sg_audio_only_remote,
                _ => true,
            });
        }
        Ok(works)
    }

    /// Count works the user can access that are in `status`, optionally excluding one id.
    pub async fn count_accessible_by_status(
        &self,
        user: &User,
        status: &str,
        exclude: Option<Uuid>,
    ) -> Result<usize> {
        let works = self.list_works_for_user(user, Some(status), None).await?;
        Ok(works
            .iter()
            .filter(|w| exclude.map(|id| w.id != id).unwrap_or(true))
            .count())
    }

    pub async fn grant_work(&self, user_id: Uuid, work_id: Uuid) -> Result<()> {
        sqlx::query("INSERT OR IGNORE INTO work_grants (user_id, work_id) VALUES (?, ?)")
            .bind(user_id.to_string())
            .bind(work_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn revoke_work(&self, user_id: Uuid, work_id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM work_grants WHERE user_id = ? AND work_id = ?")
            .bind(user_id.to_string())
            .bind(work_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_grants(&self, work_id: Uuid) -> Result<Vec<Uuid>> {
        let rows = sqlx::query("SELECT user_id FROM work_grants WHERE work_id = ?")
            .bind(work_id.to_string())
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| Uuid::parse_str(&r.get::<String, _>("user_id")).ok())
            .collect())
    }

    pub async fn add_asset(&self, asset: &WorkAsset) -> Result<()> {
        sqlx::query(
            "INSERT INTO work_assets (id, work_id, kind, relative_path, mime, bytes, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(asset.id.to_string())
        .bind(asset.work_id.to_string())
        .bind(asset.kind.as_str())
        .bind(&asset.relative_path)
        .bind(&asset.mime)
        .bind(asset.bytes)
        .bind(asset.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_assets(&self, work_id: Uuid) -> Result<Vec<WorkAsset>> {
        let rows = sqlx::query("SELECT * FROM work_assets WHERE work_id = ? ORDER BY kind")
            .bind(work_id.to_string())
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().filter_map(|r| row_asset(&r).ok()).collect())
    }

    pub async fn upsert_progress(&self, p: &ReadingProgress) -> Result<()> {
        sqlx::query(
            "INSERT INTO reading_progress (user_id, work_id, mode, position, percent, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(user_id, work_id, mode) DO UPDATE SET
               position=excluded.position, percent=excluded.percent, updated_at=excluded.updated_at",
        )
        .bind(p.user_id.to_string())
        .bind(p.work_id.to_string())
        .bind(&p.mode)
        .bind(&p.position)
        .bind(p.percent)
        .bind(p.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_progress(
        &self,
        user_id: Uuid,
        work_id: Uuid,
        mode: &str,
    ) -> Result<Option<ReadingProgress>> {
        let row = sqlx::query(
            "SELECT * FROM reading_progress WHERE user_id = ? AND work_id = ? AND mode = ?",
        )
        .bind(user_id.to_string())
        .bind(work_id.to_string())
        .bind(mode)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| row_progress(&r)).transpose()?)
    }

    pub async fn create_job(&self, kind: &str, work_id: Option<Uuid>) -> Result<Job> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO jobs (id, kind, work_id, status, detail, created_at, updated_at)
             VALUES (?, ?, ?, 'pending', NULL, ?, ?)",
        )
        .bind(id.to_string())
        .bind(kind)
        .bind(work_id.map(|w| w.to_string()))
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;
        self.get_job(id).await?.ok_or_else(|| anyhow!("job missing"))
    }

    pub async fn update_job(&self, id: Uuid, status: &str, detail: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE jobs SET status = ?, detail = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(detail)
            .bind(Utc::now().to_rfc3339())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_job(&self, id: Uuid) -> Result<Option<Job>> {
        let row = sqlx::query("SELECT * FROM jobs WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| row_job(&r)).transpose()?)
    }

    pub async fn next_pending_job(&self) -> Result<Option<Job>> {
        let row = sqlx::query(
            "SELECT * FROM jobs WHERE status = 'pending' ORDER BY created_at ASC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| row_job(&r)).transpose()?)
    }

    pub async fn set_integration_health(
        &self,
        name: &str,
        status: &str,
        last_error: Option<&str>,
        ok: bool,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let last_ok = if ok { Some(now.clone()) } else { None };
        sqlx::query(
            "INSERT INTO integration_health (name, status, last_error, last_ok_at, updated_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(name) DO UPDATE SET
               status=excluded.status,
               last_error=excluded.last_error,
               last_ok_at=COALESCE(excluded.last_ok_at, integration_health.last_ok_at),
               updated_at=excluded.updated_at",
        )
        .bind(name)
        .bind(status)
        .bind(last_error)
        .bind(last_ok)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_integration_health(&self) -> Result<Vec<IntegrationHealth>> {
        let rows = sqlx::query("SELECT * FROM integration_health ORDER BY name")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| row_health(&r).ok())
            .collect())
    }

    pub async fn get_reader_infinite_scroll(&self, user_id: Uuid) -> Result<bool> {
        let row = sqlx::query("SELECT reader_infinite_scroll FROM user_settings WHERE user_id = ?")
            .bind(user_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row
            .map(|r| r.get::<i64, _>("reader_infinite_scroll") != 0)
            .unwrap_or(false))
    }

    pub async fn set_reader_infinite_scroll(&self, user_id: Uuid, on: bool) -> Result<()> {
        sqlx::query(
            "INSERT INTO user_settings (user_id, reader_infinite_scroll) VALUES (?, ?)
             ON CONFLICT(user_id) DO UPDATE SET reader_infinite_scroll = excluded.reader_infinite_scroll",
        )
        .bind(user_id.to_string())
        .bind(on as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn works_needing_tts(&self) -> Result<Vec<Work>> {
        let rows = sqlx::query("SELECT * FROM works WHERE needs_tts = 1 ORDER BY updated_at")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().filter_map(|r| row_work(&r).ok()).collect())
    }
}

fn row_user(r: &sqlx::sqlite::SqliteRow) -> Result<User> {
    Ok(User {
        id: Uuid::parse_str(&r.get::<String, _>("id"))?,
        username: r.get("username"),
        is_admin: r.get::<i64, _>("is_admin") != 0,
        show_audio_gaps: r.get::<i64, _>("show_audio_gaps") != 0,
        created_at: chrono::DateTime::parse_from_rfc3339(&r.get::<String, _>("created_at"))?
            .with_timezone(&Utc),
    })
}

fn row_work(r: &sqlx::sqlite::SqliteRow) -> Result<Work> {
    let status_s: String = r.get("status");
    Ok(Work {
        id: Uuid::parse_str(&r.get::<String, _>("id"))?,
        title: r.get("title"),
        authors: r.get("authors"),
        isbn: r.get("isbn"),
        description: r.get("description"),
        status: ReadingStatus::parse(&status_s).unwrap_or(ReadingStatus::Unread),
        primary_code: r.get("primary_code"),
        year_list: r.get("year_list"),
        rating: r.get("rating"),
        review: r.get("review"),
        reading_direction: r.get("reading_direction"),
        is_manga: r.get::<i64, _>("is_manga") != 0,
        needs_review: r.get::<i64, _>("needs_review") != 0,
        needs_cover: r.get::<i64, _>("needs_cover") != 0,
        needs_tts: r.get::<i64, _>("needs_tts") != 0,
        needs_audio: r.get::<i64, _>("needs_audio") != 0,
        needs_transcription: r.get::<i64, _>("needs_transcription") != 0,
        sg_review_dirty: r.get::<i64, _>("sg_review_dirty") != 0,
        sg_needs_add: r.get::<i64, _>("sg_needs_add") != 0,
        sg_audio_only_remote: r.get::<i64, _>("sg_audio_only_remote") != 0,
        sg_matched: r.get::<i64, _>("sg_matched") != 0,
        sg_book_id: r.get("sg_book_id"),
        created_by: r
            .get::<Option<String>, _>("created_by")
            .and_then(|s| Uuid::parse_str(&s).ok()),
        created_at: chrono::DateTime::parse_from_rfc3339(&r.get::<String, _>("created_at"))?
            .with_timezone(&Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339(&r.get::<String, _>("updated_at"))?
            .with_timezone(&Utc),
    })
}

fn row_asset(r: &sqlx::sqlite::SqliteRow) -> Result<WorkAsset> {
    let kind_s: String = r.get("kind");
    Ok(WorkAsset {
        id: Uuid::parse_str(&r.get::<String, _>("id"))?,
        work_id: Uuid::parse_str(&r.get::<String, _>("work_id"))?,
        kind: AssetKind::parse(&kind_s).context("bad asset kind")?,
        relative_path: r.get("relative_path"),
        mime: r.get("mime"),
        bytes: r.get("bytes"),
        created_at: chrono::DateTime::parse_from_rfc3339(&r.get::<String, _>("created_at"))?
            .with_timezone(&Utc),
    })
}

fn row_progress(r: &sqlx::sqlite::SqliteRow) -> Result<ReadingProgress> {
    Ok(ReadingProgress {
        user_id: Uuid::parse_str(&r.get::<String, _>("user_id"))?,
        work_id: Uuid::parse_str(&r.get::<String, _>("work_id"))?,
        mode: r.get("mode"),
        position: r.get("position"),
        percent: r.get("percent"),
        updated_at: chrono::DateTime::parse_from_rfc3339(&r.get::<String, _>("updated_at"))?
            .with_timezone(&Utc),
    })
}

fn row_job(r: &sqlx::sqlite::SqliteRow) -> Result<Job> {
    Ok(Job {
        id: Uuid::parse_str(&r.get::<String, _>("id"))?,
        kind: r.get("kind"),
        work_id: r
            .get::<Option<String>, _>("work_id")
            .and_then(|s| Uuid::parse_str(&s).ok()),
        status: r.get("status"),
        detail: r.get("detail"),
        created_at: chrono::DateTime::parse_from_rfc3339(&r.get::<String, _>("created_at"))?
            .with_timezone(&Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339(&r.get::<String, _>("updated_at"))?
            .with_timezone(&Utc),
    })
}

fn row_health(r: &sqlx::sqlite::SqliteRow) -> Result<IntegrationHealth> {
    Ok(IntegrationHealth {
        name: r.get("name"),
        status: r.get("status"),
        last_error: r.get("last_error"),
        last_ok_at: r
            .get::<Option<String>, _>("last_ok_at")
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&Utc)),
        updated_at: chrono::DateTime::parse_from_rfc3339(&r.get::<String, _>("updated_at"))?
            .with_timezone(&Utc),
    })
}

pub fn content_hash(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}
