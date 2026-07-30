use tracing::info;

use crate::state::AppState;

/// Probe integrations and attempt simple repairs.
pub async fn probe_all(state: &AppState) {
    probe_pandoc(state).await;
    probe_openlibrary(state).await;
    if state.config.storygraph_username.is_some() {
        let _ = crate::storygraph::pull_lists(state).await;
    }
    if which("rmapi") || state.config.remarkable_token.is_some() {
        let _ = state
            .db
            .set_integration_health("remarkable", "ok", None, true)
            .await;
    } else {
        let _ = state
            .db
            .set_integration_health(
                "remarkable",
                "degraded",
                Some("rmapi not installed"),
                false,
            )
            .await;
    }
    if state.config.localai_url.is_some() {
        let _ = state
            .db
            .set_integration_health("localai", "ok", None, true)
            .await;
    }
    info!("integration probe complete");
}

async fn probe_pandoc(state: &AppState) {
    let ok = tokio::process::Command::new("pandoc")
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok {
        let _ = state
            .db
            .set_integration_health("pandoc", "ok", None, true)
            .await;
    } else {
        let _ = state
            .db
            .set_integration_health("pandoc", "broken", Some("pandoc missing"), false)
            .await;
    }
}

async fn probe_openlibrary(state: &AppState) {
    match state
        .http
        .get("https://openlibrary.org/isbn/9780140328721.json")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            let _ = state
                .db
                .set_integration_health("openlibrary", "ok", None, true)
                .await;
        }
        Ok(_) | Err(_) => {
            let _ = state
                .db
                .set_integration_health("openlibrary", "degraded", Some("probe failed"), false)
                .await;
        }
    }
}

pub async fn repair(state: &AppState, name: &str) -> Result<(), String> {
    match name {
        "pandoc" => {
            probe_pandoc(state).await;
            Ok(())
        }
        "openlibrary" | "loc" => {
            probe_openlibrary(state).await;
            Ok(())
        }
        "storygraph" => {
            crate::storygraph::pull_lists(state)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        "remarkable" => {
            if which("rmapi") {
                let _ = state
                    .db
                    .set_integration_health("remarkable", "ok", None, true)
                    .await;
                Ok(())
            } else {
                Err("install rmapi on PATH".into())
            }
        }
        other => Err(format!("unknown adapter: {other}")),
    }
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .any(|p| std::path::Path::new(&p).join(bin).exists())
        })
        .unwrap_or(false)
}
