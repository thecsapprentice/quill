//! Prompt board handlers: prompts, tags and account settings.

use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use quill_proto::{
    board::{
        AccountSettings, CreatePrompt, CreateTag, Maturity, Prompt, PromptAuthor, PromptPage,
        PromptQuery, PromptState, Tag, TagNamespace, TagOrigin, TagPolicy, TagQuery, TagStatus,
        UpdateAccountSettings, UpdatePrompt,
    },
    limits::{DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE, SEARCH_QUERY_MAX_CHARS},
    validate::{self, TagSpec},
};
use sqlx::{AssertSqlSafe, PgConnection, PgPool, QueryBuilder};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::{Authed, MaybeAuthed},
    error::ApiError,
    markdown,
    routes::ApiJson,
    server_settings,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/account", get(get_account).patch(update_account))
        .route("/prompts", get(list_prompts).post(create_prompt))
        .route(
            "/prompts/{id}",
            get(get_prompt).patch(update_prompt).delete(delete_prompt),
        )
        .route("/tag-namespaces", get(list_namespaces))
        .route("/tags", get(list_tags).post(create_user_tag))
}

pub(crate) fn parse_enum<T: std::str::FromStr>(value: &str) -> Result<T, ApiError> {
    value
        .parse()
        .map_err(|_| ApiError::Internal(anyhow::anyhow!("unexpected enum value {value:?}")))
}

// ---- Account settings ----------------------------------------------------

/// Highest maturity this viewer may see on the board.
pub async fn viewer_cap(pool: &PgPool, viewer: Option<Uuid>) -> Result<Maturity, ApiError> {
    let Some(account) = viewer else {
        return Ok(Maturity::DEFAULT_MAX);
    };
    let raw: Option<String> =
        sqlx::query_scalar("SELECT maturity_max::text FROM accounts WHERE id = $1")
            .bind(account)
            .fetch_optional(pool)
            .await?;
    match raw {
        Some(raw) => parse_enum(&raw),
        None => Err(ApiError::Unauthorized),
    }
}

/// Whether `viewer` is a server administrator.
pub(crate) async fn viewer_is_admin(pool: &PgPool, viewer: Option<Uuid>) -> Result<bool, ApiError> {
    let Some(account) = viewer else {
        return Ok(false);
    };
    Ok(
        sqlx::query_scalar("SELECT is_admin FROM accounts WHERE id = $1")
            .bind(account)
            .fetch_optional(pool)
            .await?
            .unwrap_or(false),
    )
}

pub async fn account_settings(
    state: &AppState,
    account: Uuid,
) -> Result<AccountSettings, ApiError> {
    let row: Option<(String, bool)> =
        sqlx::query_as("SELECT maturity_max::text, is_admin FROM accounts WHERE id = $1")
            .bind(account)
            .fetch_optional(&state.pool)
            .await?;
    let (maturity, is_admin) = row.ok_or(ApiError::Unauthorized)?;
    Ok(AccountSettings {
        maturity_max: parse_enum(&maturity)?,
        is_admin,
    })
}

pub async fn update_account_settings(
    state: &AppState,
    account: Uuid,
    body: &UpdateAccountSettings,
) -> Result<AccountSettings, ApiError> {
    if let Some(m) = body.maturity_max {
        sqlx::query("UPDATE accounts SET maturity_max = $2::maturity WHERE id = $1")
            .bind(account)
            .bind(m.as_str())
            .execute(&state.pool)
            .await?;
    }
    account_settings(state, account).await
}

async fn get_account(
    State(state): State<AppState>,
    authed: Authed,
) -> Result<Json<AccountSettings>, ApiError> {
    account_settings(&state, authed.account_id).await.map(Json)
}

async fn update_account(
    State(state): State<AppState>,
    authed: Authed,
    ApiJson(body): ApiJson<UpdateAccountSettings>,
) -> Result<Json<AccountSettings>, ApiError> {
    update_account_settings(&state, authed.account_id, &body)
        .await
        .map(Json)
}

// ---- Loading prompts -----------------------------------------------------

const PROMPT_SELECT: &str = "SELECT p.id, p.title, p.body_md, p.body_html, \
     p.maturity::text AS maturity, p.state::text AS state, p.tag_ids, \
     p.created_at, p.updated_at, p.removed_at IS NOT NULL AS removed, \
     a.id AS author_id, a.preferred_username, a.display_name, a.account_id \
     FROM prompts p JOIN actors a ON a.id = p.actor_id";

#[derive(sqlx::FromRow)]
struct PromptRow {
    id: Uuid,
    title: String,
    body_md: String,
    body_html: String,
    maturity: String,
    state: String,
    tag_ids: Vec<i64>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    removed: bool,
    author_id: Uuid,
    preferred_username: String,
    display_name: Option<String>,
    account_id: Uuid,
}

async fn load_tags(pool: &PgPool, ids: &[i64]) -> Result<HashMap<i64, Tag>, ApiError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(i64, String, String, String, String, String)> = sqlx::query_as(
        "SELECT id, namespace, slug, display, origin, status FROM tags WHERE id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(id, namespace, slug, display, origin, status)| {
            Ok((
                id,
                Tag {
                    namespace,
                    slug,
                    display,
                    origin: parse_enum::<TagOrigin>(&origin)?,
                    status: parse_enum(&status)?,
                },
            ))
        })
        .collect()
}

/// Whether a viewer may see a tag on a prompt. Hidden tags are for
/// administrators; tags awaiting approval also show to the prompt's author.
fn tag_visible_to(tag: &Tag, admin: bool, is_author: bool) -> bool {
    match tag.status {
        TagStatus::Visible => true,
        TagStatus::Hidden => admin,
        TagStatus::Pending => admin || is_author,
    }
}

/// Turns rows into API prompts. `viewer` decides whether the "removed" flag
/// is shown (only to the author) and which tags are included.
async fn hydrate(
    pool: &PgPool,
    rows: Vec<PromptRow>,
    viewer: Option<Uuid>,
) -> Result<Vec<Prompt>, ApiError> {
    let admin = viewer_is_admin(pool, viewer).await?;
    let mut ids: Vec<i64> = rows
        .iter()
        .flat_map(|r| r.tag_ids.iter().copied())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let tags = load_tags(pool, &ids).await?;
    rows.into_iter()
        .map(|r| {
            Ok(Prompt {
                id: r.id,
                author: PromptAuthor {
                    id: r.author_id,
                    handle: r.preferred_username,
                    display_name: r.display_name,
                },
                title: r.title,
                body: r.body_md,
                body_html: r.body_html,
                maturity: parse_enum(&r.maturity)?,
                state: parse_enum(&r.state)?,
                tags: r
                    .tag_ids
                    .iter()
                    .filter_map(|id| tags.get(id))
                    .filter(|t| tag_visible_to(t, admin, viewer == Some(r.account_id)))
                    .cloned()
                    .collect(),
                accepting_sessions: false,
                removed: r.removed && viewer == Some(r.account_id),
                created_at: r.created_at,
                updated_at: r.updated_at,
            })
        })
        .collect()
}

/// Loads one prompt. Removed prompts are visible only to their author (or
/// to administrators when `include_removed` is set).
pub async fn load_prompt(
    pool: &PgPool,
    id: Uuid,
    viewer: Option<Uuid>,
    include_removed: bool,
) -> Result<Option<Prompt>, ApiError> {
    // Safe: built only from compile-time constants; values are bound.
    let sql = AssertSqlSafe(format!("{PROMPT_SELECT} WHERE p.id = $1"));
    let row: Option<PromptRow> = sqlx::query_as(sql).bind(id).fetch_optional(pool).await?;
    let Some(row) = row else { return Ok(None) };
    if row.removed && !include_removed && viewer != Some(row.account_id) {
        return Ok(None);
    }
    Ok(hydrate(pool, vec![row], viewer).await?.pop())
}

// ---- Tag resolution ------------------------------------------------------

/// Maps local `(namespace, slug)` pairs to canonical tag ids. Unknown pairs
/// are absent from the result, and so are tags whose canonical tag isn't
/// visible when `visible_only` is set.
pub(crate) async fn resolve_tags(
    conn: &mut PgConnection,
    keys: &[(String, String)],
    visible_only: bool,
) -> Result<HashMap<(String, String), i64>, ApiError> {
    if keys.is_empty() {
        return Ok(HashMap::new());
    }
    let (namespaces, slugs): (Vec<String>, Vec<String>) = keys.iter().cloned().unzip();
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT t.namespace, t.slug, COALESCE(t.canonical_id, t.id)
         FROM tags t
         LEFT JOIN tags c ON c.id = t.canonical_id
         JOIN unnest($1::text[], $2::text[]) AS f(ns, slug)
           ON t.namespace = f.ns AND t.slug = f.slug
         WHERE t.instance = ''
           AND (NOT $3 OR COALESCE(c.status, t.status) = 'visible')",
    )
    .bind(&namespaces)
    .bind(&slugs)
    .bind(visible_only)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(|(n, s, id)| ((n, s), id)).collect())
}

/// Resolves a prompt's tags, creating user tags where the server's policy
/// and the namespace allow it. Returns canonical ids in the author's order.
async fn resolve_for_write(
    conn: &mut PgConnection,
    specs: Vec<TagSpec>,
) -> Result<Vec<i64>, ApiError> {
    let keys: Vec<(String, String)> = specs
        .iter()
        .map(|s| (s.namespace.clone(), s.slug.clone()))
        .collect();
    let known = resolve_tags(conn, &keys, false).await?;
    let mut policy = None;
    let mut ids = Vec::with_capacity(specs.len());
    for spec in specs {
        let id = match known.get(&(spec.namespace.clone(), spec.slug.clone())) {
            Some(id) => *id,
            None => {
                let policy = match policy {
                    Some(p) => p,
                    None => *policy.insert(server_settings::tag_policy(conn).await?),
                };
                create_user_tag_in(conn, policy, &spec).await?.0
            }
        };
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Creates a user tag if allowed. Returns `(canonical id, created)`.
async fn create_user_tag_in(
    conn: &mut PgConnection,
    policy: TagPolicy,
    spec: &TagSpec,
) -> Result<(i64, bool), ApiError> {
    let locked: Option<bool> =
        sqlx::query_scalar("SELECT locked FROM tag_namespaces WHERE name = $1")
            .bind(&spec.namespace)
            .fetch_optional(&mut *conn)
            .await?;
    let missing = |why: String| {
        ApiError::BadRequest(format!(
            "\"{}:{}\" is not a tag on this server, and {why}",
            spec.namespace, spec.display
        ))
    };
    match locked {
        None => {
            return Err(ApiError::BadRequest(format!(
                "unknown tag namespace \"{}\"",
                spec.namespace
            )));
        }
        Some(true) => {
            return Err(missing(format!(
                "new tags can't be created in \"{}\"",
                spec.namespace
            )));
        }
        Some(false) if !policy.allows_new_tags() => {
            return Err(missing("this server doesn't accept new tags".to_owned()));
        }
        Some(false) => {}
    }
    // With approval required, the tag waits, hidden from everyone but its
    // authors and the administrators.
    let status = if policy == TagPolicy::Approval {
        TagStatus::Pending
    } else {
        TagStatus::Visible
    };
    let inserted: Option<i64> = sqlx::query_scalar(
        "INSERT INTO tags (namespace, slug, display, origin, status)
         VALUES ($1, $2, $3, 'user', $4)
         ON CONFLICT (instance, namespace, slug) DO NOTHING RETURNING id",
    )
    .bind(&spec.namespace)
    .bind(&spec.slug)
    .bind(&spec.display)
    .bind(status.as_str())
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = inserted {
        return Ok((id, true));
    }
    // Created concurrently by someone else: use theirs.
    let id: i64 = sqlx::query_scalar(
        "SELECT COALESCE(canonical_id, id) FROM tags
         WHERE instance = '' AND namespace = $1 AND slug = $2",
    )
    .bind(&spec.namespace)
    .bind(&spec.slug)
    .fetch_one(&mut *conn)
    .await?;
    Ok((id, false))
}

// ---- Listing and search --------------------------------------------------

fn encode_cursor(created_at: OffsetDateTime, id: Uuid) -> String {
    URL_SAFE_NO_PAD.encode(format!("{}.{}", created_at.unix_timestamp_nanos(), id))
}

fn decode_cursor(cursor: &str) -> Result<(OffsetDateTime, Uuid), ApiError> {
    let bad = || ApiError::BadRequest("invalid cursor".into());
    let bytes = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| bad())?;
    let text = String::from_utf8(bytes).map_err(|_| bad())?;
    let (nanos, id) = text.split_once('.').ok_or_else(bad)?;
    let nanos: i128 = nanos.parse().map_err(|_| bad())?;
    let at = OffsetDateTime::from_unix_timestamp_nanos(nanos).map_err(|_| bad())?;
    Ok((at, id.parse().map_err(|_| bad())?))
}

async fn list_prompts(
    State(state): State<AppState>,
    MaybeAuthed(authed): MaybeAuthed,
    Query(q): Query<PromptQuery>,
) -> Result<Json<PromptPage>, ApiError> {
    search_prompts(&state, authed.map(|a| a.account_id), &q)
        .await
        .map(Json)
}

/// Searches the board as `viewer` (an account id, or anonymous).
pub async fn search_prompts(
    state: &AppState,
    viewer: Option<Uuid>,
    q: &PromptQuery,
) -> Result<PromptPage, ApiError> {
    let limit = q.limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE);
    let mine = q.author.as_deref() == Some("me");
    if mine && viewer.is_none() {
        return Err(ApiError::Unauthorized);
    }
    let cap = viewer_cap(&state.pool, viewer).await?;
    let cap = q.maturity_max.map_or(cap, |m| m.min(cap));

    let include_keys = validate::tag_filter(q.include.as_deref())?;
    let exclude_keys = validate::tag_filter(q.exclude.as_deref())?;
    let mut conn = state.pool.acquire().await?;
    // Tags only administrators may see filter like unknown tags for everyone
    // else, so filtering can't reveal them.
    let visible_only = !viewer_is_admin(&state.pool, viewer).await?;
    let known_include = resolve_tags(&mut conn, &include_keys, visible_only).await?;
    if known_include.len() < include_keys.len() {
        // A required tag doesn't exist, so nothing can match.
        return Ok(PromptPage {
            items: Vec::new(),
            next_cursor: None,
        });
    }
    let include: Vec<i64> = known_include.into_values().collect();
    let exclude: Vec<i64> = resolve_tags(&mut conn, &exclude_keys, visible_only)
        .await?
        .into_values()
        .collect();
    drop(conn);

    let text =
        q.q.as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| t.chars().take(SEARCH_QUERY_MAX_CHARS).collect::<String>());
    let cursor = q.cursor.as_deref().map(decode_cursor).transpose()?;

    let mut qb = QueryBuilder::new(PROMPT_SELECT);
    qb.push(" WHERE true");
    if mine {
        qb.push(" AND a.account_id = ").push_bind(viewer);
    } else {
        qb.push(" AND p.removed_at IS NULL AND p.maturity <= ")
            .push_bind(cap.as_str())
            .push("::maturity");
    }
    if let Some(handle) = q.author.as_deref().filter(|h| *h != "me") {
        qb.push(" AND a.preferred_username = ")
            .push_bind(validate::username(handle).unwrap_or_default());
    }
    if mine && let Some(m) = q.maturity_max {
        qb.push(" AND p.maturity <= ")
            .push_bind(m.as_str())
            .push("::maturity");
    }
    if let Some(s) = q.state {
        qb.push(" AND p.state = ")
            .push_bind(s.as_str())
            .push("::prompt_state");
    }
    if !include.is_empty() {
        qb.push(" AND p.tag_ids @> ")
            .push_bind(include)
            .push("::bigint[]");
    }
    if !exclude.is_empty() {
        qb.push(" AND NOT (p.tag_ids && ")
            .push_bind(exclude)
            .push("::bigint[])");
    }
    if let Some(text) = text {
        qb.push(" AND p.search @@ websearch_to_tsquery('simple', ")
            .push_bind(text)
            .push(")");
    }
    if let Some((at, id)) = cursor {
        qb.push(" AND (p.created_at, p.id) < (")
            .push_bind(at)
            .push(", ")
            .push_bind(id)
            .push(")");
    }
    qb.push(" ORDER BY p.created_at DESC, p.id DESC LIMIT ")
        .push_bind(i64::from(limit) + 1);

    let mut rows: Vec<PromptRow> = qb.build_query_as().fetch_all(&state.pool).await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = if more {
        rows.last().map(|r| encode_cursor(r.created_at, r.id))
    } else {
        None
    };
    let items = hydrate(&state.pool, rows, viewer).await?;
    Ok(PromptPage { items, next_cursor })
}

/// `GET /prompts/{id}`. The maturity cap is not applied to a direct link;
/// clients warn before showing a prompt above the viewer's setting.
async fn get_prompt(
    State(state): State<AppState>,
    MaybeAuthed(authed): MaybeAuthed,
    Path(id): Path<Uuid>,
) -> Result<Json<Prompt>, ApiError> {
    let viewer = authed.map(|a| a.account_id);
    load_prompt(&state.pool, id, viewer, false)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

// ---- Writing prompts -----------------------------------------------------

async fn create_prompt(
    State(state): State<AppState>,
    authed: Authed,
    ApiJson(body): ApiJson<CreatePrompt>,
) -> Result<impl IntoResponse, ApiError> {
    let prompt = post_prompt(&state, authed.account_id, &body).await?;
    Ok((StatusCode::CREATED, Json(prompt)))
}

/// Posts a prompt as one of `account`'s personas.
pub async fn post_prompt(
    state: &AppState,
    account: Uuid,
    body: &CreatePrompt,
) -> Result<Prompt, ApiError> {
    let title = validate::prompt_title(&body.title)?;
    let markdown_body = validate::prompt_body(&body.body)?;
    let specs = validate::tag_specs(&body.tags)?;
    let prompt_state = body.state.unwrap_or(PromptState::Open);

    let mut tx = state.pool.begin().await?;
    let owns: Option<i32> =
        sqlx::query_scalar("SELECT 1 FROM actors WHERE id = $1 AND account_id = $2")
            .bind(body.persona)
            .bind(account)
            .fetch_optional(&mut *tx)
            .await?;
    if owns.is_none() {
        return Err(ApiError::BadRequest("unknown persona".into()));
    }
    let tag_ids = resolve_for_write(&mut tx, specs).await?;
    check_required_types(&mut tx, &tag_ids).await?;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO prompts (id, actor_id, title, body_md, body_html, maturity, state, tag_ids)
         VALUES ($1, $2, $3, $4, $5, $6::maturity, $7::prompt_state, $8)",
    )
    .bind(id)
    .bind(body.persona)
    .bind(&title)
    .bind(&markdown_body)
    .bind(markdown::render(&markdown_body))
    .bind(body.maturity.as_str())
    .bind(prompt_state.as_str())
    .bind(&tag_ids)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    load_prompt(&state.pool, id, Some(account), true)
        .await?
        .ok_or(ApiError::NotFound)
}

async fn update_prompt(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
    ApiJson(body): ApiJson<UpdatePrompt>,
) -> Result<Json<Prompt>, ApiError> {
    edit_prompt(&state, authed.account_id, id, &body)
        .await
        .map(Json)
}

/// Edits a prompt posted by one of `account`'s personas.
pub async fn edit_prompt(
    state: &AppState,
    account: Uuid,
    id: Uuid,
    body: &UpdatePrompt,
) -> Result<Prompt, ApiError> {
    let title = body
        .title
        .as_deref()
        .map(validate::prompt_title)
        .transpose()?;
    let markdown_body = body
        .body
        .as_deref()
        .map(validate::prompt_body)
        .transpose()?;
    let specs = body.tags.as_deref().map(validate::tag_specs).transpose()?;

    let mut tx = state.pool.begin().await?;
    let current: Option<(bool, Vec<i64>)> = sqlx::query_as(
        "SELECT p.removed_at IS NOT NULL, p.tag_ids FROM prompts p JOIN actors a ON a.id = p.actor_id
         WHERE p.id = $1 AND a.account_id = $2 FOR UPDATE OF p",
    )
    .bind(id)
    .bind(account)
    .fetch_optional(&mut *tx)
    .await?;
    let current_tags = match current {
        None => return Err(ApiError::NotFound),
        Some((true, _)) => {
            return Err(ApiError::Forbidden(
                "this prompt was removed by a server administrator".into(),
            ));
        }
        Some((false, tags)) => tags,
    };
    let tag_ids = match specs {
        Some(specs) => {
            let mut ids = resolve_for_write(&mut tx, specs).await?;
            // Editors who can't see a prompt's hidden tags can't remove them
            // either: they weren't on the form, so keep them.
            if !viewer_is_admin(&state.pool, Some(account)).await? {
                let hidden: Vec<i64> = sqlx::query_scalar(
                    "SELECT id FROM tags WHERE id = ANY($1) AND status = 'hidden'",
                )
                .bind(&current_tags)
                .fetch_all(&mut *tx)
                .await?;
                for id in current_tags.iter().filter(|id| hidden.contains(id)) {
                    if !ids.contains(id) {
                        ids.push(*id);
                    }
                }
            }
            Some(ids)
        }
        None => None,
    };
    // Any edit must meet the current required tag types. Changing only the
    // open/busy/closed status doesn't count as an edit.
    let edits_content = body.title.is_some()
        || body.body.is_some()
        || body.maturity.is_some()
        || body.tags.is_some();
    if edits_content {
        check_required_types(&mut tx, tag_ids.as_deref().unwrap_or(&current_tags)).await?;
    }
    let body_html = markdown_body.as_deref().map(markdown::render);
    sqlx::query(
        "UPDATE prompts SET
             title     = COALESCE($2, title),
             body_md   = COALESCE($3, body_md),
             body_html = COALESCE($4, body_html),
             maturity  = COALESCE($5::maturity, maturity),
             state     = COALESCE($6::prompt_state, state),
             tag_ids   = COALESCE($7, tag_ids),
             updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(title)
    .bind(markdown_body)
    .bind(body_html)
    .bind(body.maturity.map(Maturity::as_str))
    .bind(body.state.map(PromptState::as_str))
    .bind(tag_ids)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    load_prompt(&state.pool, id, Some(account), true)
        .await?
        .ok_or(ApiError::NotFound)
}

async fn delete_prompt(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    remove_own_prompt(&state, authed.account_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes a prompt posted by one of `account`'s personas.
pub async fn remove_own_prompt(state: &AppState, account: Uuid, id: Uuid) -> Result<(), ApiError> {
    // Phase 3 adds a federated `Delete`; Phase 2 expires pending requests.
    let result = sqlx::query(
        "DELETE FROM prompts p USING actors a
         WHERE p.id = $1 AND p.actor_id = a.id AND a.account_id = $2",
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

// ---- Tags ----------------------------------------------------------------

async fn list_namespaces(
    State(state): State<AppState>,
) -> Result<Json<Vec<TagNamespace>>, ApiError> {
    namespaces(&state).await.map(Json)
}

pub async fn namespaces(state: &AppState) -> Result<Vec<TagNamespace>, ApiError> {
    let rows: Vec<(String, String, bool, bool)> =
        sqlx::query_as("SELECT name, display, locked, required FROM tag_namespaces ORDER BY name")
            .fetch_all(&state.pool)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(name, display, locked, required)| TagNamespace {
            name,
            display,
            locked,
            required,
        })
        .collect())
}

/// One tag type, optionally locked for update.
pub(crate) async fn load_namespace(
    conn: &mut PgConnection,
    name: &str,
    for_update: bool,
) -> Result<TagNamespace, ApiError> {
    let sql = if for_update {
        "SELECT name, display, locked, required FROM tag_namespaces WHERE name = $1 FOR UPDATE"
    } else {
        "SELECT name, display, locked, required FROM tag_namespaces WHERE name = $1"
    };
    let (name, display, locked, required): (String, String, bool, bool) = sqlx::query_as(sql)
        .bind(name)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(TagNamespace {
        name,
        display,
        locked,
        required,
    })
}

/// Refuses a prompt whose tags lack a currently required tag type.
async fn check_required_types(conn: &mut PgConnection, tag_ids: &[i64]) -> Result<(), ApiError> {
    let missing: Vec<String> = sqlx::query_scalar(
        "SELECT n.display FROM tag_namespaces n
         WHERE n.required AND NOT EXISTS (
             SELECT 1 FROM tags t WHERE t.id = ANY($1) AND t.namespace = n.name)
         ORDER BY n.display",
    )
    .bind(tag_ids)
    .fetch_all(&mut *conn)
    .await?;
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "Every prompt needs a tag of each required type. Add: {}.",
            missing.join(", ")
        )))
    }
}

pub(crate) fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

async fn list_tags(
    State(state): State<AppState>,
    Query(q): Query<TagQuery>,
) -> Result<Json<Vec<Tag>>, ApiError> {
    search_tags(&state, &q).await.map(Json)
}

/// Canonical local tags, optionally filtered by namespace and prefix.
pub async fn search_tags(state: &AppState, q: &TagQuery) -> Result<Vec<Tag>, ApiError> {
    let limit = q.limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE);
    let mut qb = QueryBuilder::new(
        "SELECT namespace, slug, display, origin, status FROM tags
         WHERE instance = '' AND canonical_id IS NULL AND status = 'visible'",
    );
    if let Some(ns) = q.namespace.as_deref() {
        qb.push(" AND namespace = ")
            .push_bind(validate::namespace(ns)?);
    }
    if let Some(prefix) = q.prefix.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        let display = format!("{}%", escape_like(&prefix.to_lowercase()));
        qb.push(" AND (lower(display) LIKE ").push_bind(display);
        if let Ok(slug) = validate::tag_slug(prefix) {
            qb.push(" OR slug LIKE ")
                .push_bind(format!("{}%", escape_like(&slug)));
        }
        qb.push(")");
    }
    qb.push(" ORDER BY (origin = 'server') DESC, namespace, slug LIMIT ")
        .push_bind(i64::from(limit));
    let rows: Vec<(String, String, String, String, String)> =
        qb.build_query_as().fetch_all(&state.pool).await?;
    rows.into_iter()
        .map(|(namespace, slug, display, origin, status)| {
            Ok(Tag {
                namespace,
                slug,
                display,
                origin: parse_enum(&origin)?,
                status: parse_enum(&status)?,
            })
        })
        .collect()
}

pub(crate) async fn load_tag(
    conn: &mut PgConnection,
    namespace: &str,
    slug: &str,
) -> Result<Tag, ApiError> {
    let (namespace, slug, display, origin, status): (String, String, String, String, String) =
        sqlx::query_as(
            "SELECT namespace, slug, display, origin, status FROM tags
             WHERE instance = '' AND namespace = $1 AND slug = $2",
        )
        .bind(namespace)
        .bind(slug)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Tag {
        namespace,
        slug,
        display,
        origin: parse_enum(&origin)?,
        status: parse_enum(&status)?,
    })
}

/// `POST /tags`: create a user tag ahead of using it.
async fn create_user_tag(
    State(state): State<AppState>,
    _authed: Authed,
    ApiJson(body): ApiJson<CreateTag>,
) -> Result<impl IntoResponse, ApiError> {
    let spec = TagSpec {
        namespace: validate::namespace(&body.namespace)?,
        slug: validate::tag_slug(&body.display)?,
        display: validate::tag_display(&body.display)?,
    };
    let mut conn = state.pool.acquire().await?;
    let policy = server_settings::tag_policy(&mut conn).await?;
    let (_, created) = create_user_tag_in(&mut conn, policy, &spec).await?;
    let tag = load_tag(&mut conn, &spec.namespace, &spec.slug).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(tag)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trip() {
        let at = OffsetDateTime::from_unix_timestamp_nanos(1_760_861_400_123_456_000).unwrap();
        let id = Uuid::now_v7();
        assert_eq!(decode_cursor(&encode_cursor(at, id)).unwrap(), (at, id));
        assert!(decode_cursor("not-a-cursor").is_err());
    }

    #[test]
    fn like_escaping() {
        assert_eq!(escape_like(r"50%_off\"), r"50\%\_off\\");
    }
}
