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
    events::Event,
    keys::{actor_uri, b64, unb64},
    sessions::{Ack, DeviceWelcome, LeaveWelcome, Session, SessionMember},
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
        .route("/sessions/{id}/ack", post(ack))
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
                g.epoch, g.last_seq
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
    state.events.send(
        vec![authed.account_id],
        Event::Session {
            id: b64(&group),
            ended: false,
        },
    );
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
    let members = crate::events::group_accounts(&mut tx, &group).await?;
    tx.commit().await?;
    state.events.send(
        members,
        Event::Session {
            id: b64(&group),
            ended: true,
        },
    );
    Ok(StatusCode::NO_CONTENT)
}

/// How long an acknowledged message stays before it's deleted.
const GRACE_DAYS: i32 = 7;

/// `POST /sessions/{id}/ack`: one of the caller's devices has read the log
/// this far. Then deletes what every device has.
async fn ack(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Ack>,
) -> Result<StatusCode, ApiError> {
    let group = decode("session id", &id)?;
    let seq =
        i64::try_from(body.seq).map_err(|_| ApiError::BadRequest("seq out of range".into()))?;
    let mut tx = state.pool.begin().await?;
    member(&mut tx, authed.account_id, &group).await?;
    let own: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM devices d
             JOIN session_members m ON m.actor_id = d.actor_id AND m.group_id = $1
             JOIN actors a ON a.id = d.actor_id
             WHERE d.id = $2 AND a.account_id = $3)",
    )
    .bind(&group)
    .bind(body.device)
    .bind(authed.account_id)
    .fetch_one(&mut *tx)
    .await?;
    if !own {
        return Err(ApiError::BadRequest(
            "Acknowledge for your own persona's devices in the session only.".into(),
        ));
    }
    let last: i64 = sqlx::query_scalar("SELECT last_seq FROM mls_groups WHERE group_id = $1")
        .bind(&group)
        .fetch_one(&mut *tx)
        .await?;
    if seq > last {
        return Err(ApiError::BadRequest(
            "That's past the end of the log.".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO session_acks (group_id, device_id, seq) VALUES ($1, $2, $3)
         ON CONFLICT (group_id, device_id)
         DO UPDATE SET seq = GREATEST(session_acks.seq, EXCLUDED.seq)",
    )
    .bind(&group)
    .bind(body.device)
    .bind(seq)
    .execute(&mut *tx)
    .await?;
    prune(&mut tx, &group).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes the messages every current device of the session's personas
/// has acknowledged, once they're older than the grace period. A device
/// that hasn't acknowledged anything holds everything.
pub(crate) async fn prune(conn: &mut PgConnection, group: &[u8]) -> Result<u64, ApiError> {
    let deleted = sqlx::query(
        "WITH needed AS (
             SELECT d.id FROM devices d
             JOIN session_members m ON m.actor_id = d.actor_id AND m.group_id = $1
             JOIN actors a ON a.id = d.actor_id
             WHERE d.revocation IS NULL AND d.key_version = a.identity_key_version
         ), floor AS (
             SELECT min(coalesce(k.seq, 0)) AS seq FROM needed n
             LEFT JOIN session_acks k ON k.device_id = n.id AND k.group_id = $1
         )
         DELETE FROM group_messages
         WHERE group_id = $1 AND seq <= (SELECT seq FROM floor)
           AND logged_at < now() - make_interval(days => $2)",
    )
    .bind(group)
    .bind(GRACE_DAYS)
    .execute(&mut *conn)
    .await?;
    // Blobs go with the messages that carry them; ones no message ever
    // named go after a day.
    sqlx::query(
        "DELETE FROM blobs b WHERE b.group_id = $1 AND (
             (b.seq IS NOT NULL AND NOT EXISTS (
                 SELECT 1 FROM group_messages m WHERE m.group_id = b.group_id AND m.seq = b.seq))
             OR (b.seq IS NULL AND b.uploaded_at < now() - interval '1 day'))",
    )
    .bind(group)
    .execute(&mut *conn)
    .await?;
    Ok(deleted.rows_affected())
}
