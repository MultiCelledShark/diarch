use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use uuid::Uuid;

use crate::state::AppState;

const REMOTE_FOLDER: &str = "Diarch";
pub const CONNECT_URL: &str = "https://my.remarkable.com/device/browser/connect";
const AUTH_HOST: &str = "https://webapp-prod.cloud.remarkable.engineering";
const DEVICE_NEW: &str = "/token/json/2/device/new";
const USER_NEW: &str = "/token/json/2/user/new";

#[derive(Debug, Serialize)]
pub struct RemarkableStatus {
    pub rmapi_installed: bool,
    pub authenticated: bool,
    pub connect_url: &'static str,
    pub config_path: String,
    pub detail: Option<String>,
}

/// Path where rmapi stores tokens (`RMAPI_CONFIG`, else `~/.rmapi` / `~/.config/rmapi/rmapi.conf`).
pub fn rmapi_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("RMAPI_CONFIG") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let legacy = home.join(".rmapi");
    if legacy.is_file() {
        return legacy;
    }
    home.join(".config").join("rmapi").join("rmapi.conf")
}

pub fn status() -> RemarkableStatus {
    let path = rmapi_config_path();
    let installed = rmapi_available();
    let tokens = load_tokens(&path);
    let authenticated = tokens
        .as_ref()
        .map(|t| !t.device_token.is_empty() && !t.user_token.is_empty())
        .unwrap_or(false);
    let detail = if !installed {
        Some("rmapi not installed on PATH".into())
    } else if !authenticated {
        Some("not authenticated — open the connect URL, then enter the 8-character code".into())
    } else {
        None
    };
    RemarkableStatus {
        rmapi_installed: installed,
        authenticated,
        connect_url: CONNECT_URL,
        config_path: path.display().to_string(),
        detail,
    }
}

/// Exchange a one-time browser code for device+user tokens and write rmapi config.
pub async fn authenticate(state: &AppState, code: &str) -> Result<RemarkableStatus> {
    let code = code.trim().to_ascii_lowercase();
    if code.len() != 8 || !code.chars().all(|c| c.is_ascii_alphanumeric()) {
        bail!("code must be exactly 8 letters/digits from the reMarkable connect page");
    }

    let device_id = Uuid::new_v4().to_string();
    let body = serde_json::json!({
        "code": code,
        "deviceDesc": "desktop-linux",
        "deviceID": device_id,
    });

    let device_resp = state
        .http
        .post(format!("{AUTH_HOST}{DEVICE_NEW}"))
        .header("Authorization", "Bearer")
        .header("User-Agent", "rmapi")
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow!("device token request failed: {e}"))?;
    if !device_resp.status().is_success() {
        let st = device_resp.status();
        let text = device_resp.text().await.unwrap_or_default();
        bail!(
            "device token failed (HTTP {st}): {}",
            text.chars().take(200).collect::<String>()
        );
    }
    let device_token = device_resp.text().await?.trim().to_string();
    if device_token.is_empty() {
        bail!("empty device token from reMarkable");
    }

    let user_resp = state
        .http
        .post(format!("{AUTH_HOST}{USER_NEW}"))
        .header("Authorization", format!("Bearer {device_token}"))
        .header("User-Agent", "rmapi")
        .send()
        .await
        .map_err(|e| anyhow!("user token request failed: {e}"))?;
    if !user_resp.status().is_success() {
        let st = user_resp.status();
        let text = user_resp.text().await.unwrap_or_default();
        bail!(
            "user token failed (HTTP {st}): {}",
            text.chars().take(200).collect::<String>()
        );
    }
    let user_token = user_resp.text().await?.trim().to_string();
    if user_token.is_empty() {
        bail!("empty user token from reMarkable");
    }

    let path = rmapi_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Same YAML shape as rmapi (devicetoken / usertoken).
    let yaml = format!("devicetoken: {device_token}\nusertoken: {user_token}\n");
    std::fs::write(&path, yaml.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }

    let _ = state
        .db
        .set_integration_health("remarkable", "ok", None, true)
        .await;

    Ok(status())
}

pub async fn send_epub(state: &AppState, work_id: Uuid) -> Result<()> {
    let epub = state
        .config
        .library_dir()
        .join(work_id.to_string())
        .join("book.epub");
    if !epub.exists() {
        return Err(anyhow!("no EPUB for work"));
    }

    if !rmapi_available() {
        let _ = state
            .db
            .set_integration_health(
                "remarkable",
                "degraded",
                Some("rmapi not installed"),
                false,
            )
            .await;
        return Err(anyhow!("install rmapi on PATH"));
    }

    if !status().authenticated {
        let _ = state
            .db
            .set_integration_health(
                "remarkable",
                "degraded",
                Some("not authenticated — use Integrations → reMarkable"),
                false,
            )
            .await;
        return Err(anyhow!(
            "reMarkable not authenticated — open Integrations and enter a connect code"
        ));
    }

    let title = state
        .db
        .get_work(work_id)
        .await
        .ok()
        .flatten()
        .map(|w| w.title)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "Untitled".into());
    // Storage is always book.epub; rmapi uses the local basename as the cloud title.
    let upload_name = diarch_core::export_filename(&title, "epub");
    let tmp_dir = std::env::temp_dir().join(format!("diarch-rmapi-{work_id}"));
    std::fs::create_dir_all(&tmp_dir)?;
    let upload_path = tmp_dir.join(&upload_name);
    std::fs::copy(&epub, &upload_path)?;

    // Ensure remote folder exists (ignore failure if already present).
    let _ = rmapi_cmd()
        .args(["mkdir", REMOTE_FOLDER])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;

    let output = rmapi_cmd()
        .arg("put")
        .arg(&upload_path)
        .arg(REMOTE_FOLDER)
        .stdin(Stdio::null())
        .output()
        .await;
    let _ = std::fs::remove_file(&upload_path);
    let _ = std::fs::remove_dir(&tmp_dir);

    let output = output?;
    if output.status.success() {
        let _ = state
            .db
            .set_integration_health("remarkable", "ok", None, true)
            .await;
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = [stderr.trim(), stdout.trim()]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("rmapi put failed")
        .chars()
        .take(400)
        .collect::<String>();

    let _ = state
        .db
        .set_integration_health("remarkable", "broken", Some(&detail), false)
        .await;
    Err(anyhow!(detail))
}

fn rmapi_cmd() -> Command {
    let mut cmd = Command::new("rmapi");
    let path = rmapi_config_path();
    cmd.env("RMAPI_CONFIG", &path);
    cmd
}

/// True when `rmapi` is on PATH (usable for send).
pub fn rmapi_available() -> bool {
    which("rmapi")
}

struct Tokens {
    device_token: String,
    user_token: String,
}

fn load_tokens(path: &Path) -> Option<Tokens> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut device_token = String::new();
    let mut user_token = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("devicetoken:") {
            device_token = rest.trim().trim_matches('"').to_string();
        } else if let Some(rest) = line.strip_prefix("usertoken:") {
            user_token = rest.trim().trim_matches('"').to_string();
        }
    }
    if device_token.is_empty() && user_token.is_empty() {
        None
    } else {
        Some(Tokens {
            device_token,
            user_token,
        })
    }
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|p| Path::new(&p).join(bin).exists())
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_url_is_official() {
        assert!(CONNECT_URL.contains("my.remarkable.com"));
        assert!(CONNECT_URL.contains("connect"));
    }

    #[test]
    fn load_tokens_parses_yamlish() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rmapi.conf");
        std::fs::write(&path, "devicetoken: abc\nusertoken: def\n").unwrap();
        let t = load_tokens(&path).unwrap();
        assert_eq!(t.device_token, "abc");
        assert_eq!(t.user_token, "def");
    }

}
