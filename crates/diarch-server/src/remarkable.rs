use anyhow::{anyhow, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;
use uuid::Uuid;

use crate::state::AppState;

const REMOTE_FOLDER: &str = "Diarch";

pub async fn send_epub(state: &AppState, work_id: Uuid) -> Result<()> {
    let epub = state
        .config
        .library_dir()
        .join(work_id.to_string())
        .join("book.epub");
    if !epub.exists() {
        return Err(anyhow!("no EPUB for work"));
    }

    if !which("rmapi") {
        if state.config.remarkable_token.is_none() {
            let _ = state
                .db
                .set_integration_health(
                    "remarkable",
                    "degraded",
                    Some("no rmapi and no DIARCH_REMARKABLE_TOKEN"),
                    false,
                )
                .await;
            return Err(anyhow!(
                "install rmapi on PATH (or set DIARCH_REMARKABLE_TOKEN as a reminder)"
            ));
        }
        let _ = state
            .db
            .set_integration_health(
                "remarkable",
                "degraded",
                Some("token set; install rmapi for uploads"),
                false,
            )
            .await;
        return Err(anyhow!(
            "rmapi not installed — DIARCH_REMARKABLE_TOKEN alone cannot upload"
        ));
    }

    // Ensure remote folder exists (ignore failure if already present).
    let _ = Command::new("rmapi")
        .args(["mkdir", REMOTE_FOLDER])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;

    let output = Command::new("rmapi")
        .arg("put")
        .arg(&epub)
        .arg(REMOTE_FOLDER)
        .stdin(Stdio::null())
        .output()
        .await?;

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

/// True when `rmapi` is on PATH (usable for send).
pub fn rmapi_available() -> bool {
    which("rmapi")
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|p| Path::new(&p).join(bin).exists())
        })
        .unwrap_or(false)
}
