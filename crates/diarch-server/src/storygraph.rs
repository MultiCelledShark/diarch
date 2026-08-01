use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;

use crate::state::AppState;

#[derive(Debug, Clone, Deserialize, serde::Serialize, PartialEq)]
pub struct SgBook {
    pub title: String,
    pub authors: Option<String>,
    pub isbn: Option<String>,
    pub rating: Option<f64>,
    pub review: Option<String>,
    pub book_id: Option<String>,
    pub is_audiobook: bool,
    #[serde(default)]
    pub list: Option<String>,
}

/// Pull StoryGraph lists via unofficial HTML scrape.
/// Requires `DIARCH_STORYGRAPH_USER`; cookie recommended for private profiles.
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

    if state.config.storygraph_cookie.is_none() {
        let _ = state
            .db
            .set_integration_health(
                "storygraph",
                "degraded",
                Some("DIARCH_STORYGRAPH_COOKIE not set (public lists only)"),
                false,
            )
            .await;
    }

    let lists = [
        ("currently-reading", "currently_reading"),
        ("to-read", "to_read"),
        ("books-read", "books_read"),
    ];

    let mut by_id: HashMap<String, SgBook> = HashMap::new();
    let mut any_ok = false;
    let mut last_err: Option<String> = None;

    for (path, list_name) in lists {
        let url = format!("https://app.thestorygraph.com/{path}/{user}");
        match fetch_list_html(state, &url).await {
            Ok(html) => {
                any_ok = true;
                for mut book in parse_sg_html(&html) {
                    book.list = Some(list_name.into());
                    let key = book
                        .book_id
                        .clone()
                        .unwrap_or_else(|| normalize_key(&book.title));
                    by_id.entry(key).or_insert(book);
                }
            }
            Err(e) => last_err = Some(e.to_string()),
        }
    }

    // Fallback: profile page (older scrape path)
    if by_id.is_empty() {
        let url = format!("https://app.thestorygraph.com/profile/{user}");
        match fetch_list_html(state, &url).await {
            Ok(html) => {
                any_ok = true;
                for book in parse_sg_html(&html) {
                    let key = book
                        .book_id
                        .clone()
                        .unwrap_or_else(|| normalize_key(&book.title));
                    by_id.entry(key).or_insert(book);
                }
            }
            Err(e) => last_err = Some(e.to_string()),
        }
    }

    if !any_ok {
        let msg = last_err.unwrap_or_else(|| "StoryGraph fetch failed".into());
        let _ = state
            .db
            .set_integration_health("storygraph", "broken", Some(&msg), false)
            .await;
        return Err(anyhow::anyhow!(msg));
    }

    if state.config.storygraph_cookie.is_some() {
        let _ = state
            .db
            .set_integration_health("storygraph", "ok", None, true)
            .await;
    }

    let mut books: Vec<SgBook> = by_id.into_values().collect();
    books.sort_by(|a, b| a.title.to_ascii_lowercase().cmp(&b.title.to_ascii_lowercase()));
    Ok(books)
}

async fn fetch_list_html(state: &AppState, url: &str) -> Result<String> {
    let mut req = state
        .http
        .get(url)
        .header(
            "User-Agent",
            "Diarch/0.1 (+personal library; StoryGraph pull)",
        )
        .header("Accept", "text/html");
    if let Some(cookie) = &state.config.storygraph_cookie {
        let cookie_val = if cookie.contains('=') {
            cookie.clone()
        } else {
            format!("remember_user_token={cookie}")
        };
        req = req.header("Cookie", cookie_val);
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} for {url}", resp.status());
    }
    Ok(resp.text().await?)
}

/// Parse StoryGraph list/profile HTML into book rows.
pub fn parse_sg_html(html: &str) -> Vec<SgBook> {
    let mut books = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Preferred: book-title-author-and-series blocks used by StoryGraph list pages.
    for block in html.split("book-title-author-and-series").skip(1) {
        let chunk = block.split("</div>").next().unwrap_or(block);
        if let Some(book) = parse_book_chunk(chunk) {
            let key = book
                .book_id
                .clone()
                .unwrap_or_else(|| normalize_key(&book.title));
            if seen.insert(key) {
                books.push(book);
            }
        }
    }

    // Fallback: any /books/ anchors (profile / older markup).
    if books.is_empty() {
        for part in html.split("href=\"/books/").skip(1) {
            if let Some(book) = parse_href_book(part) {
                let key = book
                    .book_id
                    .clone()
                    .unwrap_or_else(|| normalize_key(&book.title));
                if seen.insert(key) {
                    books.push(book);
                }
            }
        }
    }

    books
}

fn parse_book_chunk(chunk: &str) -> Option<SgBook> {
    let id_and_rest = chunk.split("href=\"/books/").nth(1)?;
    parse_href_book(id_and_rest).map(|mut book| {
        // Author often appears as a later /authors/ link in the same block.
        if book.authors.is_none() {
            if let Some(auth_part) = chunk.split("href=\"/authors/").nth(1) {
                let author = auth_part
                    .split('>')
                    .nth(1)
                    .and_then(|s| s.split('<').next())
                    .map(|s| decode_basic_entities(s.trim()))
                    .filter(|s| !s.is_empty());
                book.authors = author;
            }
        }
        book
    })
}

fn parse_href_book(part: &str) -> Option<SgBook> {
    let id = part.split('"').next()?.trim().to_string();
    if id.is_empty() || id.contains('?') || id.contains('#') {
        return None;
    }
    let title = part
        .split('>')
        .nth(1)
        .and_then(|s| s.split('<').next())
        .map(|s| decode_basic_entities(s.trim()))
        .filter(|s| !s.is_empty())?;
    if title.eq_ignore_ascii_case("unknown") {
        return None;
    }
    // Skip nav chrome that sometimes links into /books/
    let lower = title.to_ascii_lowercase();
    if lower == "books" || lower.starts_with("see all") {
        return None;
    }
    let window = &part[..part.len().min(800)];
    let is_audiobook = window.to_ascii_lowercase().contains("audiobook")
        || window.to_ascii_lowercase().contains("audio edition");
    Some(SgBook {
        title,
        authors: None,
        isbn: None,
        rating: None,
        review: None,
        book_id: Some(id),
        is_audiobook,
        list: None,
    })
}

fn decode_basic_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn normalize_key(title: &str) -> String {
    diarch_core::normalize_title(title)
}

/// Match SG books against library; set / clear StoryGraph flags.
/// Does **not** clear `sg_review_dirty` (manual after you update SG).
pub async fn sync_flags(state: &AppState) -> Result<SyncReport> {
    let sg = pull_lists(state).await?;
    let admin_works = list_all_works(&state.db).await?;

    let mut audio_only_remote = 0usize;
    let mut matched = 0usize;
    let mut needs_add = 0usize;

    let lib_titles: Vec<(String, Option<String>)> = admin_works
        .iter()
        .filter(|w| w.status != diarch_core::ReadingStatus::Wishlist)
        .map(|w| (w.title.clone(), w.isbn.clone()))
        .collect();

    let missing_locally = sg
        .iter()
        .filter(|b| {
            !lib_titles.iter().any(|(t, isbn)| {
                titles_match(t, &b.title) || (b.isbn.is_some() && isbn == &b.isbn)
            })
        })
        .count();

    for mut w in admin_works {
        if w.status == diarch_core::ReadingStatus::Wishlist {
            continue;
        }

        let found = sg.iter().find(|b| {
            titles_match(&w.title, &b.title)
                || (b.isbn.is_some() && w.isbn.is_some() && w.isbn == b.isbn)
        });

        match found {
            Some(book) => {
                matched += 1;
                w.sg_matched = true;
                w.sg_needs_add = false;
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
                        audio_only_remote += 1;
                    } else {
                        w.sg_audio_only_remote = false;
                    }
                } else {
                    w.sg_audio_only_remote = false;
                }
                if let Some(r) = book.rating {
                    if w.rating != Some(r) {
                        w.rating = Some(r);
                    }
                }
                let _ = state.db.update_work(&w).await;
            }
            None => {
                needs_add += 1;
                w.sg_matched = false;
                w.sg_needs_add = true;
                w.sg_audio_only_remote = false;
                let _ = state.db.update_work(&w).await;
            }
        }
    }

    Ok(SyncReport {
        sg_count: sg.len(),
        matched,
        missing_locally,
        needs_add,
        audio_only_remote,
        books: sg,
    })
}

#[derive(Debug, serde::Serialize)]
pub struct SyncReport {
    pub sg_count: usize,
    pub matched: usize,
    /// SG titles with no matching library work.
    pub missing_locally: usize,
    /// Library works (non-wishlist) not found on StoryGraph.
    pub needs_add: usize,
    /// Matched SG audiobooks that have no local EPUB/MD/audio asset.
    pub audio_only_remote: usize,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_list_markup_with_authors() {
        let html = r#"
        <div class="book-pane">
          <div class="book-title-author-and-series">
            <p><a href="/books/abc-123-def">The Eye of the World</a></p>
            <p><a href="/authors/robert-jordan">Robert Jordan</a></p>
            <p>Audiobook</p>
          </div>
          <div class="book-title-author-and-series">
            <p><a href="/books/xyz-999">Dune</a></p>
            <p><a href="/authors/frank-herbert">Frank Herbert</a></p>
          </div>
        </div>"#;
        let books = parse_sg_html(html);
        assert_eq!(books.len(), 2);
        assert_eq!(books[0].title, "The Eye of the World");
        assert_eq!(books[0].book_id.as_deref(), Some("abc-123-def"));
        assert_eq!(books[0].authors.as_deref(), Some("Robert Jordan"));
        assert!(books[0].is_audiobook);
        assert_eq!(books[1].title, "Dune");
        assert!(!books[1].is_audiobook);
    }

    #[test]
    fn parses_href_fallback_and_dedupes() {
        let html = r#"
        <a href="/books/one">Book One</a>
        <a href="/books/one">Book One</a>
        <a href="/books/two">Book &amp; Two</a>
        "#;
        let books = parse_sg_html(html);
        assert_eq!(books.len(), 2);
        assert_eq!(books[1].title, "Book & Two");
    }

    #[test]
    fn skips_chrome_links() {
        let html = r#"<a href="/books/">Books</a><a href="/books/real-id">Real Title</a>"#;
        let books = parse_sg_html(html);
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Real Title");
    }
}
