//! Storage use (2i): what a login's sessions take on the server, for the
//! login itself and for administrators. Only sizes and counts of
//! ciphertext: nothing here says what was written. The administrators'
//! per-login view leaves out persona handles, so it doesn't link personas
//! to logins.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use quill_proto::{
    keys::b64,
    sessions::{AccountStorage, AccountStorageSummary, ServerStorage, SessionStorage},
};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{
    AppState,
    auth::{Admin, Authed},
    error::ApiError,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/account/storage", get(own))
        .route("/admin/storage", get(server))
        .route("/admin/storage/{account}", get(one))
}

fn n(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

/// `(group, ended, persona, partner, messages, message bytes, files, file
/// bytes, own file bytes)`.
type Row = (
    Vec<u8>,
    bool,
    Option<String>,
    Option<String>,
    i64,
    i64,
    i64,
    i64,
    i64,
);

/// A login's storage. `with_personas` adds persona handles (the login's own
/// view only).
pub(crate) async fn account_storage(
    conn: &mut PgConnection,
    account: Uuid,
    quota: u64,
    with_personas: bool,
) -> Result<AccountStorage, ApiError> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT s.group_id, s.ended_at IS NOT NULL,
                (SELECT a.preferred_username FROM session_members m JOIN actors a ON a.id = m.actor_id
                 WHERE m.group_id = s.group_id AND a.account_id = $1 LIMIT 1),
                (SELECT a.preferred_username FROM session_members m JOIN actors a ON a.id = m.actor_id
                 WHERE m.group_id = s.group_id AND a.account_id <> $1 LIMIT 1),
                (SELECT count(*) FROM group_messages g WHERE g.group_id = s.group_id),
                (SELECT coalesce(sum(octet_length(g.data)), 0)::bigint FROM group_messages g
                 WHERE g.group_id = s.group_id),
                (SELECT count(*) FROM blobs b WHERE b.group_id = s.group_id),
                (SELECT coalesce(sum(b.size), 0)::bigint FROM blobs b WHERE b.group_id = s.group_id),
                (SELECT coalesce(sum(b.size), 0)::bigint FROM blobs b
                 WHERE b.group_id = s.group_id AND b.account_id = $1)
         FROM sessions s
         WHERE EXISTS (SELECT 1 FROM session_members m JOIN actors a ON a.id = m.actor_id
                       WHERE m.group_id = s.group_id AND a.account_id = $1)
            OR EXISTS (SELECT 1 FROM blobs b WHERE b.group_id = s.group_id AND b.account_id = $1)
         ORDER BY s.group_id",
    )
    .bind(account)
    .fetch_all(&mut *conn)
    .await?;
    // The device group's messages and blobs (history bundles, 2k).
    let device_sync: i64 = sqlx::query_scalar(
        "SELECT ((SELECT coalesce(sum(octet_length(m.data)), 0)
                 FROM group_messages m JOIN mls_groups g ON g.group_id = m.group_id
                 WHERE g.kind = 'personal' AND g.account_id = $1)
              + (SELECT coalesce(sum(b.size), 0)
                 FROM blobs b JOIN mls_groups g ON g.group_id = b.group_id
                 WHERE g.kind = 'personal' AND g.account_id = $1))::bigint",
    )
    .bind(account)
    .fetch_one(&mut *conn)
    .await?;
    let backup: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(octet_length(data)), 0)::bigint FROM backup_parts WHERE account_id = $1",
    )
    .bind(account)
    .fetch_one(&mut *conn)
    .await?;
    let quota_used: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(size), 0)::bigint FROM blobs WHERE account_id = $1",
    )
    .bind(account)
    .fetch_one(&mut *conn)
    .await?;
    let sessions: Vec<SessionStorage> = rows
        .into_iter()
        .map(
            |(group, ended, persona, partner, messages, message_bytes, files, file_bytes, own)| {
                SessionStorage {
                    id: b64(&group),
                    persona: persona.filter(|_| with_personas),
                    partner: partner.filter(|_| with_personas),
                    ended,
                    messages: n(messages),
                    message_bytes: n(message_bytes),
                    files: n(files),
                    file_bytes: n(file_bytes),
                    own_file_bytes: n(own),
                }
            },
        )
        .collect();
    let total = sessions
        .iter()
        .map(|s| s.message_bytes + s.file_bytes)
        .sum::<u64>()
        + n(device_sync)
        + n(backup);
    Ok(AccountStorage {
        total_bytes: total,
        quota_used_bytes: n(quota_used),
        quota_bytes: quota,
        device_sync_bytes: n(device_sync),
        backup_bytes: n(backup),
        sessions,
    })
}

/// `GET /account/storage`: the caller's own.
async fn own(
    State(state): State<AppState>,
    authed: Authed,
) -> Result<Json<AccountStorage>, ApiError> {
    let mut conn = state.pool.acquire().await?;
    account_storage(
        &mut conn,
        authed.account_id,
        state.config.limits.account_quota_bytes,
        true,
    )
    .await
    .map(Json)
}

/// `GET /admin/storage/{account}`: one login, without persona handles.
async fn one(
    State(state): State<AppState>,
    _admin: Admin,
    Path(account): Path<Uuid>,
) -> Result<Json<AccountStorage>, ApiError> {
    let mut conn = state.pool.acquire().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM accounts WHERE id = $1)")
        .bind(account)
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Err(ApiError::NotFound);
    }
    account_storage(
        &mut conn,
        account,
        state.config.limits.account_quota_bytes,
        false,
    )
    .await
    .map(Json)
}

/// `(account, login, sessions, message bytes, file bytes, device sync bytes)`.
type SummaryRow = (Uuid, String, i64, i64, i64, i64);

/// The whole server's storage, by login.
pub(crate) async fn server_storage(
    conn: &mut PgConnection,
    quota: u64,
) -> Result<ServerStorage, ApiError> {
    let rows: Vec<SummaryRow> = sqlx::query_as(
        "WITH mine AS (
             SELECT DISTINCT a.account_id, m.group_id
             FROM session_members m JOIN actors a ON a.id = m.actor_id
         ), sizes AS (
             SELECT group_id, sum(octet_length(data))::bigint AS bytes
             FROM group_messages GROUP BY group_id
         )
         SELECT acc.id, acc.login_name,
                (SELECT count(*) FROM mine WHERE mine.account_id = acc.id),
                coalesce((SELECT sum(sizes.bytes) FROM mine JOIN sizes USING (group_id)
                          JOIN mls_groups g USING (group_id)
                          WHERE mine.account_id = acc.id AND g.kind = 'session'), 0)::bigint,
                coalesce((SELECT sum(size) FROM blobs WHERE blobs.account_id = acc.id), 0)::bigint,
                coalesce((SELECT sizes.bytes FROM mls_groups g JOIN sizes USING (group_id)
                          WHERE g.kind = 'personal' AND g.account_id = acc.id), 0)::bigint
         FROM accounts acc",
    )
    .fetch_all(&mut *conn)
    .await?;
    let (messages, files): (i64, i64) = sqlx::query_as(
        "SELECT coalesce((SELECT sum(octet_length(data)) FROM group_messages), 0)::bigint,
                coalesce((SELECT sum(size) FROM blobs), 0)::bigint",
    )
    .fetch_one(&mut *conn)
    .await?;
    let mut accounts: Vec<AccountStorageSummary> = rows
        .into_iter()
        .filter(|r| r.3 + r.4 + r.5 > 0 || r.2 > 0)
        .map(
            |(account, login, sessions, message_bytes, file_bytes, device_sync)| {
                AccountStorageSummary {
                    account,
                    login,
                    sessions: n(sessions),
                    message_bytes: n(message_bytes),
                    file_bytes: n(file_bytes),
                    device_sync_bytes: n(device_sync),
                    quota_bytes: quota,
                }
            },
        )
        .collect();
    accounts.sort_by(|a, b| {
        (b.message_bytes + b.file_bytes + b.device_sync_bytes)
            .cmp(&(a.message_bytes + a.file_bytes + a.device_sync_bytes))
            .then_with(|| a.login.cmp(&b.login))
    });
    Ok(ServerStorage {
        total_bytes: n(messages) + n(files),
        message_bytes: n(messages),
        file_bytes: n(files),
        accounts,
    })
}

/// `GET /admin/storage`.
async fn server(
    State(state): State<AppState>,
    _admin: Admin,
) -> Result<Json<ServerStorage>, ApiError> {
    let mut conn = state.pool.acquire().await?;
    server_storage(&mut conn, state.config.limits.account_quota_bytes)
        .await
        .map(Json)
}
