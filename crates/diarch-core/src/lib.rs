use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub mod taxonomy;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub listen: String,
    pub data_dir: PathBuf,
    pub admin_username: String,
    pub admin_password: String,
    pub localai_url: Option<String>,
    pub hermes_url: Option<String>,
    /// LocalAI image model id (e.g. `flux.2-klein-4b`). Defaults when URL is set.
    pub localai_image_model: Option<String>,
    /// LocalAI ASR model id (e.g. `nemo-parakeet-tdt-0.6b`). Defaults when URL is set.
    pub localai_transcribe_model: Option<String>,
    pub storygraph_cookie: Option<String>,
    pub storygraph_username: Option<String>,
    pub remarkable_token: Option<String>,
    /// Audible activation bytes for AAX → M4B (never log this value).
    pub audible_key: Option<String>,
    /// Optional Google Books API key (avoids unauthenticated daily quota exhaustion).
    pub google_books_key: Option<String>,
    pub show_audio_gaps: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8083".into(),
            data_dir: PathBuf::from("./data"),
            admin_username: "admin".into(),
            admin_password: "admin".into(),
            localai_url: None,
            hermes_url: None,
            localai_image_model: None,
            localai_transcribe_model: None,
            storygraph_cookie: None,
            storygraph_username: None,
            remarkable_token: None,
            audible_key: None,
            google_books_key: None,
            show_audio_gaps: true,
        }
    }
}

impl Config {
    pub fn from_env() -> Self {
        let mut c = Self::default();
        if let Ok(v) = std::env::var("DIARCH_LISTEN") {
            c.listen = v;
        }
        if let Ok(v) = std::env::var("DIARCH_DATA_DIR") {
            c.data_dir = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("DIARCH_ADMIN_USER") {
            c.admin_username = v;
        }
        if let Ok(v) = std::env::var("DIARCH_ADMIN_PASS") {
            c.admin_password = v;
        }
        c.localai_url = std::env::var("DIARCH_LOCALAI_URL").ok().filter(|s| !s.is_empty());
        c.hermes_url = std::env::var("DIARCH_HERMES_URL").ok().filter(|s| !s.is_empty());
        c.localai_image_model = std::env::var("DIARCH_LOCALAI_IMAGE_MODEL")
            .ok()
            .filter(|s| !s.is_empty());
        c.localai_transcribe_model = std::env::var("DIARCH_LOCALAI_TRANSCRIBE_MODEL")
            .ok()
            .filter(|s| !s.is_empty());
        c.storygraph_cookie = std::env::var("DIARCH_STORYGRAPH_COOKIE").ok();
        c.storygraph_username = std::env::var("DIARCH_STORYGRAPH_USER").ok();
        c.remarkable_token = std::env::var("DIARCH_REMARKABLE_TOKEN").ok();
        c.audible_key = std::env::var("DIARCH_AUDIBLE_KEY").ok().filter(|s| !s.is_empty());
        c.google_books_key =
            std::env::var("DIARCH_GOOGLE_BOOKS_KEY").ok().filter(|s| !s.is_empty());
        if let Ok(v) = std::env::var("DIARCH_SHOW_AUDIO_GAPS") {
            c.show_audio_gaps = matches!(v.as_str(), "1" | "true" | "yes" | "on");
        }
        // Sensible defaults when LocalAI is configured but models are unset.
        if c.localai_url.is_some() {
            if c.localai_image_model.is_none() {
                c.localai_image_model = Some("flux.2-klein-4b".into());
            }
            if c.localai_transcribe_model.is_none() {
                c.localai_transcribe_model = Some("nemo-parakeet-tdt-0.6b".into());
            }
        }
        c
    }

    /// Candidate cover path (staged AI generate — not live until approve).
    pub fn cover_candidate_name() -> &'static str {
        "cover.candidate.jpg"
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("diarch.db")
    }

    pub fn library_dir(&self) -> PathBuf {
        self.data_dir.join("library")
    }

    pub fn work_dir(&self, work_id: Uuid) -> PathBuf {
        self.library_dir().join(work_id.to_string())
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.library_dir())?;
        std::fs::create_dir_all(self.data_dir.join("imports"))?;
        std::fs::create_dir_all(self.data_dir.join("queue/needs_tts"))?;
        std::fs::create_dir_all(self.data_dir.join("queue/incoming_audio"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingStatus {
    Unread,
    /// Short "up next" shelf (max [`MAX_TO_READ`]).
    ToRead,
    /// Actively reading / listening (max [`MAX_CURRENTLY_READING`]).
    Reading,
    Read,
    Wishlist,
}

/// Cap for the Currently Reading shelf.
pub const MAX_CURRENTLY_READING: usize = 3;
/// Cap for the To Read shelf.
pub const MAX_TO_READ: usize = 9;

impl ReadingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unread => "unread",
            Self::ToRead => "to_read",
            Self::Reading => "reading",
            Self::Read => "read",
            Self::Wishlist => "wishlist",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "unread" => Some(Self::Unread),
            "to_read" => Some(Self::ToRead),
            "reading" => Some(Self::Reading),
            "read" => Some(Self::Read),
            "wishlist" => Some(Self::Wishlist),
            _ => None,
        }
    }

    pub fn shelf_cap(self) -> Option<usize> {
        match self {
            Self::Reading => Some(MAX_CURRENTLY_READING),
            Self::ToRead => Some(MAX_TO_READ),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Epub,
    Markdown,
    Audio,
    Cover,
    Media,
}

impl AssetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Epub => "epub",
            Self::Markdown => "markdown",
            Self::Audio => "audio",
            Self::Cover => "cover",
            Self::Media => "media",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "epub" => Some(Self::Epub),
            "markdown" => Some(Self::Markdown),
            "audio" => Some(Self::Audio),
            "cover" => Some(Self::Cover),
            "media" => Some(Self::Media),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub is_admin: bool,
    pub show_audio_gaps: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Work {
    pub id: Uuid,
    pub title: String,
    pub authors: String,
    pub isbn: Option<String>,
    pub description: Option<String>,
    pub status: ReadingStatus,
    pub primary_code: Option<i32>,
    pub year_list: Option<i32>,
    pub rating: Option<f64>,
    pub review: Option<String>,
    pub reading_direction: String,
    pub is_manga: bool,
    pub needs_review: bool,
    pub needs_cover: bool,
    pub needs_tts: bool,
    pub needs_audio: bool,
    pub needs_transcription: bool,
    pub sg_review_dirty: bool,
    pub sg_needs_add: bool,
    pub sg_audio_only_remote: bool,
    pub sg_matched: bool,
    pub sg_book_id: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkAsset {
    pub id: Uuid,
    pub work_id: Uuid,
    pub kind: AssetKind,
    pub relative_path: String,
    pub mime: Option<String>,
    pub bytes: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxonomyNode {
    pub code: i32,
    pub name: String,
    pub parent_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadingProgress {
    pub user_id: Uuid,
    pub work_id: Uuid,
    pub mode: String,
    pub position: String,
    pub percent: f64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub kind: String,
    pub work_id: Option<Uuid>,
    pub status: String,
    pub detail: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationHealth {
    pub name: String,
    pub status: String,
    pub last_error: Option<String>,
    pub last_ok_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

pub fn work_epub_path(root: &Path, work_id: Uuid) -> PathBuf {
    root.join(work_id.to_string()).join("book.epub")
}

pub fn work_markdown_path(root: &Path, work_id: Uuid) -> PathBuf {
    root.join(work_id.to_string()).join("book.md")
}

pub fn work_media_dir(root: &Path, work_id: Uuid) -> PathBuf {
    root.join(work_id.to_string()).join("media")
}

pub fn work_cover_path(root: &Path, work_id: Uuid) -> PathBuf {
    root.join(work_id.to_string()).join("cover.jpg")
}

pub fn work_audio_path(root: &Path, work_id: Uuid, filename: &str) -> PathBuf {
    root.join(work_id.to_string()).join("audio").join(filename)
}

/// Prefer RTL for manga taxonomy band 8900–8939 or explicit flag.
pub fn infer_manga(primary_code: Option<i32>, codes: &[i32], is_manga_flag: bool) -> bool {
    if is_manga_flag {
        return true;
    }
    codes
        .iter()
        .copied()
        .chain(primary_code)
        .any(|c| (8900..=8939).contains(&c))
}

/// Normalize titles for fuzzy matching (ISBN-less StoryGraph sync).
pub fn normalize_title(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub fn titles_match(a: &str, b: &str) -> bool {
    normalize_title(a) == normalize_title(b)
}

/// On-disk library files stay canonical (`book.epub`, `book.md`, …).
/// Outbound names (download, reMarkable, queue export) come from SQLite title at export time.
pub fn export_stem(title: &str) -> String {
    let mut s: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    s = s
        .split_whitespace()
        .map(|w| w.trim_matches('-'))
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    while s.ends_with('.') {
        s.pop();
    }
    if s.len() > 120 {
        s.truncate(120);
        s = s.trim_end_matches([' ', '-', '.']).to_string();
    }
    if s.is_empty() {
        "Untitled".into()
    } else {
        s
    }
}

/// `{export_stem(title)}.{ext}` — e.g. `A Covenant of Ice.epub`.
pub fn export_filename(title: &str, extension: &str) -> String {
    let ext = extension.trim().trim_start_matches('.');
    if ext.is_empty() {
        export_stem(title)
    } else {
        format!("{}.{}", export_stem(title), ext)
    }
}

/// `Content-Disposition: attachment` value using a sanitized SQLite title.
/// Header values must be ASCII; non-ASCII title chars become `_`.
pub fn content_disposition_attachment(title: &str, extension: &str) -> String {
    let name: String = export_filename(title, extension)
        .chars()
        .map(|c| {
            if c == '"' {
                '_'
            } else if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("attachment; filename=\"{name}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_paths() {
        let c = Config {
            data_dir: "/var/lib/diarch".into(),
            ..Config::default()
        };
        assert_eq!(c.db_path(), std::path::PathBuf::from("/var/lib/diarch/diarch.db"));
        assert_eq!(
            c.library_dir(),
            std::path::PathBuf::from("/var/lib/diarch/library")
        );
    }

    #[test]
    fn config_show_audio_gaps_env() {
        std::env::set_var("DIARCH_SHOW_AUDIO_GAPS", "off");
        let c = Config::from_env();
        assert!(!c.show_audio_gaps);
        std::env::remove_var("DIARCH_SHOW_AUDIO_GAPS");
    }

    #[test]
    fn reading_status_roundtrip() {
        for s in ["unread", "to_read", "reading", "read", "wishlist"] {
            let st = ReadingStatus::parse(s).unwrap();
            assert_eq!(st.as_str(), s);
        }
        assert!(ReadingStatus::parse("nope").is_none());
        assert_eq!(ReadingStatus::Reading.shelf_cap(), Some(3));
        assert_eq!(ReadingStatus::ToRead.shelf_cap(), Some(9));
        assert_eq!(ReadingStatus::Unread.shelf_cap(), None);
    }

    #[test]
    fn asset_kind_roundtrip() {
        for s in ["epub", "markdown", "audio", "cover", "media"] {
            assert_eq!(AssetKind::parse(s).unwrap().as_str(), s);
        }
    }

    #[test]
    fn infer_manga_from_codes_and_flag() {
        assert!(infer_manga(None, &[8920], false));
        assert!(infer_manga(Some(8920), &[], false));
        assert!(!infer_manga(Some(8201), &[8940], false));
        assert!(infer_manga(None, &[], true));
        assert!(!infer_manga(None, &[8899], false));
    }

    #[test]
    fn title_matching_ignores_punctuation_and_case() {
        assert!(titles_match("The Eye of the World!", "the eye of the world"));
        assert!(!titles_match("Dune", "Dune Messiah"));
    }

    #[test]
    fn export_filename_from_sqlite_title() {
        assert_eq!(
            export_filename("A Covenant of Ice", "epub"),
            "A Covenant of Ice.epub"
        );
        assert_eq!(export_stem("Foo/Bar: Baz?"), "Foo-Bar Baz");
        assert_eq!(export_stem("   "), "Untitled");
        assert!(content_disposition_attachment("Ice", "epub").contains("Ice.epub"));
    }
}
