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
    /// OpenAI-style `WxH` for cover generation (default `512x512`).
    pub localai_image_size: Option<String>,
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
            localai_image_size: None,
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
        c.localai_image_size = std::env::var("DIARCH_LOCALAI_IMAGE_SIZE")
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
            if c.localai_image_size.is_none() {
                c.localai_image_size = Some("512x512".into());
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

/// Sanitize a multipart upload filename for use under `imports/`.
/// Returns only a basename with safe characters; rejects traversal and empty names.
pub fn sanitize_upload_filename(name: &str) -> Result<String, &'static str> {
    use std::path::Path;
    if name.is_empty() || name.contains('\0') || name.contains("..") {
        return Err("empty or invalid filename");
    }
    if name.contains('/') || name.contains('\\') {
        // Allow only after taking basename — reject raw path separators in the
        // original string so clients cannot sneak directory components in.
        let base = Path::new(name)
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("invalid filename")?;
        return sanitize_upload_filename(base);
    }
    let base = name;
    if base.is_empty() || base == "." || base == ".." {
        return Err("invalid filename");
    }
    let mut out = String::with_capacity(base.len());
    for c in base.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '.' | '_' | '-' | ' ' => out.push(c),
            _ => out.push('_'),
        }
    }
    let out = out.trim().trim_matches('.').to_string();
    if out.is_empty() || out == "." || out == ".." {
        return Err("invalid filename");
    }
    if out.len() > 200 {
        Ok(out.chars().take(200).collect())
    } else {
        Ok(out)
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

/// Sniff raster image magic bytes. Returns the MIME type for JPEG/PNG/WebP only;
/// anything else (SVG, HTML, arbitrary binaries, etc.) is rejected.
pub fn sniff_image_mime(data: &[u8]) -> Option<&'static str> {
    if data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return Some("image/jpeg");
    }
    if data.len() >= 8 && data[0..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        return Some("image/png");
    }
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
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

/// Per-user ebook/markdown reader typography (persisted in `user_settings`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReaderTypography {
    #[serde(default = "default_palette")]
    pub palette: String,
    #[serde(default = "default_font")]
    pub font: String,
    /// Percent of base size (80–200).
    #[serde(default = "default_size")]
    pub size: u32,
    #[serde(default = "default_line_height")]
    pub line_height: String,
    #[serde(default = "default_measure")]
    pub measure: String,
    #[serde(default)]
    pub justify: bool,
    #[serde(default = "default_letter_spacing")]
    pub letter_spacing: String,
    #[serde(default = "default_paragraph_spacing")]
    pub paragraph_spacing: String,
    #[serde(default)]
    pub indent: bool,
    #[serde(default)]
    pub hyphenate: bool,
}

fn default_palette() -> String {
    "dark".into()
}
fn default_font() -> String {
    "serif".into()
}
fn default_size() -> u32 {
    100
}
fn default_line_height() -> String {
    "normal".into()
}
fn default_measure() -> String {
    "medium".into()
}
fn default_letter_spacing() -> String {
    "normal".into()
}
fn default_paragraph_spacing() -> String {
    "normal".into()
}

impl Default for ReaderTypography {
    fn default() -> Self {
        Self {
            palette: default_palette(),
            font: default_font(),
            size: default_size(),
            line_height: default_line_height(),
            measure: default_measure(),
            justify: false,
            letter_spacing: default_letter_spacing(),
            paragraph_spacing: default_paragraph_spacing(),
            indent: false,
            hyphenate: false,
        }
    }
}

impl ReaderTypography {
    /// Clamp / whitelist fields from client JSON.
    pub fn sanitize(mut self) -> Self {
        const PALETTES: &[&str] = &["dark", "sepia", "light", "paper", "night", "contrast"];
        const FONTS: &[&str] = &["serif", "sans", "mono", "dyslexia", "literata"];
        const LINE: &[&str] = &["tight", "normal", "loose"];
        const MEASURE: &[&str] = &["narrow", "medium", "wide"];
        const TRACK: &[&str] = &["tight", "normal", "wide"];
        const PARA: &[&str] = &["compact", "normal", "roomy"];

        if !PALETTES.contains(&self.palette.as_str()) {
            self.palette = default_palette();
        }
        if !FONTS.contains(&self.font.as_str()) {
            self.font = default_font();
        }
        self.size = self.size.clamp(80, 200);
        self.size = (self.size / 5) * 5;
        if self.size < 80 {
            self.size = 80;
        }
        if !LINE.contains(&self.line_height.as_str()) {
            self.line_height = default_line_height();
        }
        if !MEASURE.contains(&self.measure.as_str()) {
            self.measure = default_measure();
        }
        if !TRACK.contains(&self.letter_spacing.as_str()) {
            self.letter_spacing = default_letter_spacing();
        }
        if !PARA.contains(&self.paragraph_spacing.as_str()) {
            self.paragraph_spacing = default_paragraph_spacing();
        }
        self
    }

    pub fn from_json_str(s: &str) -> Self {
        serde_json::from_str::<ReaderTypography>(s)
            .unwrap_or_default()
            .sanitize()
    }

    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.clone().sanitize())
    }
}

const MAX_UI_THEME_CSS: usize = 16 * 1024;

/// CSS `data-theme` name: `forest`, `mytheme`, …
pub fn sanitize_ui_theme_name(name: &str) -> String {
    let s = name.trim().to_ascii_lowercase();
    let ok = (1..=40).contains(&s.len())
        && s.starts_with(|c: char| c.is_ascii_alphabetic())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if ok { s } else { "diarch".into() }
}

fn allowed_theme_prop(prop: &str) -> bool {
    matches!(prop, "--border" | "--depth" | "--noise" | "--font-family")
        || prop.starts_with("--color-")
        || prop.starts_with("--radius-")
        || prop.starts_with("--size-")
}

fn allowed_theme_css_value(val: &str) -> bool {
    let v = val.trim();
    if v.is_empty() || v.len() > 220 {
        return false;
    }
    let lower = v.to_ascii_lowercase();
    if lower.contains("url(")
        || lower.contains("expression")
        || lower.contains("javascript")
        || lower.contains('@')
        || lower.contains('<')
        || lower.contains('>')
        || lower.contains('{')
        || lower.contains('}')
        || lower.contains('\\')
    {
        return false;
    }
    v.chars().all(|c| c.is_ascii() && !c.is_ascii_control())
}

/// Rebuild a `[data-theme]` block from a daisyUI generator paste. Empty on junk.
pub fn sanitize_ui_theme_css(name: &str, raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    if raw.len() > MAX_UI_THEME_CSS * 2 {
        return String::new();
    }
    let name = sanitize_ui_theme_name(name);
    let mut color_scheme: Option<&str> = None;
    let lower = raw.to_ascii_lowercase();
    if let Some(i) = lower.find("color-scheme") {
        let slice = &lower[i..i.saturating_add(40).min(lower.len())];
        if slice.contains("dark") {
            color_scheme = Some("dark");
        } else if slice.contains("light") {
            color_scheme = Some("light");
        }
    }
    let mut decls: Vec<(String, String)> = Vec::new();
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'-' && bytes[i + 1] == b'-' {
            let rest = &raw[i..];
            let Some(colon) = rest.find(':') else { break };
            let prop = rest[..colon].trim().to_ascii_lowercase();
            let after = &rest[colon + 1..];
            let Some(semi) = after.find(';') else { break };
            let val = after[..semi].trim();
            i += colon + 1 + semi + 1;
            if allowed_theme_prop(&prop) && allowed_theme_css_value(val) {
                if !decls.iter().any(|(p, _)| p == &prop) {
                    decls.push((prop, val.to_string()));
                }
            }
            continue;
        }
        i += 1;
    }
    if decls.len() < 4 {
        return String::new();
    }
    let mut out = format!("[data-theme=\"{name}\"] {{\n");
    if let Some(cs) = color_scheme {
        out.push_str("  color-scheme: ");
        out.push_str(cs);
        out.push_str(";\n");
    }
    for (prop, val) in decls {
        out.push_str("  ");
        out.push_str(&prop);
        out.push_str(": ");
        out.push_str(&val);
        out.push_str(";\n");
    }
    out.push('}');
    if out.len() > MAX_UI_THEME_CSS {
        return String::new();
    }
    out
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
        // SAFETY: this test owns the process environment and does not run in parallel
        // with other tests that read DIARCH_SHOW_AUDIO_GAPS.
        unsafe { std::env::set_var("DIARCH_SHOW_AUDIO_GAPS", "off") };
        let c = Config::from_env();
        assert!(!c.show_audio_gaps);
        unsafe { std::env::remove_var("DIARCH_SHOW_AUDIO_GAPS") };
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

    #[test]
    fn sanitize_upload_filename_rejects_traversal() {
        assert!(sanitize_upload_filename("../../etc/passwd").is_err());
        assert!(sanitize_upload_filename("..\\win.ini").is_err());
        assert!(sanitize_upload_filename("").is_err());
        assert!(sanitize_upload_filename("..").is_err());
        assert_eq!(
            sanitize_upload_filename("My Book.epub").unwrap(),
            "My Book.epub"
        );
        // Basename-only from a path without `..`
        assert_eq!(
            sanitize_upload_filename("subdir/evil.epub").unwrap(),
            "evil.epub"
        );
    }

    #[test]
    fn sniff_image_mime_detects_known_formats_only() {
        assert_eq!(sniff_image_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(
            sniff_image_mime(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00]),
            Some("image/png")
        );
        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&[0, 0, 0, 0]);
        webp.extend_from_slice(b"WEBP");
        assert_eq!(sniff_image_mime(&webp), Some("image/webp"));
        assert_eq!(sniff_image_mime(b"<svg xmlns=..."), None);
        assert_eq!(sniff_image_mime(b"not an image"), None);
        assert_eq!(sniff_image_mime(&[]), None);
    }

    #[test]
    fn reader_typography_sanitizes_unknowns() {
        let t = ReaderTypography {
            palette: "neon".into(),
            font: "comic".into(),
            size: 173,
            line_height: "huge".into(),
            measure: "ultrawide".into(),
            justify: true,
            letter_spacing: "loose".into(),
            paragraph_spacing: "big".into(),
            indent: true,
            hyphenate: true,
        }
        .sanitize();
        assert_eq!(t.palette, "dark");
        assert_eq!(t.font, "serif");
        assert_eq!(t.size, 170);
        assert_eq!(t.line_height, "normal");
        assert_eq!(t.measure, "medium");
        assert_eq!(t.letter_spacing, "normal");
        assert_eq!(t.paragraph_spacing, "normal");
        assert!(t.justify && t.indent && t.hyphenate);
    }

    #[test]
    fn reader_typography_accepts_camel_case_aliases() {
        let t = ReaderTypography::from_json_str(
            r#"{"palette":"sepia","font":"literata","size":110,"lineHeight":"loose","letterSpacing":"wide","paragraphSpacing":"roomy","indent":true,"hyphenate":true}"#,
        );
        assert_eq!(t.palette, "sepia");
        assert_eq!(t.font, "literata");
        assert_eq!(t.size, 110);
        assert_eq!(t.line_height, "loose");
        assert_eq!(t.letter_spacing, "wide");
        assert_eq!(t.paragraph_spacing, "roomy");
        assert!(t.indent && t.hyphenate);
    }

    #[test]
    fn ui_theme_name_sanitizes() {
        assert_eq!(sanitize_ui_theme_name("Forest"), "forest");
        assert_eq!(sanitize_ui_theme_name("my-theme"), "my-theme");
        assert_eq!(sanitize_ui_theme_name("../x"), "diarch");
        assert_eq!(sanitize_ui_theme_name(""), "diarch");
        assert_eq!(sanitize_ui_theme_name("1bad"), "diarch");
    }

    #[test]
    fn ui_theme_css_keeps_daisy_tokens() {
        let css = sanitize_ui_theme_css(
            "grove",
            r#"@plugin "daisyui/theme" {
              name: "grove";
              color-scheme: dark;
              --color-base-100: oklch(20% 0.02 140);
              --color-base-200: oklch(18% 0.02 140);
              --color-base-300: oklch(16% 0.02 140);
              --color-base-content: oklch(90% 0.02 140);
              --color-primary: #c4a35a;
            }"#,
        );
        assert!(css.contains("[data-theme=\"grove\"]"));
        assert!(css.contains("color-scheme: dark;"));
        assert!(css.contains("--color-primary: #c4a35a;"));
        assert!(!css.contains("@plugin"));
    }

    #[test]
    fn ui_theme_css_rejects_url_and_markup() {
        assert!(sanitize_ui_theme_css(
            "x",
            "--color-base-100: red; --color-base-200: url(https://evil); --color-base-300: a; --color-primary: b;"
        )
        .is_empty());
        assert!(sanitize_ui_theme_css(
            "x",
            "--color-base-100: red; --color-base-200: </style>; --color-base-300: a; --color-primary: b;"
        )
        .is_empty());
    }
}
