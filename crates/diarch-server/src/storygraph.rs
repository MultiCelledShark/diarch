use anyhow::{bail, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone, serde::Serialize)]
pub struct StoryGraphStatus {
    pub username_set: bool,
    pub cookie_set: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Masked cookie hint (never the full secret).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cookie_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Absolute path of the on-disk credentials file (for ops).
    pub config_path: String,
}

#[derive(Debug, Clone, Default)]
struct SgCreds {
    username: Option<String>,
    cookie: Option<String>,
}

fn creds_path(state: &AppState) -> PathBuf {
    state.config.data_dir.join("storygraph.conf")
}

fn load_creds_file(path: &Path) -> Option<SgCreds> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut username = None;
    let mut cookie = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("username:") {
            let v = rest.trim().trim_matches('"').to_string();
            if !v.is_empty() {
                username = Some(v);
            }
        } else if let Some(rest) = line.strip_prefix("cookie:") {
            let v = rest.trim().trim_matches('"').to_string();
            if !v.is_empty() {
                cookie = Some(v);
            }
        }
    }
    if username.is_none() && cookie.is_none() {
        None
    } else {
        Some(SgCreds { username, cookie })
    }
}

/// File credentials override env; used for UI-configured StoryGraph auth.
fn resolved_creds(state: &AppState) -> SgCreds {
    if let Some(file) = load_creds_file(&creds_path(state)) {
        return SgCreds {
            username: file
                .username
                .or_else(|| state.config.storygraph_username.clone()),
            cookie: file
                .cookie
                .or_else(|| state.config.storygraph_cookie.clone()),
        };
    }
    SgCreds {
        username: state.config.storygraph_username.clone(),
        cookie: state.config.storygraph_cookie.clone(),
    }
}

fn mask_cookie(cookie: &str) -> String {
    let raw = cookie
        .strip_prefix("remember_user_token=")
        .unwrap_or(cookie)
        .trim();
    if raw.len() <= 4 {
        "set".into()
    } else {
        format!("…{}", &raw[raw.len() - 4..])
    }
}

pub fn status(state: &AppState) -> StoryGraphStatus {
    let path = creds_path(state);
    let creds = resolved_creds(state);
    let username_set = creds.username.as_ref().is_some_and(|s| !s.is_empty());
    let cookie_set = creds.cookie.as_ref().is_some_and(|s| !s.is_empty());
    let detail = match (username_set, cookie_set) {
        (false, false) => Some("username and cookie not set".into()),
        (false, true) => Some("username not set".into()),
        (true, false) => Some("cookie not set (Cloudflare / enrich will fail)".into()),
        (true, true) => {
            if path.exists() {
                Some("configured via Integrations".into())
            } else {
                Some("configured via environment".into())
            }
        }
    };
    StoryGraphStatus {
        username_set,
        cookie_set,
        username: creds.username.clone(),
        cookie_hint: creds.cookie.as_deref().map(mask_cookie),
        detail,
        config_path: path.display().to_string(),
    }
}

/// Save username + cookie to `data_dir/storygraph.conf` (like rmapi.conf for reMarkable).
pub fn save_credentials(state: &AppState, username: &str, cookie: &str) -> Result<StoryGraphStatus> {
    let username = username.trim();
    let cookie = cookie.trim();
    if username.is_empty() {
        bail!("username required");
    }
    if cookie.is_empty() {
        bail!("cookie required (remember_user_token from StoryGraph)");
    }
    let cookie_val = if cookie.contains('=') {
        cookie.to_string()
    } else {
        format!("remember_user_token={cookie}")
    };
    let path = creds_path(state);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = format!("username: {username}\ncookie: {cookie_val}\n");
    std::fs::write(&path, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(status(state))
}

/// Pull StoryGraph lists via unofficial HTML scrape.
/// Requires username; cookie required for Cloudflare / private lists / enrich.
pub async fn pull_lists(state: &AppState) -> Result<Vec<SgBook>> {
    let creds = resolved_creds(state);
    let Some(user) = creds.username.as_ref().filter(|s| !s.is_empty()) else {
        let _ = state
            .db
            .set_integration_health(
                "storygraph",
                "degraded",
                Some("StoryGraph username not set (Integrations or DIARCH_STORYGRAPH_USER)"),
                false,
            )
            .await;
        return Ok(vec![]);
    };

    if creds.cookie.as_ref().is_none_or(|s| s.is_empty()) {
        let _ = state
            .db
            .set_integration_health(
                "storygraph",
                "degraded",
                Some("StoryGraph cookie not set (Integrations or DIARCH_STORYGRAPH_COOKIE)"),
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

    if creds.cookie.as_ref().is_some_and(|s| !s.is_empty()) {
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
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
        )
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("Accept-Language", "en-US,en;q=0.9");
    if let Some(cookie) = resolved_creds(state).cookie.filter(|s| !s.is_empty()) {
        let cookie_val = if cookie.contains('=') {
            cookie
        } else {
            format!("remember_user_token={cookie}")
        };
        req = req.header("Cookie", cookie_val);
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} for {url}", resp.status());
    }
    let text = resp.text().await?;
    if looks_like_cloudflare_challenge(&text) {
        anyhow::bail!(
            "Cloudflare challenge from StoryGraph — save cookie in Integrations (remember_user_token)"
        );
    }
    Ok(text)
}

fn looks_like_cloudflare_challenge(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    lower.contains("just a moment...")
        || (lower.contains("cloudflare") && lower.contains("challenge"))
        || lower.contains("cf-browser-verification")
}

/// Search StoryGraph browse for metadata enrich hits.
/// Cookie strongly recommended (Cloudflare often blocks anonymous requests).
pub async fn search_metadata(
    state: &AppState,
    title: &str,
    author: Option<&str>,
) -> Result<Vec<crate::metadata::MetaHit>> {
    let title = title.trim();
    if title.is_empty() {
        return Ok(vec![]);
    }
    if resolved_creds(state).cookie.as_ref().is_none_or(|s| s.is_empty()) {
        anyhow::bail!("StoryGraph cookie not set (Integrations or DIARCH_STORYGRAPH_COOKIE)");
    }

    let mut term = title.to_string();
    if let Some(a) = author.map(str::trim).filter(|s| !s.is_empty()) {
        term.push(' ');
        term.push_str(a);
    }
    let url = format!(
        "https://app.thestorygraph.com/browse?search_term={}",
        urlencoding_encode(&term)
    );
    let html = fetch_list_html(state, &url).await?;
    let mut paired = parse_sg_browse_paired(&html);
    paired.sort_by_key(|(_, h)| {
        let exact = diarch_core::titles_match(&h.title, title);
        (!exact, h.title.to_ascii_lowercase())
    });
    paired.truncate(8);

    let enrich_n = paired.len().min(3);
    for (id, hit) in paired.iter_mut().take(enrich_n) {
        if let Ok(Some(detail)) = fetch_book_metadata(state, id).await {
            if hit.description.is_none() {
                hit.description = detail.description;
            }
            if hit.isbn.is_none() {
                hit.isbn = detail.isbn;
            }
            if hit.cover_url.is_none() {
                hit.cover_url = detail.cover_url;
            }
            if hit.subjects.is_empty() {
                hit.subjects = detail.subjects;
            }
        }
    }

    let _ = state
        .db
        .set_integration_health("storygraph", "ok", None, true)
        .await;
    Ok(paired.into_iter().map(|(_, h)| h).collect())
}

/// Look up a StoryGraph book page by id (from `sg_book_id` or browse).
pub async fn fetch_book_metadata(
    state: &AppState,
    book_id: &str,
) -> Result<Option<crate::metadata::MetaHit>> {
    let book_id = book_id.trim();
    if book_id.is_empty() || book_id == "new" {
        return Ok(None);
    }
    if resolved_creds(state).cookie.as_ref().is_none_or(|s| s.is_empty()) {
        anyhow::bail!("StoryGraph cookie not set (Integrations or DIARCH_STORYGRAPH_COOKIE)");
    }
    let url = format!("https://app.thestorygraph.com/books/{book_id}");
    let html = fetch_list_html(state, &url).await?;
    Ok(parse_sg_book_page(&html, book_id))
}

/// ISBN search via StoryGraph browse.
pub async fn lookup_isbn_metadata(
    state: &AppState,
    isbn: &str,
) -> Result<Option<crate::metadata::MetaHit>> {
    let isbn = isbn.replace('-', "");
    if isbn.is_empty() {
        return Ok(None);
    }
    if resolved_creds(state).cookie.as_ref().is_none_or(|s| s.is_empty()) {
        anyhow::bail!("StoryGraph cookie not set (Integrations or DIARCH_STORYGRAPH_COOKIE)");
    }
    let url = format!(
        "https://app.thestorygraph.com/browse?search_term={}",
        urlencoding_encode(&isbn)
    );
    let html = fetch_list_html(state, &url).await?;
    let paired = parse_sg_browse_paired(&html);
    let want = isbn.to_ascii_lowercase();
    if let Some((id, hit)) = paired.into_iter().find(|(_, h)| {
        h.isbn
            .as_deref()
            .map(|i| i.replace('-', "").eq_ignore_ascii_case(&want))
            .unwrap_or(false)
    }) {
        if let Ok(Some(detail)) = fetch_book_metadata(state, &id).await {
            return Ok(Some(detail));
        }
        return Ok(Some(hit));
    }
    Ok(None)
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Parse browse / search result panes into metadata hits.
pub fn parse_sg_browse_hits(html: &str) -> Vec<crate::metadata::MetaHit> {
    parse_sg_browse_paired(html)
        .into_iter()
        .map(|(_, h)| h)
        .collect()
}

fn parse_sg_browse_paired(html: &str) -> Vec<(String, crate::metadata::MetaHit)> {
    let mut hits = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let starts = book_pane_starts(html);
    for (i, &abs) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(html.len());
        let chunk = &html[abs..end.min(abs + 16_000)];
        if let Some((id, hit)) = parse_browse_pane(chunk) {
            let key = hit
                .isbn
                .clone()
                .unwrap_or_else(|| normalize_key(&hit.title));
            if seen.insert(key) {
                hits.push((id, hit));
            }
        }
    }
    hits
}

/// Offsets of top-level `book-pane` cards (not `book-pane-*` subclasses).
fn book_pane_starts(html: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(rel) = html[start..].find("book-pane") {
        let abs = start + rel;
        let after_idx = abs + "book-pane".len();
        let next = html.as_bytes().get(after_idx).copied();
        if next != Some(b'-') {
            out.push(abs);
        }
        start = after_idx;
    }
    out
}

fn parse_browse_pane(chunk: &str) -> Option<(String, crate::metadata::MetaHit)> {
    let id = extract_attr_after(chunk, "data-book-id=\"")?;
    if id.is_empty() || id == "new" {
        return None;
    }
    let title = extract_book_title_for_id(chunk, &id)?;
    if title.eq_ignore_ascii_case("unknown")
        || title.eq_ignore_ascii_case("books")
        || title.to_ascii_lowercase().starts_with("see all")
    {
        return None;
    }
    let authors = extract_author(chunk).unwrap_or_default();
    let isbn = extract_isbn_uid(chunk);
    let cover_url = extract_cover_url(chunk);
    let mut subjects = extract_teal_tags(chunk);
    let is_audiobook = chunk.to_ascii_lowercase().contains("audiobook")
        || chunk.to_ascii_lowercase().contains("audio edition");
    if is_audiobook && !subjects.iter().any(|s| s.eq_ignore_ascii_case("audiobook")) {
        subjects.push("audiobook".into());
    }
    Some((
        id,
        crate::metadata::MetaHit {
            title,
            authors,
            isbn,
            description: None,
            subjects,
            source: "storygraph".into(),
            cover_url,
            ..Default::default()
        },
    ))
}

/// Parse a StoryGraph book detail page into a MetaHit.
pub fn parse_sg_book_page(html: &str, book_id: &str) -> Option<crate::metadata::MetaHit> {
    let title = extract_detail_title(html).or_else(|| {
        extract_meta_content(html, "og:title").and_then(|t| {
            t.rsplit_once(" by ")
                .map(|(title, _)| title.trim().to_string())
                .or(Some(t))
        })
    })?;
    let authors = extract_author(html)
        .or_else(|| {
            extract_meta_content(html, "og:title").and_then(|t| {
                t.rsplit_once(" by ")
                    .map(|(_, a)| a.trim().to_string())
                    .filter(|a| !a.is_empty())
            })
        })
        .unwrap_or_default();
    let isbn = extract_isbn_uid(html).filter(|s| {
        let lower = s.to_ascii_lowercase();
        lower != "none" && lower != "null" && !lower.is_empty()
    });
    let description =
        extract_description(html).or_else(|| extract_meta_content(html, "og:description"));
    let cover_url = extract_cover_url(html).or_else(|| extract_meta_content(html, "og:image"));
    let subjects = extract_teal_tags(html);
    let _ = book_id;
    Some(crate::metadata::MetaHit {
        title,
        authors,
        isbn,
        description,
        subjects,
        source: "storygraph".into(),
        cover_url,
        ..Default::default()
    })
}

fn extract_attr_after(hay: &str, marker: &str) -> Option<String> {
    let rest = hay.split(marker).nth(1)?;
    let val = rest.split('"').next()?.trim();
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

fn extract_book_title_for_id(chunk: &str, id: &str) -> Option<String> {
    let marker = format!("href=\"/books/{id}\"");
    for part in chunk.split(&marker).skip(1) {
        let title = part
            .split('>')
            .nth(1)
            .and_then(|s| s.split('<').next())
            .map(|s| decode_basic_entities(s.trim()))
            .filter(|s| !s.is_empty());
        let Some(title) = title else {
            continue;
        };
        if title.to_ascii_lowercase().contains("edition") {
            continue;
        }
        return Some(title);
    }
    None
}

fn extract_detail_title(html: &str) -> Option<String> {
    if let Some(rest) = html.split("book-title-author-and-series").nth(1) {
        if let Some(h3) = rest.split("<h3").nth(1) {
            let after = h3.split('>').nth(1)?;
            let title = after
                .split("</h3>")
                .next()?
                .split('<')
                .next()
                .map(|s| decode_basic_entities(s.trim()))
                .filter(|s| !s.is_empty())?;
            return Some(title);
        }
    }
    None
}

fn extract_author(html: &str) -> Option<String> {
    let part = html.split("href=\"/authors/").nth(1)?;
    part.split('>')
        .nth(1)
        .and_then(|s| s.split('<').next())
        .map(|s| decode_basic_entities(s.trim()))
        .filter(|s| !s.is_empty())
}

fn extract_isbn_uid(html: &str) -> Option<String> {
    let rest = html.split("ISBN/UID:").nth(1)?;
    let after = rest.split("</span>").nth(1).unwrap_or(rest);
    let raw = after
        .split('<')
        .next()?
        .trim()
        .trim_start_matches([':', ' '])
        .trim();
    let digits: String = raw
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
        .collect();
    if digits.len() == 10 || digits.len() == 13 {
        Some(digits.to_ascii_uppercase())
    } else if raw.eq_ignore_ascii_case("none") {
        None
    } else if !raw.is_empty() && raw.len() < 40 {
        Some(raw.to_string())
    } else {
        None
    }
}

fn extract_cover_url(html: &str) -> Option<String> {
    if let Some(rest) = html.split("book-cover").nth(1) {
        if let Some(src) = extract_img_src(rest) {
            if src.contains("cdn.thestorygraph.com") || src.contains("covers") {
                return Some(src);
            }
        }
    }
    for part in html.split("src=\"").skip(1) {
        let src = part.split('"').next()?.to_string();
        if src.contains("cdn.thestorygraph.com") {
            return Some(src);
        }
    }
    None
}

fn extract_img_src(html: &str) -> Option<String> {
    let rest = html.split("src=\"").nth(1)?;
    let src = rest.split('"').next()?.trim();
    if src.is_empty() {
        None
    } else {
        Some(src.to_string())
    }
}

fn extract_description(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let idx = lower.find(">description</h4>")?;
    let after = &html[idx..];
    if let Some(rest) = after.split("trix-content").nth(1) {
        let after_gt = rest.split('>').nth(1)?;
        let text = after_gt.split("</p>").next()?;
        let plain = decode_basic_entities(strip_tags(text).trim());
        if plain.len() > 20 {
            return Some(plain);
        }
    }
    None
}

fn extract_teal_tags(html: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for part in html.split("text-teal-").skip(1) {
        let Some(after) = part.split('>').nth(1) else {
            continue;
        };
        let Some(tag) = after
            .split('<')
            .next()
            .map(|s| decode_basic_entities(s.trim()))
            .filter(|s| !s.is_empty() && s.len() < 48)
        else {
            continue;
        };
        if seen.insert(tag.to_ascii_lowercase()) {
            tags.push(tag);
        }
        if tags.len() >= 8 {
            break;
        }
    }
    tags
}

fn extract_meta_content(html: &str, prop: &str) -> Option<String> {
    let markers = [
        format!("property=\"{prop}\""),
        format!("name=\"{prop}\""),
        format!("property='{prop}'"),
    ];
    for marker in markers {
        if let Some(rest) = html.split(&marker).nth(1) {
            if let Some(c) = extract_attr_after(rest, "content=\"").or_else(|| {
                rest.split("content='")
                    .nth(1)
                    .and_then(|s| s.split('\'').next().map(|v| v.to_string()))
            }) {
                let c = decode_basic_entities(c.trim());
                if !c.is_empty() {
                    return Some(c);
                }
            }
        }
    }
    None
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
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

    #[test]
    fn mask_cookie_shows_tail() {
        assert_eq!(mask_cookie("abcdefghij"), "…ghij");
        assert_eq!(mask_cookie("remember_user_token=abcdefghij"), "…ghij");
    }

    #[test]
    fn load_creds_file_parses_yamlish() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storygraph.conf");
        std::fs::write(
            &path,
            "username: igrot\ncookie: remember_user_token=secretvalue\n",
        )
        .unwrap();
        let c = load_creds_file(&path).unwrap();
        assert_eq!(c.username.as_deref(), Some("igrot"));
        assert_eq!(
            c.cookie.as_deref(),
            Some("remember_user_token=secretvalue")
        );
    }

    #[test]
    fn parses_browse_pane_isbn_cover_and_tags() {
        let html = r#"
        <div class="book-pane break-words" data-book-id="ac3ea915-993d-4f30-8632-0f91e4ad0704">
          <div class="book-cover">
            <a href="/books/ac3ea915-993d-4f30-8632-0f91e4ad0704">
              <img alt="Project Hail Mary by Andy Weir" src="https://cdn.thestorygraph.com/cover123">
            </a>
          </div>
          <div class="book-title-author-and-series">
            <h3><a href="/books/ac3ea915-993d-4f30-8632-0f91e4ad0704">Project Hail Mary</a>
            <p><a href="/authors/f58b6fd4-b07b-478b-8416-8d72f26a82f1">Andy Weir</a></p>
            </h3>
          </div>
          <div class="edition-info">
            <p><span class="font-semibold">ISBN/UID:</span>  9780593135204</p>
          </div>
          <div class="book-pane-tag-section">
            <span class="inline-block text-xs text-teal-700">fiction</span>
            <span class="inline-block text-xs text-teal-700">science fiction</span>
            <span class="inline-block text-xs text-pink-500">adventurous</span>
          </div>
        </div>"#;
        let hits = parse_sg_browse_hits(html);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Project Hail Mary");
        assert_eq!(hits[0].authors, "Andy Weir");
        assert_eq!(hits[0].isbn.as_deref(), Some("9780593135204"));
        assert_eq!(hits[0].source, "storygraph");
        assert!(hits[0]
            .cover_url
            .as_deref()
            .unwrap()
            .contains("cdn.thestorygraph.com"));
        assert!(hits[0].subjects.iter().any(|s| s == "fiction"));
        assert!(hits[0].subjects.iter().any(|s| s == "science fiction"));
        assert!(!hits[0].subjects.iter().any(|s| s == "adventurous"));
    }

    #[test]
    fn parses_book_detail_description() {
        let html = r#"
        <meta property="og:title" content="Project Hail Mary by Andy Weir">
        <meta property="og:description" content="Short blurb from og tags that is long enough.">
        <meta property="og:image" content="https://cdn.thestorygraph.com/ogcover">
        <div class="book-title-author-and-series">
          <h3 class="font-semibold">
            Project Hail Mary
          </h3>
          <p><a href="/authors/f58b6fd4">Andy Weir</a></p>
        </div>
        <p><span class="font-semibold">ISBN/UID:</span>  9780593135204</p>
        <h4>Description</h4>
        <p class="trix-content mt-3">Ryland Grace is the sole survivor on a desperate, last-chance mission—and if he fails, humanity and the earth itself will perish.</p>
        <span class="text-teal-700">fiction</span>
        "#;
        let hit = parse_sg_book_page(html, "ac3ea915").expect("hit");
        assert_eq!(hit.title, "Project Hail Mary");
        assert_eq!(hit.authors, "Andy Weir");
        assert_eq!(hit.isbn.as_deref(), Some("9780593135204"));
        assert!(hit
            .description
            .as_deref()
            .unwrap()
            .contains("Ryland Grace"));
        assert!(hit.subjects.iter().any(|s| s == "fiction"));
    }
}
