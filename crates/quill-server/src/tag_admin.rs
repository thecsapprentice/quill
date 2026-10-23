//! Tag administration: tag types (namespaces) and tags, for server
//! administrators. Every change is written to the admin log.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, patch},
};
use quill_proto::{
    board::{
        AdminAction, AdminTag, AdminTagQuery, CreateNamespace, CreateTag, DeletedTag, Tag,
        TagNamespace, UpdateNamespace, UpdateTag,
    },
    limits::MAX_PAGE_SIZE,
    validate,
};
use sqlx::{PgConnection, QueryBuilder};
use uuid::Uuid;

use crate::{
    AppState,
    auth::Admin,
    error::{ApiError, is_unique_violation},
    prompts::{escape_like, load_namespace, load_tag, parse_enum, resolve_tags},
    reports::log_action,
    routes::ApiJson,
};

/// Most tags one admin listing returns.
pub const ADMIN_TAG_LIMIT: u32 = 200;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/tag-namespaces",
            axum::routing::post(api_create_namespace),
        )
        .route("/admin/tag-namespaces/{name}", patch(api_update_namespace))
        .route("/admin/tags", get(api_list_tags).post(api_create_tag))
        .route(
            "/admin/tags/{namespace}/{slug}",
            get(api_get_tag)
                .patch(api_update_tag)
                .delete(api_delete_tag),
        )
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

// ---- Tag types -----------------------------------------------------------

/// Creates a tag type.
pub async fn create_namespace(
    state: &AppState,
    admin: Uuid,
    body: &CreateNamespace,
) -> Result<TagNamespace, ApiError> {
    let name = validate::namespace(&body.name)?;
    let display = validate::tag_display(&body.display)?;
    let mut tx = state.pool.begin().await?;
    let result = sqlx::query(
        "INSERT INTO tag_namespaces (name, display, locked, required) VALUES ($1, $2, $3, $4)",
    )
    .bind(&name)
    .bind(&display)
    .bind(body.locked)
    .bind(body.required)
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
    let detail = format!(
        "{display}; curated tags only: {}; required: {}",
        on_off(body.locked),
        on_off(body.required)
    );
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
        required: body.required,
    })
}

/// Renames a tag type or changes whether it is curated-only or required.
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
    let mut tx = state.pool.begin().await?;
    let before = load_namespace(&mut tx, name, true).await?;
    let after = TagNamespace {
        name: before.name.clone(),
        display: display.unwrap_or_else(|| before.display.clone()),
        locked: body.locked.unwrap_or(before.locked),
        required: body.required.unwrap_or(before.required),
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
    if after.required != before.required {
        changes.push(format!("required: {}", on_off(after.required)));
    }
    if changes.is_empty() {
        return Ok(before);
    }
    sqlx::query(
        "UPDATE tag_namespaces SET display = $2, locked = $3, required = $4 WHERE name = $1",
    )
    .bind(&after.name)
    .bind(&after.display)
    .bind(after.locked)
    .bind(after.required)
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

type AdminTagRow = (String, String, String, String, Option<String>, i64);

fn to_admin_tag(row: AdminTagRow) -> Result<AdminTag, ApiError> {
    let (namespace, slug, display, origin, alias_of, uses) = row;
    Ok(AdminTag {
        tag: Tag {
            namespace,
            slug,
            display,
            origin: parse_enum(&origin)?,
        },
        uses: u64::try_from(uses).unwrap_or(0),
        alias_of,
    })
}

const ADMIN_TAG_SELECT: &str = "SELECT t.namespace, t.slug, t.display, t.origin,
            c.namespace || ':' || c.slug,
            (SELECT count(*) FROM prompts p WHERE p.tag_ids @> ARRAY[t.id])
     FROM tags t LEFT JOIN tags c ON c.id = t.canonical_id
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
         DO UPDATE SET origin = 'server', display = EXCLUDED.display",
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

async fn lock_tag(
    conn: &mut PgConnection,
    namespace: &str,
    slug: &str,
) -> Result<(i64, String), ApiError> {
    sqlx::query_as(
        "SELECT id, display FROM tags
         WHERE instance = '' AND namespace = $1 AND slug = $2 FOR UPDATE",
    )
    .bind(namespace)
    .bind(slug)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(ApiError::NotFound)
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
    let (id, old_display) = lock_tag(&mut tx, namespace, slug).await?;
    let key = format!("{namespace}:{slug}");

    if let Some(display) = body.display.as_deref() {
        let display = validate::tag_display(display)?;
        if display != old_display {
            sqlx::query("UPDATE tags SET display = $2 WHERE id = $1")
                .bind(id)
                .bind(&display)
                .execute(&mut *tx)
                .await?;
            let detail = format!("renamed from {old_display} to {display}");
            log_action(&mut tx, admin, AdminAction::RenameTag, &key, Some(&detail)).await?;
        }
    }

    if let Some(target) = body.alias_of.as_deref() {
        let spec = validate::tag_spec(target)?;
        let target_key = (spec.namespace, spec.slug);
        let target_id = *resolve_tags(&mut tx, std::slice::from_ref(&target_key))
            .await?
            .get(&target_key)
            .ok_or_else(|| ApiError::BadRequest(format!("unknown tag \"{target}\"")))?;
        if target_id == id {
            return Err(ApiError::BadRequest(
                "a tag can't be an alias of itself".into(),
            ));
        }
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
        let detail = format!(
            "now an alias of {}:{}; {moved} prompt(s) moved",
            target_key.0, target_key.1
        );
        log_action(&mut tx, admin, AdminAction::AliasTag, &key, Some(&detail)).await?;
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
    let (id, display) = lock_tag(&mut tx, namespace, slug).await?;
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
    log_action(&mut tx, admin, AdminAction::DeleteTag, &key, Some(&detail)).await?;
    tx.commit().await?;
    Ok(DeletedTag { prompts_updated })
}

// ---- JSON API ------------------------------------------------------------

async fn api_create_namespace(
    State(state): State<AppState>,
    admin: Admin,
    ApiJson(body): ApiJson<CreateNamespace>,
) -> Result<impl IntoResponse, ApiError> {
    let ns = create_namespace(&state, admin.0.account_id, &body).await?;
    Ok((StatusCode::CREATED, Json(ns)))
}

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

async fn api_list_tags(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<AdminTagQuery>,
) -> Result<Json<Vec<AdminTag>>, ApiError> {
    list_tags(&state, &q).await.map(Json)
}

async fn api_get_tag(
    State(state): State<AppState>,
    _admin: Admin,
    Path((namespace, slug)): Path<(String, String)>,
) -> Result<Json<AdminTag>, ApiError> {
    get_tag(&state, &namespace, &slug).await.map(Json)
}

async fn api_create_tag(
    State(state): State<AppState>,
    admin: Admin,
    ApiJson(body): ApiJson<CreateTag>,
) -> Result<impl IntoResponse, ApiError> {
    let tag = create_tag(&state, admin.0.account_id, &body).await?;
    Ok((StatusCode::CREATED, Json(tag)))
}

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

async fn api_delete_tag(
    State(state): State<AppState>,
    admin: Admin,
    Path((namespace, slug)): Path<(String, String)>,
) -> Result<Json<DeletedTag>, ApiError> {
    delete_tag(&state, admin.0.account_id, &namespace, &slug)
        .await
        .map(Json)
}
