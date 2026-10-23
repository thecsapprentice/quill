//! MLS groups' message logs (the epoch sequencer) and the device-linking
//! mailbox. Groups are a login's personal device group (2d) or sessions,
//! registered by accepting a request (2e).
//!
//! The server reads only the clear header of each `PrivateMessage` (group
//! id, epoch, content type). The first commit for a group's current epoch
//! wins and advances it; anything for another epoch gets `409`, and the
//! client catches up and tries again. That gives every member the same
//! order of commits, which MLS needs.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use quill_mls_envelope::private_message::{self, ContentType};
use quill_proto::{
    groups::{
        CreateGroup, GroupState, LinkMailbox, LinkReply, LinkStarted, LoggedMessage, Messages,
        MessagesQuery, PostMessage, Posted, StartLink,
    },
    keys::{b64, unb64},
};
use uuid::Uuid;

use crate::{
    AppState,
    auth::Authed,
    error::{ApiError, is_unique_violation},
    routes::ApiJson,
};

/// Blobs one message can carry.
const MAX_BLOBS_PER_MESSAGE: usize = 10;
/// How long a link mailbox lives.
const LINK_SECS: u64 = 10 * 60;
/// New link mailboxes per login per hour.
const LINKS_PER_HOUR: usize = 10;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/groups", post(create_group))
        .route("/account/device-group", get(device_group))
        .route(
            "/groups/{id}/messages",
            get(read_messages).post(post_message),
        )
        .route("/link", post(start_link))
        .route("/link/{id}", get(read_link))
        .route("/link/{id}/reply", post(reply_link))
}

fn decode(field: &str, text: &str) -> Result<Vec<u8>, ApiError> {
    unb64(text).ok_or_else(|| ApiError::BadRequest(format!("{field} is not base64url")))
}

fn as_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

macro_rules! group_access {
    () => {
        "SELECT g.epoch FROM mls_groups g WHERE g.group_id = $1 AND (g.account_id = $2 OR EXISTS (
             SELECT 1 FROM session_members m JOIN actors a ON a.id = m.actor_id
             WHERE m.group_id = g.group_id AND a.account_id = $2))"
    };
}

/// A group the caller may use, locked for update when `lock`. Returns its
/// current epoch. Unknown and other people's groups look the same.
async fn group_epoch(
    conn: &mut sqlx::PgConnection,
    account: Uuid,
    group: &[u8],
    lock: bool,
) -> Result<i64, ApiError> {
    // A personal group's login, or the login of a session member.
    let sql = if lock {
        concat!(group_access!(), " FOR UPDATE OF g")
    } else {
        group_access!()
    };
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(group)
        .bind(account)
        .fetch_optional(conn)
        .await?
        .ok_or(ApiError::NotFound)
}

async fn state_of(
    conn: &mut sqlx::PgConnection,
    group: &[u8],
    epoch: i64,
) -> Result<GroupState, ApiError> {
    let last: i64 = sqlx::query_scalar("SELECT last_seq FROM mls_groups WHERE group_id = $1")
        .bind(group)
        .fetch_one(conn)
        .await?;
    Ok(GroupState {
        group_id: b64(group),
        epoch: as_u64(epoch),
        last_seq: as_u64(last),
    })
}

/// `POST /groups`: registers a new group. One personal group per login.
async fn create_group(
    State(state): State<AppState>,
    authed: Authed,
    ApiJson(body): ApiJson<CreateGroup>,
) -> Result<(StatusCode, Json<GroupState>), ApiError> {
    if body.kind != "personal" {
        return Err(ApiError::BadRequest("kind must be personal".into()));
    }
    let id = decode("groupId", &body.group_id)?;
    if !(16..=64).contains(&id.len()) {
        return Err(ApiError::BadRequest(
            "groupId must be 16 to 64 bytes".into(),
        ));
    }
    let inserted = sqlx::query(
        "INSERT INTO mls_groups (group_id, kind, account_id) VALUES ($1, 'personal', $2)",
    )
    .bind(&id)
    .bind(authed.account_id)
    .execute(&state.pool)
    .await;
    match inserted {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => {
            return Err(ApiError::Conflict(
                "This login already has a personal device group.".into(),
            ));
        }
        Err(e) => return Err(e.into()),
    }
    let mut conn = state.pool.acquire().await?;
    Ok((
        StatusCode::CREATED,
        Json(state_of(&mut conn, &id, 0).await?),
    ))
}

/// `GET /account/device-group`: the login's personal device group.
async fn device_group(
    State(state): State<AppState>,
    authed: Authed,
) -> Result<Json<GroupState>, ApiError> {
    let row: Option<(Vec<u8>, i64)> = sqlx::query_as(
        "SELECT group_id, epoch FROM mls_groups WHERE account_id = $1 AND kind = 'personal'",
    )
    .bind(authed.account_id)
    .fetch_optional(&state.pool)
    .await?;
    let (id, epoch) = row.ok_or(ApiError::NotFound)?;
    let mut conn = state.pool.acquire().await?;
    state_of(&mut conn, &id, epoch).await.map(Json)
}

/// `POST /groups/{id}/messages`: appends a message for the group's current
/// epoch; a commit also advances the epoch.
async fn post_message(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<PostMessage>,
) -> Result<Json<Posted>, ApiError> {
    let group = decode("group id", &id)?;
    let raw = decode("message", &body.message)?;
    if u64::try_from(raw.len()).unwrap_or(u64::MAX) > state.config.limits.max_message_bytes {
        return Err(ApiError::PayloadTooLarge);
    }
    if body.blobs.len() > MAX_BLOBS_PER_MESSAGE {
        return Err(ApiError::BadRequest(format!(
            "at most {MAX_BLOBS_PER_MESSAGE} attachments per message"
        )));
    }
    if !state.limiter.allow(
        &format!("post:{}", authed.account_id),
        usize::try_from(state.config.limits.messages_per_minute).unwrap_or(usize::MAX),
        std::time::Duration::from_secs(60),
    ) {
        return Err(ApiError::TooManyRequests);
    }
    let header = private_message::parse(&raw)
        .map_err(|e| ApiError::BadRequest(format!("message refused: {e}")))?;
    if header.group_id != group {
        return Err(ApiError::BadRequest(
            "message refused: it's for another group".into(),
        ));
    }
    let mut tx = state.pool.begin().await?;
    let epoch = group_epoch(&mut tx, authed.account_id, &group, true).await?;
    let ended: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sessions WHERE group_id = $1 AND ended_at IS NOT NULL)",
    )
    .bind(&group)
    .fetch_one(&mut *tx)
    .await?;
    if ended {
        return Err(ApiError::Forbidden("The session has ended.".into()));
    }
    if i64::try_from(header.epoch).ok() != Some(epoch) {
        return Err(ApiError::Conflict(format!(
            "The group is at epoch {epoch}; catch up and try again."
        )));
    }
    // Counted on the group, so numbers never repeat once old messages are
    // deleted.
    let seq: i64 = sqlx::query_scalar(
        "UPDATE mls_groups SET last_seq = last_seq + 1 WHERE group_id = $1 RETURNING last_seq",
    )
    .bind(&group)
    .fetch_one(&mut *tx)
    .await?;
    let commit = header.content_type == ContentType::Commit;
    sqlx::query(
        "INSERT INTO group_messages (group_id, seq, epoch, is_commit, data)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&group)
    .bind(seq)
    .bind(epoch)
    .bind(commit)
    .bind(&raw)
    .execute(&mut *tx)
    .await?;
    let epoch = if commit {
        sqlx::query("UPDATE mls_groups SET epoch = epoch + 1 WHERE group_id = $1")
            .bind(&group)
            .execute(&mut *tx)
            .await?;
        epoch + 1
    } else {
        epoch
    };
    if !body.blobs.is_empty() {
        // The blobs it carries: the caller's own, in this group, not yet
        // carried by another message. They now last as long as it does.
        let bound = sqlx::query(
            "UPDATE blobs SET seq = $1
             WHERE id = ANY($2) AND group_id = $3 AND account_id = $4 AND seq IS NULL",
        )
        .bind(seq)
        .bind(&body.blobs)
        .bind(&group)
        .bind(authed.account_id)
        .execute(&mut *tx)
        .await?;
        if usize::try_from(bound.rows_affected()).unwrap_or(0) != body.blobs.len() {
            return Err(ApiError::BadRequest(
                "message refused: it names blobs that aren't yours to attach here".into(),
            ));
        }
    }
    let readers = crate::events::group_accounts(&mut tx, &group).await?;
    tx.commit().await?;
    state.events.send(
        readers,
        quill_proto::events::Event::Message {
            group: b64(&group),
            seq: as_u64(seq),
        },
    );
    Ok(Json(Posted {
        seq: as_u64(seq),
        epoch: as_u64(epoch),
    }))
}

/// `GET /groups/{id}/messages?after=&limit=`: the log, in order.
async fn read_messages(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<String>,
    Query(q): Query<MessagesQuery>,
) -> Result<Json<Messages>, ApiError> {
    let group = decode("group id", &id)?;
    let mut conn = state.pool.acquire().await?;
    group_epoch(&mut conn, authed.account_id, &group, false).await?;
    let limit = i64::from(q.limit.unwrap_or(100).clamp(1, 500));
    let after = i64::try_from(q.after.unwrap_or(0)).unwrap_or(i64::MAX);
    let mut rows: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT seq, data FROM group_messages
         WHERE group_id = $1 AND seq > $2 ORDER BY seq LIMIT $3",
    )
    .bind(&group)
    .bind(after)
    .bind(limit + 1)
    .fetch_all(&mut *conn)
    .await?;
    let more = rows.len() > usize::try_from(limit).unwrap_or(usize::MAX);
    rows.truncate(usize::try_from(limit).unwrap_or(0));
    Ok(Json(Messages {
        messages: rows
            .into_iter()
            .map(|(seq, data)| LoggedMessage {
                seq: as_u64(seq),
                message: b64(&data),
            })
            .collect(),
        more,
    }))
}

// ---- Linking ------------------------------------------------------------------

async fn drop_expired_links(state: &AppState) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM link_mailbox WHERE expires_at <= now()")
        .execute(&state.pool)
        .await?;
    Ok(())
}

/// `POST /link`: a new device leaves its KeyPackage for 10 minutes.
async fn start_link(
    State(state): State<AppState>,
    authed: Authed,
    ApiJson(body): ApiJson<StartLink>,
) -> Result<Json<LinkStarted>, ApiError> {
    if !state.limiter.allow(
        &format!("link:{}", authed.account_id),
        LINKS_PER_HOUR,
        std::time::Duration::from_secs(3600),
    ) {
        return Err(ApiError::TooManyRequests);
    }
    let package = decode("keyPackage", &body.key_package)?;
    if package.len() > 16 * 1024 {
        return Err(ApiError::PayloadTooLarge);
    }
    quill_mls_envelope::key_package::parse(&package)
        .map_err(|e| ApiError::BadRequest(format!("KeyPackage refused: {e}")))?;
    drop_expired_links(&state).await?;
    let mut id = [0u8; 16];
    getrandom::fill(&mut id).map_err(|e| ApiError::Internal(anyhow::anyhow!("randomness: {e}")))?;
    sqlx::query(
        "INSERT INTO link_mailbox (link_id, account_id, key_package, expires_at)
         VALUES ($1, $2, $3, now() + make_interval(secs => $4))",
    )
    .bind(id.as_slice())
    .bind(authed.account_id)
    .bind(&package)
    .bind(f64::from(u32::try_from(LINK_SECS).unwrap_or(600)))
    .execute(&state.pool)
    .await?;
    Ok(Json(LinkStarted {
        link_id: b64(&id),
        expires_in_secs: LINK_SECS,
    }))
}

/// `(key_package, welcome, from_seq)`.
type MailboxRow = (Vec<u8>, Option<Vec<u8>>, Option<i64>);

/// `GET /link/{id}`: the KeyPackage, and the reply once there is one. A
/// reply is handed out once: the mailbox is deleted when it's read.
async fn read_link(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<String>,
) -> Result<Json<LinkMailbox>, ApiError> {
    let id = decode("link id", &id)?;
    drop_expired_links(&state).await?;
    let row: Option<MailboxRow> = sqlx::query_as(
        "SELECT key_package, welcome, from_seq FROM link_mailbox
         WHERE link_id = $1 AND account_id = $2",
    )
    .bind(&id)
    .bind(authed.account_id)
    .fetch_optional(&state.pool)
    .await?;
    let (package, welcome, from_seq) = row.ok_or(ApiError::NotFound)?;
    if welcome.is_some() {
        sqlx::query("DELETE FROM link_mailbox WHERE link_id = $1")
            .bind(&id)
            .execute(&state.pool)
            .await?;
    }
    Ok(Json(LinkMailbox {
        key_package: b64(&package),
        welcome: welcome.as_deref().map(b64),
        from_seq: from_seq.map(as_u64),
    }))
}

/// `POST /link/{id}/reply`: the device already in use answers once.
async fn reply_link(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<LinkReply>,
) -> Result<StatusCode, ApiError> {
    let id = decode("link id", &id)?;
    let welcome = decode("welcome", &body.welcome)?;
    if u64::try_from(welcome.len()).unwrap_or(u64::MAX) > state.config.limits.max_message_bytes {
        return Err(ApiError::PayloadTooLarge);
    }
    // An MLSMessage holding a Welcome: MLS 1.0, wire format 3.
    if welcome.get(..4) != Some(&[0, 1, 0, 3]) {
        return Err(ApiError::BadRequest("welcome is not an MLS Welcome".into()));
    }
    drop_expired_links(&state).await?;
    let updated = sqlx::query(
        "UPDATE link_mailbox SET welcome = $3, from_seq = $4
         WHERE link_id = $1 AND account_id = $2 AND welcome IS NULL",
    )
    .bind(&id)
    .bind(authed.account_id)
    .bind(&welcome)
    .bind(i64::try_from(body.from_seq).unwrap_or(i64::MAX))
    .execute(&state.pool)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}
