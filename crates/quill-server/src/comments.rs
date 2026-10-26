//! Comments on announcements (#41): flat, oldest first, written as a
//! persona.
//!
//! A comment can't be edited, so a reported comment stays as it was
//! reported; its author can delete it. Administrators remove comments
//! through the report inbox or directly. A removal is logged with the
//! comment's hash, not its text, and the author gets a notice, as for a
//! removed prompt.

use std::time::Duration;
use utoipa_axum::{router::OpenApiRouter, routes};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use quill_proto::{
    announcements::{
        Comment, CommentPage, CommentQuery, CreateComment, RemoveComment, ReportComment,
    },
    board::{PromptAuthor, ReportReason},
    limits::{DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE},
    notices::{NoticeFields, NoticeKind},
    validate,
};
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::{Admin, Authed, MaybeAuthed},
    error::ApiError,
    markdown, notices,
    prompts::{decode_cursor, encode_cursor, viewer_is_admin},
    reports::{self, comment_hash},
    routes::ApiJson,
};

/// Comments one account may post per hour, across its personas.
const COMMENTS_PER_HOUR: usize = 30;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(api_list, api_post))
        .routes(routes!(api_delete))
        .routes(routes!(api_report))
        .routes(routes!(api_remove))
}

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    announcement_id: Uuid,
    actor_id: Uuid,
    handle: String,
    display_name: Option<String>,
    body_md: String,
    body_html: String,
    removed: bool,
    created_at: OffsetDateTime,
}

impl From<Row> for Comment {
    fn from(r: Row) -> Self {
        Self {
            id: r.id,
            announcement: r.announcement_id,
            author: PromptAuthor {
                id: r.actor_id,
                handle: r.handle,
                display_name: r.display_name,
            },
            body: r.body_md,
            body_html: r.body_html,
            removed: r.removed,
            created_at: r.created_at,
        }
    }
}

const SELECT: &str = "SELECT c.id, c.announcement_id, a.id AS actor_id, \
     a.preferred_username AS handle, a.display_name, c.body_md, c.body_html, \
     c.removed_at IS NOT NULL AS removed, c.created_at \
     FROM announcement_comments c JOIN actors a ON a.id = c.actor_id";

// ---- Reading ---------------------------------------------------------------

/// A comment in any state, for the administrators' inbox.
pub async fn load(pool: &PgPool, id: Uuid) -> Result<Option<Comment>, ApiError> {
    // Safe: built only from compile-time constants; values are bound.
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!("{SELECT} WHERE c.id = $1")))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(Comment::from))
}

/// The comments on a live announcement, oldest first. Removed comments are
/// left out, except for their author and administrators, who see them
/// marked as removed.
pub async fn list(
    state: &AppState,
    viewer: Option<Uuid>,
    announcement: Uuid,
    q: &CommentQuery,
) -> Result<CommentPage, ApiError> {
    crate::announcements::get(state, announcement).await?;
    let admin = viewer_is_admin(&state.pool, viewer).await?;
    let limit = q.limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE);
    let (at, id) = q.cursor.as_deref().map(decode_cursor).transpose()?.unzip();
    // Safe: built only from compile-time constants; values are bound.
    let mut rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{SELECT} WHERE c.announcement_id = $1
           AND (c.removed_at IS NULL OR a.account_id = $2 OR $6)
           AND ($3::timestamptz IS NULL OR (c.created_at, c.id) > ($3, $4))
         ORDER BY c.created_at, c.id LIMIT $5"
    )))
    .bind(announcement)
    .bind(viewer)
    .bind(at)
    .bind(id)
    .bind(i64::from(limit) + 1)
    .bind(admin)
    .fetch_all(&state.pool)
    .await?;
    // One extra row says whether there's another page.
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = if more {
        rows.last().map(|r| encode_cursor(r.created_at, r.id))
    } else {
        None
    };
    Ok(CommentPage {
        items: rows.into_iter().map(Comment::from).collect(),
        next_cursor,
    })
}

/// The announcement a comment is on, if the viewer may see the comment.
pub async fn announcement_of(
    state: &AppState,
    viewer: Option<Uuid>,
    id: Uuid,
) -> Result<Uuid, ApiError> {
    let admin = viewer_is_admin(&state.pool, viewer).await?;
    let found: Option<Uuid> = sqlx::query_scalar(
        "SELECT c.announcement_id FROM announcement_comments c
         JOIN actors a ON a.id = c.actor_id
         JOIN announcements an ON an.id = c.announcement_id
         WHERE c.id = $1 AND an.deleted_at IS NULL
           AND (c.removed_at IS NULL OR a.account_id = $2 OR $3)",
    )
    .bind(id)
    .bind(viewer)
    .bind(admin)
    .fetch_optional(&state.pool)
    .await?;
    found.ok_or(ApiError::NotFound)
}

// ---- Writing ---------------------------------------------------------------

/// Posts a comment as one of `account`'s personas.
pub async fn post(
    state: &AppState,
    account: Uuid,
    announcement: Uuid,
    body: &CreateComment,
) -> Result<Comment, ApiError> {
    let text = validate::comment_body(&body.body)?;
    let a = crate::announcements::get(state, announcement).await?;
    if !a.comments_open {
        return Err(ApiError::Forbidden(
            "Comments are closed on this announcement.".into(),
        ));
    }
    let owns: Option<i32> =
        sqlx::query_scalar("SELECT 1 FROM actors WHERE id = $1 AND account_id = $2")
            .bind(body.persona)
            .bind(account)
            .fetch_optional(&state.pool)
            .await?;
    if owns.is_none() {
        return Err(ApiError::BadRequest("unknown persona".into()));
    }
    let key = format!("comment:{account}");
    if !state
        .limiter
        .allow(&key, COMMENTS_PER_HOUR, Duration::from_secs(60 * 60))
    {
        return Err(ApiError::TooManyRequests);
    }
    let id = Uuid::now_v7();
    // Closing comments while this is posted wins: the insert checks again.
    let inserted = sqlx::query(
        "INSERT INTO announcement_comments (id, announcement_id, actor_id, body_md, body_html)
         SELECT $1, $2, $3, $4, $5 FROM announcements
         WHERE id = $2 AND deleted_at IS NULL AND comments_open",
    )
    .bind(id)
    .bind(announcement)
    .bind(body.persona)
    .bind(&text)
    .bind(markdown::render(&text))
    .execute(&state.pool)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::Forbidden(
            "Comments are closed on this announcement.".into(),
        ));
    }
    load(&state.pool, id).await?.ok_or(ApiError::NotFound)
}

/// Deletes one of `account`'s comments for good. Reports about it stay,
/// without it.
pub async fn delete_own(state: &AppState, account: Uuid, id: Uuid) -> Result<(), ApiError> {
    let result = sqlx::query(
        "DELETE FROM announcement_comments c USING actors a
         WHERE c.id = $1 AND a.id = c.actor_id AND a.account_id = $2",
    )
    .bind(id)
    .bind(account)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

// ---- Removing --------------------------------------------------------------

/// An administrator removes a comment without a report.
pub async fn remove(
    state: &AppState,
    admin: Uuid,
    id: Uuid,
    reason: ReportReason,
) -> Result<(), ApiError> {
    let mut tx = state.pool.begin().await?;
    remove_locked(&mut tx, admin, id, reason, None).await?;
    tx.commit().await?;
    Ok(())
}

/// Removes a comment, logs it with the comment's hash, and tells its author
/// the reason's category. `report` names the report it resolves, if any.
pub(crate) async fn remove_locked(
    conn: &mut PgConnection,
    admin: Uuid,
    id: Uuid,
    reason: ReportReason,
    report: Option<Uuid>,
) -> Result<(), ApiError> {
    let row: Option<(String, bool, Uuid, String, String)> = sqlx::query_as(
        "SELECT c.body_md, c.removed_at IS NOT NULL, an.id, an.title, a.preferred_username
         FROM announcement_comments c
         JOIN actors a ON a.id = c.actor_id
         JOIN announcements an ON an.id = c.announcement_id
         WHERE c.id = $1 FOR UPDATE OF c",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let (text, removed, announcement, title, handle) = row.ok_or(ApiError::NotFound)?;
    if removed {
        return Err(ApiError::Conflict("comment already removed".into()));
    }
    sqlx::query(
        "UPDATE announcement_comments SET removed_at = now(), removed_reason = $2::report_reason
         WHERE id = $1",
    )
    .bind(id)
    .bind(reason.as_str())
    .execute(&mut *conn)
    .await?;
    let log_id: i64 = sqlx::query_scalar(
        "INSERT INTO admin_log
             (action, report_id, comment_id, prompt_hash, reason, admin_id, actor, admin_login)
         VALUES ('delete_comment', $1, $2, $3, $4::report_reason, $5, 'admin',
                 (SELECT login_name FROM accounts WHERE id = $5))
         RETURNING id",
    )
    .bind(report)
    .bind(id)
    .bind(comment_hash(&text))
    .bind(reason.as_str())
    .bind(admin)
    .fetch_one(&mut *conn)
    .await?;
    let author = notices::affected_by_comment(conn, id).await?;
    let fields = NoticeFields {
        reason: Some(reason),
        announcement: Some(announcement),
        announcement_title: Some(title),
        handle: Some(handle),
        ..NoticeFields::default()
    };
    notices::record(conn, log_id, NoticeKind::CommentRemoved, &fields, &author).await
}

// ---- JSON API --------------------------------------------------------------

#[utoipa::path(
    get, path = "/announcements/{id}/comments", tag = "announcements", security((), ("bearer" = [])),
    summary = "Comments on an announcement, oldest first",
    description = "Removed comments are left out, except for their author and administrators, who see them marked as removed. Pages by `cursor`, which the previous page returns.",
    params(("id" = Uuid, Path, description = "The announcement"), CommentQuery),
    responses((status = 200, body = CommentPage), (status = 404, description = "No such announcement, or deleted")),
)]
async fn api_list(
    State(state): State<AppState>,
    MaybeAuthed(authed): MaybeAuthed,
    Path(id): Path<Uuid>,
    Query(q): Query<CommentQuery>,
) -> Result<Json<CommentPage>, ApiError> {
    list(&state, authed.map(|a| a.account_id), id, &q)
        .await
        .map(Json)
}

#[utoipa::path(
    post, path = "/announcements/{id}/comments", tag = "announcements", security(("bearer" = [])),
    summary = "Comment on an announcement as one of the login's personas",
    description = "At most 30 comments per login an hour. Comments can't be edited.",
    params(("id" = Uuid, Path, description = "The announcement")),
    request_body = CreateComment,
    responses(
        (status = 201, body = Comment),
        (status = 403, description = "Comments are closed on this announcement"),
        (status = 404, description = "No such announcement, or deleted"),
        (status = 429, description = "Too many comments"),
    ),
)]
async fn api_post(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
    ApiJson(body): ApiJson<CreateComment>,
) -> Result<(StatusCode, Json<Comment>), ApiError> {
    let comment = post(&state, authed.account_id, id, &body).await?;
    Ok((StatusCode::CREATED, Json(comment)))
}

#[utoipa::path(
    delete, path = "/comments/{id}", tag = "announcements", security(("bearer" = [])),
    summary = "Delete one of the login's comments",
    params(("id" = Uuid, Path, description = "The comment")),
    responses((status = 204, description = "Deleted"), (status = 404, description = "Not one of the login's comments")),
)]
async fn api_delete(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    delete_own(&state, authed.account_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post, path = "/comments/{id}/report", tag = "reports", security((), ("bearer" = [])),
    summary = "Report a comment to the administrators",
    description = "Signing in is optional. The author never learns who reported, and reports (of prompts and comments together) are rate-limited per reporter.",
    params(("id" = Uuid, Path, description = "The comment")),
    request_body = ReportComment,
    responses(
        (status = 202, description = "Received"),
        (status = 404, description = "No such comment"),
        (status = 429, description = "Too many reports"),
    ),
)]
async fn api_report(
    State(state): State<AppState>,
    MaybeAuthed(authed): MaybeAuthed,
    Path(id): Path<Uuid>,
    ApiJson(body): ApiJson<ReportComment>,
) -> Result<StatusCode, ApiError> {
    reports::file_comment_report(&state, authed.map(|a| a.account_id), id, &body).await?;
    Ok(StatusCode::ACCEPTED)
}

#[utoipa::path(
    post, path = "/admin/comments/{id}/remove", tag = "admin", security(("bearer" = [])),
    summary = "Remove a comment without a report",
    description = "Its author still sees it, marked as removed, and is told the reason's category.",
    params(("id" = Uuid, Path, description = "The comment")),
    request_body = RemoveComment,
    responses(
        (status = 204, description = "Removed"),
        (status = 404, description = "No such comment"),
        (status = 409, description = "Already removed"),
    ),
)]
async fn api_remove(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    ApiJson(body): ApiJson<RemoveComment>,
) -> Result<StatusCode, ApiError> {
    remove(&state, admin.0.account_id, id, body.reason).await?;
    Ok(StatusCode::NO_CONTENT)
}
