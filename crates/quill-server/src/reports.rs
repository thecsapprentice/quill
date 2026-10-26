//! Prompt reports, the administrators' inbox and the append-only admin log.

use std::time::Duration;
use utoipa_axum::{router::OpenApiRouter, routes};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use quill_proto::board::{
    AdminAction, AdminLogDetail, AdminLogEntry, AdminLogPrompt, AdminLogQuery, AdminLogTarget,
    AdminLogTargetQuery, CreateReport, LOG_TARGET_PREVIEW, Report, ReportKind, ReportQuery,
    ReportReason, Resolution, ResolveReport,
};
use sha2::{Digest, Sha256};
use sqlx::{AssertSqlSafe, PgConnection};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::{Admin, MaybeAuthed},
    error::ApiError,
    notices,
    prompts::load_prompt,
    routes::ApiJson,
};
use quill_proto::{
    announcements::ReportComment,
    notices::{NoticeFields, NoticeKind, NoticePrompt},
};

/// Reports one signed-in account may file per hour.
const ACCOUNT_REPORTS_PER_HOUR: usize = 20;
/// Reports all anonymous visitors together may file per hour. There is no
/// per-visitor limit because the server does not track IP addresses.
const ANONYMOUS_REPORTS_PER_HOUR: usize = 60;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(create_report))
        .routes(routes!(list_reports))
        .routes(routes!(resolve_report))
        .routes(routes!(read_log))
        .routes(routes!(read_log_entry))
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

/// SHA-256 of a comment's text, recorded instead of the text. Length-
/// prefixed like [`prompt_hash`].
pub fn comment_hash(body: &str) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update((body.len() as u64).to_be_bytes());
    h.update(body.as_bytes());
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
#[utoipa::path(
    post, path = "/reports", tag = "reports", security((), ("bearer" = [])),
    summary = "Report a prompt to the administrators",
    description = "Signing in is optional. The author never learns who reported, and reports are rate-limited per reporter.",
    request_body = CreateReport,
    responses(
        (status = 202, description = "Received"),
        (status = 404, description = "No such prompt"),
        (status = 429, description = "Too many reports"),
    ),
)]
async fn create_report(
    State(state): State<AppState>,
    MaybeAuthed(authed): MaybeAuthed,
    ApiJson(body): ApiJson<CreateReport>,
) -> Result<StatusCode, ApiError> {
    file_report(&state, authed.map(|a| a.account_id), &body).await?;
    Ok(StatusCode::ACCEPTED)
}

/// Counts a report against its reporter's hourly limit, prompts and
/// comments together.
fn allow_report(state: &AppState, reporter: Option<Uuid>) -> Result<(), ApiError> {
    let (key, max) = match reporter {
        Some(id) => (format!("report:{id}"), ACCOUNT_REPORTS_PER_HOUR),
        None => ("report:anonymous".to_owned(), ANONYMOUS_REPORTS_PER_HOUR),
    };
    if state.limiter.allow(&key, max, Duration::from_secs(60 * 60)) {
        Ok(())
    } else {
        Err(ApiError::TooManyRequests)
    }
}

/// Files a report from `reporter` (an account id, or anonymous).
pub async fn file_report(
    state: &AppState,
    reporter: Option<Uuid>,
    body: &CreateReport,
) -> Result<(), ApiError> {
    let details = quill_proto::validate::report_details(body.details.as_deref())?;
    allow_report(state, reporter)?;

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
    let log_id: i64 = sqlx::query_scalar(
        "INSERT INTO admin_log (action, report_id, prompt_id, prompt_hash, reason, actor)
         VALUES ('report_received', $1, $2, $3, $4::report_reason, 'reporter') RETURNING id",
    )
    .bind(report_id)
    .bind(body.prompt)
    .bind(&hash)
    .bind(body.reason.as_str())
    .fetch_one(&mut *tx)
    .await?;
    let author = notices::affected_by_prompt(&mut tx, body.prompt).await?;
    log_targets(&mut tx, log_id, &author).await?;
    tx.commit().await?;
    Ok(())
}

/// Files a report about a comment from `reporter`. Removed comments, and
/// those on a deleted announcement, can't be reported.
pub async fn file_comment_report(
    state: &AppState,
    reporter: Option<Uuid>,
    comment: Uuid,
    body: &ReportComment,
) -> Result<(), ApiError> {
    let details = quill_proto::validate::report_details(body.details.as_deref())?;
    // Only those who may read comments can report them.
    crate::comments::check_readable(state, reporter).await?;
    allow_report(state, reporter)?;
    let text: Option<String> = sqlx::query_scalar(
        "SELECT c.body_md FROM announcement_comments c
         JOIN announcements an ON an.id = c.announcement_id
         WHERE c.id = $1 AND c.removed_at IS NULL AND an.deleted_at IS NULL",
    )
    .bind(comment)
    .fetch_optional(&state.pool)
    .await?;
    let hash = comment_hash(&text.ok_or(ApiError::NotFound)?);

    let mut tx = state.pool.begin().await?;
    let report_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO reports (id, kind, comment_id, reporter_account, reason, details, prompt_hash)
         VALUES ($1, 'comment', $2, $3, $4::report_reason, $5, $6)",
    )
    .bind(report_id)
    .bind(comment)
    .bind(reporter)
    .bind(body.reason.as_str())
    .bind(details)
    .bind(&hash)
    .execute(&mut *tx)
    .await?;
    let log_id: i64 = sqlx::query_scalar(
        "INSERT INTO admin_log (action, report_id, comment_id, prompt_hash, reason, actor)
         VALUES ('report_received', $1, $2, $3, $4::report_reason, 'reporter') RETURNING id",
    )
    .bind(report_id)
    .bind(comment)
    .bind(&hash)
    .bind(body.reason.as_str())
    .fetch_one(&mut *tx)
    .await?;
    let author = notices::affected_by_comment(&mut tx, comment).await?;
    log_targets(&mut tx, log_id, &author).await?;
    tx.commit().await?;
    Ok(())
}

type ReportRow = (
    Uuid,
    String,
    Option<Uuid>,
    Option<Uuid>,
    String,
    Option<String>,
    Vec<u8>,
    OffsetDateTime,
    Option<OffsetDateTime>,
    Option<String>,
);

const REPORT_SELECT: &str = "SELECT id, kind, prompt_id, comment_id, reason::text, details, \
     prompt_hash, created_at, resolved_at, resolution FROM reports";

async fn to_report(state: &AppState, row: ReportRow) -> Result<Report, ApiError> {
    let (
        id,
        kind,
        prompt_id,
        comment_id,
        reason,
        details,
        hash,
        created_at,
        resolved_at,
        resolution,
    ) = row;
    let prompt = match prompt_id {
        Some(pid) => load_prompt(&state.pool, pid, None, true).await?,
        None => None,
    };
    let comment = match comment_id {
        Some(cid) => crate::comments::load(&state.pool, cid).await?,
        None => None,
    };
    Ok(Report {
        id,
        kind: crate::prompts::parse_enum(&kind)?,
        prompt,
        comment,
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
#[utoipa::path(
    get, path = "/admin/reports", tag = "admin", security(("bearer" = [])),
    summary = "Reports, open ones by default",
    params(ReportQuery),
    responses((status = 200, body = Vec<Report>)),
)]
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
#[utoipa::path(
    post, path = "/admin/reports/{id}/resolve", tag = "admin", security(("bearer" = [])),
    summary = "Resolve a report: dismiss it, or remove or restore the prompt",
    params(("id" = Uuid, Path, description = "The report")),
    request_body = ResolveReport,
    responses((status = 200, body = Report), (status = 404, description = "No such report")),
)]
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
    /// `(kind, prompt, comment, reason, hash, resolved)`.
    type Locked = (String, Option<Uuid>, Option<Uuid>, String, Vec<u8>, bool);
    let row: Option<Locked> = sqlx::query_as(
        "SELECT kind, prompt_id, comment_id, reason::text, prompt_hash, resolved_at IS NOT NULL
             FROM reports WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (kind, prompt_id, comment_id, reason, hash, resolved) = row.ok_or(ApiError::NotFound)?;
    if resolved {
        return Err(ApiError::Conflict("report already resolved".into()));
    }
    if kind == ReportKind::Comment.as_str() {
        resolve_comment_report(&mut tx, admin, id, comment_id, &reason, &hash, action).await?;
        tx.commit().await?;
        return report(state, id).await;
    }
    let logged = match action {
        Resolution::Discard => "discard",
        Resolution::Delete => {
            // Federation (Phase 5) also sends a federated `Delete` for local prompts.
            sqlx::query(
                "UPDATE prompts SET removed_at = now() WHERE id = $1 AND removed_at IS NULL",
            )
            .bind(prompt_id)
            .execute(&mut *tx)
            .await?;
            // Its waiting requests go nowhere: requesters see "no response",
            // and the author can't accept them. Running sessions carry on.
            if let Some(prompt) = prompt_id {
                crate::requests::expire_for_prompt(&mut tx, prompt).await?;
            }
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
    let log_id: i64 = sqlx::query_scalar(
        "INSERT INTO admin_log
             (action, report_id, prompt_id, prompt_hash, reason, admin_id, actor, admin_login)
         VALUES ($1, $2, $3, $4, $5::report_reason, $6, 'admin',
                 (SELECT login_name FROM accounts WHERE id = $6))
         RETURNING id",
    )
    .bind(logged)
    .bind(id)
    .bind(prompt_id)
    .bind(&hash)
    .bind(&reason)
    .bind(admin)
    .fetch_one(&mut *tx)
    .await?;
    // The author is told their prompt was removed and the report's category,
    // never who reported it. A discarded report tells nobody anything.
    let affected = match prompt_id {
        Some(prompt) => notices::affected_by_prompt(&mut tx, prompt).await?,
        None => Vec::new(),
    };
    // The decision was about the author's prompt either way, so the log
    // names them; only a removal tells them.
    log_targets(&mut tx, log_id, &affected).await?;
    if action == Resolution::Delete && prompt_id.is_some() {
        let fields = NoticeFields {
            reason: Some(parse_reason(&reason)?),
            ..NoticeFields::default()
        };
        notices::record(
            &mut tx,
            log_id,
            NoticeKind::PromptRemoved,
            &fields,
            &affected,
        )
        .await?;
    }
    tx.commit().await?;
    report(state, id).await
}

/// One report, as the inbox shows it.
async fn report(state: &AppState, id: Uuid) -> Result<Report, ApiError> {
    // Safe: built only from compile-time constants; values are bound.
    let sql = AssertSqlSafe(format!("{REPORT_SELECT} WHERE id = $1"));
    let row: ReportRow = sqlx::query_as(sql).bind(id).fetch_one(&state.pool).await?;
    to_report(state, row).await
}

/// Resolves a report about a comment: removing the comment works as a
/// direct removal does, with the report named in the log.
async fn resolve_comment_report(
    conn: &mut PgConnection,
    admin: Uuid,
    id: Uuid,
    comment: Option<Uuid>,
    reason: &str,
    hash: &[u8],
    action: Resolution,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE reports SET resolved_at = now(), resolution = $2, resolved_by = $3 WHERE id = $1",
    )
    .bind(id)
    .bind(action.as_str())
    .bind(admin)
    .execute(&mut *conn)
    .await?;
    let reason = parse_reason(reason)?;
    match (action, comment) {
        (Resolution::Delete, Some(comment)) => {
            crate::comments::remove_locked(conn, admin, comment, reason, Some(id)).await
        }
        // The author deleted it already: nothing left to remove.
        (Resolution::Delete, None) => Err(ApiError::Conflict(
            "The comment was deleted by its author; discard the report instead.".into(),
        )),
        (Resolution::Discard, _) => {
            let log_id: i64 = sqlx::query_scalar(
                "INSERT INTO admin_log
                     (action, report_id, comment_id, prompt_hash, reason, admin_id, actor, admin_login)
                 VALUES ('discard', $1, $2, $3, $4::report_reason, $5, 'admin',
                         (SELECT login_name FROM accounts WHERE id = $5))
                 RETURNING id",
            )
            .bind(id)
            .bind(comment)
            .bind(hash)
            .bind(reason.as_str())
            .bind(admin)
            .fetch_one(&mut *conn)
            .await?;
            let affected = match comment {
                Some(c) => notices::affected_by_comment(conn, c).await?,
                None => Vec::new(),
            };
            log_targets(conn, log_id, &affected).await
        }
    }
}

/// Records an administrator's change to something other than a report.
pub(crate) async fn log_action(
    conn: &mut PgConnection,
    admin: Uuid,
    action: AdminAction,
    subject: &str,
    detail: Option<&str>,
) -> Result<i64, ApiError> {
    Ok(sqlx::query_scalar(
        "INSERT INTO admin_log (action, subject, detail, admin_id, actor, admin_login)
         VALUES ($1, $2, $3, $4, 'admin', (SELECT login_name FROM accounts WHERE id = $4))
         RETURNING id",
    )
    .bind(action.as_str())
    .bind(subject)
    .bind(detail)
    .bind(admin)
    .fetch_one(&mut *conn)
    .await?)
}

/// Records a change made by the server itself, such as syncing the
/// reserved-handles file at startup. No administrator is named.
pub(crate) async fn log_system(
    conn: &mut PgConnection,
    action: AdminAction,
    subject: &str,
    detail: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO admin_log (action, subject, detail, actor) VALUES ($1, $2, $3, 'server')",
    )
    .bind(action.as_str())
    .bind(subject)
    .bind(detail)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct LogRow {
    id: i64,
    at: OffsetDateTime,
    action: String,
    report_id: Option<Uuid>,
    prompt_id: Option<Uuid>,
    comment_id: Option<Uuid>,
    prompt_hash: Option<Vec<u8>>,
    reason: Option<String>,
    subject: Option<String>,
    detail: Option<String>,
    admin_id: Option<Uuid>,
    actor: String,
    admin_login: Option<String>,
    targets: Vec<String>,
    target_count: i64,
}

impl LogRow {
    fn into_entry(self) -> Result<AdminLogEntry, ApiError> {
        Ok(AdminLogEntry {
            id: self.id,
            at: self.at,
            action: self.action.parse().map_err(|_| {
                ApiError::Internal(anyhow::anyhow!("unexpected log action {:?}", self.action))
            })?,
            report_id: self.report_id,
            prompt_id: self.prompt_id,
            comment_id: self.comment_id,
            prompt_hash: self.prompt_hash.as_deref().map(hex),
            reason: self.reason.as_deref().map(parse_reason).transpose()?,
            subject: self.subject,
            detail: self.detail,
            admin: self.admin_id,
            actor: crate::prompts::parse_enum(&self.actor)?,
            admin_login: self.admin_login,
            targets: self.targets,
            target_count: u64::try_from(self.target_count).unwrap_or(0),
        })
    }
}

/// `GET /admin/log`, newest first.
#[utoipa::path(
    get, path = "/admin/log", tag = "admin", security(("bearer" = [])),
    summary = "The append-only administrator log, newest first",
    description = "Pages by `before`, the last id of the previous page.",
    params(AdminLogQuery),
    responses((status = 200, body = Vec<AdminLogEntry>)),
)]
async fn read_log(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<AdminLogQuery>,
) -> Result<Json<Vec<AdminLogEntry>>, ApiError> {
    admin_log(&state, &q).await.map(Json)
}

/// `GET /admin/log/{id}`: one entry and a page of the accounts it was against.
#[utoipa::path(
    get, path = "/admin/log/{id}", tag = "admin", security(("bearer" = [])),
    summary = "One log entry, with a page of the accounts it touched",
    params(("id" = i64, Path, description = "The entry"), AdminLogTargetQuery),
    responses((status = 200, body = AdminLogDetail), (status = 404, description = "No such entry")),
)]
async fn read_log_entry(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<i64>,
    Query(q): Query<AdminLogTargetQuery>,
) -> Result<Json<AdminLogDetail>, ApiError> {
    admin_log_entry(&state, id, &q).await.map(Json)
}

/// Entries matching the filters, newest first. Each names only the first
/// few accounts it was against (an account filtered for comes first), and
/// counts them all: one decision can reach thousands.
async fn fetch_log(
    state: &AppState,
    q: &AdminLogQuery,
    only: Option<i64>,
) -> Result<Vec<AdminLogEntry>, ApiError> {
    let limit = i64::from(q.limit.unwrap_or(50).clamp(1, 200));
    let admin = q.admin.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let account = q
        .account
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let rows: Vec<LogRow> = sqlx::query_as(
        "SELECT l.id, l.at, l.action, l.report_id, l.prompt_id, l.comment_id, l.prompt_hash,
                l.reason::text AS reason, l.subject, l.detail, l.admin_id, l.actor, l.admin_login,
                ARRAY(SELECT t.login FROM admin_log_targets t WHERE t.log_id = l.id
                      ORDER BY t.login = lower($4) DESC, t.login LIMIT $6) AS targets,
                (SELECT count(*) FROM admin_log_targets t WHERE t.log_id = l.id) AS target_count
         FROM admin_log l
         WHERE ($1::bigint IS NULL OR l.id < $1)
           AND ($5::bigint IS NULL OR l.id = $5)
           AND ($3::text IS NULL OR l.admin_login = lower($3))
           AND ($4::text IS NULL OR EXISTS (
                   SELECT 1 FROM admin_log_targets t
                   WHERE t.log_id = l.id AND t.login = lower($4)))
         ORDER BY l.id DESC LIMIT $2",
    )
    .bind(q.before)
    .bind(limit)
    .bind(admin)
    .bind(account)
    .bind(only)
    .bind(i64::try_from(LOG_TARGET_PREVIEW).unwrap_or(5))
    .fetch_all(&state.pool)
    .await?;
    rows.into_iter().map(LogRow::into_entry).collect()
}

/// The admin log, newest first, optionally before entry `before`.
pub async fn admin_log(
    state: &AppState,
    q: &AdminLogQuery,
) -> Result<Vec<AdminLogEntry>, ApiError> {
    fetch_log(state, q, None).await
}

/// `(login, account deleted, the prompts its notice recorded)`.
type TargetRow = (String, bool, Option<sqlx::types::Json<Vec<NoticePrompt>>>);

/// One entry, with a page of the accounts it was against (alphabetical,
/// after `q.after`) and each one's prompts as the decision found them.
pub async fn admin_log_entry(
    state: &AppState,
    id: i64,
    q: &AdminLogTargetQuery,
) -> Result<AdminLogDetail, ApiError> {
    let entry = fetch_log(state, &AdminLogQuery::default(), Some(id))
        .await?
        .pop()
        .ok_or(ApiError::NotFound)?;
    let limit = i64::from(q.limit.unwrap_or(100).clamp(1, 500));
    let after = q.after.as_deref().filter(|s| !s.is_empty());
    // One extra row says whether there's another page.
    let mut rows: Vec<TargetRow> = sqlx::query_as(
        "SELECT t.login, t.account_id IS NULL,
                    (SELECT n.prompts FROM account_notices n
                     WHERE n.log_id = t.log_id AND n.account_id = t.account_id
                     ORDER BY n.id LIMIT 1)
             FROM admin_log_targets t
             WHERE t.log_id = $1 AND ($2::text IS NULL OR t.login > $2)
             ORDER BY t.login LIMIT $3",
    )
    .bind(id)
    .bind(after)
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await?;
    let next = if rows.len() > usize::try_from(limit).unwrap_or(usize::MAX) {
        rows.pop();
        rows.last().map(|r| r.0.clone())
    } else {
        None
    };
    // Which of the page's prompts are still on the server.
    let ids: Vec<Uuid> = rows
        .iter()
        .filter_map(|r| r.2.as_ref())
        .flat_map(|p| p.0.iter().map(|p| p.id))
        .collect();
    let existing: std::collections::HashSet<Uuid> = if ids.is_empty() {
        Default::default()
    } else {
        sqlx::query_scalar("SELECT id FROM prompts WHERE id = ANY($1)")
            .bind(&ids)
            .fetch_all(&state.pool)
            .await?
            .into_iter()
            .collect()
    };
    let targets = rows
        .into_iter()
        .map(|(login, account_deleted, prompts)| AdminLogTarget {
            login,
            account_deleted,
            prompts: prompts
                .map(|p| p.0)
                .unwrap_or_default()
                .into_iter()
                .map(|p| AdminLogPrompt {
                    exists: existing.contains(&p.id),
                    id: p.id,
                    title: p.title,
                    handle: p.handle,
                })
                .collect(),
        })
        .collect();
    Ok(AdminLogDetail {
        entry,
        targets,
        next,
    })
}

/// Records which accounts an entry's decision was against, by login name as
/// it is now (kept if the account is later deleted).
pub(crate) async fn log_targets(
    conn: &mut PgConnection,
    log_id: i64,
    affected: &[notices::Affected],
) -> Result<(), ApiError> {
    let accounts: Vec<Uuid> = affected.iter().map(|a| a.account).collect();
    if accounts.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO admin_log_targets (log_id, account_id, login)
         SELECT $1, id, login_name FROM accounts WHERE id = ANY($2)
         ON CONFLICT DO NOTHING",
    )
    .bind(log_id)
    .bind(&accounts)
    .execute(&mut *conn)
    .await?;
    Ok(())
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
