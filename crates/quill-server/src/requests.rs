//! Collaboration requests (2e; protocol draft, section 4).
//!
//! A request carries an MLS `Welcome` and the pitch, a `PrivateMessage` in
//! the new group; the server reads only the pitch's clear header. It files
//! each request by the prompt's state: `open` makes it pending, `busy`
//! holds it, `closed` refuses it (kept without its ciphertext, never shown).
//! The requester sees only *waiting*, *session* or *no response*: held,
//! refused and declined requests look the same as pending ones until they
//! expire. Accepting registers the group as a session on the group message
//! log, starting with the pitch.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use quill_mls_envelope::private_message::{self, ContentType};
use quill_proto::{
    groups::GroupState,
    keys::{actor_uri, b64, unb64},
    requests::{
        IncomingRequest, MAX_EXPIRY_DAYS, OutgoingRequest, Queue, QueueQuery, RequestPersona,
        RequestSent, RequestState, RequestStatus, SendRequest,
    },
};
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{AppState, auth::Authed, error::ApiError, routes::ApiJson};

/// New requests per login per hour.
const REQUESTS_PER_HOUR: usize = 20;
/// The unique index behind "one waiting request per login per prompt".
const ONE_WAITING: &str = "session_requests_one_waiting_idx";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/requests", get(list_own).post(send))
        .route("/requests/{id}", get(status).delete(withdraw))
        .route("/requests/{id}/accept", post(accept))
        .route("/requests/{id}/reject", post(decline))
        .route("/prompts/{id}/requests", get(queue))
}

fn decode(field: &str, text: &str) -> Result<Vec<u8>, ApiError> {
    unb64(text).ok_or_else(|| ApiError::BadRequest(format!("{field} is not base64url")))
}

/// Expires waiting requests whose time is up, dropping their ciphertext.
pub(crate) async fn expire(conn: &mut PgConnection) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE session_requests SET state = 'expired', welcome = NULL, pitch = NULL
         WHERE expires_at <= now() AND state IN ('pending', 'held', 'refused', 'declined')",
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Expires a prompt's waiting requests at once, before it's deleted.
pub(crate) async fn expire_for_prompt(
    conn: &mut PgConnection,
    prompt: Uuid,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE session_requests SET state = 'expired', welcome = NULL, pitch = NULL
         WHERE prompt_id = $1 AND state IN ('pending', 'held', 'refused', 'declined')",
    )
    .bind(prompt)
    .execute(conn)
    .await?;
    Ok(())
}

/// Releases a prompt's held requests to pending (it was reopened, or its
/// author read the held queue).
pub(crate) async fn release_held(conn: &mut PgConnection, prompt: Uuid) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE session_requests SET state = 'pending' WHERE prompt_id = $1 AND state = 'held'",
    )
    .bind(prompt)
    .execute(conn)
    .await?;
    Ok(())
}

/// `POST /requests`: files a request by the prompt's state. The answer is
/// the same whether it's pending, held or refused.
async fn send(
    State(state): State<AppState>,
    authed: Authed,
    ApiJson(body): ApiJson<SendRequest>,
) -> Result<(StatusCode, Json<RequestSent>), ApiError> {
    if !state.limiter.allow(
        &format!("request:{}", authed.account_id),
        REQUESTS_PER_HOUR,
        std::time::Duration::from_secs(3600),
    ) {
        return Err(ApiError::TooManyRequests);
    }
    let welcome = decode("welcome", &body.welcome)?;
    let pitch = decode("pitch", &body.pitch)?;
    let max = state.config.limits.max_message_bytes;
    for part in [&welcome, &pitch] {
        if u64::try_from(part.len()).unwrap_or(u64::MAX) > max {
            return Err(ApiError::PayloadTooLarge);
        }
    }
    // An MLSMessage holding a Welcome: MLS 1.0, wire format 3.
    if welcome.get(..4) != Some(&[0, 1, 0, 3]) {
        return Err(ApiError::BadRequest("welcome is not an MLS Welcome".into()));
    }
    let header = private_message::parse(&pitch)
        .map_err(|e| ApiError::BadRequest(format!("pitch refused: {e}")))?;
    if header.content_type != ContentType::Application {
        return Err(ApiError::BadRequest(
            "pitch refused: it isn't an application message".into(),
        ));
    }
    if !(16..=64).contains(&header.group_id.len()) {
        return Err(ApiError::BadRequest(
            "pitch refused: the group id must be 16 to 64 bytes".into(),
        ));
    }
    let epoch = i64::try_from(header.epoch)
        .map_err(|_| ApiError::BadRequest("pitch refused: epoch out of range".into()))?;
    let days = body.expires_in_days.unwrap_or(MAX_EXPIRY_DAYS);
    if !(1..=MAX_EXPIRY_DAYS).contains(&days) {
        return Err(ApiError::BadRequest(format!(
            "expiresInDays must be 1 to {MAX_EXPIRY_DAYS}"
        )));
    }

    let mut tx = state.pool.begin().await?;
    let persona: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM actors WHERE id = $1 AND account_id = $2")
            .bind(body.persona)
            .bind(authed.account_id)
            .fetch_optional(&mut *tx)
            .await?;
    let persona = persona.ok_or(ApiError::NotFound)?;
    let prompt: Option<(String, Uuid, Uuid)> = sqlx::query_as(
        "SELECT p.state::text, p.actor_id, a.account_id FROM prompts p
         JOIN actors a ON a.id = p.actor_id
         WHERE p.id = $1 AND p.removed_at IS NULL
         FOR SHARE OF p",
    )
    .bind(body.prompt)
    .fetch_optional(&mut *tx)
    .await?;
    let (prompt_state, author, author_account) = prompt.ok_or(ApiError::NotFound)?;
    if author_account == authed.account_id {
        return Err(ApiError::BadRequest(
            "That's your own prompt: you can't ask to join it.".into(),
        ));
    }
    let taken: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM mls_groups WHERE group_id = $1)")
            .bind(&header.group_id)
            .fetch_one(&mut *tx)
            .await?;
    if taken {
        return Err(ApiError::Conflict("That group id is taken.".into()));
    }
    expire(&mut tx).await?;
    let filed = match prompt_state.as_str() {
        "open" => "pending",
        "busy" => "held",
        _ => "refused",
    };
    let (welcome, pitch) = if filed == "refused" {
        (None, None)
    } else {
        (Some(welcome), Some(pitch))
    };
    let id = Uuid::new_v4();
    let inserted: Result<OffsetDateTime, sqlx::Error> = sqlx::query_scalar(
        "INSERT INTO session_requests
             (id, prompt_id, author_actor_id, requester_actor_id, requester_account_id,
              group_id, epoch, state, welcome, pitch, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::request_state, $9, $10,
                 now() + make_interval(days => $11))
         RETURNING expires_at",
    )
    .bind(id)
    .bind(body.prompt)
    .bind(author)
    .bind(persona)
    .bind(authed.account_id)
    .bind(&header.group_id)
    .bind(epoch)
    .bind(filed)
    .bind(welcome)
    .bind(pitch)
    .bind(i32::try_from(days).unwrap_or(30))
    .fetch_one(&mut *tx)
    .await;
    let expires_at = match inserted {
        Ok(t) => t,
        Err(e) => {
            let constraint = e
                .as_database_error()
                .and_then(|d| d.constraint())
                .map(str::to_owned);
            return Err(match constraint.as_deref() {
                Some(ONE_WAITING) => ApiError::Conflict(
                    "You already have a request waiting on this prompt. Withdraw it to send another."
                        .into(),
                ),
                Some(_) if crate::error::is_unique_violation(&e) => {
                    ApiError::Conflict("That group id is taken.".into())
                }
                _ => e.into(),
            });
        }
    };
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(RequestSent { id, expires_at })))
}

/// What the requester may know: waiting, session, or no response.
fn requester_view(state: &str) -> RequestStatus {
    match state {
        "accepted" => RequestStatus::Session,
        "expired" => RequestStatus::NoResponse,
        _ => RequestStatus::Waiting,
    }
}

fn author_view(state: &str) -> RequestStatus {
    match state {
        "pending" => RequestStatus::Pending,
        "held" => RequestStatus::Held,
        "accepted" => RequestStatus::Session,
        "declined" => RequestStatus::Declined,
        // A refused request is never shown to the author; treat it as gone.
        _ => RequestStatus::Expired,
    }
}

type OwnRow = (Uuid, Option<Uuid>, Uuid, String, Vec<u8>, OffsetDateTime);

/// `GET /requests`: the login's own requests, newest first.
async fn list_own(
    State(state): State<AppState>,
    authed: Authed,
) -> Result<Json<Vec<OutgoingRequest>>, ApiError> {
    let mut conn = state.pool.acquire().await?;
    expire(&mut conn).await?;
    let rows: Vec<OwnRow> = sqlx::query_as(
        "SELECT id, prompt_id, requester_actor_id, state::text, group_id, expires_at
         FROM session_requests WHERE requester_account_id = $1
         ORDER BY received_at DESC, id",
    )
    .bind(authed.account_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(
                |(id, prompt, persona, state, group, expires_at)| OutgoingRequest {
                    id,
                    prompt,
                    persona,
                    status: requester_view(&state),
                    group_id: b64(&group),
                    expires_at,
                },
            )
            .collect(),
    ))
}

/// `(state, requester login, author login)`.
/// The author's login is `None` once the author persona is deleted.
type SidesRow = (String, Uuid, Option<Uuid>);

async fn sides(conn: &mut PgConnection, id: Uuid, lock: bool) -> Result<SidesRow, ApiError> {
    let sql = if lock {
        "SELECT r.state::text, r.requester_account_id, a.account_id
         FROM session_requests r LEFT JOIN actors a ON a.id = r.author_actor_id
         WHERE r.id = $1 FOR UPDATE OF r"
    } else {
        "SELECT r.state::text, r.requester_account_id, a.account_id
         FROM session_requests r LEFT JOIN actors a ON a.id = r.author_actor_id
         WHERE r.id = $1"
    };
    sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(conn)
        .await?
        .ok_or(ApiError::NotFound)
}

/// `GET /requests/{id}`: its status, as the caller's side may see it.
async fn status(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
) -> Result<Json<RequestState>, ApiError> {
    let mut conn = state.pool.acquire().await?;
    expire(&mut conn).await?;
    let (st, requester, author) = sides(&mut conn, id, false).await?;
    let status = if author == Some(authed.account_id) {
        if st == "refused" {
            return Err(ApiError::NotFound);
        }
        author_view(&st)
    } else if requester == authed.account_id {
        requester_view(&st)
    } else {
        return Err(ApiError::NotFound);
    };
    Ok(Json(RequestState { id, status }))
}

/// `DELETE /requests/{id}`: the requester withdraws a request that isn't
/// a session (or dismisses one that got no response).
async fn withdraw(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    let (st, requester, _) = sides(&mut tx, id, true).await?;
    if requester != authed.account_id {
        return Err(ApiError::NotFound);
    }
    if st == "accepted" {
        return Err(ApiError::Conflict(
            "It's a session now; leave the session instead.".into(),
        ));
    }
    sqlx::query("DELETE FROM session_requests WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

type QueueRow = (
    Uuid,
    String,
    Option<String>,
    Vec<u8>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    OffsetDateTime,
    OffsetDateTime,
);

/// `GET /prompts/{id}/requests?state=pending|held`: the author's queue.
/// Asking for held requests releases them to pending first.
async fn queue(
    State(state): State<AppState>,
    authed: Authed,
    Path(prompt): Path<Uuid>,
    Query(q): Query<QueueQuery>,
) -> Result<Json<Queue>, ApiError> {
    let release = match q.state.as_deref() {
        None | Some("pending") => false,
        Some("held") => true,
        Some(_) => {
            return Err(ApiError::BadRequest("state must be pending or held".into()));
        }
    };
    let mut tx = state.pool.begin().await?;
    let own: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM prompts p JOIN actors a ON a.id = p.actor_id
                        WHERE p.id = $1 AND a.account_id = $2)",
    )
    .bind(prompt)
    .bind(authed.account_id)
    .fetch_one(&mut *tx)
    .await?;
    if !own {
        return Err(ApiError::NotFound);
    }
    expire(&mut tx).await?;
    if release {
        release_held(&mut tx, prompt).await?;
    }
    let rows: Vec<QueueRow> = sqlx::query_as(
        "SELECT r.id, a.preferred_username, a.display_name, r.group_id, r.welcome, r.pitch,
                r.received_at, r.expires_at
         FROM session_requests r JOIN actors a ON a.id = r.requester_actor_id
         WHERE r.prompt_id = $1 AND r.state = 'pending'
         ORDER BY r.received_at, r.id",
    )
    .bind(prompt)
    .fetch_all(&mut *tx)
    .await?;
    let held: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM session_requests WHERE prompt_id = $1 AND state = 'held'",
    )
    .bind(prompt)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    let domain = &state.config.domain;
    Ok(Json(Queue {
        requests: rows
            .into_iter()
            .map(
                |(id, handle, display_name, group, welcome, pitch, received_at, expires_at)| {
                    IncomingRequest {
                        id,
                        prompt,
                        requester: RequestPersona {
                            actor_uri: actor_uri(domain, &handle),
                            handle,
                            display_name,
                        },
                        group_id: b64(&group),
                        welcome: b64(&welcome.unwrap_or_default()),
                        pitch: b64(&pitch.unwrap_or_default()),
                        received_at,
                        expires_at,
                    }
                },
            )
            .collect(),
        held: u32::try_from(held).unwrap_or(u32::MAX),
    }))
}

/// `(state, author login, group id, epoch, pitch, author persona, requester persona)`.
type AcceptRow = (String, Uuid, Vec<u8>, i64, Option<Vec<u8>>, Uuid, Uuid);

/// `POST /requests/{id}/accept`: the group becomes a session. Its log
/// starts with the pitch, and both personas' logins can use it.
async fn accept(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
) -> Result<Json<GroupState>, ApiError> {
    let mut tx = state.pool.begin().await?;
    expire(&mut tx).await?;
    let row: Option<AcceptRow> = sqlx::query_as(
        "SELECT r.state::text, a.account_id, r.group_id, r.epoch, r.pitch,
                r.author_actor_id, r.requester_actor_id
         FROM session_requests r LEFT JOIN actors a ON a.id = r.author_actor_id
         WHERE r.id = $1 FOR UPDATE OF r",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((st, author_account, group, epoch, pitch, author, requester)) = row else {
        return Err(ApiError::NotFound);
    };
    if author_account != authed.account_id || st == "refused" {
        return Err(ApiError::NotFound);
    }
    let pitch = match (st.as_str(), pitch) {
        ("pending" | "held", Some(p)) => p,
        ("accepted", _) => return Err(ApiError::Conflict("It's already a session.".into())),
        ("declined", _) => return Err(ApiError::Conflict("It was declined.".into())),
        _ => return Err(ApiError::Conflict("It expired.".into())),
    };
    let registered =
        sqlx::query("INSERT INTO mls_groups (group_id, kind, epoch) VALUES ($1, 'session', $2)")
            .bind(&group)
            .bind(epoch)
            .execute(&mut *tx)
            .await;
    match registered {
        Ok(_) => {}
        Err(e) if crate::error::is_unique_violation(&e) => {
            return Err(ApiError::Conflict("Its group id is taken.".into()));
        }
        Err(e) => return Err(e.into()),
    }
    sqlx::query(
        "INSERT INTO group_messages (group_id, seq, epoch, is_commit, data)
         VALUES ($1, 1, $2, false, $3)",
    )
    .bind(&group)
    .bind(epoch)
    .bind(&pitch)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO session_members (group_id, actor_id) VALUES ($1, $2), ($1, $3)")
        .bind(&group)
        .bind(author)
        .bind(requester)
        .execute(&mut *tx)
        .await?;
    // The Welcome moves to the session, for devices that haven't opened it.
    sqlx::query(
        "INSERT INTO sessions (group_id, request_id, prompt_id, welcome)
         SELECT group_id, id, prompt_id, welcome FROM session_requests WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE session_requests SET state = 'accepted', pitch = NULL, welcome = NULL
         WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(GroupState {
        group_id: b64(&group),
        epoch: u64::try_from(epoch).unwrap_or(0),
        last_seq: 1,
    }))
}

/// `POST /requests/{id}/reject`: declines. Nothing reaches the requester;
/// the record stays, without ciphertext, until it expires.
async fn decline(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    expire(&mut tx).await?;
    let (st, _, author) = sides(&mut tx, id, true).await?;
    if author != Some(authed.account_id) || st == "refused" {
        return Err(ApiError::NotFound);
    }
    match st.as_str() {
        "pending" | "held" => {
            sqlx::query(
                "UPDATE session_requests SET state = 'declined', welcome = NULL, pitch = NULL
                 WHERE id = $1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        "accepted" => return Err(ApiError::Conflict("It's already a session.".into())),
        // Declined or expired already: nothing to do.
        _ => {}
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
