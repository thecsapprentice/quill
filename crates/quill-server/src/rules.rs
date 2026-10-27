//! The server rules (#42): a Markdown page the administrators keep, and
//! whether accounts have accepted it.
//!
//! Every save gets the next version. A save that isn't a minor edit also
//! becomes the version accounts must accept (`accept_version`), from that
//! moment (`accept_since`). The acceptance setting decides when that's
//! enforced: on joining, after a change, both or never. See `docs/plan.md`,
//! "Server rules", for the reasoning behind each rule.

use std::sync::atomic::Ordering;

use axum::{Json, extract::State, http::StatusCode};
use quill_proto::{
    board::AdminAction,
    rules::{AcceptRules, Rules, UpdateRules},
    validate,
};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    AppState,
    auth::{Admin, Authed, MaybeAuthed},
    error::ApiError,
    markdown,
    reports::log_action,
    routes::ApiJson,
    server_settings::{self, RULES_ACCEPTANCE},
};

/// The `server_pages` row holding the rules.
const PAGE: &str = "rules";

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(api_get))
        .routes(routes!(api_accept))
        .routes(routes!(api_update))
}

/// SQL that is true when `account` (a column or `$n`) must accept the rules
/// before writing: the setting asks for acceptance after a change, the
/// account existed before the version to accept, and hasn't accepted it.
/// Accounts that joined later joined under that version.
pub(crate) fn must_accept_sql(account: &str) -> String {
    format!(
        "EXISTS (
            SELECT 1 FROM server_pages p JOIN accounts acc ON acc.id = {account}
            WHERE p.name = '{PAGE}'
              AND (SELECT value FROM server_settings WHERE key = '{RULES_ACCEPTANCE}')
                  IN ('on-change', 'both')
              AND acc.created_at < p.accept_since
              AND NOT EXISTS (
                  SELECT 1 FROM rules_acceptances r
                  WHERE r.account_id = acc.id AND r.version >= p.accept_version))"
    )
}

#[derive(sqlx::FromRow)]
struct Row {
    body_md: String,
    body_html: String,
    version: i32,
    updated_at: OffsetDateTime,
}

fn version(v: i32) -> u32 {
    u32::try_from(v).unwrap_or(0)
}

/// Whether `account` must accept the rules before posting, editing,
/// commenting or reporting.
pub async fn must_accept(conn: &mut PgConnection, account: Uuid) -> Result<bool, ApiError> {
    // Safe: built only from compile-time constants; the account is bound.
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT {}",
        must_accept_sql("$1")
    )))
    .bind(account)
    .fetch_one(&mut *conn)
    .await?)
}

/// Refuses with `rules-not-accepted` while `account` must accept the rules.
pub async fn require_accepted(state: &AppState, account: Uuid) -> Result<(), ApiError> {
    let mut conn = state.pool.acquire().await?;
    if must_accept(&mut conn, account).await? {
        return Err(ApiError::RulesNotAccepted(
            "The server rules have changed. Read and accept them to continue.".into(),
        ));
    }
    Ok(())
}

/// The rules, or `None` before an administrator first saves them.
/// `viewer`, when signed in, fills in `must_accept`.
pub async fn get(state: &AppState, viewer: Option<Uuid>) -> Result<Option<Rules>, ApiError> {
    let mut conn = state.pool.acquire().await?;
    let row: Option<Row> = sqlx::query_as(
        "SELECT body_md, body_html, version, updated_at FROM server_pages WHERE name = $1",
    )
    .bind(PAGE)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    // Another process may have published them first.
    state.rules_published.store(true, Ordering::Relaxed);
    let must = match viewer {
        Some(account) => must_accept(&mut conn, account).await?,
        None => false,
    };
    Ok(Some(Rules {
        body: row.body_md,
        body_html: row.body_html,
        version: version(row.version),
        updated_at: row.updated_at,
        must_accept: must,
    }))
}

/// When the rules last changed, for `InstanceInfo`.
pub async fn updated_at(state: &AppState) -> Result<Option<OffsetDateTime>, ApiError> {
    Ok(
        sqlx::query_scalar("SELECT updated_at FROM server_pages WHERE name = $1")
            .bind(PAGE)
            .fetch_optional(&state.pool)
            .await?,
    )
}

/// Whether any rules have been saved, read once at startup.
pub(crate) async fn published(pool: &sqlx::PgPool) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM server_pages WHERE name = $1)")
        .bind(PAGE)
        .fetch_one(pool)
        .await
}

/// The current version, if there are rules.
async fn current_version(conn: &mut PgConnection) -> Result<Option<u32>, ApiError> {
    let v: Option<i32> = sqlx::query_scalar("SELECT version FROM server_pages WHERE name = $1")
        .bind(PAGE)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(v.map(version))
}

/// Saves new rules as the next version, logging the change with a hash of
/// the text. The administrator who saved them has accepted them.
pub async fn save(state: &AppState, admin: Uuid, body: &UpdateRules) -> Result<Rules, ApiError> {
    let text = validate::rules_body(&body.body)?;
    let html = markdown::render(&text);
    let mut tx = state.pool.begin().await?;
    // Serializes saves so versions go up one at a time.
    let before: Option<i32> =
        sqlx::query_scalar("SELECT version FROM server_pages WHERE name = $1 FOR UPDATE")
            .bind(PAGE)
            .fetch_optional(&mut *tx)
            .await?;
    // The first version always counts as a change to accept.
    let minor = body.minor && before.is_some();
    let next = before.unwrap_or(0) + 1;
    sqlx::query(
        "INSERT INTO server_pages
             (name, body_md, body_html, version, accept_version, accept_since, updated_by)
         VALUES ($1, $2, $3, $4, $4, now(), $5)
         ON CONFLICT (name) DO UPDATE SET
             body_md = EXCLUDED.body_md,
             body_html = EXCLUDED.body_html,
             version = EXCLUDED.version,
             accept_version = CASE WHEN $6 THEN server_pages.accept_version
                                   ELSE EXCLUDED.version END,
             accept_since = CASE WHEN $6 THEN server_pages.accept_since ELSE now() END,
             updated_at = now(),
             updated_by = EXCLUDED.updated_by",
    )
    .bind(PAGE)
    .bind(&text)
    .bind(&html)
    .bind(next)
    .bind(admin)
    .bind(minor)
    .execute(&mut *tx)
    .await?;
    record_acceptance(&mut tx, admin, version(next)).await?;
    let hash: String = Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let kind = if minor {
        "minor edit"
    } else {
        "accounts accept it again where the setting asks"
    };
    let detail = format!("version {next} ({kind}); SHA-256 {hash}");
    log_action(
        &mut tx,
        admin,
        AdminAction::UpdateRules,
        PAGE,
        Some(&detail),
    )
    .await?;
    tx.commit().await?;
    state.rules_published.store(true, Ordering::Relaxed);
    get(state, Some(admin)).await?.ok_or(ApiError::NotFound)
}

async fn record_acceptance(
    conn: &mut PgConnection,
    account: Uuid,
    version: u32,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO rules_acceptances (account_id, version) VALUES ($1, $2)
         ON CONFLICT (account_id) DO UPDATE
         SET version = GREATEST(rules_acceptances.version, EXCLUDED.version),
             accepted_at = now()",
    )
    .bind(account)
    .bind(i32::try_from(version).unwrap_or(i32::MAX))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Records that `account` accepted `version`, which must be the current one:
/// accepting rules the person didn't see would mean nothing.
pub async fn accept(state: &AppState, account: Uuid, version: u32) -> Result<(), ApiError> {
    let mut conn = state.pool.acquire().await?;
    match current_version(&mut conn).await? {
        None => Err(ApiError::NotFound),
        Some(v) if v != version => Err(ApiError::Conflict(
            "The rules have changed since you read them. Read them again.".into(),
        )),
        Some(v) => record_acceptance(&mut conn, account, v).await,
    }
}

/// Checks a sign-up against the acceptance setting, before the account
/// exists. Returns the version to record once it does.
pub async fn check_on_join(
    state: &AppState,
    accepted: Option<u32>,
) -> Result<Option<u32>, ApiError> {
    let mut conn = state.pool.acquire().await?;
    let Some(current) = current_version(&mut conn).await? else {
        return Ok(None);
    };
    let required = server_settings::rules_acceptance(&mut conn)
        .await?
        .on_join();
    match accepted {
        Some(v) if v == current => Ok(Some(v)),
        Some(_) if required => Err(ApiError::RulesNotAccepted(
            "The server rules changed while you were signing up. Read them again.".into(),
        )),
        None if required => Err(ApiError::RulesNotAccepted(
            "Accept the server rules to create an account.".into(),
        )),
        // Not required: an outdated acceptance is simply not recorded.
        _ => Ok(None),
    }
}

/// Records the acceptance [`check_on_join`] allowed, for a new account.
pub async fn record_on_join(state: &AppState, account: Uuid, version: u32) -> Result<(), ApiError> {
    let mut conn = state.pool.acquire().await?;
    record_acceptance(&mut conn, account, version).await
}

// ---- API ---------------------------------------------------------------------

#[utoipa::path(
    get, path = "/rules", tag = "rules", security(()),
    summary = "The server rules",
    description = "Signed-in callers also learn whether they must accept this version before posting, editing, commenting or reporting.",
    responses(
        (status = 200, body = Rules),
        (status = 404, description = "The administrators haven't written any rules"),
    ),
)]
async fn api_get(
    State(state): State<AppState>,
    MaybeAuthed(authed): MaybeAuthed,
) -> Result<Json<Rules>, ApiError> {
    get(&state, authed.map(|a| a.account_id))
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    post, path = "/rules/accept", tag = "rules", security(("bearer" = [])),
    summary = "Accept the server rules",
    request_body = AcceptRules,
    responses(
        (status = 204, description = "Accepted"),
        (status = 404, description = "There are no rules"),
        (status = 409, description = "The rules changed since that version"),
    ),
)]
async fn api_accept(
    State(state): State<AppState>,
    authed: Authed,
    ApiJson(body): ApiJson<AcceptRules>,
) -> Result<StatusCode, ApiError> {
    accept(&state, authed.account_id, body.version).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    put, path = "/admin/rules", tag = "admin", security(("bearer" = [])),
    summary = "Write the server rules",
    description = "Saves the next version. Unless `minor` is set, accounts accept it again where the rules acceptance setting asks for that.",
    request_body = UpdateRules,
    responses((status = 200, body = Rules)),
)]
async fn api_update(
    State(state): State<AppState>,
    admin: Admin,
    ApiJson(body): ApiJson<UpdateRules>,
) -> Result<Json<Rules>, ApiError> {
    save(&state, admin.0.account_id, &body).await.map(Json)
}
