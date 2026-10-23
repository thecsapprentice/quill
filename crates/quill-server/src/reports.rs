//! Prompt reports, the administrators' inbox and the append-only admin log.

use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use quill_proto::board::{
    AdminAction, AdminLogEntry, CreateReport, Report, ReportQuery, ReportReason, Resolution,
    ResolveReport,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::{AssertSqlSafe, PgConnection};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::{Admin, MaybeAuthed},
    error::ApiError,
    prompts::load_prompt,
    routes::ApiJson,
};

/// Reports one signed-in account may file per hour.
const ACCOUNT_REPORTS_PER_HOUR: usize = 20;
/// Reports all anonymous visitors together may file per hour. There is no
/// per-visitor limit because the server does not track IP addresses.
const ANONYMOUS_REPORTS_PER_HOUR: usize = 60;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/reports", post(create_report))
        .route("/admin/reports", get(list_reports))
        .route("/admin/reports/{id}/resolve", post(resolve_report))
        .route("/admin/log", get(read_log))
}

/// SHA-256 of the prompt's title and body, recorded instead of its text.
/// Each field is length-prefixed so different title/body splits can't
/// produce the same hash.
pub fn prompt_hash(title: &str, body: &str) -> Vec<u8> {
    let mut h = Sha256::new();
    for field in [title, body] {
        h.update((field.len() as u64).to_be_bytes());
        h.update(field.as_bytes());
    }
    h.finalize().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_reason(raw: &str) -> Result<ReportReason, ApiError> {
    raw.parse()
        .map_err(|_| ApiError::Internal(anyhow::anyhow!("unexpected report reason {raw:?}")))
}

/// `POST /reports`. Files a Review Message in the administrators' inbox.
/// The reporter's account, if any, is kept for abuse control only.
async fn create_report(
    State(state): State<AppState>,
    MaybeAuthed(authed): MaybeAuthed,
    ApiJson(body): ApiJson<CreateReport>,
) -> Result<StatusCode, ApiError> {
    file_report(&state, authed.map(|a| a.account_id), &body).await?;
    Ok(StatusCode::ACCEPTED)
}

/// Files a report from `reporter` (an account id, or anonymous).
pub async fn file_report(
    state: &AppState,
    reporter: Option<Uuid>,
    body: &CreateReport,
) -> Result<(), ApiError> {
    let details = quill_proto::validate::report_details(body.details.as_deref())?;
    let (key, max) = match reporter {
        Some(id) => (format!("report:{id}"), ACCOUNT_REPORTS_PER_HOUR),
        None => ("report:anonymous".to_owned(), ANONYMOUS_REPORTS_PER_HOUR),
    };
    if !state.limiter.allow(&key, max, Duration::from_secs(60 * 60)) {
        return Err(ApiError::TooManyRequests);
    }

    let row: Option<(String, String)> =
        sqlx::query_as("SELECT title, body_md FROM prompts WHERE id = $1 AND removed_at IS NULL")
            .bind(body.prompt)
            .fetch_optional(&state.pool)
            .await?;
    let (title, text) = row.ok_or(ApiError::NotFound)?;
    let hash = prompt_hash(&title, &text);

    let mut tx = state.pool.begin().await?;
    let report_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO reports (id, prompt_id, reporter_account, reason, details, prompt_hash)
         VALUES ($1, $2, $3, $4::report_reason, $5, $6)",
    )
    .bind(report_id)
    .bind(body.prompt)
    .bind(reporter)
    .bind(body.reason.as_str())
    .bind(details)
    .bind(&hash)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO admin_log (action, report_id, prompt_id, prompt_hash, reason)
         VALUES ('report_received', $1, $2, $3, $4::report_reason)",
    )
    .bind(report_id)
    .bind(body.prompt)
    .bind(&hash)
    .bind(body.reason.as_str())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

type ReportRow = (
    Uuid,
    Option<Uuid>,
    String,
    Option<String>,
    Vec<u8>,
    OffsetDateTime,
    Option<OffsetDateTime>,
    Option<String>,
);

const REPORT_SELECT: &str = "SELECT id, prompt_id, reason::text, details, prompt_hash, \
     created_at, resolved_at, resolution FROM reports";

async fn to_report(state: &AppState, row: ReportRow) -> Result<Report, ApiError> {
    let (id, prompt_id, reason, details, hash, created_at, resolved_at, resolution) = row;
    let prompt = match prompt_id {
        Some(pid) => load_prompt(&state.pool, pid, None, true).await?,
        None => None,
    };
    Ok(Report {
        id,
        prompt,
        reason: parse_reason(&reason)?,
        details,
        prompt_hash: hex(&hash),
        created_at,
        resolved_at,
        resolution: resolution
            .map(|r| r.parse::<Resolution>())
            .transpose()
            .map_err(|e| ApiError::Internal(e.into()))?,
    })
}

/// `GET /admin/reports?state=open|all`, oldest open reports first.
async fn list_reports(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<ReportQuery>,
) -> Result<Json<Vec<Report>>, ApiError> {
    let all = match q.state.as_deref() {
        None | Some("open") => false,
        Some("all") => true,
        Some(_) => return Err(ApiError::BadRequest("state must be open or all".into())),
    };
    reports(&state, all).await.map(Json)
}

/// The administrators' inbox: open reports oldest first, or every report
/// newest first.
pub async fn reports(state: &AppState, all: bool) -> Result<Vec<Report>, ApiError> {
    let sql = if all {
        format!("{REPORT_SELECT} ORDER BY created_at DESC LIMIT 200")
    } else {
        format!("{REPORT_SELECT} WHERE resolved_at IS NULL ORDER BY created_at LIMIT 200")
    };
    // Safe: `sql` is one of two strings built from compile-time constants.
    let rows: Vec<ReportRow> = sqlx::query_as(AssertSqlSafe(sql))
        .fetch_all(&state.pool)
        .await?;
    let mut reports = Vec::with_capacity(rows.len());
    for row in rows {
        reports.push(to_report(state, row).await?);
    }
    Ok(reports)
}

/// `POST /admin/reports/{id}/resolve`: discard the report, or remove the
/// prompt from the board. Both are logged.
async fn resolve_report(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<Uuid>,
    ApiJson(body): ApiJson<ResolveReport>,
) -> Result<Json<Report>, ApiError> {
    resolve(&state, admin.account_id, id, body.action)
        .await
        .map(Json)
}

/// Resolves a report as administrator `admin`.
pub async fn resolve(
    state: &AppState,
    admin: Uuid,
    id: Uuid,
    action: Resolution,
) -> Result<Report, ApiError> {
    let mut tx = state.pool.begin().await?;
    let row: Option<(Option<Uuid>, String, Vec<u8>, bool)> = sqlx::query_as(
        "SELECT prompt_id, reason::text, prompt_hash, resolved_at IS NOT NULL
         FROM reports WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (prompt_id, reason, hash, resolved) = row.ok_or(ApiError::NotFound)?;
    if resolved {
        return Err(ApiError::Conflict("report already resolved".into()));
    }
    let logged = match action {
        Resolution::Discard => "discard",
        Resolution::Delete => {
            // Phase 3 also sends a federated `Delete` for local prompts.
            sqlx::query(
                "UPDATE prompts SET removed_at = now() WHERE id = $1 AND removed_at IS NULL",
            )
            .bind(prompt_id)
            .execute(&mut *tx)
            .await?;
            "delete_prompt"
        }
    };
    sqlx::query(
        "UPDATE reports SET resolved_at = now(), resolution = $2, resolved_by = $3 WHERE id = $1",
    )
    .bind(id)
    .bind(action.as_str())
    .bind(admin)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO admin_log (action, report_id, prompt_id, prompt_hash, reason, admin_id)
         VALUES ($1, $2, $3, $4, $5::report_reason, $6)",
    )
    .bind(logged)
    .bind(id)
    .bind(prompt_id)
    .bind(&hash)
    .bind(&reason)
    .bind(admin)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    // Safe: built only from compile-time constants; values are bound.
    let sql = AssertSqlSafe(format!("{REPORT_SELECT} WHERE id = $1"));
    let row: ReportRow = sqlx::query_as(sql).bind(id).fetch_one(&state.pool).await?;
    to_report(state, row).await
}

#[derive(Debug, Deserialize)]
struct LogQuery {
    /// Return entries older than this id.
    before: Option<i64>,
    limit: Option<u32>,
}

/// Records an administrator's change to something other than a report.
pub(crate) async fn log_action(
    conn: &mut PgConnection,
    admin: Uuid,
    action: AdminAction,
    subject: &str,
    detail: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO admin_log (action, subject, detail, admin_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(action.as_str())
    .bind(subject)
    .bind(detail)
    .bind(admin)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Records a change made by the server itself, such as syncing the
/// reserved-handles file at startup. No administrator is named.
pub(crate) async fn log_system(
    conn: &mut PgConnection,
    action: AdminAction,
    subject: &str,
    detail: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO admin_log (action, subject, detail) VALUES ($1, $2, $3)")
        .bind(action.as_str())
        .bind(subject)
        .bind(detail)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

type LogRow = (
    i64,
    OffsetDateTime,
    String,
    Option<Uuid>,
    Option<Uuid>,
    Option<Vec<u8>>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<Uuid>,
);

/// `GET /admin/log`, newest first.
async fn read_log(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<LogQuery>,
) -> Result<Json<Vec<AdminLogEntry>>, ApiError> {
    admin_log(&state, q.before, q.limit).await.map(Json)
}

/// The admin log, newest first, optionally before entry `before`.
pub async fn admin_log(
    state: &AppState,
    before: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<AdminLogEntry>, ApiError> {
    let limit = i64::from(limit.unwrap_or(50).clamp(1, 200));
    let rows: Vec<LogRow> = sqlx::query_as(
        "SELECT id, at, action, report_id, prompt_id, prompt_hash, reason::text, subject, detail,
                admin_id
         FROM admin_log WHERE ($1::bigint IS NULL OR id < $1)
         ORDER BY id DESC LIMIT $2",
    )
    .bind(before)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    rows.into_iter()
        .map(
            |(id, at, action, report_id, prompt_id, hash, reason, subject, detail, admin)| {
                Ok(AdminLogEntry {
                    id,
                    at,
                    action: action.parse().map_err(|_| {
                        ApiError::Internal(anyhow::anyhow!("unexpected log action {action:?}"))
                    })?,
                    report_id,
                    prompt_id,
                    prompt_hash: hash.as_deref().map(hex),
                    reason: reason.as_deref().map(parse_reason).transpose()?,
                    subject,
                    detail,
                    admin,
                })
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_separates_fields() {
        assert_eq!(prompt_hash("a", "b"), prompt_hash("a", "b"));
        assert_ne!(prompt_hash("a\nb", ""), prompt_hash("a", "b\n"));
        assert_eq!(hex(&[0, 255, 16]), "00ff10");
    }
}
