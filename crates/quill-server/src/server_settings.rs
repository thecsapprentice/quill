//! Settings administrators change while the server runs, stored in the
//! database. Environment variables only supply their starting values.

use axum::{Json, extract::State};
use quill_proto::{
    announcements::CommentVisibility,
    board::{AdminAction, ServerSettings, TagPolicy, UpdateServerSettings},
};
use sqlx::{PgConnection, PgPool};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    AppState, auth::Admin, config::Config, error::ApiError, prompts::parse_enum,
    reports::log_action, routes::ApiJson,
};

/// Whether writers may create new tags.
const NEW_TAGS: &str = "new_tags";
/// Whether sessions take attachments.
const ATTACHMENTS: &str = "attachments";
/// Who can read comments on announcements. Not seeded: absent is public.
const COMMENT_VISIBILITY: &str = "comment_visibility";

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(api_get, api_update))
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

/// Who can read comments on announcements.
pub async fn comment_visibility(conn: &mut PgConnection) -> Result<CommentVisibility, ApiError> {
    let raw: Option<String> =
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = $1")
            .bind(COMMENT_VISIBILITY)
            .fetch_optional(&mut *conn)
            .await?;
    // Never set: public, as comments were before the setting existed.
    raw.map_or(Ok(CommentVisibility::Public), |r| parse_enum(&r))
}

pub async fn settings(state: &AppState) -> Result<ServerSettings, ApiError> {
    let mut conn = state.pool.acquire().await?;
    Ok(ServerSettings {
        new_tags: tag_policy(&mut conn).await?,
        attachments: attachments(&mut conn).await?,
        comment_visibility: comment_visibility(&mut conn).await?,
    })
}

fn visibility_label(v: CommentVisibility) -> &'static str {
    match v {
        CommentVisibility::Public => "everyone",
        CommentVisibility::SignedIn => "signed-in writers only",
    }
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
    let was = comment_visibility(&mut tx).await?;
    if let Some(now) = body.comment_visibility.filter(|v| *v != was) {
        sqlx::query(
            "INSERT INTO server_settings (key, value) VALUES ($1, $2)
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(COMMENT_VISIBILITY)
        .bind(now.as_str())
        .execute(&mut *tx)
        .await?;
        let detail = format!(
            "comments readable by: {} → {}",
            visibility_label(was),
            visibility_label(now)
        );
        log_action(
            &mut tx,
            admin,
            AdminAction::ChangeSetting,
            COMMENT_VISIBILITY,
            Some(&detail),
        )
        .await?;
    }
    tx.commit().await?;
    settings(state).await
}

#[utoipa::path(
    get, path = "/admin/settings", tag = "admin", security(("bearer" = [])),
    summary = "The server's settings",
    responses((status = 200, body = ServerSettings)),
)]
async fn api_get(
    State(state): State<AppState>,
    _admin: Admin,
) -> Result<Json<ServerSettings>, ApiError> {
    settings(&state).await.map(Json)
}

#[utoipa::path(
    patch, path = "/admin/settings", tag = "admin", security(("bearer" = [])),
    summary = "Change the server's settings",
    request_body = UpdateServerSettings,
    responses((status = 200, body = ServerSettings)),
)]
async fn api_update(
    State(state): State<AppState>,
    admin: Admin,
    ApiJson(body): ApiJson<UpdateServerSettings>,
) -> Result<Json<ServerSettings>, ApiError> {
    update(&state, admin.0.account_id, &body).await.map(Json)
}
