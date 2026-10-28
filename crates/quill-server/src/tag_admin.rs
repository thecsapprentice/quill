//! Tag administration: tag types (namespaces) and tags, for server
//! administrators. Every change is written to the admin log.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use quill_proto::notices::{NoticeFields, NoticeKind};
use quill_proto::{
    board::{
        AdminAction, AdminTag, AdminTagQuery, ChipColor, CreateNamespace, CreateTag, DeletedTag,
        Tag, TagNamespace, TagStatus, UpdateNamespace, UpdateTag, limit_label,
    },
    limits::{MAX_PAGE_SIZE, PROMPT_MAX_TAGS},
    validate,
};
use sqlx::{PgConnection, QueryBuilder};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    AppState,
    auth::Admin,
    error::{ApiError, is_unique_violation},
    notices,
    prompts::{TagRow, escape_like, load_namespace, load_tag, parse_enum, resolve_tags},
    reports::log_action,
    routes::ApiJson,
};

/// Most tags one admin listing returns.
pub const ADMIN_TAG_LIMIT: u32 = 200;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(api_create_namespace))
        .routes(routes!(api_update_namespace))
        .routes(routes!(api_list_tags, api_create_tag))
        .routes(routes!(api_get_tag, api_update_tag, api_delete_tag))
}

/// Notice fields naming a tag.
fn tag_fields(key: &str, display: &str) -> NoticeFields {
    NoticeFields {
        tag: Some(key.to_owned()),
        tag_name: Some(display.to_owned()),
        ..NoticeFields::default()
    }
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

/// Validates an optional chip colour.
fn checked_color(color: Option<&ChipColor>) -> Result<Option<ChipColor>, ApiError> {
    Ok(color.map(quill_proto::color::chip).transpose()?)
}

/// "#1e3a8a on #dbeafe", "automatic text on #1e3a8a", or "none".
fn describe_color(color: Option<&ChipColor>) -> String {
    match color {
        None => "none".to_owned(),
        Some(ChipColor {
            background,
            text: Some(text),
        }) => format!("{text} on {background}"),
        Some(ChipColor {
            background,
            text: None,
        }) => format!("automatic text on {background}"),
    }
}

// ---- Tag types -----------------------------------------------------------

/// Checks a tag type's limits against the other types and its tags: a
/// curated-only type can't ask for more tags than writers can pick, and the
/// minimums together must fit on one prompt.
async fn check_limits(
    conn: &mut PgConnection,
    name: &str,
    display: &str,
    locked: bool,
    min: u32,
    max: Option<u32>,
) -> Result<(), ApiError> {
    validate::tag_limits(min, max)?;
    if min == 0 {
        return Ok(());
    }
    if locked {
        let visible: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM tags
             WHERE instance = '' AND namespace = $1 AND canonical_id IS NULL
               AND status = 'visible'",
        )
        .bind(name)
        .fetch_one(&mut *conn)
        .await?;
        if i64::from(min) > visible {
            return Err(ApiError::BadRequest(format!(
                "{display} is curated-only and has {visible} visible {}, so writers couldn't pick {min}. Add tags first, or lower the minimum.",
                if visible == 1 { "tag" } else { "tags" }
            )));
        }
    }
    let others: i64 = sqlx::query_scalar(
        "SELECT COALESCE(sum(min_tags), 0) FROM tag_namespaces WHERE name <> $1",
    )
    .bind(name)
    .fetch_one(&mut *conn)
    .await?;
    let cap = i64::try_from(PROMPT_MAX_TAGS).unwrap_or(i64::MAX);
    if others + i64::from(min) > cap {
        return Err(ApiError::BadRequest(format!(
            "Together the tag types would need {} tags on every prompt, but a prompt can have at most {PROMPT_MAX_TAGS}.",
            others + i64::from(min)
        )));
    }
    Ok(())
}

fn to_db(n: u32) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// Prompts on the board whose tags don't meet `min`/`max` for tag type
/// `name`: how many, and the newest `limit` of them. Changing limits never
/// changes prompts; these are fixed when their authors next edit them.
pub async fn prompts_outside_limits(
    state: &AppState,
    name: &str,
    min: u32,
    max: Option<u32>,
    limit: u32,
) -> Result<(u64, Vec<(Uuid, String)>), ApiError> {
    const OUTSIDE: &str = "WITH counted AS (
             SELECT p.id, p.title, p.created_at,
                    (SELECT count(*) FROM tags t
                     WHERE t.id = ANY(p.tag_ids) AND t.namespace = $1) AS n
             FROM prompts p WHERE p.removed_at IS NULL)
         SELECT id, title, created_at FROM counted
         WHERE n < $2 OR ($3::integer IS NOT NULL AND n > $3)";
    // Safe: built only from compile-time constants; values are bound.
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM ({OUTSIDE}) o"
    )))
    .bind(name)
    .bind(to_db(min))
    .bind(max.map(to_db))
    .fetch_one(&state.pool)
    .await?;
    let rows: Vec<(Uuid, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT id, title FROM ({OUTSIDE}) o ORDER BY created_at DESC, id DESC LIMIT $4"
    )))
    .bind(name)
    .bind(to_db(min))
    .bind(max.map(to_db))
    .bind(i64::from(limit))
    .fetch_all(&state.pool)
    .await?;
    Ok((u64::try_from(total).unwrap_or(0), rows))
}

/// Creates a tag type.
pub async fn create_namespace(
    state: &AppState,
    admin: Uuid,
    body: &CreateNamespace,
) -> Result<TagNamespace, ApiError> {
    let name = validate::namespace(&body.name)?;
    let display = validate::tag_display(&body.display)?;
    let color = checked_color(body.color.as_ref())?;
    let mut tx = state.pool.begin().await?;
    // Serializes limit changes, which are checked against every type.
    sqlx::query("LOCK TABLE tag_namespaces IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    check_limits(&mut tx, &name, &display, body.locked, body.min, body.max).await?;
    let result = sqlx::query(
        "INSERT INTO tag_namespaces (name, display, locked, min_tags, max_tags, chip_bg, chip_fg)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&name)
    .bind(&display)
    .bind(body.locked)
    .bind(to_db(body.min))
    .bind(body.max.map(to_db))
    .bind(color.as_ref().map(|c| c.background.clone()))
    .bind(color.as_ref().and_then(|c| c.text.clone()))
    .execute(&mut *tx)
    .await;
    match result {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => {
            return Err(ApiError::Conflict(format!(
                "a tag type named \"{name}\" already exists"
            )));
        }
        Err(e) => return Err(e.into()),
    }
    let mut detail = format!(
        "{display}; curated tags only: {}; tags per prompt: {}",
        on_off(body.locked),
        limit_label(body.min, body.max)
    );
    if color.is_some() {
        detail.push_str(&format!("; colour: {}", describe_color(color.as_ref())));
    }
    log_action(
        &mut tx,
        admin,
        AdminAction::CreateTagType,
        &name,
        Some(&detail),
    )
    .await?;
    tx.commit().await?;
    Ok(TagNamespace {
        name,
        display,
        locked: body.locked,
        min: body.min,
        max: body.max,
        color,
    })
}

/// Renames a tag type, or changes whether it is curated-only, its limits
/// or its colours.
pub async fn update_namespace(
    state: &AppState,
    admin: Uuid,
    name: &str,
    body: &UpdateNamespace,
) -> Result<TagNamespace, ApiError> {
    let display = body
        .display
        .as_deref()
        .map(validate::tag_display)
        .transpose()?;
    let new_color = checked_color(body.color.as_ref())?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("LOCK TABLE tag_namespaces IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    let before = load_namespace(&mut tx, name, true).await?;
    let color = match (new_color, body.clear_color) {
        (Some(c), _) => Some(c),
        (None, true) => None,
        (None, false) => before.color.clone(),
    };
    let after = TagNamespace {
        name: before.name.clone(),
        display: display.unwrap_or_else(|| before.display.clone()),
        locked: body.locked.unwrap_or(before.locked),
        min: body.min.unwrap_or(before.min),
        max: match (body.max, body.clear_max) {
            (Some(m), _) => Some(m),
            (None, true) => None,
            (None, false) => before.max,
        },
        color,
    };
    let mut changes = Vec::new();
    if after.display != before.display {
        changes.push(format!(
            "renamed from {} to {}",
            before.display, after.display
        ));
    }
    if after.locked != before.locked {
        changes.push(format!("curated tags only: {}", on_off(after.locked)));
    }
    if (after.min, after.max) != (before.min, before.max) {
        changes.push(format!(
            "tags per prompt: {} → {}",
            before.limit_label(),
            after.limit_label()
        ));
    }
    if after.color != before.color {
        changes.push(format!("colour: {}", describe_color(after.color.as_ref())));
    }
    if changes.is_empty() {
        return Ok(before);
    }
    if (after.locked, after.min, after.max) != (before.locked, before.min, before.max) {
        check_limits(
            &mut tx,
            &after.name,
            &after.display,
            after.locked,
            after.min,
            after.max,
        )
        .await?;
    }
    sqlx::query(
        "UPDATE tag_namespaces
         SET display = $2, locked = $3, min_tags = $4, max_tags = $5, chip_bg = $6, chip_fg = $7
         WHERE name = $1",
    )
    .bind(&after.name)
    .bind(&after.display)
    .bind(after.locked)
    .bind(to_db(after.min))
    .bind(after.max.map(to_db))
    .bind(after.color.as_ref().map(|c| c.background.clone()))
    .bind(after.color.as_ref().and_then(|c| c.text.clone()))
    .execute(&mut *tx)
    .await?;
    log_action(
        &mut tx,
        admin,
        AdminAction::UpdateTagType,
        &after.name,
        Some(&changes.join("; ")),
    )
    .await?;
    tx.commit().await?;
    Ok(after)
}

// ---- Tags ----------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct AdminTagRow {
    #[sqlx(flatten)]
    tag: TagRow,
    own_bg: Option<String>,
    own_fg: Option<String>,
    alias_of: Option<String>,
    uses: i64,
}

fn to_admin_tag(row: AdminTagRow) -> Result<AdminTag, ApiError> {
    let AdminTagRow {
        tag,
        own_bg,
        own_fg,
        alias_of,
        uses,
    } = row;
    Ok(AdminTag {
        tag: tag.into_tag()?,
        own_color: own_bg.map(|background| ChipColor {
            background,
            text: own_fg,
        }),
        uses: u64::try_from(uses).unwrap_or(0),
        alias_of,
    })
}

const ADMIN_TAG_SELECT: &str = "SELECT t.id, t.namespace, t.slug, t.display, t.origin, t.status,
            COALESCE(t.chip_bg, n.chip_bg) AS chip_bg,
            CASE WHEN t.chip_bg IS NOT NULL THEN t.chip_fg ELSE n.chip_fg END AS chip_fg,
            t.chip_bg AS own_bg, t.chip_fg AS own_fg,
            c.namespace || ':' || c.slug AS alias_of,
            (SELECT count(*) FROM prompts p WHERE p.tag_ids @> ARRAY[t.id]) AS uses
     FROM tags t
     JOIN tag_namespaces n ON n.name = t.namespace
     LEFT JOIN tags c ON c.id = t.canonical_id
     WHERE t.instance = ''";

/// Local tags with their usage, aliases included, by type and then name.
pub async fn list_tags(state: &AppState, q: &AdminTagQuery) -> Result<Vec<AdminTag>, ApiError> {
    let limit = q
        .limit
        .unwrap_or(ADMIN_TAG_LIMIT)
        .clamp(1, ADMIN_TAG_LIMIT.max(MAX_PAGE_SIZE));
    let mut qb = QueryBuilder::new(ADMIN_TAG_SELECT);
    if let Some(ns) = q.namespace.as_deref().filter(|n| !n.is_empty()) {
        qb.push(" AND t.namespace = ")
            .push_bind(validate::namespace(ns)?);
    }
    if let Some(search) = q.search.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let pattern = format!("%{}%", escape_like(&search.to_lowercase()));
        qb.push(" AND (t.slug LIKE ")
            .push_bind(pattern.clone())
            .push(" OR lower(t.display) LIKE ")
            .push_bind(pattern)
            .push(")");
    }
    if let Some(status) = q.status {
        qb.push(" AND t.status = ").push_bind(status.as_str());
    }
    qb.push(" ORDER BY t.namespace, t.slug LIMIT ")
        .push_bind(i64::from(limit));
    let rows: Vec<AdminTagRow> = qb.build_query_as().fetch_all(&state.pool).await?;
    rows.into_iter().map(to_admin_tag).collect()
}

/// One local tag with its usage.
pub async fn get_tag(state: &AppState, namespace: &str, slug: &str) -> Result<AdminTag, ApiError> {
    let mut qb = QueryBuilder::new(ADMIN_TAG_SELECT);
    qb.push(" AND t.namespace = ")
        .push_bind(namespace)
        .push(" AND t.slug = ")
        .push_bind(slug);
    let row: Option<AdminTagRow> = qb.build_query_as().fetch_optional(&state.pool).await?;
    row.map(to_admin_tag).ok_or(ApiError::NotFound)?
}

/// Creates a curated tag, or makes an existing user tag curated.
pub async fn create_tag(state: &AppState, admin: Uuid, body: &CreateTag) -> Result<Tag, ApiError> {
    let namespace = validate::namespace(&body.namespace)?;
    let slug = validate::tag_slug(&body.display)?;
    let display = validate::tag_display(&body.display)?;
    let mut tx = state.pool.begin().await?;
    load_namespace(&mut tx, &namespace, false)
        .await
        .map_err(|_| ApiError::BadRequest(format!("unknown tag type \"{namespace}\"")))?;
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT origin FROM tags WHERE instance = '' AND namespace = $1 AND slug = $2",
    )
    .bind(&namespace)
    .bind(&slug)
    .fetch_optional(&mut *tx)
    .await?;
    if existing.as_deref() == Some("server") {
        return Err(ApiError::Conflict(format!(
            "\"{namespace}:{slug}\" is already a curated tag"
        )));
    }
    sqlx::query(
        "INSERT INTO tags (namespace, slug, display, origin) VALUES ($1, $2, $3, 'server')
         ON CONFLICT (instance, namespace, slug)
         DO UPDATE SET origin = 'server', display = EXCLUDED.display,
             -- Curating a requested tag approves it; a hidden tag stays hidden.
             status = CASE WHEN tags.status = 'pending' THEN 'visible' ELSE tags.status END",
    )
    .bind(&namespace)
    .bind(&slug)
    .bind(&display)
    .execute(&mut *tx)
    .await?;
    let detail = if existing.is_some() {
        format!("{display}; was a user tag, now curated")
    } else {
        display.clone()
    };
    let key = format!("{namespace}:{slug}");
    log_action(&mut tx, admin, AdminAction::CreateTag, &key, Some(&detail)).await?;
    let tag = load_tag(&mut tx, &namespace, &slug).await?;
    tx.commit().await?;
    Ok(tag)
}

/// Tag requests waiting for an administrator.
pub async fn pending_count(state: &AppState) -> Result<u64, ApiError> {
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM tags WHERE status = 'pending'")
        .fetch_one(&state.pool)
        .await?;
    Ok(u64::try_from(n).unwrap_or(0))
}

struct LockedTag {
    id: i64,
    display: String,
    status: TagStatus,
}

async fn lock_tag(
    conn: &mut PgConnection,
    namespace: &str,
    slug: &str,
) -> Result<LockedTag, ApiError> {
    let (id, display, status): (i64, String, String) = sqlx::query_as(
        "SELECT id, display, status FROM tags
         WHERE instance = '' AND namespace = $1 AND slug = $2 FOR UPDATE",
    )
    .bind(namespace)
    .bind(slug)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(LockedTag {
        id,
        display,
        status: parse_enum(&status)?,
    })
}

/// Renames a tag, or merges it into another tag as an alias.
pub async fn update_tag(
    state: &AppState,
    admin: Uuid,
    namespace: &str,
    slug: &str,
    body: &UpdateTag,
) -> Result<Tag, ApiError> {
    let mut tx = state.pool.begin().await?;
    let LockedTag {
        id,
        display: old_display,
        status,
    } = lock_tag(&mut tx, namespace, slug).await?;
    let key = format!("{namespace}:{slug}");

    if body.status == Some(TagStatus::Pending) {
        return Err(ApiError::BadRequest(
            "only writers' new tags can be awaiting approval".into(),
        ));
    }
    if let Some(new_status) = body.status.filter(|s| *s != status) {
        let action = match (status, new_status) {
            (TagStatus::Pending, TagStatus::Visible) => AdminAction::ApproveTag,
            (_, TagStatus::Visible) => AdminAction::ShowTag,
            (_, TagStatus::Hidden | TagStatus::Pending) => AdminAction::HideTag,
        };
        sqlx::query("UPDATE tags SET status = $2 WHERE id = $1")
            .bind(id)
            .bind(new_status.as_str())
            .execute(&mut *tx)
            .await?;
        let detail = (status == TagStatus::Pending && new_status == TagStatus::Hidden)
            .then_some("was awaiting approval; kept hidden");
        let log_id = log_action(&mut tx, admin, action, &key, detail).await?;
        let kind = match action {
            AdminAction::ApproveTag => NoticeKind::TagApproved,
            AdminAction::ShowTag => NoticeKind::TagShown,
            _ => NoticeKind::TagHidden,
        };
        let affected = notices::affected_by_tag(&mut tx, id).await?;
        notices::record(
            &mut tx,
            log_id,
            kind,
            &tag_fields(&key, &old_display),
            &affected,
        )
        .await?;
    }

    if let Some(display) = body.display.as_deref() {
        let display = validate::tag_display(display)?;
        if display != old_display {
            sqlx::query("UPDATE tags SET display = $2 WHERE id = $1")
                .bind(id)
                .bind(&display)
                .execute(&mut *tx)
                .await?;
            let detail = format!("renamed from {old_display} to {display}");
            let log_id =
                log_action(&mut tx, admin, AdminAction::RenameTag, &key, Some(&detail)).await?;
            let fields = NoticeFields {
                from: Some(old_display.clone()),
                to: Some(display.clone()),
                ..tag_fields(&key, &old_display)
            };
            let affected = notices::affected_by_tag(&mut tx, id).await?;
            notices::record(&mut tx, log_id, NoticeKind::TagRenamed, &fields, &affected).await?;
        }
    }

    let new_color = checked_color(body.color.as_ref())?;
    if new_color.is_some() || body.clear_color {
        let (old_bg, old_fg): (Option<String>, Option<String>) =
            sqlx::query_as("SELECT chip_bg, chip_fg FROM tags WHERE id = $1")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        let old = old_bg.map(|background| ChipColor {
            background,
            text: old_fg,
        });
        if new_color != old {
            sqlx::query("UPDATE tags SET chip_bg = $2, chip_fg = $3 WHERE id = $1")
                .bind(id)
                .bind(new_color.as_ref().map(|c| c.background.clone()))
                .bind(new_color.as_ref().and_then(|c| c.text.clone()))
                .execute(&mut *tx)
                .await?;
            let detail = match &new_color {
                Some(_) => format!("colour: {}", describe_color(new_color.as_ref())),
                None => "colour: same as its type".to_owned(),
            };
            log_action(&mut tx, admin, AdminAction::RecolorTag, &key, Some(&detail)).await?;
        }
    }

    if let Some(target) = body.alias_of.as_deref() {
        let spec = validate::tag_spec(target)?;
        let target_key = (spec.namespace, spec.slug);
        let target_id = *resolve_tags(&mut tx, std::slice::from_ref(&target_key), false)
            .await?
            .get(&target_key)
            .ok_or_else(|| ApiError::BadRequest(format!("unknown tag \"{target}\"")))?;
        if target_id == id {
            return Err(ApiError::BadRequest(
                "a tag can't be an alias of itself".into(),
            ));
        }
        // Who is told: the authors of prompts carrying it, before they move.
        let affected = notices::affected_by_tag(&mut tx, id).await?;
        // This tag and every alias of it now point at the target.
        sqlx::query("UPDATE tags SET canonical_id = $2 WHERE id = $1 OR canonical_id = $1")
            .bind(id)
            .bind(target_id)
            .execute(&mut *tx)
            .await?;
        // Move prompts over, keeping each prompt's tag order and dropping
        // duplicates.
        let moved = sqlx::query(
            "UPDATE prompts SET tag_ids = ARRAY(
                 SELECT t FROM unnest(array_replace(tag_ids, $1, $2)) WITH ORDINALITY AS u(t, n)
                 GROUP BY t ORDER BY min(n))
             WHERE tag_ids @> ARRAY[$1::bigint]",
        )
        .bind(id)
        .bind(target_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        // A merged request is resolved: the name now just points at the target.
        sqlx::query("UPDATE tags SET status = 'visible' WHERE id = $1 AND status = 'pending'")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let request = if status == TagStatus::Pending {
            "request merged; "
        } else {
            ""
        };
        let detail = format!(
            "{request}now an alias of {}:{}; {moved} prompt(s) moved",
            target_key.0, target_key.1
        );
        let log_id = log_action(&mut tx, admin, AdminAction::AliasTag, &key, Some(&detail)).await?;
        let fields = NoticeFields {
            into: Some(format!("{}:{}", target_key.0, target_key.1)),
            ..tag_fields(&key, &old_display)
        };
        notices::record(&mut tx, log_id, NoticeKind::TagMerged, &fields, &affected).await?;
    }
    let tag = load_tag(&mut tx, namespace, slug).await?;
    tx.commit().await?;
    Ok(tag)
}

/// Deletes a tag and its aliases, removing it from every prompt.
pub async fn delete_tag(
    state: &AppState,
    admin: Uuid,
    namespace: &str,
    slug: &str,
) -> Result<DeletedTag, ApiError> {
    let mut tx = state.pool.begin().await?;
    let LockedTag {
        id,
        display,
        status,
    } = lock_tag(&mut tx, namespace, slug).await?;
    let affected = notices::affected_by_tag(&mut tx, id).await?;
    let prompts_updated = sqlx::query(
        "UPDATE prompts SET tag_ids = array_remove(tag_ids, $1) WHERE tag_ids @> ARRAY[$1::bigint]",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let aliases = sqlx::query("DELETE FROM tags WHERE canonical_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    sqlx::query("DELETE FROM tags WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let mut detail = format!("{display}; removed from {prompts_updated} prompt(s)");
    if aliases > 0 {
        detail.push_str(&format!("; {aliases} alias(es) deleted too"));
    }
    let key = format!("{namespace}:{slug}");
    // Deleting a tag awaiting approval is how a request is rejected.
    let action = if status == TagStatus::Pending {
        AdminAction::RejectTag
    } else {
        AdminAction::DeleteTag
    };
    let log_id = log_action(&mut tx, admin, action, &key, Some(&detail)).await?;
    let kind = if status == TagStatus::Pending {
        NoticeKind::TagRejected
    } else {
        NoticeKind::TagDeleted
    };
    notices::record(
        &mut tx,
        log_id,
        kind,
        &tag_fields(&key, &display),
        &affected,
    )
    .await?;
    tx.commit().await?;
    Ok(DeletedTag { prompts_updated })
}

// ---- JSON API ------------------------------------------------------------

#[utoipa::path(
    post, path = "/admin/tag-namespaces", tag = "admin", security(("bearer" = [])),
    summary = "Create a tag namespace",
    request_body = CreateNamespace,
    responses((status = 201, body = TagNamespace), (status = 409, description = "It exists")),
)]
async fn api_create_namespace(
    State(state): State<AppState>,
    admin: Admin,
    ApiJson(body): ApiJson<CreateNamespace>,
) -> Result<impl IntoResponse, ApiError> {
    let ns = create_namespace(&state, admin.0.account_id, &body).await?;
    Ok((StatusCode::CREATED, Json(ns)))
}

#[utoipa::path(
    patch, path = "/admin/tag-namespaces/{name}", tag = "admin", security(("bearer" = [])),
    summary = "Change a namespace: its label, whether it is locked, its limits or colours",
    params(("name" = String, Path, description = "The namespace")),
    request_body = UpdateNamespace,
    responses((status = 200, body = TagNamespace), (status = 404, description = "No such namespace")),
)]
async fn api_update_namespace(
    State(state): State<AppState>,
    admin: Admin,
    Path(name): Path<String>,
    ApiJson(body): ApiJson<UpdateNamespace>,
) -> Result<Json<TagNamespace>, ApiError> {
    update_namespace(&state, admin.0.account_id, &name, &body)
        .await
        .map(Json)
}

#[utoipa::path(
    get, path = "/admin/tags", tag = "admin", security(("bearer" = [])),
    summary = "Tags with their use and status",
    params(AdminTagQuery),
    responses((status = 200, body = Vec<AdminTag>)),
)]
async fn api_list_tags(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<AdminTagQuery>,
) -> Result<Json<Vec<AdminTag>>, ApiError> {
    list_tags(&state, &q).await.map(Json)
}

#[utoipa::path(
    get, path = "/admin/tags/{namespace}/{slug}", tag = "admin", security(("bearer" = [])),
    summary = "One tag",
    params(("namespace" = String, Path), ("slug" = String, Path)),
    responses((status = 200, body = AdminTag), (status = 404, description = "No such tag")),
)]
async fn api_get_tag(
    State(state): State<AppState>,
    _admin: Admin,
    Path((namespace, slug)): Path<(String, String)>,
) -> Result<Json<AdminTag>, ApiError> {
    get_tag(&state, &namespace, &slug).await.map(Json)
}

#[utoipa::path(
    post, path = "/admin/tags", tag = "admin", security(("bearer" = [])),
    summary = "Create a tag, in any namespace",
    request_body = CreateTag,
    responses((status = 201, body = Tag), (status = 409, description = "It exists")),
)]
async fn api_create_tag(
    State(state): State<AppState>,
    admin: Admin,
    ApiJson(body): ApiJson<CreateTag>,
) -> Result<impl IntoResponse, ApiError> {
    let tag = create_tag(&state, admin.0.account_id, &body).await?;
    Ok((StatusCode::CREATED, Json(tag)))
}

#[utoipa::path(
    patch, path = "/admin/tags/{namespace}/{slug}", tag = "admin", security(("bearer" = [])),
    summary = "Rename, recolour, approve, merge or hide a tag",
    params(("namespace" = String, Path), ("slug" = String, Path)),
    request_body = UpdateTag,
    responses(
        (status = 200, body = Tag),
        (status = 404, description = "No such tag"),
        (status = 409, description = "The change clashes with another tag"),
    ),
)]
async fn api_update_tag(
    State(state): State<AppState>,
    admin: Admin,
    Path((namespace, slug)): Path<(String, String)>,
    ApiJson(body): ApiJson<UpdateTag>,
) -> Result<Json<Tag>, ApiError> {
    update_tag(&state, admin.0.account_id, &namespace, &slug, &body)
        .await
        .map(Json)
}

#[utoipa::path(
    delete, path = "/admin/tags/{namespace}/{slug}", tag = "admin", security(("bearer" = [])),
    summary = "Delete a tag, taking it off every prompt",
    params(("namespace" = String, Path), ("slug" = String, Path)),
    responses((status = 200, body = DeletedTag), (status = 404, description = "No such tag")),
)]
async fn api_delete_tag(
    State(state): State<AppState>,
    admin: Admin,
    Path((namespace, slug)): Path<(String, String)>,
) -> Result<Json<DeletedTag>, ApiError> {
    delete_tag(&state, admin.0.account_id, &namespace, &slug)
        .await
        .map(Json)
}
