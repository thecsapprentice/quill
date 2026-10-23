//! Quill server library: builds the axum application so the binary and the
//! end-to-end tests share one code path.

pub mod auth;
pub mod backups;
pub mod blobs;
pub mod config;
pub mod error;
pub mod events;
pub mod groups;
pub mod keys;
mod markdown;
pub mod notices;
pub mod prompts;
mod ratelimit;
pub mod reports;
pub mod requests;
pub mod reserved;
pub mod retention;
pub mod routes;
pub mod server_settings;
pub mod sessions;
pub mod storage;
pub mod tag_admin;
mod web;

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
    limiter: Arc<ratelimit::RateLimiter>,
    /// Live notices for `GET /events`.
    pub events: events::Events,
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
        server_settings::init(&pool, &config)
            .await
            .context("storing starting server settings")?;
        reserved::init(&pool, &config)
            .await
            .context("loading reserved handles")?;
        Ok(Self {
            pool,
            config: Arc::new(config),
            dummy_hash: Arc::new(dummy),
            limiter: Arc::default(),
            events: events::Events::default(),
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

/// Grants or revokes administrator rights by login name. Returns false if no
/// such account exists.
pub async fn set_admin(pool: &PgPool, login_name: &str, admin: bool) -> Result<bool> {
    let login = quill_proto::validate::username(login_name).unwrap_or_default();
    let result = sqlx::query("UPDATE accounts SET is_admin = $2 WHERE login_name = $1")
        .bind(login)
        .bind(admin)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() == 1)
}

/// The full HTTP application.
pub fn router(state: AppState) -> Router {
    // Bodies over the envelope limit are rejected before they are buffered.
    let body_limit = usize::try_from(state.config.limits.max_envelope_bytes).unwrap_or(usize::MAX);
    let api = routes::api()
        .fallback(|| async { error::ApiError::NotFound })
        .layer(DefaultBodyLimit::max(body_limit));
    Router::new()
        .nest(quill_proto::API_PREFIX, api)
        .merge(web::router())
        .with_state(state)
}
