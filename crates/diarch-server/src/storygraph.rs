use anyhow::Result;
use serde::Deserialize;

use crate::state::AppState;

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct SgBook {
    pub title: String,
    pub authors: Option<String>,
    pub isbn: Option<String>,
    pub rating: Option<f64>,
    pub review: Option<String>,
    pub book_id: Option<String>,
    pub is_audiobook: bool,
}

/// Pull StoryGraph lists via unofficial HTML/API patterns.
/// Requires DIARCH_STORYGRAPH_COOKIE and DIARCH_STORYGRAPH_USER for private lists.
pub async fn pull_lists(state: &AppState) -> Result<Vec<SgBook>> {
    let Some(user) = state.config.storygraph_username.as_ref() else {
        let _ = state
            .db
            .set_integration_health(
                "storygraph",
                "degraded",
                Some("DIARCH_STORYGRAPH_USER not set"),
                false,
            )
            .await;
        return Ok(vec![]);
    };

    let mut req = state
        .http
        .get(format!("https://app.thestorygraph.com/profile/{user}"));
    if let Some(cookie) = &state.config.storygraph_cookie {
        req = req.header("Cookie", format!("remember_user_token={cookie}"));
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            let _ = state
                .db
                .set_integration_health("storygraph", "broken", Some(&e.to_string()), false)
                .await;
            return Err(e.into());
        }
    };
    if !resp.status().is_success() {
        let _ = state
            .db
            .set_integration_health("storygraph", "broken", Some("profile fetch failed"), false)
            .await;
        return Ok(vec![]);
    }
    let html = resp.text().await?;
    let _ = state
        .db
        .set_integration_health("storygraph", "ok", None, true)
        .await;

    // Minimal scrape: book titles from profile HTML anchors
    let mut books = Vec::new();
    for part in html.split("href=\"/books/").skip(1) {
        let id = part.split('"').next().unwrap_or("").to_string();
        let title = part
            .split('>')
            .nth(1)
            .and_then(|s| s.split('<').next())
            .unwrap_or("Unknown")
            .trim()
            .to_string();
        if title.is_empty() || title == "Unknown" {
            continue;
        }
        let is_audiobook = part.to_ascii_lowercase().contains("audiobook");
        books.push(SgBook {
            title,
            authors: None,
            isbn: None,
            rating: None,
            review: None,
            book_id: Some(id),
            is_audiobook,
        });
    }
    Ok(books)
}

/// Match SG books against library; set flags on works / create report entries via flags.
pub async fn sync_flags(state: &AppState) -> Result<SyncReport> {
    let sg = pull_lists(state).await?;
    let admin_works = list_all_works(&state.db).await?;

    let mut missing_locally = 0usize;
    let mut matched = 0usize;

    for book in &sg {
        let found = admin_works.iter().find(|w| {
            titles_match(&w.title, &book.title)
                || (book.isbn.is_some() && w.isbn == book.isbn)
        });
        match found {
            Some(w) => {
                matched += 1;
                let mut w = w.clone();
                w.sg_matched = true;
                w.sg_book_id = book.book_id.clone();
                if book.is_audiobook {
                    let assets = state.db.list_assets(w.id).await.unwrap_or_default();
                    let has_local = assets.iter().any(|a| {
                        matches!(
                            a.kind,
                            diarch_core::AssetKind::Epub
                                | diarch_core::AssetKind::Markdown
                                | diarch_core::AssetKind::Audio
                        )
                    });
                    if !has_local {
                        w.sg_audio_only_remote = true;
                        missing_locally += 1;
                    }
                }
                if let Some(r) = book.rating {
                    if w.rating != Some(r) {
                        w.rating = Some(r);
                    }
                }
                let _ = state.db.update_work(&w).await;
            }
            None => {
                missing_locally += 1;
            }
        }
    }

    // Library books not on SG
    let mut needs_add = 0usize;
    for w in &admin_works {
        if w.status == diarch_core::ReadingStatus::Wishlist {
            continue;
        }
        let on_sg = sg.iter().any(|b| titles_match(&w.title, &b.title));
        if !on_sg {
            let mut w = w.clone();
            w.sg_needs_add = true;
            let _ = state.db.update_work(&w).await;
            needs_add += 1;
        }
    }

    Ok(SyncReport {
        sg_count: sg.len(),
        matched,
        missing_locally,
        needs_add,
        books: sg,
    })
}

#[derive(Debug, serde::Serialize)]
pub struct SyncReport {
    pub sg_count: usize,
    pub matched: usize,
    pub missing_locally: usize,
    pub needs_add: usize,
    pub books: Vec<SgBook>,
}

fn titles_match(a: &str, b: &str) -> bool {
    diarch_core::titles_match(a, b)
}

async fn list_all_works(db: &diarch_db::Db) -> Result<Vec<diarch_core::Work>> {
    let admin = diarch_core::User {
        id: uuid::Uuid::nil(),
        username: "system".into(),
        is_admin: true,
        show_audio_gaps: true,
        created_at: chrono::Utc::now(),
    };
    Ok(db.list_works_for_user(&admin, None, None).await?)
}
