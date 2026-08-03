use tracing::info;

use crate::state::AppState;

/// Probe integrations and update `integration_health` rows.
pub async fn probe_all(state: &AppState) {
    probe_bin(state, "pandoc", &["--version"]).await;
    probe_bin(state, "ocrmypdf", &["--version"]).await;
    probe_bin(state, "pdftohtml", &["-v"]).await;
    probe_bin(state, "ffmpeg", &["-version"]).await;
    probe_openlibrary(state).await;
    probe_google_books(state).await;
    probe_storygraph(state).await;
    probe_remarkable(state).await;
    probe_localai(state).await;
    probe_audible(state).await;
    info!("integration probe complete");
}

async fn probe_bin(state: &AppState, name: &str, args: &[&str]) {
    let ok = tokio::process::Command::new(name)
        .args(args)
        .output()
        .await
        .map(|o| o.status.success() || !o.stdout.is_empty() || !o.stderr.is_empty())
        .unwrap_or(false);
    if ok {
        let _ = state
            .db
            .set_integration_health(name, "ok", None, true)
            .await;
    } else {
        let hint = match name {
            "ocrmypdf" => "ocrmypdf missing (required for PDF import)",
            "pdftohtml" => "pdftohtml missing (poppler; required for PDF import)",
            "ffmpeg" => "ffmpeg missing (required for AAX → M4B)",
            _ => "missing from PATH",
        };
        let _ = state
            .db
            .set_integration_health(name, "broken", Some(hint), false)
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
        Ok(r) => {
            let _ = state
                .db
                .set_integration_health(
                    "openlibrary",
                    "degraded",
                    Some(&format!("HTTP {}", r.status())),
                    false,
                )
                .await;
        }
        Err(e) => {
            let _ = state
                .db
                .set_integration_health("openlibrary", "degraded", Some(&e.to_string()), false)
                .await;
        }
    }
}

async fn probe_google_books(state: &AppState) {
    let mut url = "https://www.googleapis.com/books/v1/volumes?q=isbn:9780140328721&maxResults=1"
        .to_string();
    if let Some(key) = &state.config.google_books_key {
        url.push_str("&key=");
        url.push_str(key);
    }
    match state.http.get(&url).send().await {
        Ok(r) if r.status().is_success() => {
            let _ = state
                .db
                .set_integration_health("googlebooks", "ok", None, true)
                .await;
        }
        Ok(r) if r.status().as_u16() == 429 => {
            let hint = if state.config.google_books_key.is_some() {
                "rate limited (API key set)"
            } else {
                "rate limited — set DIARCH_GOOGLE_BOOKS_KEY"
            };
            let _ = state
                .db
                .set_integration_health("googlebooks", "degraded", Some(hint), false)
                .await;
        }
        Ok(r) => {
            let _ = state
                .db
                .set_integration_health(
                    "googlebooks",
                    "degraded",
                    Some(&format!("HTTP {}", r.status())),
                    false,
                )
                .await;
        }
        Err(e) => {
            let _ = state
                .db
                .set_integration_health("googlebooks", "degraded", Some(&e.to_string()), false)
                .await;
        }
    }
}

async fn probe_storygraph(state: &AppState) {
    let st = crate::storygraph::status(state);
    if !st.username_set {
        let _ = state
            .db
            .set_integration_health(
                "storygraph",
                "degraded",
                Some("StoryGraph username not set (Integrations or DIARCH_STORYGRAPH_USER)"),
                false,
            )
            .await;
        return;
    }
    // Light probe: pull lists updates health itself.
    let _ = crate::storygraph::pull_lists(state).await;
}

async fn probe_remarkable(state: &AppState) {
    let st = crate::remarkable::status();
    if st.rmapi_installed && st.authenticated {
        let _ = state
            .db
            .set_integration_health("remarkable", "ok", None, true)
            .await;
    } else if st.rmapi_installed {
        let _ = state
            .db
            .set_integration_health(
                "remarkable",
                "degraded",
                Some("rmapi installed; authenticate via Integrations"),
                false,
            )
            .await;
    } else {
        let _ = state
            .db
            .set_integration_health(
                "remarkable",
                "degraded",
                Some(st.detail.as_deref().unwrap_or("rmapi not installed")),
                false,
            )
            .await;
    }
}

async fn probe_localai(state: &AppState) {
    let (status, detail, auto) = crate::localai::probe(state).await;
    let _ = state
        .db
        .set_integration_health("localai", &status, detail.as_deref(), auto)
        .await;
}

async fn probe_audible(state: &AppState) {
    if state.config.audible_key.is_some() {
        let ffmpeg_ok = which("ffmpeg");
        if ffmpeg_ok {
            let _ = state
                .db
                .set_integration_health("audible", "ok", None, true)
                .await;
        } else {
            let _ = state
                .db
                .set_integration_health(
                    "audible",
                    "degraded",
                    Some("DIARCH_AUDIBLE_KEY set but ffmpeg missing"),
                    false,
                )
                .await;
        }
    } else {
        let _ = state
            .db
            .set_integration_health(
                "audible",
                "degraded",
                Some("DIARCH_AUDIBLE_KEY not set"),
                false,
            )
            .await;
    }
}

pub async fn repair(state: &AppState, name: &str) -> Result<(), String> {
    match name {
        "pandoc" => {
            probe_bin(state, "pandoc", &["--version"]).await;
            Ok(())
        }
        "ocrmypdf" => {
            probe_bin(state, "ocrmypdf", &["--version"]).await;
            Ok(())
        }
        "pdftohtml" => {
            probe_bin(state, "pdftohtml", &["-v"]).await;
            Ok(())
        }
        "ffmpeg" => {
            probe_bin(state, "ffmpeg", &["-version"]).await;
            Ok(())
        }
        "openlibrary" | "loc" => {
            probe_openlibrary(state).await;
            Ok(())
        }
        "googlebooks" => {
            probe_google_books(state).await;
            Ok(())
        }
        "storygraph" => {
            crate::storygraph::pull_lists(state)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        "remarkable" => {
            probe_remarkable(state).await;
            let st = crate::remarkable::status();
            if !st.rmapi_installed {
                Err("install rmapi on PATH".into())
            } else if !st.authenticated {
                Err("authenticate via Integrations (reMarkable connect code)".into())
            } else {
                Ok(())
            }
        }
        "localai" => {
            probe_localai(state).await;
            Ok(())
        }
        "audible" => {
            probe_audible(state).await;
            Ok(())
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
