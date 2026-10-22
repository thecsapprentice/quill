use anyhow::{Context, Result};
use quill_server::{AppState, Config, connect_and_migrate, router};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // Logs carry no client IPs, request bodies or headers; nothing per request
    // is logged except internal errors.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env()?;
    let bind = config.bind;
    let pool = connect_and_migrate(&config.database_url).await?;
    let state = AppState::new(pool, config).await?;

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(%bind, "quill-server listening");
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("server error")
}
