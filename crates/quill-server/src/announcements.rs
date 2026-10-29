//! Announcements (#41): news the administrators post on the board.
//!
//! Anyone can read them; only administrators post, edit and delete them,
//! and each change is in the admin log. Deleting is a soft delete
//! (`deleted_at`), after which an announcement shows nowhere.

use utoipa_axum::{router::OpenApiRouter, routes};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use quill_proto::{
    announcements::{
        Announcement, AnnouncementPage, AnnouncementQuery, CreateAnnouncement, UpdateAnnouncement,
    },
    board::AdminAction,
    limits::{DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE},
    validate,
};
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::Admin,
    error::ApiError,
    markdown,
    prompts::{decode_cursor, encode_cursor},
    reports::log_action,
    routes::ApiJson,
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(api_list))
        .routes(routes!(api_get))
        .routes(routes!(api_post))
        .routes(routes!(api_update, api_delete))
}

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    title: String,
    body_md: String,
    body_html: String,
    comments_open: bool,
    sticky: bool,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

const SELECT: &str = "SELECT id, title, body_md, body_html, comments_open, sticky, created_at, \
     updated_at FROM announcements";

fn to_announcement(r: Row) -> Announcement {
    Announcement {
        id: r.id,
        title: r.title,
        body: r.body_md,
        body_html: r.body_html,
        comments_open: r.comments_open,
        sticky: r.sticky,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

// ---- Reading ---------------------------------------------------------------

/// Announcements newest first, a page at a time, optionally only the
/// sticky ones or only the others.
pub async fn list(state: &AppState, q: &AnnouncementQuery) -> Result<AnnouncementPage, ApiError> {
    let limit = q.limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE);
    let cursor = q.cursor.as_deref().map(decode_cursor).transpose()?;
    let (at, id) = cursor.unzip();
    let mut rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{SELECT} WHERE deleted_at IS NULL
           AND ($1::timestamptz IS NULL OR (created_at, id) < ($1, $2))
           AND ($4::boolean IS NULL OR sticky = $4)
         ORDER BY created_at DESC, id DESC LIMIT $3"
    )))
    .bind(at)
    .bind(id)
    .bind(i64::from(limit) + 1)
    .bind(q.sticky)
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
    Ok(AnnouncementPage {
        items: rows.into_iter().map(to_announcement).collect(),
        next_cursor,
    })
}

/// What the board's banners show: every sticky announcement (newest first,
/// up to a page), and the newest of the others.
pub async fn banners(
    state: &AppState,
) -> Result<(Vec<Announcement>, Option<Announcement>), ApiError> {
    let sticky = list(
        state,
        &AnnouncementQuery {
            limit: Some(MAX_PAGE_SIZE),
            sticky: Some(true),
            ..AnnouncementQuery::default()
        },
    )
    .await?;
    let newest = list(
        state,
        &AnnouncementQuery {
            limit: Some(1),
            sticky: Some(false),
            ..AnnouncementQuery::default()
        },
    )
    .await?;
    Ok((sticky.items, newest.items.into_iter().next()))
}

pub async fn get(state: &AppState, id: Uuid) -> Result<Announcement, ApiError> {
    // Safe: built only from compile-time constants; values are bound.
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{SELECT} WHERE id = $1 AND deleted_at IS NULL"
    )))
    .bind(id)
    .fetch_optional(&state.pool)
    .await?;
    row.map(to_announcement).ok_or(ApiError::NotFound)
}

// ---- Administration --------------------------------------------------------

fn comments_label(open: bool) -> &'static str {
    if open { "open" } else { "closed" }
}

fn sticky_label(sticky: bool) -> &'static str {
    if sticky { "yes" } else { "no" }
}

/// Posts an announcement as the server; the log names `admin`.
pub async fn post(
    state: &AppState,
    admin: Uuid,
    body: &CreateAnnouncement,
) -> Result<Announcement, ApiError> {
    let title = validate::announcement_title(&body.title)?;
    let text = validate::announcement_body(&body.body)?;
    let id = Uuid::now_v7();
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO announcements
             (id, title, body_md, body_html, comments_open, sticky, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(&title)
    .bind(&text)
    .bind(markdown::render(&text))
    .bind(body.comments_open)
    .bind(body.sticky)
    .bind(admin)
    .execute(&mut *tx)
    .await?;
    let notes: Vec<&str> = [
        body.sticky.then_some("sticky"),
        (!body.comments_open).then_some("comments closed"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let detail = (!notes.is_empty()).then(|| notes.join(", "));
    log_action(
        &mut tx,
        admin,
        AdminAction::PostAnnouncement,
        &title,
        detail.as_deref(),
    )
    .await?;
    tx.commit().await?;
    get(state, id).await
}

/// What a change may alter: title, text, comments open, sticky.
type Locked = (String, String, bool, bool);

/// Locks a live announcement for a change.
async fn lock(conn: &mut PgConnection, id: Uuid) -> Result<Locked, ApiError> {
    let row: Option<Locked> = sqlx::query_as(
        "SELECT title, body_md, comments_open, sticky FROM announcements
         WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    row.ok_or(ApiError::NotFound)
}

/// Edits an announcement's title or text, opens or closes its comments,
/// and makes it sticky or not. Each kind of change is logged on its own;
/// no change, no entry.
pub async fn update(
    state: &AppState,
    admin: Uuid,
    id: Uuid,
    body: &UpdateAnnouncement,
) -> Result<Announcement, ApiError> {
    let title = body
        .title
        .as_deref()
        .map(validate::announcement_title)
        .transpose()?;
    let text = body
        .body
        .as_deref()
        .map(validate::announcement_body)
        .transpose()?;
    let mut tx = state.pool.begin().await?;
    let (old_title, old_text, old_open, old_sticky) = lock(&mut tx, id).await?;
    let new_title = title.unwrap_or_else(|| old_title.clone());
    let new_text = text.unwrap_or_else(|| old_text.clone());
    let renamed = new_title != old_title;
    let rewritten = new_text != old_text;
    if renamed || rewritten {
        sqlx::query(
            "UPDATE announcements SET title = $2, body_md = $3, body_html = $4, updated_at = now()
             WHERE id = $1",
        )
        .bind(id)
        .bind(&new_title)
        .bind(&new_text)
        .bind(markdown::render(&new_text))
        .execute(&mut *tx)
        .await?;
        let detail = match (renamed, rewritten) {
            (true, true) => format!("title and text changed; was “{old_title}”"),
            (true, false) => format!("title changed; was “{old_title}”"),
            _ => "text changed".to_owned(),
        };
        log_action(
            &mut tx,
            admin,
            AdminAction::EditAnnouncement,
            &new_title,
            Some(&detail),
        )
        .await?;
    }
    if let Some(open) = body.comments_open.filter(|o| *o != old_open) {
        sqlx::query("UPDATE announcements SET comments_open = $2 WHERE id = $1")
            .bind(id)
            .bind(open)
            .execute(&mut *tx)
            .await?;
        let detail = format!(
            "comments: {} → {}",
            comments_label(old_open),
            comments_label(open)
        );
        log_action(
            &mut tx,
            admin,
            AdminAction::SetAnnouncementComments,
            &new_title,
            Some(&detail),
        )
        .await?;
    }
    if let Some(sticky) = body.sticky.filter(|s| *s != old_sticky) {
        sqlx::query("UPDATE announcements SET sticky = $2 WHERE id = $1")
            .bind(id)
            .bind(sticky)
            .execute(&mut *tx)
            .await?;
        let detail = format!(
            "sticky: {} → {}",
            sticky_label(old_sticky),
            sticky_label(sticky)
        );
        log_action(
            &mut tx,
            admin,
            AdminAction::SetAnnouncementSticky,
            &new_title,
            Some(&detail),
        )
        .await?;
    }
    tx.commit().await?;
    get(state, id).await
}

/// Deletes an announcement: it shows nowhere from now on.
pub async fn delete(state: &AppState, admin: Uuid, id: Uuid) -> Result<(), ApiError> {
    let mut tx = state.pool.begin().await?;
    let (title, ..) = lock(&mut tx, id).await?;
    sqlx::query("UPDATE announcements SET deleted_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    log_action(
        &mut tx,
        admin,
        AdminAction::DeleteAnnouncement,
        &title,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---- JSON API --------------------------------------------------------------

#[utoipa::path(
    get, path = "/announcements", tag = "announcements", security(()),
    summary = "The administrators' announcements, newest first",
    description = "Pages by `cursor`, which the previous page returns. `sticky=true` gives the sticky ones, whose banners clients show at the top of the board without a way to dismiss them.",
    params(AnnouncementQuery),
    responses((status = 200, body = AnnouncementPage)),
)]
async fn api_list(
    State(state): State<AppState>,
    Query(q): Query<AnnouncementQuery>,
) -> Result<Json<AnnouncementPage>, ApiError> {
    list(&state, &q).await.map(Json)
}

#[utoipa::path(
    get, path = "/announcements/{id}", tag = "announcements", security(()),
    summary = "One announcement",
    params(("id" = Uuid, Path, description = "The announcement")),
    responses((status = 200, body = Announcement), (status = 404, description = "No such announcement, or deleted")),
)]
async fn api_get(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Announcement>, ApiError> {
    get(&state, id).await.map(Json)
}

#[utoipa::path(
    post, path = "/admin/announcements", tag = "admin", security(("bearer" = [])),
    summary = "Post an announcement as the server",
    request_body = CreateAnnouncement,
    responses((status = 201, body = Announcement)),
)]
async fn api_post(
    State(state): State<AppState>,
    admin: Admin,
    ApiJson(body): ApiJson<CreateAnnouncement>,
) -> Result<(StatusCode, Json<Announcement>), ApiError> {
    let posted = post(&state, admin.0.account_id, &body).await?;
    Ok((StatusCode::CREATED, Json(posted)))
}

#[utoipa::path(
    patch, path = "/admin/announcements/{id}", tag = "admin", security(("bearer" = [])),
    summary = "Edit an announcement, or open or close its comments",
    params(("id" = Uuid, Path, description = "The announcement")),
    request_body = UpdateAnnouncement,
    responses((status = 200, body = Announcement), (status = 404, description = "No such announcement, or deleted")),
)]
async fn api_update(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    ApiJson(body): ApiJson<UpdateAnnouncement>,
) -> Result<Json<Announcement>, ApiError> {
    update(&state, admin.0.account_id, id, &body)
        .await
        .map(Json)
}

#[utoipa::path(
    delete, path = "/admin/announcements/{id}", tag = "admin", security(("bearer" = [])),
    summary = "Delete an announcement",
    params(("id" = Uuid, Path, description = "The announcement")),
    responses((status = 204, description = "Deleted"), (status = 404, description = "No such announcement, or already deleted")),
)]
async fn api_delete(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    delete(&state, admin.0.account_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
