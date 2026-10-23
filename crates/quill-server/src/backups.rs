//! Encrypted backups (2j). Each is an `age` file sealed with a recovery
//! code the server never sees. It's uploaded in parts under a new
//! generation and committed as a whole, replacing the login's previous
//! backup only once the new one is complete.

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use quill_proto::{
    limits::BACKUP_PART_BYTES,
    sessions::{BackupInfo, CommitBackup},
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{AppState, auth::Authed, error::ApiError, routes::ApiJson};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/backup", get(info).delete(remove))
        .route("/backup/{part}", get(fetch))
        .route(
            "/backup/{generation}/{part}",
            // Parts are larger than the API's envelope limit; the part limit
            // is enforced while reading, below.
            put(upload).layer(DefaultBodyLimit::disable()),
        )
        .route("/backup/{generation}/commit", post(commit))
}

fn too_big(max: u64) -> ApiError {
    ApiError::Forbidden(format!(
        "A backup can be at most {} MiB here.",
        max / (1024 * 1024)
    ))
}

/// `PUT /backup/{generation}/{part}`: one part of a new backup.
async fn upload(
    State(state): State<AppState>,
    authed: Authed,
    Path((generation, part)): Path<(Uuid, u32)>,
    headers: HeaderMap,
    body: Body,
) -> Result<StatusCode, ApiError> {
    let max = state.config.limits.max_backup_bytes;
    if u64::from(part) * BACKUP_PART_BYTES >= max {
        return Err(too_big(max));
    }
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > BACKUP_PART_BYTES) {
        return Err(ApiError::PayloadTooLarge);
    }
    let data = axum::body::to_bytes(
        body,
        usize::try_from(BACKUP_PART_BYTES).unwrap_or(usize::MAX),
    )
    .await
    .map_err(|_| ApiError::PayloadTooLarge)?;
    if data.is_empty() {
        return Err(ApiError::BadRequest("an empty part".into()));
    }
    let mut tx = state.pool.begin().await?;
    // One upload at a time per login; a new generation drops unfinished
    // ones.
    sqlx::query("SELECT id FROM accounts WHERE id = $1 FOR UPDATE")
        .bind(authed.account_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "DELETE FROM backup_parts WHERE account_id = $1 AND generation <> $2
           AND generation <> coalesce((SELECT generation FROM backups WHERE account_id = $1),
                                      '00000000-0000-0000-0000-000000000000')",
    )
    .bind(authed.account_id)
    .bind(generation)
    .execute(&mut *tx)
    .await?;
    let used: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(octet_length(data)), 0)::bigint FROM backup_parts
         WHERE account_id = $1 AND generation = $2 AND part <> $3",
    )
    .bind(authed.account_id)
    .bind(generation)
    .bind(i32::try_from(part).unwrap_or(i32::MAX))
    .fetch_one(&mut *tx)
    .await?;
    if u64::try_from(used).unwrap_or(u64::MAX) + data.len() as u64 > max {
        return Err(too_big(max));
    }
    sqlx::query(
        "INSERT INTO backup_parts (account_id, generation, part, data) VALUES ($1, $2, $3, $4)
         ON CONFLICT (account_id, generation, part) DO UPDATE SET data = EXCLUDED.data",
    )
    .bind(authed.account_id)
    .bind(generation)
    .bind(i32::try_from(part).unwrap_or(i32::MAX))
    .bind(data.as_ref())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /backup/{generation}/commit`: parts `0..parts` are all there; the
/// new backup replaces the old.
async fn commit(
    State(state): State<AppState>,
    authed: Authed,
    Path(generation): Path<Uuid>,
    ApiJson(body): ApiJson<CommitBackup>,
) -> Result<Json<BackupInfo>, ApiError> {
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM accounts WHERE id = $1 FOR UPDATE")
        .bind(authed.account_id)
        .execute(&mut *tx)
        .await?;
    let (count, max_part, bytes): (i64, Option<i32>, i64) = sqlx::query_as(
        "SELECT count(*), max(part), coalesce(sum(octet_length(data)), 0)::bigint
         FROM backup_parts WHERE account_id = $1 AND generation = $2",
    )
    .bind(authed.account_id)
    .bind(generation)
    .fetch_one(&mut *tx)
    .await?;
    let parts = i64::from(body.parts);
    if parts == 0 || count != parts || max_part.map(i64::from) != Some(parts - 1) {
        return Err(ApiError::BadRequest(
            "the backup's parts aren't all there".into(),
        ));
    }
    let created_at: OffsetDateTime = sqlx::query_scalar(
        "INSERT INTO backups (account_id, generation, parts, bytes) VALUES ($1, $2, $3, $4)
         ON CONFLICT (account_id) DO UPDATE SET generation = EXCLUDED.generation,
             parts = EXCLUDED.parts, bytes = EXCLUDED.bytes, created_at = now()
         RETURNING created_at",
    )
    .bind(authed.account_id)
    .bind(generation)
    .bind(i32::try_from(body.parts).unwrap_or(i32::MAX))
    .bind(bytes)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM backup_parts WHERE account_id = $1 AND generation <> $2")
        .bind(authed.account_id)
        .bind(generation)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(BackupInfo {
        generation,
        parts: body.parts,
        bytes: u64::try_from(bytes).unwrap_or(0),
        created_at,
    }))
}

/// `GET /backup`: the committed backup, if any.
async fn info(State(state): State<AppState>, authed: Authed) -> Result<Json<BackupInfo>, ApiError> {
    let row: Option<(Uuid, i32, i64, OffsetDateTime)> = sqlx::query_as(
        "SELECT generation, parts, bytes, created_at FROM backups WHERE account_id = $1",
    )
    .bind(authed.account_id)
    .fetch_optional(&state.pool)
    .await?;
    let (generation, parts, bytes, created_at) = row.ok_or(ApiError::NotFound)?;
    Ok(Json(BackupInfo {
        generation,
        parts: u32::try_from(parts).unwrap_or(0),
        bytes: u64::try_from(bytes).unwrap_or(0),
        created_at,
    }))
}

/// `GET /backup/{part}`: one part of the committed backup.
async fn fetch(
    State(state): State<AppState>,
    authed: Authed,
    Path(part): Path<u32>,
) -> Result<Response, ApiError> {
    let data: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT p.data FROM backup_parts p JOIN backups b
           ON b.account_id = p.account_id AND b.generation = p.generation
         WHERE p.account_id = $1 AND p.part = $2",
    )
    .bind(authed.account_id)
    .bind(i32::try_from(part).unwrap_or(i32::MAX))
    .fetch_optional(&state.pool)
    .await?;
    let data = data.ok_or(ApiError::NotFound)?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], data).into_response())
}

/// `DELETE /backup`: the backup and any unfinished upload.
async fn remove(State(state): State<AppState>, authed: Authed) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM backups WHERE account_id = $1")
        .bind(authed.account_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM backup_parts WHERE account_id = $1")
        .bind(authed.account_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
