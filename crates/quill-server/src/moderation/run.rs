//! Running a hook's chain on a submission, and carrying out what it
//! decided.
//!
//! [`check`] runs before anything is saved: a refusal is returned as
//! [`ApiError::ContentRejected`], logged, and noted on the writer's account.
//! A flag or a dismissal comes back as a [`Verdict`] for the caller to carry
//! out once its change is saved. Architecture document, "Automated
//! moderation: running a chain".

use std::sync::Arc;
use std::time::Duration;

use quill_proto::automod::{
    DryRun, DryRunDecision, DryRunResult, DryRunStep, GENERIC_REFUSAL, Hook, OnError,
    UNCHECKED_REFUSAL,
};
use quill_proto::board::{AdminAction, ReportReason};
use quill_proto::notices::{NoticeFields, NoticeKind};
use rhai::Dynamic;
use rhai::serde::to_dynamic;
use sqlx::PgConnection;
use uuid::Uuid;

use super::context::{self, Subject};
use super::data;
use super::engine::{Outcome, Script, ScriptError};
use super::scripts::describe;
use crate::{AppState, error::ApiError, notices, prompts::parse_enum};

/// What the caller does once its change is saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing: no script handled it.
    Pass,
    /// File a report for an administrator.
    Flag(Automated),
    /// `prompt_reported` only: file the writer's report already closed.
    Dismiss(Automated),
}

/// Which script decided, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Automated {
    pub script_id: Uuid,
    pub script_name: String,
    pub reason: String,
    pub category: ReportReason,
}

/// One enabled step of a chain, with what's needed to run it.
struct Step {
    id: Uuid,
    name: String,
    log_only: bool,
    script: Result<Arc<Script>, ScriptError>,
}

/// What happened at one step.
struct StepResult {
    step: usize,
    result: Result<Outcome, ScriptError>,
}

/// Whether moderation scripts check what `account` writes. Administrators'
/// own writing isn't checked: they're the ones who set the rules.
pub async fn applies_to(state: &AppState, account: Uuid) -> Result<bool, ApiError> {
    Ok(!crate::prompts::viewer_is_admin(&state.pool, Some(account)).await?)
}

/// Runs `subject.hook`'s chain. Refusals come back as errors.
pub async fn check(state: &AppState, subject: Subject) -> Result<Verdict, ApiError> {
    let hook = subject.hook;
    let on_error: Option<String> =
        sqlx::query_scalar("SELECT on_error FROM moderation_chains WHERE hook = $1")
            .bind(hook.as_str())
            .fetch_optional(&state.pool)
            .await?;
    let rows: Vec<(Uuid, String, String, Vec<u8>, bool)> = sqlx::query_as(
        "SELECT s.id, s.name, s.source, s.source_hash, s.log_only
         FROM moderation_chain_steps c JOIN moderation_scripts s ON s.id = c.script_id
         WHERE c.hook = $1 AND s.enabled ORDER BY c.position",
    )
    .bind(hook.as_str())
    .fetch_all(&state.pool)
    .await?;
    if rows.is_empty() {
        return Ok(Verdict::Pass);
    }
    let on_error: OnError = on_error
        .as_deref()
        .map(parse_enum)
        .transpose()?
        .unwrap_or(OnError::Continue);

    let limits = &state.config.automod;
    let steps: Vec<Step> = rows
        .into_iter()
        .map(|(id, name, source, hash, log_only)| Step {
            script: state
                .automod_scripts
                .get_or_compile(id, &hash, &source, limits),
            id,
            name,
            log_only,
        })
        .collect();
    let mut names: Vec<String> = steps
        .iter()
        .filter_map(|s| s.script.as_ref().ok())
        .flat_map(|s| s.data_names().iter().cloned())
        .collect();
    names.sort();
    names.dedup();
    let snapshot = data::snapshot(&state.pool, &state.automod_data, &names).await?;
    let post = context::build(&state.pool, subject.clone()).await?;
    let post =
        to_dynamic(&post).map_err(|e| ApiError::Internal(anyhow::anyhow!("context: {e}")))?;

    let time_limit = Duration::from_millis(limits.time_ms);
    let (steps, results) = tokio::task::spawn_blocking(move || {
        snapshot.warm();
        let results = run_steps(&steps, &post, &snapshot, time_limit, hook, on_error);
        (steps, results)
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;

    let mut verdict = Verdict::Pass;
    for r in results {
        let step = &steps[r.step];
        match r.result {
            Err(err) => {
                let detail = format!("{}; chain setting: {on_error}", describe(&err));
                log_automod(
                    state,
                    &subject,
                    AdminAction::AutomodError,
                    &step.name,
                    &detail,
                )
                .await?;
                match on_error {
                    OnError::Continue => {}
                    OnError::Reject => {
                        return Err(
                            refuse(state, &subject, step, Some(UNCHECKED_REFUSAL), true).await?
                        );
                    }
                    OnError::Flag => {
                        verdict = Verdict::Flag(Automated {
                            script_id: step.id,
                            script_name: step.name.clone(),
                            reason: format!("The script failed: {}", describe(&err)),
                            category: ReportReason::Other,
                        });
                    }
                }
            }
            Ok(Outcome::Pass) => {}
            Ok(outcome) if step.log_only => {
                let detail = format!("log only, would have done: {}", summary(&outcome));
                let action = match outcome {
                    Outcome::Reject { .. } => AdminAction::AutomodReject,
                    Outcome::Dismiss { .. } => AdminAction::AutomodDismiss,
                    _ => AdminAction::AutomodFlag,
                };
                log_automod(state, &subject, action, &step.name, &detail).await?;
            }
            Ok(Outcome::Reject { message }) => {
                return Err(refuse(state, &subject, step, message.as_deref(), false).await?);
            }
            Ok(Outcome::Flag { reason, category }) => {
                verdict = Verdict::Flag(Automated {
                    script_id: step.id,
                    script_name: step.name.clone(),
                    reason,
                    category,
                });
            }
            Ok(Outcome::Dismiss { reason }) => {
                verdict = Verdict::Dismiss(Automated {
                    script_id: step.id,
                    script_name: step.name.clone(),
                    reason,
                    category: ReportReason::Other,
                });
            }
        }
    }
    Ok(verdict)
}

/// Runs steps in order until one handles the submission (or fails under a
/// setting that ends the chain). Log-only steps never stop the chain.
fn run_steps(
    steps: &[Step],
    post: &Dynamic,
    data: &data::Snapshot,
    time_limit: Duration,
    hook: Hook,
    on_error: OnError,
) -> Vec<StepResult> {
    let mut out = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        let result = match &step.script {
            Ok(script) => {
                script
                    .run(&[("post", post.clone())], data, time_limit)
                    .result
            }
            Err(e) => Err(e.clone()),
        };
        let result = match result {
            Ok(Outcome::Dismiss { .. }) if hook != Hook::PromptReported => Err(ScriptError {
                kind: super::engine::ErrorKind::Runtime,
                message: "dismiss(...) only closes reports; use it in the prompt_reported chain"
                    .into(),
                line: None,
            }),
            other => other,
        };
        let stops = match &result {
            Ok(outcome) => outcome.handled() && !step.log_only,
            Err(_) => on_error != OnError::Continue,
        };
        out.push(StepResult { step: i, result });
        if stops {
            break;
        }
    }
    out
}

fn summary(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Pass => "nothing".into(),
        Outcome::Reject { message } => {
            format!("reject ({})", message.as_deref().unwrap_or(GENERIC_REFUSAL))
        }
        Outcome::Flag { reason, category } => format!("flag ({category}: {reason})"),
        Outcome::Dismiss { reason } => format!("dismiss the report ({reason})"),
    }
}

/// What a refusal notice calls the submission.
fn refused_kind(hook: Hook) -> &'static str {
    match hook {
        Hook::PromptCreated => "prompt",
        Hook::PromptEdited => "edit",
        Hook::CommentCreated => "comment",
        Hook::TagRequested => "tag",
        Hook::PromptReported => "report",
    }
}

/// Inserts an automated log entry naming the script, and the account it
/// concerns as its target. Returns the entry's id.
async fn insert_log(
    conn: &mut PgConnection,
    subject: &Subject,
    action: AdminAction,
    script: &str,
    detail: &str,
) -> Result<i64, ApiError> {
    let title = if subject.title.is_empty() {
        subject.about.clone()
    } else {
        Some(subject.title.clone())
    };
    let log_id: i64 = sqlx::query_scalar(
        "INSERT INTO admin_log (action, subject, detail, prompt_id, actor, script_name)
         VALUES ($1, $2, $3, $4, 'automod', $5) RETURNING id",
    )
    .bind(action.as_str())
    .bind(title)
    .bind(detail)
    .bind(subject.prompt)
    .bind(script)
    .fetch_one(&mut *conn)
    .await?;
    let target = notices::Affected {
        account: subject.account,
        prompts: Vec::new(),
    };
    crate::reports::log_targets(conn, log_id, &[target]).await?;
    Ok(log_id)
}

async fn log_automod(
    state: &AppState,
    subject: &Subject,
    action: AdminAction,
    script: &str,
    detail: &str,
) -> Result<(), ApiError> {
    let mut conn = state.pool.acquire().await?;
    insert_log(&mut conn, subject, action, script, detail)
        .await
        .map(drop)
}

/// Logs a refusal and notes it on the submitter's account. Returns the
/// error to send back. The notice holds the title and message, never the
/// refused text, and is already read: the writer saw the message.
async fn refuse(
    state: &AppState,
    subject: &Subject,
    step: &Step,
    message: Option<&str>,
    unchecked: bool,
) -> Result<ApiError, ApiError> {
    let message = message.unwrap_or(GENERIC_REFUSAL).to_owned();
    let detail = if unchecked {
        format!("couldn't check: {message}")
    } else {
        message.clone()
    };
    let mut tx = state.pool.begin().await?;
    let log_id = insert_log(
        &mut tx,
        subject,
        AdminAction::AutomodReject,
        &step.name,
        &detail,
    )
    .await?;
    if let Some(submitter) = subject.submitter {
        let title = if subject.title.is_empty() {
            subject.about.clone()
        } else {
            Some(subject.title.clone())
        };
        let fields = NoticeFields {
            refused: Some(refused_kind(subject.hook).to_owned()),
            title,
            message: Some(message.clone()),
            ..NoticeFields::default()
        };
        let affected = notices::Affected {
            account: submitter,
            prompts: Vec::new(),
        };
        notices::record(
            &mut tx,
            log_id,
            NoticeKind::ContentRejected,
            &fields,
            &[affected],
        )
        .await?;
        sqlx::query("UPDATE account_notices SET read_at = now() WHERE log_id = $1")
            .bind(log_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(ApiError::ContentRejected(message))
}

/// Runs a chain, or one script, without doing anything: every script runs,
/// whether or not an earlier one decided, and nothing is logged.
pub async fn dry_run(state: &AppState, req: &DryRun) -> Result<DryRunResult, ApiError> {
    let rows: Vec<(Uuid, String, String, Vec<u8>, bool)> = match req.script {
        Some(id) => sqlx::query_as(
            "SELECT id, name, source, source_hash, log_only FROM moderation_scripts WHERE id = $1",
        )
        .bind(id)
        .fetch_all(&state.pool)
        .await?,
        None => {
            sqlx::query_as(
                "SELECT s.id, s.name, s.source, s.source_hash, s.log_only
                 FROM moderation_chain_steps c JOIN moderation_scripts s ON s.id = c.script_id
                 WHERE c.hook = $1 AND s.enabled ORDER BY c.position",
            )
            .bind(req.hook.as_str())
            .fetch_all(&state.pool)
            .await?
        }
    };
    if req.script.is_some() && rows.is_empty() {
        return Err(ApiError::NotFound);
    }
    let limits = &state.config.automod;
    let steps: Vec<Step> = rows
        .into_iter()
        .map(|(id, name, source, hash, log_only)| Step {
            script: state
                .automod_scripts
                .get_or_compile(id, &hash, &source, limits),
            id,
            name,
            log_only,
        })
        .collect();
    let names = data_names(&steps);
    let overrides = req.data.iter().map(|d| (d.name.clone(), d.content.clone()));
    let snapshot = data::snapshot(&state.pool, &state.automod_data, &names)
        .await?
        .with_overrides(overrides);
    let post = match req.prompt {
        Some(prompt) => {
            context::build(&state.pool, existing_prompt(state, req.hook, prompt).await?).await?
        }
        None => {
            let mut post = context::sample(req.hook);
            if !req.title.is_empty() || !req.body.is_empty() {
                post.title = req.title.clone();
                post.body = req.body.clone();
            }
            post
        }
    };
    let post =
        to_dynamic(&post).map_err(|e| ApiError::Internal(anyhow::anyhow!("context: {e}")))?;
    let time_limit = Duration::from_millis(limits.time_ms);
    let hook = req.hook;
    let out = tokio::task::spawn_blocking(move || {
        snapshot.warm();
        let mut decided = false;
        steps
            .iter()
            .map(|step| {
                let (result, printed, took) = match &step.script {
                    Ok(script) => {
                        let run = script.run(&[("post", post.clone())], &snapshot, time_limit);
                        (run.result, run.printed, run.took)
                    }
                    Err(e) => (Err(e.clone()), Vec::new(), Duration::ZERO),
                };
                let (decision, detail, category) = match &result {
                    Ok(Outcome::Pass) => (DryRunDecision::Pass, None, None),
                    Ok(Outcome::Reject { message }) => (
                        DryRunDecision::Reject,
                        Some(
                            message
                                .clone()
                                .unwrap_or_else(|| GENERIC_REFUSAL.to_owned()),
                        ),
                        None,
                    ),
                    Ok(Outcome::Flag { reason, category }) => (
                        DryRunDecision::Flag,
                        Some(reason.clone()),
                        Some(category.as_str().to_owned()),
                    ),
                    Ok(Outcome::Dismiss { reason }) if hook == Hook::PromptReported => {
                        (DryRunDecision::Dismiss, Some(reason.clone()), None)
                    }
                    Ok(Outcome::Dismiss { .. }) => (
                        DryRunDecision::Error,
                        Some(
                            "dismiss(...) only closes reports; use it in the prompt_reported chain"
                                .into(),
                        ),
                        None,
                    ),
                    Err(e) => (DryRunDecision::Error, Some(describe(e)), None),
                };
                let handled = !matches!(decision, DryRunDecision::Pass | DryRunDecision::Error);
                let decides = handled && !step.log_only && !decided;
                decided |= decides;
                DryRunStep {
                    script_id: step.id,
                    name: step.name.clone(),
                    decision,
                    detail,
                    category,
                    printed,
                    micros: u64::try_from(took.as_micros()).unwrap_or(u64::MAX),
                    log_only: step.log_only,
                    decides,
                }
            })
            .collect()
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(DryRunResult { steps: out })
}

fn data_names(steps: &[Step]) -> Vec<String> {
    let mut names: Vec<String> = steps
        .iter()
        .filter_map(|s| s.script.as_ref().ok())
        .flat_map(|s| s.data_names().iter().cloned())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// An existing prompt as `hook` would see it, with its writer's history.
async fn existing_prompt(state: &AppState, hook: Hook, prompt: Uuid) -> Result<Subject, ApiError> {
    let row: Option<(String, String, Uuid, Uuid, String, Vec<i64>)> = sqlx::query_as(
        "SELECT p.title, p.body_md, p.actor_id, a.account_id, p.maturity::text, p.tag_ids
         FROM prompts p JOIN actors a ON a.id = p.actor_id WHERE p.id = $1",
    )
    .bind(prompt)
    .fetch_optional(&state.pool)
    .await?;
    let (title, body, actor, account, maturity, tag_ids) = row.ok_or(ApiError::NotFound)?;
    let tags = crate::prompts::load_tags(&state.pool, &tag_ids).await?;
    Ok(Subject {
        hook,
        title,
        body,
        tags: tag_ids
            .iter()
            .filter_map(|t| tags.get(t))
            .map(|t| t.key())
            .collect(),
        maturity,
        previous: None,
        tag: None,
        report: None,
        actor: Some(actor),
        account,
        submitter: None,
        prompt: Some(prompt),
        about: None,
    })
}

/// What a flag is about, once it's saved.
pub enum FlagTarget<'a> {
    Prompt {
        id: Uuid,
        title: &'a str,
        body: &'a str,
    },
    Comment {
        id: Uuid,
        body: &'a str,
    },
    /// A pending tag, by id and `namespace:slug`.
    Tag {
        id: i64,
        key: &'a str,
    },
}

/// Carries out a flag after the caller's change is saved: a report in the
/// inbox, or a mark on a pending tag. Failing here doesn't undo the change,
/// so errors are logged rather than returned.
pub async fn file_flag(state: &AppState, flag: &Automated, target: FlagTarget<'_>, author: Uuid) {
    if let Err(e) = file_flag_inner(state, flag, target, author).await {
        tracing::error!(error = ?e, script = %flag.script_name, "filing an automated flag");
    }
}

async fn file_flag_inner(
    state: &AppState,
    flag: &Automated,
    target: FlagTarget<'_>,
    author: Uuid,
) -> Result<(), ApiError> {
    let mut tx = state.pool.begin().await?;
    let (prompt, comment, hash, kind) = match target {
        FlagTarget::Prompt { id, title, body } => (
            Some(id),
            None,
            crate::reports::prompt_hash(title, body),
            "prompt",
        ),
        FlagTarget::Comment { id, body } => (
            None,
            Some(id),
            crate::reports::comment_hash(body),
            "comment",
        ),
        FlagTarget::Tag { id, key } => {
            sqlx::query("UPDATE tags SET flag_script = $2, flag_reason = $3 WHERE id = $1")
                .bind(id)
                .bind(&flag.script_name)
                .bind(&flag.reason)
                .execute(&mut *tx)
                .await?;
            let log_id: i64 = sqlx::query_scalar(
                "INSERT INTO admin_log (action, subject, detail, actor, script_name)
                 VALUES ('automod_flag', $1, $2, 'automod', $3) RETURNING id",
            )
            .bind(key)
            .bind(&flag.reason)
            .bind(&flag.script_name)
            .fetch_one(&mut *tx)
            .await?;
            let target = notices::Affected {
                account: author,
                prompts: Vec::new(),
            };
            crate::reports::log_targets(&mut tx, log_id, &[target]).await?;
            tx.commit().await?;
            return Ok(());
        }
    };
    let report_id = Uuid::now_v7();
    // At most one per script for each version: a repeat does nothing.
    let inserted = sqlx::query(
        "INSERT INTO reports (id, kind, prompt_id, comment_id, reason, details, prompt_hash,
                              script_id, script_name)
         VALUES ($1, $2, $3, $4, $5::report_reason, $6, $7, $8, $9)
         ON CONFLICT DO NOTHING",
    )
    .bind(report_id)
    .bind(kind)
    .bind(prompt)
    .bind(comment)
    .bind(flag.category.as_str())
    .bind(&flag.reason)
    .bind(&hash)
    .bind(flag.script_id)
    .bind(&flag.script_name)
    .execute(&mut *tx)
    .await?;
    if inserted.rows_affected() == 0 {
        return Ok(());
    }
    let log_id: i64 = sqlx::query_scalar(
        "INSERT INTO admin_log (action, report_id, prompt_id, comment_id, prompt_hash, reason,
                                detail, actor, script_name)
         VALUES ('report_received', $1, $2, $3, $4, $5::report_reason, $6, 'automod', $7)
         RETURNING id",
    )
    .bind(report_id)
    .bind(prompt)
    .bind(comment)
    .bind(&hash)
    .bind(flag.category.as_str())
    .bind(&flag.reason)
    .bind(&flag.script_name)
    .fetch_one(&mut *tx)
    .await?;
    let target = notices::Affected {
        account: author,
        prompts: Vec::new(),
    };
    crate::reports::log_targets(&mut tx, log_id, &[target]).await?;
    tx.commit().await?;
    Ok(())
}
