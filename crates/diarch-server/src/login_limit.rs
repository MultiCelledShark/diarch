//! In-memory login rate limiting with exponential backoff.
//!
//! Keyed by `username + client IP` so a single account can't be pounded from
//! one source without also giving noisy neighbours their own budget. This is
//! process-local (not shared across replicas) — fine for Diarch's
//! single-instance deployment model.

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use axum::http::HeaderMap;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Like `Option<ConnectInfo<SocketAddr>>`, but implemented directly against
/// request extensions instead of relying on axum's `OptionalFromRequestParts`
/// specialization (which `ConnectInfo` doesn't opt into). This lets the
/// extractor degrade to `None` in tests that bypass `MakeService` (e.g.
/// `tower::ServiceExt::oneshot`) instead of failing the whole request.
pub struct MaybeConnectInfo(pub Option<SocketAddr>);

impl<S> FromRequestParts<S> for MaybeConnectInfo
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(MaybeConnectInfo(
            parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0),
        ))
    }
}

/// Cap on tracked keys before we start evicting stale entries, to bound
/// memory under a sustained distributed guessing attack.
const MAX_TRACKED_KEYS: usize = 20_000;
/// Entries idle longer than this are dropped on the next eviction sweep.
const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
/// Delay after the first failed attempt (grows exponentially after this).
const BASE_DELAY_SECS: u64 = 1;
/// Upper bound on the backoff delay.
const MAX_DELAY_SECS: u64 = 300;

struct Entry {
    failures: u32,
    locked_until: Option<Instant>,
    last_attempt: Instant,
}

/// Tracks failed login attempts per `(username, ip)` and enforces
/// exponentially growing lockout windows after repeated failures.
pub struct LoginLimiter {
    entries: Mutex<HashMap<String, Entry>>,
}

impl Default for LoginLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl LoginLimiter {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn key(username: &str, ip: &str) -> String {
        format!("{}|{ip}", username.trim().to_ascii_lowercase())
    }

    /// Returns `Some(remaining)` when the caller is currently locked out.
    pub fn check(&self, username: &str, ip: &str) -> Option<Duration> {
        let now = Instant::now();
        let mut map = self.entries.lock().unwrap();
        evict_stale_if_needed(&mut map, now);
        let entry = map.get(&Self::key(username, ip))?;
        let until = entry.locked_until?;
        if now < until {
            Some(until - now)
        } else {
            None
        }
    }

    /// Record a failed attempt and extend the lockout window.
    pub fn record_failure(&self, username: &str, ip: &str) {
        let now = Instant::now();
        let mut map = self.entries.lock().unwrap();
        evict_stale_if_needed(&mut map, now);
        let entry = map.entry(Self::key(username, ip)).or_insert(Entry {
            failures: 0,
            locked_until: None,
            last_attempt: now,
        });
        entry.failures = entry.failures.saturating_add(1);
        entry.last_attempt = now;
        entry.locked_until = Some(now + backoff_delay(entry.failures));
    }

    /// Clear any tracked failures on successful login.
    pub fn record_success(&self, username: &str, ip: &str) {
        let mut map = self.entries.lock().unwrap();
        map.remove(&Self::key(username, ip));
    }
}

fn backoff_delay(failures: u32) -> Duration {
    if failures == 0 {
        return Duration::ZERO;
    }
    // failures=1 -> 1s, 2 -> 2s, 3 -> 4s, ... capped at MAX_DELAY_SECS.
    let exp = failures.saturating_sub(1).min(16);
    let secs = BASE_DELAY_SECS.saturating_mul(1u64 << exp);
    Duration::from_secs(secs.min(MAX_DELAY_SECS))
}

fn evict_stale_if_needed(map: &mut HashMap<String, Entry>, now: Instant) {
    if map.len() < MAX_TRACKED_KEYS {
        return;
    }
    map.retain(|_, e| now.duration_since(e.last_attempt) < STALE_AFTER);
}

/// Resolve the client IP used to key rate limiting.
///
/// Only trusts `X-Forwarded-For` when `DIARCH_TRUST_PROXY=1` (set this only
/// when Diarch sits behind a reverse proxy that overwrites/sanitizes the
/// header — otherwise any client can spoof it to dodge rate limits).
pub fn client_ip(headers: &HeaderMap, connect_ip: Option<IpAddr>) -> String {
    let trust_proxy = std::env::var("DIARCH_TRUST_PROXY")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    if trust_proxy {
        if let Some(xff) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
        {
            if let Some(first) = xff.split(',').next() {
                let candidate = first.trim();
                if !candidate.is_empty() {
                    return candidate.to_string();
                }
            }
        }
    }
    connect_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff_delay(0), Duration::ZERO);
        assert_eq!(backoff_delay(1), Duration::from_secs(1));
        assert_eq!(backoff_delay(2), Duration::from_secs(2));
        assert_eq!(backoff_delay(3), Duration::from_secs(4));
        assert_eq!(backoff_delay(20), Duration::from_secs(MAX_DELAY_SECS));
    }

    #[test]
    fn limiter_locks_out_after_failure_then_clears_on_success() {
        let limiter = LoginLimiter::new();
        assert!(limiter.check("bob", "1.2.3.4").is_none());
        limiter.record_failure("bob", "1.2.3.4");
        assert!(limiter.check("bob", "1.2.3.4").is_some());
        // A different IP for the same username has its own budget.
        assert!(limiter.check("bob", "5.6.7.8").is_none());
        limiter.record_success("bob", "1.2.3.4");
        assert!(limiter.check("bob", "1.2.3.4").is_none());
    }

    #[test]
    fn client_ip_ignores_xff_unless_trust_proxy_set() {
        // SAFETY: this test owns the process environment and does not run in parallel
        // with other tests that read DIARCH_TRUST_PROXY.
        unsafe { std::env::remove_var("DIARCH_TRUST_PROXY") };
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "9.9.9.9".parse().unwrap());
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        assert_eq!(client_ip(&headers, Some(ip)), "127.0.0.1");

        unsafe { std::env::set_var("DIARCH_TRUST_PROXY", "1") };
        assert_eq!(client_ip(&headers, Some(ip)), "9.9.9.9");
        unsafe { std::env::remove_var("DIARCH_TRUST_PROXY") };
    }
}
