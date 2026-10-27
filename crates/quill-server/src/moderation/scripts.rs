//! Moderation scripts: upload, replace, remove, switch on and off.
//!
//! A script is only saved if it compiles, every common data entry it names
//! exists, and it runs without error on a sample post for each hook it's
//! used at, so admins see their mistakes when they upload rather than on
//! writers' posts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use quill_proto::automod::{Hook, ModerationScript, ReplaceScript, SetScriptMode, UploadScript};
use quill_proto::board::AdminAction;
use rhai::serde::to_dynamic;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use super::context;
use super::data::{self, Snapshot};
use super::engine::{ErrorKind, Script, ScriptError};
use crate::{AppState, error::ApiError, error::is_unique_violation};

/// The variables every script is given.
pub const VARIABLES: [&str; 1] = ["post"];
/// Largest script source, in bytes.
pub const MAX_SOURCE: usize = 256 << 10;

/// A compiled script and the hash of the source it came from.
type Compiled = (Vec<u8>, Arc<Script>);

/// Compiled scripts by id, each kept with the hash of the source it was
/// compiled from, so a replaced script is compiled again on next use.
#[derive(Debug, Default)]
pub struct ScriptCache(Mutex<HashMap<Uuid, Compiled>>);

impl ScriptCache {
    pub fn get_or_compile(
        &self,
        id: Uuid,
        hash: &[u8],
        source: &str,
        limits: &super::Limits,
    ) -> Result<Arc<Script>, ScriptError> {
        if let Some((h, script)) = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
            && h == hash
        {
            return Ok(script.clone());
        }
        let script = Arc::new(Script::compile(source, &VARIABLES, limits)?);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, (hash.to_vec(), script.clone()));
        Ok(script)
    }

    fn forget(&self, id: Uuid) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
    }
}

pub fn source_hash(source: &str) -> Vec<u8> {
    Sha256::digest(source.as_bytes()).to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// An error in a script, worded for the administrator who wrote it.
pub fn describe(err: &ScriptError) -> String {
    match err.line {
        Some(line) => format!("line {line}: {}", err.message),
        None => err.message.clone(),
    }
}

fn check_name(name: &str) -> Result<String, ApiError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::BadRequest(
            "a script's name must be 1 to 100 characters".into(),
        ));
    }
    Ok(name.to_owned())
}

/// Compiles `source` and tries it on each hook's sample post, with the
/// common data as it is now. Returns the compiled script.
async fn check(state: &AppState, source: &str, hooks: &[Hook]) -> Result<Script, ApiError> {
    if source.len() > MAX_SOURCE {
        return Err(ApiError::BadRequest(format!(
            "scripts can be up to {MAX_SOURCE} bytes"
        )));
    }
    let script = Script::compile(source, &VARIABLES, &state.config.automod)
        .map_err(|e| ApiError::BadRequest(describe(&e)))?;
    let names: Vec<String> = script.data_names().iter().cloned().collect();
    let existing: Vec<String> =
        sqlx::query_scalar("SELECT name FROM moderation_data WHERE name = ANY($1)")
            .bind(&names)
            .fetch_all(&state.pool)
            .await?;
    let missing: Vec<&str> = names
        .iter()
        .filter(|n| !existing.contains(n))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "there is no common data named {}; add it first",
            missing.join(", ")
        )));
    }
    let snapshot = data::snapshot(&state.pool, &state.automod_data, &names).await?;
    let time_limit = Duration::from_millis(state.config.automod.time_ms);
    let (script, failure) = tokio::task::spawn_blocking({
        let hooks = hooks.to_vec();
        move || {
            snapshot.warm();
            let failure = try_on_samples(&script, &hooks, &snapshot, time_limit);
            (script, failure)
        }
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;
    match failure {
        Some((hook, err)) => Err(ApiError::BadRequest(format!(
            "on a sample {} post ({hook}), {}",
            hook.kind(),
            describe(&err)
        ))),
        None => Ok(script),
    }
}

/// The first hook whose sample post the script fails on.
fn try_on_samples(
    script: &Script,
    hooks: &[Hook],
    data: &Snapshot,
    time_limit: Duration,
) -> Option<(Hook, ScriptError)> {
    let hooks = if hooks.is_empty() {
        &[Hook::PromptCreated][..]
    } else {
        hooks
    };
    for hook in hooks {
        let post = match to_dynamic(context::sample(*hook)) {
            Ok(post) => post,
            Err(e) => {
                return Some((
                    *hook,
                    ScriptError {
                        kind: ErrorKind::Runtime,
                        message: e.to_string(),
                        line: None,
                    },
                ));
            }
        };
        if let Err(err) = script.run(&[("post", post)], data, time_limit).result {
            return Some((*hook, err));
        }
    }
    None
}

/// Logs an administrator's change to a script, naming the script.
async fn log_script(
    conn: &mut PgConnection,
    admin: Uuid,
    action: AdminAction,
    name: &str,
    detail: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO admin_log (action, subject, detail, admin_id, actor, admin_login, script_name)
         VALUES ($1, $2, $3, $4, 'admin', (SELECT login_name FROM accounts WHERE id = $4), $2)",
    )
    .bind(action.as_str())
    .bind(name)
    .bind(detail)
    .bind(admin)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn set_data_names(
    conn: &mut PgConnection,
    id: Uuid,
    names: &[String],
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM moderation_script_data WHERE script_id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO moderation_script_data (script_id, data_name) SELECT $1, unnest($2::text[])",
    )
    .bind(id)
    .bind(names)
    .execute(&mut *conn)
    .await
    .map_err(|e| {
        if e.as_database_error()
            .is_some_and(|d| d.is_foreign_key_violation())
        {
            ApiError::Conflict("a common data entry this script reads was just deleted".into())
        } else {
            e.into()
        }
    })?;
    Ok(())
}

fn name_taken(name: &str) -> ApiError {
    ApiError::Conflict(format!("there is already a script named {name:?}"))
}

pub async fn upload(
    state: &AppState,
    admin: Uuid,
    body: &UploadScript,
) -> Result<ModerationScript, ApiError> {
    let name = check_name(&body.name)?;
    let script = check(state, &body.source, &[]).await?;
    let names: Vec<String> = script.data_names().iter().cloned().collect();
    let id = Uuid::now_v7();
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO moderation_scripts (id, name, source, source_hash, log_only, uploaded_by)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(&name)
    .bind(&body.source)
    .bind(source_hash(&body.source))
    .bind(body.log_only)
    .bind(admin)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        if is_unique_violation(&e) {
            name_taken(&name)
        } else {
            e.into()
        }
    })?;
    set_data_names(&mut tx, id, &names).await?;
    let detail = body.log_only.then_some("log only");
    log_script(&mut tx, admin, AdminAction::UploadScript, &name, detail).await?;
    tx.commit().await?;
    get(&state.pool, id).await
}

/// Replaces a script's source or name. New source is checked on the
/// sample post of every hook whose chain runs the script.
pub async fn replace(
    state: &AppState,
    admin: Uuid,
    id: Uuid,
    body: &ReplaceScript,
) -> Result<ModerationScript, ApiError> {
    let current = get(&state.pool, id).await?;
    let name = body.name.as_deref().map(check_name).transpose()?;
    let checked = match &body.source {
        Some(source) => Some(check(state, source, &current.chains).await?),
        None => None,
    };
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE moderation_scripts SET
             name = COALESCE($2, name),
             source = COALESCE($3, source),
             source_hash = COALESCE($4, source_hash),
             updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(&name)
    .bind(&body.source)
    .bind(body.source.as_deref().map(source_hash))
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        if is_unique_violation(&e) {
            name_taken(name.as_deref().unwrap_or_default())
        } else {
            e.into()
        }
    })?;
    if let Some(script) = &checked {
        let names: Vec<String> = script.data_names().iter().cloned().collect();
        set_data_names(&mut tx, id, &names).await?;
    }
    let new_name = name.as_deref().unwrap_or(&current.name);
    let detail = match (&name, &body.source) {
        (Some(n), _) if *n != current.name => Some(format!("renamed from {:?}", current.name)),
        _ => None,
    };
    log_script(
        &mut tx,
        admin,
        AdminAction::ReplaceScript,
        new_name,
        detail.as_deref(),
    )
    .await?;
    tx.commit().await?;
    state.automod_scripts.forget(id);
    get(&state.pool, id).await
}

/// Removes a script, and its place in every chain.
pub async fn remove(state: &AppState, admin: Uuid, id: Uuid) -> Result<(), ApiError> {
    let current = get(&state.pool, id).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM moderation_scripts WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let detail = (!current.chains.is_empty()).then(|| {
        let hooks: Vec<&str> = current.chains.iter().map(|h| h.as_str()).collect();
        format!("removed from {}", hooks.join(", "))
    });
    log_script(
        &mut tx,
        admin,
        AdminAction::RemoveScript,
        &current.name,
        detail.as_deref(),
    )
    .await?;
    tx.commit().await?;
    state.automod_scripts.forget(id);
    Ok(())
}

pub async fn set_mode(
    state: &AppState,
    admin: Uuid,
    id: Uuid,
    body: &SetScriptMode,
) -> Result<ModerationScript, ApiError> {
    let current = get(&state.pool, id).await?;
    let enabled = body.enabled.unwrap_or(current.enabled);
    let log_only = body.log_only.unwrap_or(current.log_only);
    if (enabled, log_only) == (current.enabled, current.log_only) {
        return Ok(current);
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE moderation_scripts SET enabled = $2, log_only = $3 WHERE id = $1")
        .bind(id)
        .bind(enabled)
        .bind(log_only)
        .execute(&mut *tx)
        .await?;
    let detail = match (enabled, log_only) {
        (false, _) => "off",
        (true, true) => "on, log only",
        (true, false) => "on",
    };
    log_script(
        &mut tx,
        admin,
        AdminAction::SetScriptMode,
        &current.name,
        Some(detail),
    )
    .await?;
    tx.commit().await?;
    get(&state.pool, id).await
}

#[derive(sqlx::FromRow)]
struct ScriptRow {
    id: Uuid,
    name: String,
    source: String,
    source_hash: Vec<u8>,
    enabled: bool,
    log_only: bool,
    data: Vec<String>,
    chains: Vec<String>,
    uploaded_by: Option<String>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl ScriptRow {
    fn into_script(self) -> Result<ModerationScript, ApiError> {
        Ok(ModerationScript {
            id: self.id,
            name: self.name,
            source: self.source,
            source_hash: hex(&self.source_hash),
            enabled: self.enabled,
            log_only: self.log_only,
            data: self.data,
            chains: self
                .chains
                .iter()
                .map(|h| crate::prompts::parse_enum(h))
                .collect::<Result<_, _>>()?,
            uploaded_by: self.uploaded_by,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

/// Scripts, by name. `only` picks one.
async fn fetch(pool: &PgPool, only: Option<Uuid>) -> Result<Vec<ModerationScript>, ApiError> {
    let rows: Vec<ScriptRow> = sqlx::query_as(
        "SELECT s.id, s.name, s.source, s.source_hash, s.enabled, s.log_only,
                ARRAY(SELECT d.data_name FROM moderation_script_data d
                      WHERE d.script_id = s.id ORDER BY d.data_name) AS data,
                ARRAY(SELECT c.hook FROM moderation_chain_steps c
                      WHERE c.script_id = s.id ORDER BY c.hook) AS chains,
                a.login_name AS uploaded_by, s.created_at, s.updated_at
         FROM moderation_scripts s LEFT JOIN accounts a ON a.id = s.uploaded_by
         WHERE $1::uuid IS NULL OR s.id = $1
         ORDER BY lower(s.name)",
    )
    .bind(only)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(ScriptRow::into_script).collect()
}

pub async fn list(pool: &PgPool) -> Result<Vec<ModerationScript>, ApiError> {
    fetch(pool, None).await
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<ModerationScript, ApiError> {
    fetch(pool, Some(id)).await?.pop().ok_or(ApiError::NotFound)
}

/// Checks a script that's already saved on one hook's sample post, as when
/// it's added to that hook's chain.
pub(crate) async fn check_saved(state: &AppState, id: Uuid, hook: Hook) -> Result<(), ApiError> {
    let script = get(&state.pool, id).await?;
    check(state, &script.source, &[hook])
        .await
        .map(drop)
        .map_err(|e| match e {
            ApiError::BadRequest(m) => ApiError::BadRequest(format!("{}: {m}", script.name)),
            other => other,
        })
}
