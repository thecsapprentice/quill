//! The calls the board's write paths make: one before a change is saved
//! (which may refuse it), and one after (which carries out flags). Each
//! write path calls these at one place.

use quill_proto::board::Maturity;
use quill_proto::validate::TagSpec;
use uuid::Uuid;

use super::context::{Previous, ReportInfo, Subject, TagInfo};
use super::run::{FlagTarget, Verdict, applies_to, check, file_flag};
use super::{Automated, Hook};
use crate::{AppState, error::ApiError};

/// A prompt as submitted, before it's saved.
pub(crate) struct PromptDraft<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub tags: &'a [TagSpec],
    pub maturity: Maturity,
}

/// What the checks before a prompt is saved decided.
#[derive(Debug, Default)]
pub(crate) struct PromptChecks {
    prompt: Option<Automated>,
    /// New tags a script flagged, by `namespace:slug`.
    tags: Vec<(String, Automated)>,
}

fn keys(tags: &[TagSpec]) -> Vec<String> {
    tags.iter()
        .map(|t| format!("{}:{}", t.namespace, t.slug))
        .collect()
}

fn subject(hook: Hook, account: Uuid, persona: Uuid, draft: &PromptDraft<'_>) -> Subject {
    Subject {
        hook,
        title: draft.title.to_owned(),
        body: draft.body.to_owned(),
        tags: keys(draft.tags),
        maturity: draft.maturity.as_str().to_owned(),
        previous: None,
        tag: None,
        report: None,
        actor: Some(persona),
        account,
        submitter: Some(account),
        prompt: None,
        about: None,
    }
}

fn flag_of(verdict: Verdict) -> Option<Automated> {
    match verdict {
        Verdict::Flag(f) => Some(f),
        _ => None,
    }
}

/// Before a new prompt is saved: `tag_requested` for each tag the server
/// doesn't have yet, then `prompt_created`.
pub(crate) async fn before_new_prompt(
    state: &AppState,
    account: Uuid,
    persona: Uuid,
    draft: &PromptDraft<'_>,
) -> Result<PromptChecks, ApiError> {
    if !applies_to(state, account).await? {
        return Ok(PromptChecks::default());
    }
    let mut checks = PromptChecks {
        tags: new_tags(state, account, persona, draft).await?,
        ..PromptChecks::default()
    };
    checks.prompt =
        flag_of(check(state, subject(Hook::PromptCreated, account, persona, draft)).await?);
    Ok(checks)
}

/// Before an edit is saved: `tag_requested` for new tags, then
/// `prompt_edited` with the prompt as it was.
pub(crate) async fn before_edit(
    state: &AppState,
    account: Uuid,
    persona: Uuid,
    prompt: Uuid,
    draft: &PromptDraft<'_>,
    previous: Previous,
) -> Result<PromptChecks, ApiError> {
    if !applies_to(state, account).await? {
        return Ok(PromptChecks::default());
    }
    let mut checks = PromptChecks {
        tags: new_tags(state, account, persona, draft).await?,
        ..PromptChecks::default()
    };
    let mut subject = subject(Hook::PromptEdited, account, persona, draft);
    subject.previous = Some(previous);
    subject.prompt = Some(prompt);
    checks.prompt = flag_of(check(state, subject).await?);
    Ok(checks)
}

/// Runs `tag_requested` for each tag in `draft` the server doesn't have.
async fn new_tags(
    state: &AppState,
    account: Uuid,
    persona: Uuid,
    draft: &PromptDraft<'_>,
) -> Result<Vec<(String, Automated)>, ApiError> {
    if draft.tags.is_empty() {
        return Ok(Vec::new());
    }
    let pairs: Vec<(String, String)> = draft
        .tags
        .iter()
        .map(|t| (t.namespace.clone(), t.slug.clone()))
        .collect();
    let mut conn = state.pool.acquire().await?;
    let known = crate::prompts::resolve_tags(&mut conn, &pairs, false).await?;
    drop(conn);
    let mut flagged = Vec::new();
    for spec in draft.tags {
        if known.contains_key(&(spec.namespace.clone(), spec.slug.clone())) {
            continue;
        }
        let key = format!("{}:{}", spec.namespace, spec.slug);
        let mut subject = subject(Hook::TagRequested, account, persona, draft);
        subject.title = spec.display.clone();
        subject.body = String::new();
        subject.tags = vec![key.clone()];
        subject.tag = Some(TagInfo {
            namespace: spec.namespace.clone(),
            slug: spec.slug.clone(),
            display: spec.display.clone(),
        });
        if let Some(flag) = flag_of(check(state, subject).await?) {
            flagged.push((key, flag));
        }
    }
    Ok(flagged)
}

/// After a prompt (new or edited) is saved: files its flags.
pub(crate) async fn after_prompt(
    state: &AppState,
    checks: PromptChecks,
    account: Uuid,
    prompt: Uuid,
    title: &str,
    body: &str,
) {
    if let Some(flag) = &checks.prompt {
        file_flag(
            state,
            flag,
            FlagTarget::Prompt {
                id: prompt,
                title,
                body,
            },
            account,
        )
        .await;
    }
    for (key, flag) in &checks.tags {
        let Some((namespace, slug)) = key.split_once(':') else {
            continue;
        };
        let id: Result<Option<i64>, sqlx::Error> = sqlx::query_scalar(
            "SELECT id FROM tags WHERE instance = '' AND namespace = $1 AND slug = $2 AND status = 'pending'",
        )
        .bind(namespace)
        .bind(slug)
        .fetch_optional(&state.pool)
        .await;
        // A tag created visible (no approval needed) has no queue to mark;
        // the flag is filed against the prompt instead.
        match id {
            Ok(Some(id)) => file_flag(state, flag, FlagTarget::Tag { id, key }, account).await,
            Ok(None) => {
                file_flag(
                    state,
                    flag,
                    FlagTarget::Prompt {
                        id: prompt,
                        title,
                        body,
                    },
                    account,
                )
                .await
            }
            Err(e) => tracing::error!(error = ?e, "finding a flagged tag"),
        }
    }
}

/// Before a comment is saved. Returns the flag to file once it is.
pub(crate) async fn before_comment(
    state: &AppState,
    account: Uuid,
    persona: Option<Uuid>,
    announcement_title: &str,
    body: &str,
) -> Result<Option<Automated>, ApiError> {
    if !applies_to(state, account).await? {
        return Ok(None);
    }
    let subject = Subject {
        hook: Hook::CommentCreated,
        title: String::new(),
        body: body.to_owned(),
        tags: Vec::new(),
        maturity: String::new(),
        previous: None,
        tag: None,
        report: None,
        actor: persona,
        account,
        submitter: Some(account),
        prompt: None,
        about: Some(announcement_title.to_owned()),
    };
    Ok(flag_of(check(state, subject).await?))
}

pub(crate) async fn after_comment(
    state: &AppState,
    flag: Option<Automated>,
    account: Uuid,
    comment: Uuid,
    body: &str,
) {
    if let Some(flag) = &flag {
        file_flag(
            state,
            flag,
            FlagTarget::Comment { id: comment, body },
            account,
        )
        .await;
    }
}

/// The prompt a report is about, as `prompt_reported` sees it.
pub(crate) struct Reported<'a> {
    pub prompt: Uuid,
    pub persona: Uuid,
    pub author: Uuid,
    pub title: &'a str,
    pub body: &'a str,
    pub tags: Vec<String>,
    pub maturity: Maturity,
}

/// Before a writer's report is filed. The history is the reported
/// writer's; the reporter is never named to scripts.
pub(crate) async fn before_report(
    state: &AppState,
    reporter: Option<Uuid>,
    reported: Reported<'_>,
    reason: &str,
    details: &str,
) -> Result<Verdict, ApiError> {
    let subject = Subject {
        hook: Hook::PromptReported,
        title: reported.title.to_owned(),
        body: reported.body.to_owned(),
        tags: reported.tags,
        maturity: reported.maturity.as_str().to_owned(),
        previous: None,
        tag: None,
        report: Some(ReportInfo {
            reason: reason.to_owned(),
            details: details.to_owned(),
        }),
        actor: Some(reported.persona),
        account: reported.author,
        submitter: reporter,
        prompt: Some(reported.prompt),
        about: None,
    };
    check(state, subject).await
}
