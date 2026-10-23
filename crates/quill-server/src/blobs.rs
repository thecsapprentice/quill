//! Encrypted attachments (2i): blobs of ciphertext, padded to size
//! buckets, uploaded to a session and fetched with a capability token. The
//! key travels inside the message that carries the blob; the server keeps
//! only the token's hash.

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use quill_proto::{
    keys::{b64, unb64},
    limits::is_blob_bucket,
    sessions::{BLOB_TOKEN_HEADER, BlobUploaded},
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{AppState, auth::Authed, error::ApiError};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/sessions/{id}/blobs",
            // The attachment limit is enforced while reading, below; the
            // API's smaller envelope limit doesn't apply here.
            post(upload).layer(DefaultBodyLimit::disable()),
        )
        .route("/blobs/{id}", get(fetch))
}

fn quota_full() -> ApiError {
    ApiError::Forbidden(
        "Your attachment storage is full; it frees up as your sessions' messages are read.".into(),
    )
}

/// `POST /sessions/{id}/blobs`: the body is the padded ciphertext.
async fn upload(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<BlobUploaded>), ApiError> {
    let group =
        unb64(&id).ok_or_else(|| ApiError::BadRequest("session id is not base64url".into()))?;
    let limits = state.config.limits;
    let max = limits.max_attachment_bytes;
    // Refuse what's declared too big before reading any of it.
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > max) {
        return Err(ApiError::PayloadTooLarge);
    }
    let data = axum::body::to_bytes(body, usize::try_from(max).unwrap_or(usize::MAX))
        .await
        .map_err(|_| ApiError::PayloadTooLarge)?;
    let size = u64::try_from(data.len()).unwrap_or(u64::MAX);
    if !is_blob_bucket(size, max) {
        return Err(ApiError::BadRequest(
            "a blob must be padded to a size bucket (a power of two from 4 KiB, or the limit)"
                .into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    let open: Option<bool> = sqlx::query_scalar(
        "SELECT s.ended_at IS NULL FROM sessions s
         WHERE s.group_id = $1 AND EXISTS (
             SELECT 1 FROM session_members m JOIN actors a ON a.id = m.actor_id
             WHERE m.group_id = s.group_id AND a.account_id = $2)",
    )
    .bind(&group)
    .bind(authed.account_id)
    .fetch_optional(&mut *tx)
    .await?;
    match open {
        None => return Err(ApiError::NotFound),
        Some(false) => return Err(ApiError::Forbidden("The session has ended.".into())),
        Some(true) => {}
    }
    // Serialize a login's uploads so two can't both squeeze under the quota.
    sqlx::query("SELECT id FROM accounts WHERE id = $1 FOR UPDATE")
        .bind(authed.account_id)
        .execute(&mut *tx)
        .await?;
    let used: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(size), 0)::bigint FROM blobs WHERE account_id = $1",
    )
    .bind(authed.account_id)
    .fetch_one(&mut *tx)
    .await?;
    if u64::try_from(used).unwrap_or(u64::MAX).saturating_add(size) > limits.account_quota_bytes {
        return Err(quota_full());
    }
    let mut token = [0u8; 32];
    getrandom::fill(&mut token)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("randomness: {e}")))?;
    let blob = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO blobs (id, group_id, account_id, token_hash, size, data)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(blob)
    .bind(&group)
    .bind(authed.account_id)
    .bind(Sha256::digest(token).to_vec())
    .bind(i64::try_from(size).unwrap_or(i64::MAX))
    .bind(data.as_ref())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(BlobUploaded {
            id: blob,
            token: b64(&token),
        }),
    ))
}

/// `GET /blobs/{id}`: needs a login in the blob's session and the blob's
/// token (header `Blob-Token`). Anything else looks like a missing blob.
async fn fetch(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let token = headers
        .get(BLOB_TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(unb64)
        .ok_or(ApiError::NotFound)?;
    let data: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT b.data FROM blobs b
         WHERE b.id = $1 AND b.token_hash = $2 AND EXISTS (
             SELECT 1 FROM session_members m JOIN actors a ON a.id = m.actor_id
             WHERE m.group_id = b.group_id AND a.account_id = $3)",
    )
    .bind(id)
    .bind(Sha256::digest(&token).to_vec())
    .bind(authed.account_id)
    .fetch_optional(&state.pool)
    .await?;
    let data = data.ok_or(ApiError::NotFound)?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], data).into_response())
}
