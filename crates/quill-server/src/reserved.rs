//! Reserved persona handles: names writers can't take.
//!
//! The list lives in the `reserved_handles` table. It starts as Quill's
//! defaults (seeded once, so administrators can remove them), administrators
//! add and remove entries in Server settings or the API, and an optional
//! file named by `QUILL_RESERVED_HANDLES_FILE` is synced at startup for large
//! or automated lists. The server's own name is always reserved.
//!
//! How entries match is decided in [`quill_proto::handles`]; here the
//! candidates are looked up in the indexed table, so large lists stay fast.

use std::path::Path;

use anyhow::{Context, bail};
use axum::{
    Json, Router,
    extract::{Path as UrlPath, Query, State},
    http::StatusCode,
    routing::{delete, get},
};
use quill_proto::{
    board::AdminAction,
    handles::{
        self, AddReserved, AddedReserved, DEFAULT_EXACT, DEFAULT_WORDS, ReservedCheck,
        ReservedHandle, ReservedKind, ReservedList, ReservedQuery, ReservedSource,
    },
    validate,
};
use serde::Deserialize;
use sqlx::{PgConnection, PgPool, QueryBuilder};
use uuid::Uuid;

use crate::{
    AppState,
    auth::Admin,
    config::Config,
    error::ApiError,
    prompts::{escape_like, parse_enum},
    reports::{log_action, log_system},
    routes::ApiJson,
};

/// Most entries one listing returns.
pub const LIST_LIMIT: u32 = 500;
/// Marks that the defaults were seeded, so removed ones stay removed.
const DEFAULTS_SEEDED: &str = "reserved_defaults_seeded";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/reserved-handles", get(api_list).post(api_add))
        .route("/admin/reserved-handles/check", get(api_check))
        .route(
            "/admin/reserved-handles/{kind}/{pattern}",
            delete(api_remove),
        )
}

// ---- Startup ---------------------------------------------------------------

/// Seeds the defaults (once) and syncs the reserved-handles file.
pub(crate) async fn init(pool: &PgPool, config: &Config) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    let seeded: Option<i32> = sqlx::query_scalar(
        "INSERT INTO server_settings (key, value) VALUES ($1, 'yes')
         ON CONFLICT DO NOTHING RETURNING 1",
    )
    .bind(DEFAULTS_SEEDED)
    .fetch_optional(&mut *tx)
    .await?;
    if seeded.is_some() {
        let defaults = DEFAULT_WORDS
            .iter()
            .map(|w| (ReservedKind::Word, *w))
            .chain(DEFAULT_EXACT.iter().map(|e| (ReservedKind::Exact, *e)));
        for (kind, raw) in defaults {
            let pattern = handles::normalize_entry(kind, raw)?;
            insert(&mut tx, kind, &pattern, ReservedSource::Default).await?;
        }
    }
    tx.commit().await?;
    sync_file(pool, config.reserved_handles_file.as_deref()).await
}

/// Makes the table's file entries match the file exactly. Without a file,
/// file entries are removed. Entries also present as defaults or admin
/// entries keep their source. A bad line stops startup, with line numbers.
async fn sync_file(pool: &PgPool, path: Option<&Path>) -> anyhow::Result<()> {
    let wanted = match path {
        None => Vec::new(),
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading reserved handles from {}", path.display()))?;
            match handles::parse_file(&text) {
                Ok(entries) => entries,
                Err(errors) => {
                    let lines: Vec<String> = errors
                        .iter()
                        .take(10)
                        .map(|(line, e)| format!("  line {line}: {e}"))
                        .collect();
                    bail!(
                        "{} has {} bad line(s):\n{}",
                        path.display(),
                        errors.len(),
                        lines.join("\n")
                    );
                }
            }
        }
    };
    let mut tx = pool.begin().await?;
    let (kinds, patterns): (Vec<String>, Vec<String>) = wanted
        .iter()
        .map(|(k, p)| (k.as_str().to_owned(), p.clone()))
        .unzip();
    let removed = sqlx::query(
        "DELETE FROM reserved_handles r WHERE source = 'file'
           AND NOT EXISTS (SELECT 1 FROM unnest($1::text[], $2::text[]) AS w(kind, pattern)
                           WHERE w.kind = r.kind AND w.pattern = r.pattern)",
    )
    .bind(&kinds)
    .bind(&patterns)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let added = sqlx::query(
        "INSERT INTO reserved_handles (kind, pattern, source)
         SELECT kind, pattern, 'file' FROM unnest($1::text[], $2::text[]) AS w(kind, pattern)
         ON CONFLICT DO NOTHING",
    )
    .bind(&kinds)
    .bind(&patterns)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if added > 0 || removed > 0 {
        let detail = format!(
            "reserved handles file: {} entries; {added} added, {removed} removed",
            wanted.len()
        );
        log_system(&mut tx, AdminAction::ReserveHandles, "file", Some(&detail)).await?;
        tracing::info!(added, removed, "synced reserved handles file");
    }
    tx.commit().await?;
    Ok(())
}

async fn insert(
    conn: &mut PgConnection,
    kind: ReservedKind,
    pattern: &str,
    source: ReservedSource,
) -> Result<bool, sqlx::Error> {
    let r = sqlx::query(
        "INSERT INTO reserved_handles (kind, pattern, source) VALUES ($1, $2, $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(kind.as_str())
    .bind(pattern)
    .bind(source.as_str())
    .execute(&mut *conn)
    .await?;
    Ok(r.rows_affected() == 1)
}

// ---- Checking --------------------------------------------------------------

fn server_name(state: &AppState) -> Option<String> {
    handles::server_name(&state.config.domain)
}

/// The entry that reserves `handle` (already normalized), if any.
pub async fn reserved_by(
    state: &AppState,
    handle: &str,
) -> Result<Option<ReservedHandle>, ApiError> {
    if let Some(name) = server_name(state)
        && handles::is_reserved(handle, &[(ReservedKind::Word, &name)])
    {
        return Ok(Some(ReservedHandle {
            pattern: name,
            kind: ReservedKind::Word,
            source: ReservedSource::ServerName,
        }));
    }
    let (words, exact) = handles::candidates(handle);
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT pattern, kind, source FROM reserved_handles
         WHERE (kind = 'word' AND pattern = ANY($1)) OR (kind = 'exact' AND pattern = ANY($2))
         ORDER BY kind DESC, pattern LIMIT 1",
    )
    .bind(&words)
    .bind(&exact)
    .fetch_optional(&state.pool)
    .await?;
    row.map(to_entry).transpose()
}

fn to_entry((pattern, kind, source): (String, String, String)) -> Result<ReservedHandle, ApiError> {
    Ok(ReservedHandle {
        pattern,
        kind: parse_enum(&kind)?,
        source: parse_enum(&source)?,
    })
}

pub async fn check(state: &AppState, handle: &str) -> Result<ReservedCheck, ApiError> {
    let handle = validate::username(handle)?;
    Ok(ReservedCheck {
        reserved_by: reserved_by(state, &handle).await?,
        handle,
    })
}

// ---- Managing --------------------------------------------------------------

pub async fn list(state: &AppState, q: &ReservedQuery) -> Result<ReservedList, ApiError> {
    let limit = i64::from(q.limit.unwrap_or(LIST_LIMIT).clamp(1, LIST_LIMIT));
    let filter = |qb: &mut QueryBuilder<sqlx::Postgres>| {
        qb.push(" WHERE true");
        if let Some(search) = q.search.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            qb.push(" AND pattern LIKE ")
                .push_bind(format!("%{}%", escape_like(&search.to_ascii_lowercase())));
        }
        if let Some(kind) = q.kind {
            qb.push(" AND kind = ").push_bind(kind.as_str());
        }
        if let Some(source) = q.source {
            qb.push(" AND source = ").push_bind(source.as_str());
        }
    };
    let mut count = QueryBuilder::new("SELECT count(*) FROM reserved_handles");
    filter(&mut count);
    let total: i64 = count.build_query_scalar().fetch_one(&state.pool).await?;
    let mut qb = QueryBuilder::new("SELECT pattern, kind, source FROM reserved_handles");
    filter(&mut qb);
    qb.push(" ORDER BY pattern, kind LIMIT ").push_bind(limit);
    let rows: Vec<(String, String, String)> = qb.build_query_as().fetch_all(&state.pool).await?;
    Ok(ReservedList {
        entries: rows.into_iter().map(to_entry).collect::<Result<_, _>>()?,
        total: u64::try_from(total).unwrap_or(0),
        server_name: server_name(state),
        file_loaded: state.config.reserved_handles_file.is_some(),
    })
}

/// Reserves entries; all are checked before any is added.
pub async fn add(
    state: &AppState,
    admin: Uuid,
    body: &AddReserved,
) -> Result<AddedReserved, ApiError> {
    let mut patterns = Vec::new();
    for raw in body
        .patterns
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
    {
        patterns.push(handles::normalize_entry(body.kind, raw)?);
    }
    patterns.sort();
    patterns.dedup();
    if patterns.is_empty() {
        return Err(ApiError::BadRequest(
            "Enter at least one handle to reserve.".into(),
        ));
    }
    let mut tx = state.pool.begin().await?;
    let mut added = Vec::new();
    for p in &patterns {
        if insert(&mut tx, body.kind, p, ReservedSource::Admin).await? {
            added.push(p.as_str());
        }
    }
    if !added.is_empty() {
        let shown: Vec<&str> = added.iter().take(20).copied().collect();
        let more = if added.len() > shown.len() {
            format!(" and {} more", added.len() - shown.len())
        } else {
            String::new()
        };
        let detail = format!(
            "{} {}: {}{more}",
            added.len(),
            plural(body.kind),
            shown.join(", ")
        );
        log_action(
            &mut tx,
            admin,
            AdminAction::ReserveHandles,
            "handles",
            Some(&detail),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(AddedReserved {
        added: u64::try_from(added.len()).unwrap_or(0),
    })
}

fn plural(kind: ReservedKind) -> &'static str {
    match kind {
        ReservedKind::Word => "words",
        ReservedKind::Exact => "exact handles",
    }
}

/// Removes an entry. File entries can only be changed in the file.
pub async fn remove(
    state: &AppState,
    admin: Uuid,
    kind: ReservedKind,
    pattern: &str,
) -> Result<(), ApiError> {
    let mut tx = state.pool.begin().await?;
    let source: Option<String> = sqlx::query_scalar(
        "SELECT source FROM reserved_handles WHERE kind = $1 AND pattern = $2 FOR UPDATE",
    )
    .bind(kind.as_str())
    .bind(pattern)
    .fetch_optional(&mut *tx)
    .await?;
    match source.as_deref() {
        None => return Err(ApiError::NotFound),
        Some("file") => {
            return Err(ApiError::BadRequest(
                "This entry comes from the reserved-handles file. Remove it there and restart the server."
                    .into(),
            ));
        }
        Some(_) => {}
    }
    sqlx::query("DELETE FROM reserved_handles WHERE kind = $1 AND pattern = $2")
        .bind(kind.as_str())
        .bind(pattern)
        .execute(&mut *tx)
        .await?;
    let detail = format!("{kind}: {pattern}");
    log_action(
        &mut tx,
        admin,
        AdminAction::UnreserveHandle,
        "handles",
        Some(&detail),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---- JSON API --------------------------------------------------------------

async fn api_list(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<ReservedQuery>,
) -> Result<Json<ReservedList>, ApiError> {
    list(&state, &q).await.map(Json)
}

async fn api_add(
    State(state): State<AppState>,
    admin: Admin,
    ApiJson(body): ApiJson<AddReserved>,
) -> Result<(StatusCode, Json<AddedReserved>), ApiError> {
    let added = add(&state, admin.0.account_id, &body).await?;
    Ok((StatusCode::CREATED, Json(added)))
}

#[derive(Debug, Deserialize)]
struct CheckQuery {
    handle: String,
}

async fn api_check(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<CheckQuery>,
) -> Result<Json<ReservedCheck>, ApiError> {
    check(&state, &q.handle).await.map(Json)
}

async fn api_remove(
    State(state): State<AppState>,
    admin: Admin,
    UrlPath((kind, pattern)): UrlPath<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let kind: ReservedKind = kind.parse().map_err(|_| ApiError::NotFound)?;
    remove(&state, admin.0.account_id, kind, &pattern).await?;
    Ok(StatusCode::NO_CONTENT)
}
