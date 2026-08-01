use anyhow::Result;
use diarch_core::Config;
use diarch_server::{app, build_state, spawn_workers};
use std::net::SocketAddr;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // Local secrets (DIARCH_AUDIBLE_KEY, etc). Existing process env wins.
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("diarch=info".parse()?))
        .init();

    let config = Config::from_env();
    if config.audible_key.is_some() {
        tracing::info!("Audible AAX convert enabled (DIARCH_AUDIBLE_KEY set)");
    } else {
        tracing::info!("Audible AAX convert disabled (set DIARCH_AUDIBLE_KEY to enable)");
    }
    let state = build_state(config.clone()).await?;
    tracing::info!(user = %config.admin_username, "admin ready");
    spawn_workers(state.clone());

    let router = app(state);
    let addr: SocketAddr = config.listen.parse()?;
    tracing::info!(%addr, data = %config.data_dir.display(), "Diarch listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}
