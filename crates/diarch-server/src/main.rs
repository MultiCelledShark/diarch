mod auth;
mod fixer;
mod metadata;
mod remarkable;
mod routes;
mod state;
mod storygraph;

use anyhow::Result;
use axum::Router;
use diarch_core::Config;
use diarch_db::Db;
use state::AppState;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("diarch=info".parse()?))
        .init();

    let config = Config::from_env();
    config.ensure_dirs()?;

    let db = Db::connect(&config.db_path()).await?;
    let seed = include_str!("../../../taxonomy/seed.json");
    let n = db.seed_taxonomy(seed).await?;
    tracing::info!(nodes = n, "taxonomy seeded");

    let admin = db
        .ensure_admin(&config.admin_username, &config.admin_password)
        .await?;
    tracing::info!(user = %admin.username, "admin ready");

    for name in ["pandoc", "openlibrary", "loc", "storygraph", "remarkable", "localai"] {
        let _ = db
            .set_integration_health(name, "unknown", None, false)
            .await;
    }

    let state = Arc::new(AppState {
        db,
        config: config.clone(),
        http: reqwest::Client::new(),
    });

    // Background job worker
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

    // Daily-ish integration probe
    {
        let st = state.clone();
        tokio::spawn(async move {
            loop {
                fixer::probe_all(&st).await;
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            }
        });
    }

    let app = Router::new()
        .merge(routes::router())
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr: SocketAddr = config.listen.parse()?;
    tracing::info!(%addr, data = %config.data_dir.display(), "Diarch listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
