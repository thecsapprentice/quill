//! Chains: for each hook, the scripts it runs in order and what a failing
//! script means. A hook with no chain saved runs nothing.

use quill_proto::automod::{ChainStep, Hook, ModerationChain, OnError, UpdateChain};
use quill_proto::board::AdminAction;
use sqlx::PgPool;
use uuid::Uuid;

use super::scripts;
use crate::{AppState, error::ApiError, prompts::parse_enum, reports::log_action};

/// Longest chain.
const MAX_STEPS: usize = 50;

/// Every hook's chain, in hook order. Hooks never saved come back empty,
/// with `continue`, the setting the form starts on.
pub async fn list(pool: &PgPool) -> Result<Vec<ModerationChain>, ApiError> {
    let mut out = Vec::new();
    for hook in Hook::ALL {
        out.push(get(pool, *hook).await?);
    }
    Ok(out)
}

pub async fn get(pool: &PgPool, hook: Hook) -> Result<ModerationChain, ApiError> {
    let on_error: Option<String> =
        sqlx::query_scalar("SELECT on_error FROM moderation_chains WHERE hook = $1")
            .bind(hook.as_str())
            .fetch_optional(pool)
            .await?;
    let steps: Vec<(Uuid, String, bool, bool)> = sqlx::query_as(
        "SELECT s.id, s.name, s.enabled, s.log_only
         FROM moderation_chain_steps c JOIN moderation_scripts s ON s.id = c.script_id
         WHERE c.hook = $1 ORDER BY c.position",
    )
    .bind(hook.as_str())
    .fetch_all(pool)
    .await?;
    Ok(ModerationChain {
        hook,
        on_error: on_error
            .as_deref()
            .map(parse_enum)
            .transpose()?
            .unwrap_or(OnError::Continue),
        steps: steps
            .into_iter()
            .map(|(script_id, name, enabled, log_only)| ChainStep {
                script_id,
                name,
                enabled,
                log_only,
            })
            .collect(),
    })
}

/// Saves a hook's whole chain. Each script newly added is first tried on
/// the hook's sample post.
pub async fn update(
    state: &AppState,
    admin: Uuid,
    hook: Hook,
    body: &UpdateChain,
) -> Result<ModerationChain, ApiError> {
    if body.scripts.len() > MAX_STEPS {
        return Err(ApiError::BadRequest(format!(
            "a chain can run up to {MAX_STEPS} scripts"
        )));
    }
    for (i, id) in body.scripts.iter().enumerate() {
        if body.scripts[..i].contains(id) {
            return Err(ApiError::BadRequest(
                "a chain can run each script once".into(),
            ));
        }
    }
    let current = get(&state.pool, hook).await?;
    for id in &body.scripts {
        if !current.steps.iter().any(|s| s.script_id == *id) {
            scripts::check_saved(state, *id, hook).await?;
        }
    }

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO moderation_chains (hook, on_error) VALUES ($1, $2)
         ON CONFLICT (hook) DO UPDATE SET on_error = $2",
    )
    .bind(hook.as_str())
    .bind(body.on_error.as_str())
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM moderation_chain_steps WHERE hook = $1")
        .bind(hook.as_str())
        .execute(&mut *tx)
        .await?;
    let positions: Vec<i32> = (0..i32::try_from(body.scripts.len()).unwrap_or(0)).collect();
    sqlx::query(
        "INSERT INTO moderation_chain_steps (hook, position, script_id)
         SELECT $1, p, s FROM unnest($2::int[], $3::uuid[]) AS u(p, s)",
    )
    .bind(hook.as_str())
    .bind(&positions)
    .bind(&body.scripts)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        if e.as_database_error()
            .is_some_and(|d| d.is_foreign_key_violation())
        {
            ApiError::BadRequest("one of those scripts doesn't exist".into())
        } else {
            e.into()
        }
    })?;
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT s.name FROM moderation_chain_steps c JOIN moderation_scripts s ON s.id = c.script_id
         WHERE c.hook = $1 ORDER BY c.position",
    )
    .bind(hook.as_str())
    .fetch_all(&mut *tx)
    .await?;
    let detail = if names.is_empty() {
        format!("no scripts; on error: {}", body.on_error)
    } else {
        format!("{}; on error: {}", names.join(" → "), body.on_error)
    };
    log_action(
        &mut tx,
        admin,
        AdminAction::UpdateChain,
        hook.as_str(),
        Some(&detail),
    )
    .await?;
    tx.commit().await?;
    get(&state.pool, hook).await
}
