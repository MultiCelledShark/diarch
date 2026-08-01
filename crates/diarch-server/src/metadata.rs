use anyhow::Result;
use diarch_core::taxonomy::map_subjects_to_codes;
use serde::Deserialize;
use serde_json::Value;

use crate::state::AppState;

#[derive(Debug, Clone, serde::Serialize)]
pub struct MetaHit {
    pub title: String,
    pub authors: String,
    pub isbn: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub subjects: Vec<String>,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderStatus {
    pub name: String,
    /// `hit` | `miss` | `error`
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LookupReport {
    pub hit: Option<MetaHit>,
    pub providers: Vec<ProviderStatus>,
}

/// Combined local (EPUB) + Open Library subjects mapped onto Diarch codes.
#[derive(Debug, Clone, Default)]
pub struct TaxonomyHint {
    pub primary: Option<i32>,
    pub codes: Vec<i32>,
    pub subjects: Vec<String>,
    pub isbn: Option<String>,
    pub description: Option<String>,
    pub title: Option<String>,
    pub authors: Option<String>,
}

#[derive(Debug)]
enum ProviderOutcome {
    Hit(MetaHit),
    Miss,
    Error(String),
}

fn google_books_volumes_url(state: &AppState, query: &str) -> String {
    let mut url = format!(
        "https://www.googleapis.com/books/v1/volumes?q={}",
        urlencoding_encode(query)
    );
    if let Some(key) = state.config.google_books_key.as_deref() {
        url.push_str("&key=");
        url.push_str(&urlencoding_encode(key));
    }
    url
}

/// Classify Google Books HTTP status for diagnostics / tests.
pub fn classify_google_http(status: u16) -> (&'static str, Option<&'static str>) {
    match status {
        200..=299 => ("ok", None),
        429 => (
            "error",
            Some("rate limited — set DIARCH_GOOGLE_BOOKS_KEY or retry later"),
        ),
        403 => (
            "error",
            Some("forbidden — check DIARCH_GOOGLE_BOOKS_KEY / API quota"),
        ),
        _ => ("error", Some("HTTP error from Google Books")),
    }
}

pub async fn lookup_isbn(state: &AppState, isbn: &str) -> Result<Option<MetaHit>> {
    Ok(lookup_isbn_report(state, isbn).await?.hit)
}

pub async fn lookup_isbn_report(state: &AppState, isbn: &str) -> Result<LookupReport> {
    let isbn = isbn.replace('-', "");
    let mut providers = Vec::new();
    if isbn.is_empty() {
        return Ok(LookupReport {
            hit: None,
            providers,
        });
    }

    match open_library_isbn_outcome(state, &isbn).await {
        ProviderOutcome::Hit(hit) => {
            let _ = state
                .db
                .set_integration_health("openlibrary", "ok", None, true)
                .await;
            providers.push(ProviderStatus {
                name: "openlibrary".into(),
                status: "hit".into(),
                detail: None,
            });
            return Ok(LookupReport {
                hit: Some(hit),
                providers,
            });
        }
        ProviderOutcome::Miss => {
            providers.push(ProviderStatus {
                name: "openlibrary".into(),
                status: "miss".into(),
                detail: None,
            });
        }
        ProviderOutcome::Error(detail) => {
            let _ = state
                .db
                .set_integration_health("openlibrary", "degraded", Some(&detail), false)
                .await;
            providers.push(ProviderStatus {
                name: "openlibrary".into(),
                status: "error".into(),
                detail: Some(detail),
            });
        }
    }

    match google_books_isbn_outcome(state, &isbn).await {
        ProviderOutcome::Hit(hit) => {
            let _ = state
                .db
                .set_integration_health("googlebooks", "ok", None, true)
                .await;
            providers.push(ProviderStatus {
                name: "googlebooks".into(),
                status: "hit".into(),
                detail: None,
            });
            return Ok(LookupReport {
                hit: Some(hit),
                providers,
            });
        }
        ProviderOutcome::Miss => {
            providers.push(ProviderStatus {
                name: "googlebooks".into(),
                status: "miss".into(),
                detail: None,
            });
        }
        ProviderOutcome::Error(detail) => {
            let health = if detail.contains("rate limited") {
                "degraded"
            } else {
                "broken"
            };
            let _ = state
                .db
                .set_integration_health("googlebooks", health, Some(&detail), false)
                .await;
            providers.push(ProviderStatus {
                name: "googlebooks".into(),
                status: "error".into(),
                detail: Some(detail),
            });
        }
    }

    Ok(LookupReport {
        hit: None,
        providers,
    })
}

async fn open_library_isbn_outcome(state: &AppState, isbn: &str) -> ProviderOutcome {
    match open_library_books_api(state, isbn).await {
        Ok(Some(hit)) => return ProviderOutcome::Hit(hit),
        Ok(None) => {}
        Err(e) => return ProviderOutcome::Error(e.to_string()),
    }
    match open_library(state, isbn).await {
        Ok(Some(hit)) => ProviderOutcome::Hit(hit),
        Ok(None) => ProviderOutcome::Miss,
        Err(e) => ProviderOutcome::Error(e.to_string()),
    }
}

/// Download a cover image by ISBN (Open Library covers, then Google Books).
pub async fn fetch_remote_cover(state: &AppState, isbn: &str) -> Result<Option<Vec<u8>>> {
    let isbn = isbn.replace('-', "");
    if isbn.is_empty() {
        return Ok(None);
    }
    if let Some(bytes) = fetch_ol_cover_by_isbn(state, &isbn).await? {
        return Ok(Some(bytes));
    }
    if let Some(hit) = lookup_isbn(state, &isbn).await? {
        if let Some(url) = hit.cover_url {
            if let Some(bytes) = fetch_image_url(state, &url).await? {
                return Ok(Some(bytes));
            }
        }
    }
    if let Some(bytes) = fetch_google_books_cover(state, &isbn).await? {
        return Ok(Some(bytes));
    }
    Ok(None)
}

async fn fetch_image_url(state: &AppState, url: &str) -> Result<Option<Vec<u8>>> {
    let resp = state.http.get(url).send().await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !ct.starts_with("image/") {
        return Ok(None);
    }
    let bytes = resp.bytes().await?;
    // Open Library sometimes returns a tiny 1x1 GIF for missing covers.
    if bytes.len() < 2_000 {
        return Ok(None);
    }
    Ok(Some(bytes.to_vec()))
}

async fn fetch_ol_cover_by_isbn(state: &AppState, isbn: &str) -> Result<Option<Vec<u8>>> {
    let url = format!("https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg");
    fetch_image_url(state, &url).await
}

async fn fetch_google_books_cover(state: &AppState, isbn: &str) -> Result<Option<Vec<u8>>> {
    let url = google_books_volumes_url(state, &format!("isbn:{isbn}"));
    let resp = state.http.get(&url).send().await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let v: Value = resp.json().await?;
    let links = v
        .pointer("/items/0/volumeInfo/imageLinks")
        .cloned()
        .unwrap_or(Value::Null);
    for key in ["extraLarge", "large", "medium", "thumbnail", "smallThumbnail"] {
        if let Some(u) = links.get(key).and_then(|x| x.as_str()) {
            let mut u = u.replace("http://", "https://");
            if let Some(stripped) = u.strip_suffix("&edge=curl") {
                u = stripped.to_string();
            }
            u = u.replace("zoom=1", "zoom=0");
            if let Some(bytes) = fetch_image_url(state, &u).await? {
                return Ok(Some(bytes));
            }
        }
    }
    Ok(None)
}

/// Merge EPUB subjects with Open Library (when ISBN is known) and map to taxonomy.
pub async fn taxonomy_from_metadata(
    state: &AppState,
    isbn: Option<&str>,
    local_subjects: &[String],
) -> TaxonomyHint {
    let mut subjects = local_subjects.to_vec();
    let mut hint = TaxonomyHint {
        subjects: subjects.clone(),
        isbn: isbn.map(|s| s.replace('-', "")),
        ..Default::default()
    };

    if let Some(isbn_v) = isbn.filter(|s| !s.is_empty()) {
        if let Ok(Some(hit)) = lookup_isbn(state, isbn_v).await {
            if hint.title.is_none() {
                hint.title = Some(hit.title);
            }
            if hint.authors.as_ref().map(|a| a.is_empty()).unwrap_or(true) && !hit.authors.is_empty()
            {
                hint.authors = Some(hit.authors);
            }
            if hint.description.is_none() {
                hint.description = hit.description;
            }
            hint.isbn = hit.isbn.or(hint.isbn);
            for s in hit.subjects {
                if !subjects.iter().any(|x| x.eq_ignore_ascii_case(&s)) {
                    subjects.push(s);
                }
            }
        }
    }

    hint.subjects = subjects.clone();
    let (primary, codes) = map_subjects_to_codes(&subjects);
    hint.primary = primary;
    hint.codes = codes;
    hint
}

fn parse_ol_subjects(v: &Value) -> Vec<String> {
    let Some(arr) = v.get("subjects").and_then(|s| s.as_array()) else {
        return vec![];
    };
    arr.iter()
        .filter_map(|item| {
            if let Some(s) = item.as_str() {
                return Some(s.to_string());
            }
            item.get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.is_empty())
        .collect()
}

async fn open_library_books_api(state: &AppState, isbn: &str) -> Result<Option<MetaHit>> {
    let url = format!(
        "https://openlibrary.org/api/books?bibkeys=ISBN:{isbn}&format=json&jscmd=data"
    );
    let resp = state.http.get(&url).send().await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let v: Value = resp.json().await?;
    let key = format!("ISBN:{isbn}");
    let Some(book) = v.get(&key) else {
        return Ok(None);
    };
    let title = book
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let authors = book
        .get("authors")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let description = book
        .get("notes")
        .and_then(|d| {
            d.as_str().map(|s| s.to_string()).or_else(|| {
                d.get("value")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string())
            })
        })
        .or_else(|| {
            book.get("excerpts")
                .and_then(|e| e.as_array())
                .and_then(|arr| arr.first())
                .and_then(|x| x.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
        });
    let subjects = book
        .get("subjects")
        .and_then(|s| s.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let cover_url = book
        .pointer("/cover/large")
        .or_else(|| book.pointer("/cover/medium"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string());
    Ok(Some(MetaHit {
        title,
        authors,
        isbn: Some(isbn.to_string()),
        description,
        subjects,
        source: "openlibrary".into(),
        cover_url,
    }))
}

async fn resolve_ol_authors(state: &AppState, v: &Value) -> String {
    let Some(arr) = v.get("authors").and_then(|a| a.as_array()) else {
        return String::new();
    };
    let mut names = Vec::new();
    for a in arr {
        if let Some(name) = a.get("name").and_then(|n| n.as_str()) {
            names.push(name.to_string());
            continue;
        }
        let Some(key) = a.get("key").and_then(|k| k.as_str()) else {
            continue;
        };
        let url = format!("https://openlibrary.org{key}.json");
        if let Ok(resp) = state.http.get(&url).send().await {
            if let Ok(av) = resp.json::<Value>().await {
                if let Some(n) = av.get("name").and_then(|n| n.as_str()) {
                    names.push(n.to_string());
                }
            }
        }
    }
    names.join(", ")
}

async fn open_library(state: &AppState, isbn: &str) -> Result<Option<MetaHit>> {
    let url = format!("https://openlibrary.org/isbn/{isbn}.json");
    let resp = state.http.get(&url).send().await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let v: Value = resp.json().await?;
    let title = v
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let description = v.get("description").and_then(|d| {
        d.as_str().map(|s| s.to_string()).or_else(|| {
            d.get("value")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        })
    });
    let subjects = parse_ol_subjects(&v);
    let authors = resolve_ol_authors(state, &v).await;
    Ok(Some(MetaHit {
        title,
        authors,
        isbn: Some(isbn.to_string()),
        description,
        subjects,
        source: "openlibrary".into(),
        cover_url: Some(format!("https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg")),
    }))
}

async fn google_books_isbn_outcome(state: &AppState, isbn: &str) -> ProviderOutcome {
    let url = google_books_volumes_url(state, &format!("isbn:{isbn}"));
    let resp = match state.http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => return ProviderOutcome::Error(e.to_string()),
    };
    let code = resp.status().as_u16();
    let (kind, detail) = classify_google_http(code);
    if kind == "error" {
        return ProviderOutcome::Error(
            detail
                .unwrap_or("HTTP error from Google Books")
                .to_string(),
        );
    }
    let v: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => return ProviderOutcome::Error(e.to_string()),
    };
    match meta_hit_from_google_volume(&v, Some(isbn)) {
        Some(hit) => ProviderOutcome::Hit(hit),
        None => ProviderOutcome::Miss,
    }
}

fn meta_hit_from_google_volume(v: &Value, isbn_hint: Option<&str>) -> Option<MetaHit> {
    let item = v.pointer("/items/0/volumeInfo")?;
    let title = item
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let authors = item
        .get("authors")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let description = item
        .get("description")
        .and_then(|d| d.as_str())
        .map(|s| s.to_string());
    let subjects = item
        .get("categories")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let cover_url = item
        .pointer("/imageLinks/thumbnail")
        .or_else(|| item.pointer("/imageLinks/smallThumbnail"))
        .and_then(|u| u.as_str())
        .map(|u| u.replace("http://", "https://").replace("zoom=1", "zoom=0"));
    let isbn = item
        .get("industryIdentifiers")
        .and_then(|ids| ids.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|id| {
                    id.get("type")
                        .and_then(|t| t.as_str())
                        .is_some_and(|t| t == "ISBN_13" || t == "ISBN_10")
                })
                .and_then(|id| id.get("identifier").and_then(|i| i.as_str()))
                .map(|s| s.replace('-', ""))
        })
        .or_else(|| isbn_hint.map(|s| s.to_string()));
    Some(MetaHit {
        title,
        authors,
        isbn,
        description,
        subjects,
        source: "googlebooks".into(),
        cover_url,
    })
}

fn meta_hits_from_google_volumes(v: &Value, limit: usize) -> Vec<MetaHit> {
    let Some(items) = v.get("items").and_then(|i| i.as_array()) else {
        return vec![];
    };
    items
        .iter()
        .take(limit)
        .filter_map(|item| {
            let wrapped = serde_json::json!({ "items": [item] });
            meta_hit_from_google_volume(&wrapped, None)
        })
        .collect()
}

pub async fn search_title(
    state: &AppState,
    title: &str,
    author: Option<&str>,
) -> Result<Vec<MetaHit>> {
    let mut hits = Vec::new();
    if let Ok(ol) = open_library_search(state, title, author).await {
        hits.extend(ol);
    }
    if hits.len() < 8 {
        if let Ok(gb) = google_books_search(state, title, author).await {
            for hit in gb {
                if hits.len() >= 8 {
                    break;
                }
                let dup = hits.iter().any(|h| {
                    h.title.eq_ignore_ascii_case(&hit.title)
                        && h.authors.eq_ignore_ascii_case(&hit.authors)
                });
                if !dup {
                    hits.push(hit);
                }
            }
        }
    }
    if hits.is_empty() {
        if let Ok(loc) = loc_search(state, title, author).await {
            hits.extend(loc);
        }
    }
    Ok(hits)
}

async fn google_books_search(
    state: &AppState,
    title: &str,
    author: Option<&str>,
) -> Result<Vec<MetaHit>> {
    let mut q = format!("intitle:{}", title.trim());
    if let Some(a) = author.map(str::trim).filter(|s| !s.is_empty()) {
        q.push_str(" inauthor:");
        q.push_str(a);
    }
    let url = google_books_volumes_url(state, &q);
    let resp = match state.http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            let _ = state
                .db
                .set_integration_health("googlebooks", "broken", Some(&e.to_string()), false)
                .await;
            return Ok(vec![]);
        }
    };
    let code = resp.status().as_u16();
    let (kind, detail) = classify_google_http(code);
    if kind == "error" {
        let msg = detail.unwrap_or("HTTP error from Google Books");
        let health = if code == 429 { "degraded" } else { "broken" };
        let _ = state
            .db
            .set_integration_health("googlebooks", health, Some(msg), false)
            .await;
        return Ok(vec![]);
    }
    let v: Value = resp.json().await?;
    let _ = state
        .db
        .set_integration_health("googlebooks", "ok", None, true)
        .await;
    Ok(meta_hits_from_google_volumes(&v, 5))
}

async fn open_library_search(
    state: &AppState,
    title: &str,
    author: Option<&str>,
) -> Result<Vec<MetaHit>> {
    let mut url = format!(
        "https://openlibrary.org/search.json?title={}",
        urlencoding_encode(title)
    );
    if let Some(a) = author {
        url.push_str(&format!("&author={}", urlencoding_encode(a)));
    }
    url.push_str("&limit=5");
    let resp = state.http.get(&url).send().await?;
    if !resp.status().is_success() {
        let _ = state
            .db
            .set_integration_health("openlibrary", "broken", Some("search failed"), false)
            .await;
        return Ok(vec![]);
    }
    #[derive(Deserialize)]
    struct Search {
        docs: Vec<Doc>,
    }
    #[derive(Deserialize)]
    struct Doc {
        title: Option<String>,
        author_name: Option<Vec<String>>,
        isbn: Option<Vec<String>>,
        first_sentence: Option<Vec<String>>,
        cover_i: Option<i64>,
    }
    let s: Search = resp.json().await?;
    let _ = state
        .db
        .set_integration_health("openlibrary", "ok", None, true)
        .await;
    Ok(s.docs
        .into_iter()
        .map(|d| {
            let isbn = d.isbn.clone().and_then(|v| v.into_iter().next());
            let cover_url = d
                .cover_i
                .map(|id| format!("https://covers.openlibrary.org/b/id/{id}-L.jpg"))
                .or_else(|| {
                    isbn.as_ref()
                        .map(|i| format!("https://covers.openlibrary.org/b/isbn/{i}-L.jpg"))
                });
            MetaHit {
                title: d.title.unwrap_or_else(|| "Unknown".into()),
                authors: d.author_name.unwrap_or_default().join(", "),
                isbn,
                description: d.first_sentence.and_then(|v| v.into_iter().next()),
                subjects: vec![],
                source: "openlibrary".into(),
                cover_url,
            }
        })
        .collect())
}

async fn loc_search(
    state: &AppState,
    title: &str,
    author: Option<&str>,
) -> Result<Vec<MetaHit>> {
    let mut query = format!("dc.title=\"{}\"", title.replace('"', ""));
    if let Some(a) = author {
        query.push_str(&format!(" AND dc.creator=\"{}\"", a.replace('"', "")));
    }
    let url = format!(
        "http://lx2.loc.gov:210/LCDB?version=1.1&operation=searchRetrieve&query={}&maximumRecords=5&recordSchema=dc",
        urlencoding_encode(&query)
    );
    let resp = match state.http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            let _ = state
                .db
                .set_integration_health("loc", "broken", Some(&e.to_string()), false)
                .await;
            return Ok(vec![]);
        }
    };
    if !resp.status().is_success() {
        let _ = state
            .db
            .set_integration_health("loc", "degraded", Some("non-200"), false)
            .await;
        return Ok(vec![]);
    }
    let text = resp.text().await?;
    let _ = state
        .db
        .set_integration_health("loc", "ok", None, true)
        .await;
    let mut hits = Vec::new();
    for rec in text.split("<srw:record>").skip(1) {
        let t = extract_tag(rec, "dc:title").unwrap_or_else(|| title.to_string());
        let a = extract_tag(rec, "dc:creator").unwrap_or_default();
        let isbn = extract_tag(rec, "dc:identifier").filter(|s| s.contains("ISBN"));
        hits.push(MetaHit {
            title: t,
            authors: a,
            isbn,
            description: None,
            subjects: vec![],
            source: "loc".into(),
            cover_url: None,
        });
    }
    Ok(hits)
}

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
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

pub fn placeholder_cover_svg(title: &str, authors: &str) -> String {
    let t = xml_escape(title);
    let a = xml_escape(authors);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"400\" height=\"600\">\n\
  <defs>\n\
    <linearGradient id=\"g\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\">\n\
      <stop offset=\"0%\" stop-color=\"#1c2b2d\"/>\n\
      <stop offset=\"100%\" stop-color=\"#3d5a45\"/>\n\
    </linearGradient>\n\
  </defs>\n\
  <rect width=\"400\" height=\"600\" fill=\"url(#g)\"/>\n\
  <text x=\"40\" y=\"220\" fill=\"#f4f1e8\" font-family=\"Georgia, serif\" font-size=\"28\">{t}</text>\n\
  <text x=\"40\" y=\"280\" fill=\"#c5d0c0\" font-family=\"Georgia, serif\" font-size=\"18\">{a}</text>\n\
  <text x=\"40\" y=\"560\" fill=\"#8a9a88\" font-family=\"sans-serif\" font-size=\"12\">Diarch placeholder</text>\n\
</svg>"
    )
}

fn xml_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '&' => "&amp;".into(),
            '"' => "&quot;".into(),
            _ => c.to_string(),
        })
        .collect()
}

pub fn cover_prompt(title: &str, authors: &str, genre: &str) -> String {
    format!(
        "Book cover illustration for \"{title}\" by {authors}. Genre: {genre}. \
         No text on the cover. Atmospheric, print-quality, single strong visual motif."
    )
}

pub async fn generate_cover_localai(
    state: &AppState,
    prompt: &str,
) -> Result<Option<Vec<u8>>> {
    // Kept for future LocalAI/Hermes wiring; returns None when unset.
    let Some(base) = state.config.localai_url.as_ref() else {
        return Ok(None);
    };
    if let Some(hermes) = state.config.hermes_url.as_ref() {
        let body = serde_json::json!({
            "task": "generate_book_cover",
            "prompt": prompt
        });
        let resp = state.http.post(format!("{hermes}/cover")).json(&body).send().await;
        if let Ok(r) = resp {
            if r.status().is_success() {
                let bytes = r.bytes().await?;
                let _ = state
                    .db
                    .set_integration_health("localai", "ok", None, true)
                    .await;
                return Ok(Some(bytes.to_vec()));
            }
        }
    }
    let url = format!("{base}/v1/images/generations");
    let body = serde_json::json!({
        "prompt": prompt,
        "size": "512x768"
    });
    let resp = state.http.post(&url).json(&body).send().await?;
    if !resp.status().is_success() {
        let _ = state
            .db
            .set_integration_health("localai", "broken", Some("image gen failed"), false)
            .await;
        return Ok(None);
    }
    let v: Value = resp.json().await?;
    if let Some(b64) = v
        .pointer("/data/0/b64_json")
        .and_then(|x| x.as_str())
    {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64)?;
        let _ = state
            .db
            .set_integration_health("localai", "ok", None, true)
            .await;
        return Ok(Some(bytes));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_429_classified_as_error() {
        let (status, detail) = classify_google_http(429);
        assert_eq!(status, "error");
        assert!(detail.unwrap().contains("DIARCH_GOOGLE_BOOKS_KEY"));
    }

    #[test]
    fn google_403_classified_as_error() {
        let (status, detail) = classify_google_http(403);
        assert_eq!(status, "error");
        assert!(detail.unwrap().contains("forbidden"));
    }

    #[test]
    fn google_200_classified_ok() {
        let (status, detail) = classify_google_http(200);
        assert_eq!(status, "ok");
        assert!(detail.is_none());
    }

    #[test]
    fn google_volume_parses_isbn13() {
        let v = serde_json::json!({
            "items": [{
                "volumeInfo": {
                    "title": "Example",
                    "authors": ["A Author"],
                    "industryIdentifiers": [
                        {"type": "ISBN_13", "identifier": "9780140328721"}
                    ],
                    "imageLinks": {"thumbnail": "http://example.com/t.jpg"}
                }
            }]
        });
        let hit = meta_hit_from_google_volume(&v, None).unwrap();
        assert_eq!(hit.title, "Example");
        assert_eq!(hit.isbn.as_deref(), Some("9780140328721"));
        assert_eq!(hit.source, "googlebooks");
        assert!(hit.cover_url.unwrap().starts_with("https://"));
    }
}
