use anyhow::{anyhow, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;
use uuid::Uuid;

use crate::state::AppState;

pub async fn send_epub(state: &AppState, work_id: Uuid) -> Result<()> {
    let epub = state
        .config
        .library_dir()
        .join(work_id.to_string())
        .join("book.epub");
    if !epub.exists() {
        return Err(anyhow!("no EPUB for work"));
    }

    // Prefer rmapi CLI when available
    if which("rmapi") {
        let status = Command::new("rmapi")
            .arg("put")
            .arg(&epub)
            .stdin(Stdio::null())
            .status()
            .await?;
        if status.success() {
            let _ = state
                .db
                .set_integration_health("remarkable", "ok", None, true)
                .await;
            return Ok(());
        }
        let _ = state
            .db
            .set_integration_health("remarkable", "broken", Some("rmapi put failed"), false)
            .await;
        return Err(anyhow!("rmapi put failed"));
    }

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
            "install rmapi or set DIARCH_REMARKABLE_TOKEN"
        ));
    }

    // Token present but no HTTP upload implemented without reverse-engineered client;
    // mark for fixer / manual.
    let _ = state
        .db
        .set_integration_health(
            "remarkable",
            "degraded",
            Some("token set; install rmapi for uploads"),
            false,
        )
        .await;
    Err(anyhow!("rmapi not installed"))
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|p| Path::new(&p).join(bin).exists())
        })
        .unwrap_or(false)
}
