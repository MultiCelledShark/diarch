pub mod auth;
pub mod fixer;
pub mod metadata;
pub mod remarkable;
pub mod routes;
pub mod state;
pub mod storygraph;

use anyhow::Result;
use axum::Router;
use diarch_core::Config;
use diarch_db::Db;
use state::AppState;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

/// Build application state (DB, taxonomy seed, admin bootstrap).
pub async fn build_state(config: Config) -> Result<Arc<AppState>> {
    config.ensure_dirs()?;
    let db = Db::connect(&config.db_path()).await?;
    let seed = include_str!("../../../taxonomy/seed.json");
    db.seed_taxonomy(seed).await?;
    db.ensure_admin(&config.admin_username, &config.admin_password)
        .await?;
    for name in ["pandoc", "openlibrary", "loc", "storygraph", "remarkable", "localai"] {
        let _ = db
            .set_integration_health(name, "unknown", None, false)
            .await;
    }
    Ok(Arc::new(AppState {
        db,
        config,
        http: reqwest::Client::new(),
    }))
}

/// HTTP router used by the binary and integration tests.
pub fn app(state: Arc<AppState>) -> Router {
    Router::new()
        .merge(routes::router())
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// Start background job + probe workers (production only).
pub fn spawn_workers(state: Arc<AppState>) {
    {
        let st = state.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = routes::jobs::process_one(&st).await {
                    tracing::warn!(error = %e, "job worker");
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        });
    }
    {
        let st = state.clone();
        tokio::spawn(async move {
            loop {
                fixer::probe_all(&st).await;
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            }
        });
    }
}
