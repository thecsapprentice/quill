//! Quill server library: builds the axum application so the binary and the
//! end-to-end tests share one code path.

pub mod auth;
pub mod config;
pub mod error;
mod routes;

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{Router, extract::DefaultBodyLimit};
use sqlx::{PgPool, postgres::PgPoolOptions};

pub use config::Config;

#[derive(Debug, Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    /// Hash of a random password, verified against when a login names an
    /// unknown account so response timing doesn't reveal which accounts exist.
    dummy_hash: Arc<String>,
}

impl AppState {
    pub async fn new(pool: PgPool, config: Config) -> Result<Self> {
        let mut seed = [0u8; 24];
        getrandom::fill(&mut seed).map_err(|e| anyhow::anyhow!("OS RNG failed: {e}"))?;
        let dummy = auth::hash_password(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            seed,
        ))
        .await
        .map_err(|e| anyhow::anyhow!("dummy hash: {e}"))?;
        Ok(Self {
            pool,
            config: Arc::new(config),
            dummy_hash: Arc::new(dummy),
        })
    }
}

/// Connects to Postgres and applies pending migrations.
pub async fn connect_and_migrate(database_url: &str) -> Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(16)
        .connect(database_url)
        .await
        .context("connecting to Postgres")?;
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("running migrations")?;
    Ok(pool)
}

/// The full HTTP application.
pub fn router(state: AppState) -> Router {
    // Bodies over the envelope limit are rejected before they are buffered.
    let body_limit = usize::try_from(state.config.limits.max_envelope_bytes).unwrap_or(usize::MAX);
    Router::new()
        .nest(quill_proto::API_PREFIX, routes::api())
        .layer(DefaultBodyLimit::max(body_limit))
        .with_state(state)
}
