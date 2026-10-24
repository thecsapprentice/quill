//! Encrypted attachments (2i): blobs of ciphertext, padded to size
//! buckets, uploaded to a session and fetched with a capability token. The
//! key travels inside the message that carries the blob; the server keeps
//! only the token's hash.
//!
//! The login's personal device group takes blobs too (2k: history
//! bundles). The administrators' attachments switch doesn't apply to it,
//! the quota does, and the login may delete them once read.

use axum::{
    Json,
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use quill_proto::{
    keys::{b64, unb64},
    limits::is_blob_bucket,
    sessions::{BLOB_TOKEN_HEADER, BlobUploaded},
};
use sha2::{Digest, Sha256};
use utoipa_axum::router::UtoipaMethodRouterExt;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{AppState, auth::Authed, error::ApiError};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        // The attachment limit is enforced while reading, below; the API's
        // smaller envelope limit doesn't apply here.
        .routes(routes!(upload).layer(DefaultBodyLimit::disable()))
        .routes(routes!(upload_to_group).layer(DefaultBodyLimit::disable()))
        .routes(routes!(fetch, remove))
}

fn quota_full() -> ApiError {
    ApiError::Forbidden(
        "Your attachment storage is full; it frees up as your sessions' messages are read.".into(),
    )
}

/// `POST /groups/{id}/blobs`: [`upload`], documented for the login's
/// device group.
#[utoipa::path(
    post, path = "/groups/{id}/blobs", tag = "blobs", security(("bearer" = [])),
    summary = "Upload an encrypted blob to a session or the login's own device group",
    description = "As for sessions; the login's device group takes them for history bundles, whatever the attachments setting, and within the quota.",
    params(("id" = String, Path, description = "The group id, base64url")),
    request_body(content = Vec<u8>, content_type = "application/octet-stream", description = "The padded ciphertext: AES-256-GCM output padded with zeros to a size bucket, a power of two from 4 KiB or limits.maxAttachmentBytes itself (see test-vectors/limits.json)"),
    responses(
        (status = 201, body = BlobUploaded),
        (status = 403, description = "The session has ended, the server takes no attachments, or the login's quota is full"),
        (status = 404, description = "Not a group of the login's"),
        (status = 413, description = "Over limits.maxAttachmentBytes"),
    ),
)]
async fn upload_to_group(
    state: State<AppState>,
    authed: Authed,
    id: Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<BlobUploaded>), ApiError> {
    upload(state, authed, id, headers, body).await
}

/// `POST /sessions/{id}/blobs` (or `/groups/{id}/blobs`, which also takes
/// the login's device group): the body is the padded ciphertext.
#[utoipa::path(
    post, path = "/sessions/{id}/blobs", tag = "blobs", security(("bearer" = [])),
    summary = "Upload an encrypted attachment to a session",
    description = "Returns the blob's id and its token, which the server keeps only as a hash; both go inside the MLS message that carries the file's key.",
    params(("id" = String, Path, description = "The session: its group id, base64url")),
    request_body(content = Vec<u8>, content_type = "application/octet-stream", description = "The padded ciphertext: AES-256-GCM output padded with zeros to a size bucket, a power of two from 4 KiB or limits.maxAttachmentBytes itself (see test-vectors/limits.json)"),
    responses(
        (status = 201, body = BlobUploaded),
        (status = 403, description = "The session has ended, the server takes no attachments, or the login's quota is full"),
        (status = 404, description = "Not a session of the login's"),
        (status = 413, description = "Over limits.maxAttachmentBytes"),
    ),
)]
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
    let personal: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM mls_groups
                        WHERE group_id = $1 AND kind = 'personal' AND account_id = $2)",
    )
    .bind(&group)
    .bind(authed.account_id)
    .fetch_one(&mut *tx)
    .await?;
    if !personal {
        if !crate::server_settings::attachments(&mut tx).await? {
            return Err(ApiError::Forbidden(
                "This server doesn't take attachments: its administrators keep sessions to writing."
                    .into(),
            ));
        }
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

fn token_of(headers: &HeaderMap) -> Result<Vec<u8>, ApiError> {
    headers
        .get(BLOB_TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(unb64)
        .ok_or(ApiError::NotFound)
}

/// `GET /blobs/{id}`: needs a login in the blob's session (or whose device
/// group it's in) and the blob's token (header `Blob-Token`). Anything
/// else looks like a missing blob.
#[utoipa::path(
    get, path = "/blobs/{id}", tag = "blobs", security(("bearer" = [])),
    summary = "Fetch a blob",
    description = "Needs a login in the blob's group and the blob's token.",
    params(("id" = Uuid, Path, description = "The blob"), ("Blob-Token" = String, Header, description = "The token the message carried")),
    responses(
        (status = 200, description = "The padded ciphertext", content_type = "application/octet-stream", body = Vec<u8>),
        (status = 404, description = "No such blob, a wrong token, or not in its group (the reply doesn't say which)"),
    ),
)]
async fn fetch(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let token = token_of(&headers)?;
    let data: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT b.data FROM blobs b
         WHERE b.id = $1 AND b.token_hash = $2 AND (EXISTS (
             SELECT 1 FROM session_members m JOIN actors a ON a.id = m.actor_id
             WHERE m.group_id = b.group_id AND a.account_id = $3) OR EXISTS (
             SELECT 1 FROM mls_groups g
             WHERE g.group_id = b.group_id AND g.kind = 'personal' AND g.account_id = $3))",
    )
    .bind(id)
    .bind(Sha256::digest(&token).to_vec())
    .bind(authed.account_id)
    .fetch_optional(&state.pool)
    .await?;
    let data = data.ok_or(ApiError::NotFound)?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], data).into_response())
}

/// `DELETE /blobs/{id}`: a blob in the login's own device group, once the
/// device it was for has it (2k). Session attachments go only with their
/// messages.
#[utoipa::path(
    delete, path = "/blobs/{id}", tag = "blobs", security(("bearer" = [])),
    summary = "Delete a blob in the login's own device group, once read",
    params(("id" = Uuid, Path, description = "The blob"), ("Blob-Token" = String, Header, description = "The token the message carried")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "No such blob in the login's device group, or a wrong token"),
    ),
)]
async fn remove(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let token = token_of(&headers)?;
    let deleted = sqlx::query(
        "DELETE FROM blobs b USING mls_groups g
         WHERE b.id = $1 AND b.token_hash = $2 AND g.group_id = b.group_id
           AND g.kind = 'personal' AND g.account_id = $3",
    )
    .bind(id)
    .bind(Sha256::digest(&token).to_vec())
    .bind(authed.account_id)
    .execute(&state.pool)
    .await?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}
