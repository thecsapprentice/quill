//! Account notices: administrators' actions against an account's prompts and
//! tags, recorded for that account alone.
//!
//! Recorded in the same transaction as the action and its admin-log entry,
//! so a notice exists exactly when the action happened. Notices never name
//! a reporter or an administrator.

use std::collections::BTreeMap;
use utoipa_axum::{router::OpenApiRouter, routes};

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use quill_proto::notices::{AccountNotice, NoticeFields, NoticeKind, NoticePrompt};
use serde::Deserialize;
use sqlx::{PgConnection, types::Json as DbJson};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{AppState, auth::Authed, error::ApiError, prompts::parse_enum};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(api_list))
        .routes(routes!(api_mark_read))
}

/// An account and its prompts that an action touched.
#[derive(Debug, Default)]
pub(crate) struct Affected {
    pub account: Uuid,
    pub prompts: Vec<NoticePrompt>,
}

/// Groups `(account, prompt)` rows by account, keeping order.
fn group(rows: Vec<(Uuid, Uuid, String, String)>) -> Vec<Affected> {
    let mut by_account: BTreeMap<Uuid, Affected> = BTreeMap::new();
    for (account, id, title, handle) in rows {
        by_account
            .entry(account)
            .or_insert_with(|| Affected {
                account,
                prompts: Vec::new(),
            })
            .prompts
            .push(NoticePrompt { id, title, handle });
    }
    by_account.into_values().collect()
}

/// Accounts whose prompts carry tag `tag_id`, with those prompts.
pub(crate) async fn affected_by_tag(
    conn: &mut PgConnection,
    tag_id: i64,
) -> Result<Vec<Affected>, ApiError> {
    let rows: Vec<(Uuid, Uuid, String, String)> = sqlx::query_as(
        "SELECT a.account_id, p.id, p.title, a.preferred_username
         FROM prompts p JOIN actors a ON a.id = p.actor_id
         WHERE p.tag_ids @> ARRAY[$1::bigint]
         ORDER BY p.created_at, p.id",
    )
    .bind(tag_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(group(rows))
}

/// The account behind one prompt.
pub(crate) async fn affected_by_prompt(
    conn: &mut PgConnection,
    prompt: Uuid,
) -> Result<Vec<Affected>, ApiError> {
    let rows: Vec<(Uuid, Uuid, String, String)> = sqlx::query_as(
        "SELECT a.account_id, p.id, p.title, a.preferred_username
         FROM prompts p JOIN actors a ON a.id = p.actor_id WHERE p.id = $1",
    )
    .bind(prompt)
    .fetch_all(&mut *conn)
    .await?;
    Ok(group(rows))
}

/// The account behind one comment. Notices about comments list no prompts.
pub(crate) async fn affected_by_comment(
    conn: &mut PgConnection,
    comment: Uuid,
) -> Result<Vec<Affected>, ApiError> {
    let account: Option<Uuid> = sqlx::query_scalar(
        "SELECT a.account_id FROM announcement_comments c
         JOIN actors a ON a.id = c.actor_id WHERE c.id = $1",
    )
    .bind(comment)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(account
        .map(|account| Affected {
            account,
            prompts: Vec::new(),
        })
        .into_iter()
        .collect())
}

/// Records one notice per affected account for admin-log entry `log_id`.
/// Inserted in batches, since one action can reach thousands of accounts.
pub(crate) async fn record(
    conn: &mut PgConnection,
    log_id: i64,
    kind: NoticeKind,
    fields: &NoticeFields,
    affected: &[Affected],
) -> Result<(), ApiError> {
    const BATCH: usize = 1000;
    crate::reports::log_targets(conn, log_id, affected).await?;
    for chunk in affected.chunks(BATCH) {
        let accounts: Vec<Uuid> = chunk.iter().map(|a| a.account).collect();
        let prompts: Vec<DbJson<&Vec<NoticePrompt>>> =
            chunk.iter().map(|a| DbJson(&a.prompts)).collect();
        sqlx::query(
            "INSERT INTO account_notices (account_id, log_id, kind, fields, prompts)
             SELECT a, $3, $4, $5, p FROM unnest($1::uuid[], $2::jsonb[]) AS u(a, p)",
        )
        .bind(&accounts)
        .bind(&prompts)
        .bind(log_id)
        .bind(kind.as_str())
        .bind(DbJson(fields))
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

pub async fn unread_count(state: &AppState, account: Uuid) -> Result<u64, ApiError> {
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM account_notices WHERE account_id = $1 AND read_at IS NULL",
    )
    .bind(account)
    .fetch_one(&state.pool)
    .await?;
    Ok(u64::try_from(n).unwrap_or(0))
}

type NoticeRow = (
    i64,
    OffsetDateTime,
    String,
    DbJson<NoticeFields>,
    DbJson<Vec<NoticePrompt>>,
    bool,
);

/// The account's notices, newest first, optionally older than `before`.
pub async fn list(
    state: &AppState,
    account: Uuid,
    before: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<AccountNotice>, ApiError> {
    let limit = i64::from(limit.unwrap_or(50).clamp(1, 200));
    let rows: Vec<NoticeRow> = sqlx::query_as(
        "SELECT id, created_at, kind, fields, prompts, read_at IS NOT NULL
         FROM account_notices
         WHERE account_id = $1 AND ($2::bigint IS NULL OR id < $2)
         ORDER BY id DESC LIMIT $3",
    )
    .bind(account)
    .bind(before)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    rows.into_iter()
        .map(|(id, at, kind, fields, prompts, read)| {
            Ok(AccountNotice {
                id,
                at,
                kind: parse_enum(&kind)?,
                fields: fields.0,
                prompts: prompts.0,
                read,
            })
        })
        .collect()
}

/// Marks all the account's notices read.
pub async fn mark_read(state: &AppState, account: Uuid) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE account_notices SET read_at = now() WHERE account_id = $1 AND read_at IS NULL",
    )
    .bind(account)
    .execute(&state.pool)
    .await?;
    Ok(())
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct ListQuery {
    /// Notices older than this id.
    before: Option<i64>,
    limit: Option<u32>,
}

#[utoipa::path(
    get, path = "/account/notices", tag = "account", security(("bearer" = [])),
    summary = "Notices to the login about its prompts, newest first",
    description = "What administrators did to the login's prompts, and why; never who reported them. Pages by `before`, the last id of the previous page.",
    params(ListQuery),
    responses((status = 200, body = Vec<AccountNotice>)),
)]
async fn api_list(
    State(state): State<AppState>,
    authed: Authed,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<AccountNotice>>, ApiError> {
    list(&state, authed.account_id, q.before, q.limit)
        .await
        .map(Json)
}

#[utoipa::path(
    post, path = "/account/notices/read", tag = "account", security(("bearer" = [])),
    summary = "Mark every notice read",
    responses((status = 204, description = "Marked")),
)]
async fn api_mark_read(
    State(state): State<AppState>,
    authed: Authed,
) -> Result<StatusCode, ApiError> {
    mark_read(&state, authed.account_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
