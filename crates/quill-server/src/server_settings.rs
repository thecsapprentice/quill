//! Settings administrators change while the server runs, stored in the
//! database. Environment variables only supply their starting values.

use axum::{Json, Router, extract::State, routing::get};
use quill_proto::board::{AdminAction, ServerSettings, TagPolicy, UpdateServerSettings};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    AppState, auth::Admin, config::Config, error::ApiError, prompts::parse_enum,
    reports::log_action, routes::ApiJson,
};

/// Whether writers may create new tags.
const NEW_TAGS: &str = "new_tags";
/// Whether sessions take attachments.
const ATTACHMENTS: &str = "attachments";

pub fn router() -> Router<AppState> {
    Router::new().route("/admin/settings", get(api_get).patch(api_update))
}

/// Stores starting values for settings that have never been set.
pub(crate) async fn init(pool: &PgPool, config: &Config) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO server_settings (key, value) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(NEW_TAGS)
        .bind(config.tag_policy.as_str())
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO server_settings (key, value) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(ATTACHMENTS)
        .bind(if config.attachments { "on" } else { "off" })
        .execute(pool)
        .await?;
    Ok(())
}

/// The current "new tags" setting.
pub async fn tag_policy(conn: &mut PgConnection) -> Result<TagPolicy, ApiError> {
    let raw: Option<String> =
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = $1")
            .bind(NEW_TAGS)
            .fetch_optional(&mut *conn)
            .await?;
    // Absent only if `init` never ran; refusing new tags is the safe reading.
    raw.map_or(Ok(TagPolicy::Curated), |r| parse_enum(&r))
}

/// Whether sessions take attachments.
pub async fn attachments(conn: &mut PgConnection) -> Result<bool, ApiError> {
    let raw: Option<String> =
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = $1")
            .bind(ATTACHMENTS)
            .fetch_optional(&mut *conn)
            .await?;
    Ok(raw.as_deref() != Some("off"))
}

pub async fn settings(state: &AppState) -> Result<ServerSettings, ApiError> {
    let mut conn = state.pool.acquire().await?;
    Ok(ServerSettings {
        new_tags: tag_policy(&mut conn).await?,
        attachments: attachments(&mut conn).await?,
    })
}

fn policy_label(p: TagPolicy) -> &'static str {
    match p {
        TagPolicy::Open => "allowed",
        TagPolicy::Approval => "need approval",
        TagPolicy::Curated => "not allowed",
    }
}

/// Changes settings, logging each change.
pub async fn update(
    state: &AppState,
    admin: Uuid,
    body: &UpdateServerSettings,
) -> Result<ServerSettings, ApiError> {
    let mut tx = state.pool.begin().await?;
    // Serializes concurrent changes so the log reads in order.
    sqlx::query("SELECT 1 FROM server_settings WHERE key = $1 FOR UPDATE")
        .bind(NEW_TAGS)
        .execute(&mut *tx)
        .await?;
    let before = tag_policy(&mut tx).await?;
    if let Some(new_tags) = body.new_tags.filter(|p| *p != before) {
        sqlx::query(
            "INSERT INTO server_settings (key, value) VALUES ($1, $2)
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(NEW_TAGS)
        .bind(new_tags.as_str())
        .execute(&mut *tx)
        .await?;
        let detail = format!(
            "new tags from writers: {} → {}",
            policy_label(before),
            policy_label(new_tags)
        );
        log_action(
            &mut tx,
            admin,
            AdminAction::ChangeSetting,
            NEW_TAGS,
            Some(&detail),
        )
        .await?;
    }
    let had = attachments(&mut tx).await?;
    if let Some(on) = body.attachments.filter(|on| *on != had) {
        sqlx::query(
            "INSERT INTO server_settings (key, value) VALUES ($1, $2)
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(ATTACHMENTS)
        .bind(if on { "on" } else { "off" })
        .execute(&mut *tx)
        .await?;
        let label = |on: bool| if on { "allowed" } else { "not allowed" };
        let detail = format!("attachments in sessions: {} → {}", label(had), label(on));
        log_action(
            &mut tx,
            admin,
            AdminAction::ChangeSetting,
            ATTACHMENTS,
            Some(&detail),
        )
        .await?;
    }
    tx.commit().await?;
    settings(state).await
}

async fn api_get(
    State(state): State<AppState>,
    _admin: Admin,
) -> Result<Json<ServerSettings>, ApiError> {
    settings(&state).await.map(Json)
}

async fn api_update(
    State(state): State<AppState>,
    admin: Admin,
    ApiJson(body): ApiJson<UpdateServerSettings>,
) -> Result<Json<ServerSettings>, ApiError> {
    update(&state, admin.0.account_id, &body).await.map(Json)
}
