use crate::login_limit::LoginLimiter;
use diarch_core::Config;
use diarch_db::Db;

pub struct AppState {
    pub db: Db,
    pub config: Config,
    /// General-purpose HTTP (metadata, probes).
    pub http: reqwest::Client,
    /// Same as `http` but never follows redirects — used when fetching
    /// remote images so we can validate each hop against a host allowlist.
    pub http_no_redirect: reqwest::Client,
    /// Login attempt tracking for rate limiting.
    pub login_limiter: LoginLimiter,
}
