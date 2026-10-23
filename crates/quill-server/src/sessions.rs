//! Sessions (2f): accepted requests' groups. The list a login's devices
//! sync from, Welcomes left for devices added after a session began, and
//! leaving. Messages go through the group message log (`groups.rs`).

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use quill_proto::{
    keys::{actor_uri, b64, unb64},
    sessions::{DeviceWelcome, LeaveWelcome, Session, SessionMember},
};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{AppState, auth::Authed, error::ApiError, routes::ApiJson};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/sessions", get(list))
        .route("/sessions/{id}/welcomes", post(leave_welcome))
        .route("/sessions/{id}/welcomes/{device}", delete(take_welcome))
        .route("/sessions/{id}/leave", post(leave))
}

fn decode(field: &str, text: &str) -> Result<Vec<u8>, ApiError> {
    unb64(text).ok_or_else(|| ApiError::BadRequest(format!("{field} is not base64url")))
}

fn as_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

/// Whether the caller's login has a persona in the session. Unknown
/// sessions and other people's look the same.
async fn member(conn: &mut PgConnection, account: Uuid, group: &[u8]) -> Result<(), ApiError> {
    let ok: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sessions s
             JOIN session_members m ON m.group_id = s.group_id
             JOIN actors a ON a.id = m.actor_id
             WHERE s.group_id = $1 AND a.account_id = $2)",
    )
    .bind(group)
    .bind(account)
    .fetch_one(conn)
    .await?;
    if ok { Ok(()) } else { Err(ApiError::NotFound) }
}

/// `(group, request, prompt, welcome, ended, epoch, last seq)`.
type SessionRow = (Vec<u8>, Uuid, Option<Uuid>, Vec<u8>, bool, i64, i64);
/// `(group, actor, handle, display name, login)`.
type MemberRow = (Vec<u8>, Uuid, String, Option<String>, Uuid);

/// `GET /sessions`: the login's sessions, ended ones included.
async fn list(
    State(state): State<AppState>,
    authed: Authed,
) -> Result<Json<Vec<Session>>, ApiError> {
    let mut conn = state.pool.acquire().await?;
    let rows: Vec<SessionRow> = sqlx::query_as(
        "SELECT s.group_id, s.request_id, s.prompt_id, s.welcome, s.ended_at IS NOT NULL,
                g.epoch, coalesce((SELECT max(seq) FROM group_messages m
                                   WHERE m.group_id = s.group_id), 0)
         FROM sessions s JOIN mls_groups g ON g.group_id = s.group_id
         WHERE EXISTS (SELECT 1 FROM session_members m JOIN actors a ON a.id = m.actor_id
                       WHERE m.group_id = s.group_id AND a.account_id = $1)
         ORDER BY s.request_id",
    )
    .bind(authed.account_id)
    .fetch_all(&mut *conn)
    .await?;
    let groups: Vec<Vec<u8>> = rows.iter().map(|r| r.0.clone()).collect();
    let members: Vec<MemberRow> = sqlx::query_as(
        "SELECT m.group_id, a.id, a.preferred_username, a.display_name, a.account_id
         FROM session_members m JOIN actors a ON a.id = m.actor_id
         WHERE m.group_id = ANY($1) ORDER BY a.preferred_username",
    )
    .bind(&groups)
    .fetch_all(&mut *conn)
    .await?;
    // Welcomes left for any device of the login's personas.
    let welcomes: Vec<(Vec<u8>, Uuid, Vec<u8>, i64)> = sqlx::query_as(
        "SELECT w.group_id, w.device_id, w.welcome, w.from_seq
         FROM session_welcomes w JOIN devices d ON d.id = w.device_id
         JOIN actors a ON a.id = d.actor_id
         WHERE w.group_id = ANY($1) AND a.account_id = $2",
    )
    .bind(&groups)
    .bind(authed.account_id)
    .fetch_all(&mut *conn)
    .await?;
    let domain = &state.config.domain;
    Ok(Json(
        rows.into_iter()
            .map(
                |(group, request, prompt, welcome, ended, epoch, last)| Session {
                    id: b64(&group),
                    request,
                    prompt,
                    members: members
                        .iter()
                        .filter(|m| m.0 == group)
                        .map(|(_, id, handle, display_name, login)| SessionMember {
                            actor_uri: actor_uri(domain, handle),
                            handle: handle.clone(),
                            display_name: display_name.clone(),
                            persona: (*login == authed.account_id).then_some(*id),
                        })
                        .collect(),
                    epoch: as_u64(epoch),
                    last_seq: as_u64(last),
                    ended,
                    welcome: b64(&welcome),
                    device_welcomes: welcomes
                        .iter()
                        .filter(|w| w.0 == group)
                        .map(|(_, device, welcome, from)| DeviceWelcome {
                            device: *device,
                            welcome: b64(welcome),
                            from_seq: as_u64(*from),
                        })
                        .collect(),
                },
            )
            .collect(),
    ))
}

/// `POST /sessions/{id}/welcomes`: a member's device leaves a Welcome for
/// a device it just added, which must be one of the caller's own persona's
/// devices in the session.
async fn leave_welcome(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<LeaveWelcome>,
) -> Result<StatusCode, ApiError> {
    let group = decode("session id", &id)?;
    let welcome = decode("welcome", &body.welcome)?;
    if u64::try_from(welcome.len()).unwrap_or(u64::MAX) > state.config.limits.max_message_bytes {
        return Err(ApiError::PayloadTooLarge);
    }
    if welcome.get(..4) != Some(&[0, 1, 0, 3]) {
        return Err(ApiError::BadRequest("welcome is not an MLS Welcome".into()));
    }
    let mut tx = state.pool.begin().await?;
    member(&mut tx, authed.account_id, &group).await?;
    let own: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM devices d
             JOIN session_members m ON m.actor_id = d.actor_id AND m.group_id = $1
             JOIN actors a ON a.id = d.actor_id
             WHERE d.id = $2 AND a.account_id = $3 AND d.revocation IS NULL)",
    )
    .bind(&group)
    .bind(body.device)
    .bind(authed.account_id)
    .fetch_one(&mut *tx)
    .await?;
    if !own {
        return Err(ApiError::BadRequest(
            "Welcomes can only be left for your own persona's devices in the session.".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO session_welcomes (group_id, device_id, welcome, from_seq)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (group_id, device_id)
         DO UPDATE SET welcome = EXCLUDED.welcome, from_seq = EXCLUDED.from_seq",
    )
    .bind(&group)
    .bind(body.device)
    .bind(&welcome)
    .bind(i64::try_from(body.from_seq).unwrap_or(i64::MAX))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /sessions/{id}/welcomes/{device}`: the device has joined.
async fn take_welcome(
    State(state): State<AppState>,
    authed: Authed,
    Path((id, device)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let group = decode("session id", &id)?;
    let deleted = sqlx::query(
        "DELETE FROM session_welcomes w USING devices d, actors a
         WHERE w.group_id = $1 AND w.device_id = $2
           AND d.id = w.device_id AND a.id = d.actor_id AND a.account_id = $3",
    )
    .bind(&group)
    .bind(device)
    .bind(authed.account_id)
    .execute(&state.pool)
    .await?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /sessions/{id}/leave`: with two writers, one leaving ends the
/// session. Its log stays readable.
async fn leave(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let group = decode("session id", &id)?;
    let mut tx = state.pool.begin().await?;
    member(&mut tx, authed.account_id, &group).await?;
    sqlx::query("UPDATE sessions SET ended_at = now() WHERE group_id = $1 AND ended_at IS NULL")
        .bind(&group)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM session_welcomes WHERE group_id = $1")
        .bind(&group)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
