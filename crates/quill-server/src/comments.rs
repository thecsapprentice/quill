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
        Comment, CommentPage, CommentQuery, CommentVisibility, CommentVoice, CreateComment,
        RemoveComment, ReportComment,
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
    server_settings,
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
    voice: String,
    actor_id: Option<Uuid>,
    handle: Option<String>,
    display_name: Option<String>,
    staff_name: Option<String>,
    body_md: String,
    body_html: String,
    removed: bool,
    created_at: OffsetDateTime,
}

impl From<Row> for Comment {
    fn from(r: Row) -> Self {
        let author = match (r.actor_id, r.handle) {
            (Some(id), Some(handle)) => Some(PromptAuthor {
                id,
                handle,
                display_name: r.display_name,
            }),
            _ => None,
        };
        Self {
            id: r.id,
            announcement: r.announcement_id,
            // The column's CHECK allows only the three voices.
            voice: r.voice.parse().unwrap_or_default(),
            author,
            staff_name: r.staff_name,
            body: r.body_md,
            body_html: r.body_html,
            removed: r.removed,
            created_at: r.created_at,
        }
    }
}

const SELECT: &str = "SELECT c.id, c.announcement_id, c.voice, a.id AS actor_id, \
     a.preferred_username AS handle, a.display_name, c.staff_name, c.body_md, c.body_html, \
     c.removed_at IS NOT NULL AS removed, c.created_at \
     FROM announcement_comments c LEFT JOIN actors a ON a.id = c.actor_id";

// ---- Reading ---------------------------------------------------------------

/// Whether `viewer` may read comments: everyone can, unless the
/// administrators made them visible to signed-in writers only.
pub async fn readable_by(state: &AppState, viewer: Option<Uuid>) -> Result<bool, ApiError> {
    if viewer.is_some() {
        return Ok(true);
    }
    let mut conn = state.pool.acquire().await?;
    Ok(server_settings::comment_visibility(&mut conn).await? == CommentVisibility::Public)
}

/// Refuses a visitor who isn't signed in when comments are for signed-in
/// writers only. Checked before anything is looked up, so the refusal
/// doesn't say whether a comment exists.
pub(crate) async fn check_readable(state: &AppState, viewer: Option<Uuid>) -> Result<(), ApiError> {
    if readable_by(state, viewer).await? {
        Ok(())
    } else {
        Err(ApiError::Unauthorized)
    }
}

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
    check_readable(state, viewer).await?;
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
    check_readable(state, viewer).await?;
    let admin = viewer_is_admin(&state.pool, viewer).await?;
    let found: Option<Uuid> = sqlx::query_scalar(
        "SELECT c.announcement_id FROM announcement_comments c
         LEFT JOIN actors a ON a.id = c.actor_id
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

/// Who a new comment is written by, once the voice is checked.
struct Speaker {
    actor: Option<Uuid>,
    /// The administrator behind an admin voice.
    admin: Option<Uuid>,
    staff_name: Option<String>,
}

/// Checks that `account` may speak in the voice `body` asks for.
async fn speaker(
    state: &AppState,
    account: Uuid,
    body: &CreateComment,
) -> Result<Speaker, ApiError> {
    if body.voice == CommentVoice::Persona {
        let owns: Option<i32> = match body.persona {
            Some(persona) => {
                sqlx::query_scalar("SELECT 1 FROM actors WHERE id = $1 AND account_id = $2")
                    .bind(persona)
                    .bind(account)
                    .fetch_optional(&state.pool)
                    .await?
            }
            None => None,
        };
        if owns.is_none() {
            return Err(ApiError::BadRequest("unknown persona".into()));
        }
        return Ok(Speaker {
            actor: body.persona,
            admin: None,
            staff_name: None,
        });
    }
    if body.persona.is_some() {
        return Err(ApiError::BadRequest(
            "A comment speaks as a persona or as staff, not both.".into(),
        ));
    }
    let row: Option<(bool, Option<String>)> =
        sqlx::query_as("SELECT is_admin, staff_name FROM accounts WHERE id = $1")
            .bind(account)
            .fetch_optional(&state.pool)
            .await?;
    let Some((true, staff_name)) = row else {
        return Err(ApiError::Forbidden(
            "Only administrators comment as staff.".into(),
        ));
    };
    let staff_name = match body.voice {
        CommentVoice::Staff => Some(staff_name.ok_or_else(|| {
            ApiError::BadRequest("Set a staff name in your settings first.".into())
        })?),
        _ => None,
    };
    Ok(Speaker {
        actor: None,
        admin: Some(account),
        staff_name,
    })
}

/// Posts a comment as one of `account`'s personas, or as staff.
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
    let who = speaker(state, account, body).await?;
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
        "INSERT INTO announcement_comments
             (id, announcement_id, voice, actor_id, account_id, staff_name, body_md, body_html)
         SELECT $1, $2, $3, $4, $5, $6, $7, $8 FROM announcements
         WHERE id = $2 AND deleted_at IS NULL AND comments_open",
    )
    .bind(id)
    .bind(announcement)
    .bind(body.voice.as_str())
    .bind(who.actor)
    .bind(who.admin)
    .bind(&who.staff_name)
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
/// without it. Any administrator may delete a staff comment, since the
/// voice is shared.
pub async fn delete_own(state: &AppState, account: Uuid, id: Uuid) -> Result<(), ApiError> {
    let result = sqlx::query(
        "DELETE FROM announcement_comments c
         WHERE c.id = $1 AND (
             EXISTS (SELECT 1 FROM actors a WHERE a.id = c.actor_id AND a.account_id = $2)
             OR (c.voice <> 'persona'
                 AND EXISTS (SELECT 1 FROM accounts WHERE id = $2 AND is_admin)))",
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
    let row: Option<(String, bool, Uuid, String, Option<String>)> = sqlx::query_as(
        "SELECT c.body_md, c.removed_at IS NOT NULL, an.id, an.title, a.preferred_username
         FROM announcement_comments c
         LEFT JOIN actors a ON a.id = c.actor_id
         JOIN announcements an ON an.id = c.announcement_id
         WHERE c.id = $1 FOR UPDATE OF c",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let (text, removed, announcement, title, handle) = row.ok_or(ApiError::NotFound)?;
    // Staff comments have no persona: the administrators delete them.
    let Some(handle) = handle else {
        return Err(ApiError::BadRequest(
            "Comments from the administrators can't be removed; delete it instead.".into(),
        ));
    };
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
    responses(
        (status = 200, body = CommentPage),
        (status = 401, description = "This server shows comments to signed-in writers only (`commentVisibility` in `GET /instance`)"),
        (status = 404, description = "No such announcement, or deleted"),
    ),
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
    description = "At most 30 comments per login an hour. Comments can't be edited. Administrators may speak as `administrators`, or as `staff` under the staff name in their account settings; the board marks both as the administrators'.",
    params(("id" = Uuid, Path, description = "The announcement")),
    request_body = CreateComment,
    responses(
        (status = 201, body = Comment),
        (status = 403, description = "Comments are closed on this announcement, or a staff voice from a login that isn't an administrator"),
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
    description = "Signing in is optional, unless the server shows comments to signed-in writers only. The author never learns who reported, and reports (of prompts and comments together) are rate-limited per reporter.",
    params(("id" = Uuid, Path, description = "The comment")),
    request_body = ReportComment,
    responses(
        (status = 202, description = "Received"),
        (status = 401, description = "This server shows comments to signed-in writers only"),
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
