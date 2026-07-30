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

pub async fn lookup_isbn(state: &AppState, isbn: &str) -> Result<Option<MetaHit>> {
    let isbn = isbn.replace('-', "");
    if let Some(hit) = open_library(state, &isbn).await? {
        let _ = state
            .db
            .set_integration_health("openlibrary", "ok", None, true)
            .await;
        return Ok(Some(hit));
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
    let description = v
        .get("description")
        .and_then(|d| d.as_str().map(|s| s.to_string()).or_else(|| {
            d.get("value")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        }));
    let subjects = parse_ol_subjects(&v);
    Ok(Some(MetaHit {
        title,
        authors: String::new(),
        isbn: Some(isbn.to_string()),
        description,
        subjects,
        source: "openlibrary".into(),
    }))
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
    if hits.is_empty() {
        if let Ok(loc) = loc_search(state, title, author).await {
            hits.extend(loc);
        }
    }
    Ok(hits)
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
    }
    let s: Search = resp.json().await?;
    let _ = state
        .db
        .set_integration_health("openlibrary", "ok", None, true)
        .await;
    Ok(s.docs
        .into_iter()
        .map(|d| MetaHit {
            title: d.title.unwrap_or_else(|| "Unknown".into()),
            authors: d.author_name.unwrap_or_default().join(", "),
            isbn: d.isbn.and_then(|v| v.into_iter().next()),
            description: d.first_sentence.and_then(|v| v.into_iter().next()),
            subjects: vec![],
            source: "openlibrary".into(),
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
    // Very light XML scrape for dc:title / dc:creator
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
    let Some(base) = state.config.localai_url.as_ref() else {
        return Ok(None);
    };
    // Prefer Hermes agent if configured
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
