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
    sessions::{Ack, DeviceWelcome, LeaveWelcome, Rejoin, SendRejoin, Session, SessionMember},
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
        .route("/sessions/{id}/rejoin", post(rejoin))
        .route("/sessions/{id}/rejoins/{rejoin}", delete(rejoin_done))
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
    let rejoins: Vec<(Vec<u8>, Uuid, String, Vec<u8>)> = sqlx::query_as(
        "SELECT r.group_id, r.id, a.preferred_username, r.request
         FROM session_rejoins r JOIN actors a ON a.id = r.actor_id
         WHERE r.group_id = ANY($1) ORDER BY r.created_at",
    )
    .bind(&groups)
    .fetch_all(&mut *conn)
    .await?;
    // The login's persona devices that last read before the deleted part
    // of the log.
    let stale: Vec<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT k.group_id, k.device_id
         FROM session_acks k JOIN mls_groups g ON g.group_id = k.group_id
         JOIN devices d ON d.id = k.device_id JOIN actors a ON a.id = d.actor_id
         WHERE k.group_id = ANY($1) AND a.account_id = $2 AND d.revocation IS NULL
           AND k.seq < g.pruned_through",
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
                    rejoins: rejoins
                        .iter()
                        .filter(|r| r.0 == group)
                        .map(|(_, id, handle, request)| Rejoin {
                            id: *id,
                            actor_uri: actor_uri(domain, handle),
                            request: b64(request),
                        })
                        .collect(),
                    stale_devices: stale.iter().filter(|k| k.0 == group).map(|k| k.1).collect(),
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
    // The caller's own persona's device; or a device of the other persona
    // that asked to rejoin (2j).
    let own: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM devices d
             JOIN session_members m ON m.actor_id = d.actor_id AND m.group_id = $1
             JOIN actors a ON a.id = d.actor_id
             WHERE d.id = $2 AND d.revocation IS NULL
               AND (a.account_id = $3 OR EXISTS (
                   SELECT 1 FROM session_rejoins r
                   WHERE r.group_id = $1 AND r.actor_id = d.actor_id)))",
    )
    .bind(&group)
    .bind(body.device)
    .bind(authed.account_id)
    .fetch_one(&mut *tx)
    .await?;
    if !own {
        return Err(ApiError::BadRequest(
            "Welcomes can only be left for your own persona's devices in the session, or a device asking to rejoin.".into(),
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
    // The device needs nothing from before its Welcome: it counts as having
    // read that far (2k), so it isn't taken for one that fell behind.
    sqlx::query(
        "INSERT INTO session_acks (group_id, device_id, seq) VALUES ($1, $2, $3)
         ON CONFLICT (group_id, device_id)
         DO UPDATE SET seq = GREATEST(session_acks.seq, EXCLUDED.seq)",
    )
    .bind(&group)
    .bind(body.device)
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
pub(crate) const GRACE_DAYS: i32 = 7;

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
/// has acknowledged, once they're older than the grace period, and the
/// blobs they carried. A device that hasn't acknowledged anything holds
/// everything.
pub(crate) async fn prune(conn: &mut PgConnection, group: &[u8]) -> Result<u64, ApiError> {
    let deleted = prune_messages(conn, group).await?;
    crate::retention::delete_orphan_blobs(conn, Some(group)).await?;
    Ok(deleted)
}

/// The messages part of [`prune`]; records how far the log is deleted.
pub(crate) async fn prune_messages(conn: &mut PgConnection, group: &[u8]) -> Result<u64, ApiError> {
    let deleted: i64 = sqlx::query_scalar(
        "WITH needed AS (
             SELECT d.id FROM devices d
             JOIN session_members m ON m.actor_id = d.actor_id AND m.group_id = $1
             JOIN actors a ON a.id = d.actor_id
             WHERE d.revocation IS NULL AND d.key_version = a.identity_key_version
         ), floor AS (
             SELECT min(coalesce(k.seq, 0)) AS seq FROM needed n
             LEFT JOIN session_acks k ON k.device_id = n.id AND k.group_id = $1
         ), gone AS (
             DELETE FROM group_messages
             WHERE group_id = $1 AND seq <= (SELECT seq FROM floor)
               AND logged_at < now() - make_interval(days => $2)
             RETURNING seq
         ), marked AS (
             UPDATE mls_groups SET pruned_through = GREATEST(pruned_through, (SELECT max(seq) FROM gone))
             WHERE group_id = $1 AND EXISTS (SELECT 1 FROM gone)
             RETURNING 1
         )
         SELECT count(*) FROM gone",
    )
    .bind(group)
    .bind(GRACE_DAYS)
    .fetch_one(&mut *conn)
    .await?;
    Ok(u64::try_from(deleted).unwrap_or(0))
}

/// Deletes rejoin requests too old to act on.
pub(crate) async fn drop_old_rejoins(conn: &mut PgConnection) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM session_rejoins WHERE created_at < now() - make_interval(days => $1)")
        .bind(REJOIN_DAYS)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

// ---- Rejoin requests (2j) ----------------------------------------------------

/// How long a rejoin request stays.
const REJOIN_DAYS: i32 = 30;

/// `POST /sessions/{id}/rejoin`: a restored device asks to be let back in.
/// The request must be signed by the identity key of the caller's persona
/// in the session; the other writer's client checks it again.
async fn rejoin(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<SendRejoin>,
) -> Result<StatusCode, ApiError> {
    let group = decode("session id", &id)?;
    let request = decode("request", &body.request)?;
    if u64::try_from(request.len()).unwrap_or(u64::MAX) > state.config.limits.max_message_bytes {
        return Err(ApiError::PayloadTooLarge);
    }
    let mut tx = state.pool.begin().await?;
    let persona: Option<(Uuid, String, Option<Vec<u8>>, bool)> = sqlx::query_as(
        "SELECT a.id, a.preferred_username, a.identity_key, s.ended_at IS NOT NULL
         FROM session_members m JOIN actors a ON a.id = m.actor_id
         JOIN sessions s ON s.group_id = m.group_id
         WHERE m.group_id = $1 AND a.account_id = $2",
    )
    .bind(&group)
    .bind(authed.account_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (actor, handle, key, ended) = persona.ok_or(ApiError::NotFound)?;
    if ended {
        return Err(ApiError::Forbidden("The session has ended.".into()));
    }
    let key = key.ok_or_else(|| ApiError::BadRequest("the persona has no identity key".into()))?;
    quill_certs::RejoinRequest::verify(
        &request,
        &actor_uri(&state.config.domain, &handle),
        &group,
        &key,
    )
    .map_err(|e| ApiError::BadRequest(format!("rejoin request refused: {e}")))?;
    drop_old_rejoins(&mut tx).await?;
    sqlx::query(
        "INSERT INTO session_rejoins (id, group_id, actor_id, request) VALUES ($1, $2, $3, $4)
         ON CONFLICT (group_id, actor_id)
         DO UPDATE SET id = EXCLUDED.id, request = EXCLUDED.request, created_at = now()",
    )
    .bind(Uuid::new_v4())
    .bind(&group)
    .bind(actor)
    .bind(&request)
    .execute(&mut *tx)
    .await?;
    let members = crate::events::group_accounts(&mut tx, &group).await?;
    tx.commit().await?;
    state.events.send(
        members,
        Event::Session {
            id: b64(&group),
            ended: false,
        },
    );
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /sessions/{id}/rejoins/{rejoin}`: a member's client has dealt
/// with it.
async fn rejoin_done(
    State(state): State<AppState>,
    authed: Authed,
    Path((id, rejoin)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let group = decode("session id", &id)?;
    let mut tx = state.pool.begin().await?;
    member(&mut tx, authed.account_id, &group).await?;
    let deleted = sqlx::query("DELETE FROM session_rejoins WHERE id = $1 AND group_id = $2")
        .bind(rejoin)
        .bind(&group)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}
