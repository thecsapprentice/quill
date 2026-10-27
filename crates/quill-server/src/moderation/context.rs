//! What a script is given: the `post` variable. Every hook fills the same
//! shape, so a script written for prompts also reads comments and tag
//! requests; fields a hook has nothing for are empty, not missing.
//!
//! Only public board activity goes in: never a login name, a reporter, or
//! anything from a session. It's gathered once, with fixed caps, before the
//! chain runs (architecture document, "Automated moderation: context").

use serde::Serialize;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use super::Hook;
use crate::error::ApiError;

/// How far back the history looks for similar prompts.
const SIMILAR_DAYS: i32 = 30;
/// Similar prompts listed, own and board-wide.
const OWN_RECENT: i64 = 20;
const BOARD_SIMILAR: i64 = 5;
/// Recent post times listed.
const RECENT_TIMES: i64 = 20;

/// The `post` variable.
#[derive(Debug, Clone, Serialize)]
pub struct Post {
    /// Which hook is running, such as `prompt_created`.
    pub hook: &'static str,
    /// `prompt`, `comment`, `tag` or `report`.
    pub kind: &'static str,
    /// A prompt's title; a requested tag's name; empty for a comment.
    pub title: String,
    /// The Markdown text; empty for a tag.
    pub body: String,
    /// `namespace:slug` keys, such as `genre:horror`.
    pub tags: Vec<String>,
    /// `general`, `teen`, `mature` or `explicit`; empty for a comment.
    pub maturity: String,
    /// For an edit, the prompt as it was; `()` otherwise.
    pub previous: Option<Previous>,
    /// For a tag request, the tag; `()` otherwise.
    pub tag: Option<TagInfo>,
    /// For a report, what the reporter chose and wrote; `()` otherwise.
    pub report: Option<ReportInfo>,
    pub author: Author,
    pub history: History,
}

#[derive(Debug, Clone, Serialize)]
pub struct Previous {
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub maturity: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TagInfo {
    pub namespace: String,
    pub slug: String,
    pub display: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportInfo {
    pub reason: String,
    pub details: String,
}

/// The writer, as the board knows them. Counts cover the whole account,
/// across its personas.
#[derive(Debug, Clone, Serialize)]
pub struct Author {
    /// The persona's handle; empty when nobody signed in is reporting.
    pub handle: String,
    pub account_age_days: f64,
    /// Prompts and comments administrators have removed.
    pub removals: i64,
    /// Submissions moderation scripts have refused.
    pub rejections: i64,
}

/// Posting activity, counted per account.
#[derive(Debug, Clone, Default, Serialize)]
pub struct History {
    pub prompts_last_hour: i64,
    pub prompts_last_day: i64,
    pub prompts_last_week: i64,
    /// Prompts whose latest edit was in the window.
    pub edited_last_hour: i64,
    pub edited_last_day: i64,
    pub comments_last_hour: i64,
    pub comments_last_day: i64,
    /// Minutes since each of the account's latest prompts and comments,
    /// newest first.
    pub recent_minutes_ago: Vec<f64>,
    /// The account's own prompts in the last 30 days, most alike first.
    pub own_recent: Vec<Similar>,
    /// The most alike prompts on the board from any account, last 30 days.
    pub similar_on_board: Vec<Similar>,
    /// The highest similarity in `similar_on_board`, 0.0 if none.
    pub most_similar_on_board: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Similar {
    pub title: String,
    /// 0.0 to 1.0, by shared trigrams of title and text.
    pub similarity: f64,
    /// Whether it's the same account's.
    pub same_account: bool,
    pub minutes_ago: f64,
}

/// What's being checked, before the history is added.
#[derive(Debug, Clone)]
pub struct Subject {
    pub hook: Hook,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub maturity: String,
    pub previous: Option<Previous>,
    pub tag: Option<TagInfo>,
    pub report: Option<ReportInfo>,
    /// The persona writing, if a persona is.
    pub actor: Option<Uuid>,
    /// The account writing.
    pub account: Uuid,
    /// The prompt being edited or reported, left out of "similar" lists.
    pub prompt: Option<Uuid>,
}

fn minutes_since(at: OffsetDateTime, now: OffsetDateTime) -> f64 {
    ((now - at).as_seconds_f64() / 60.0).max(0.0)
}

/// Builds `post` for `subject`, with the author's history.
pub async fn build(pool: &PgPool, subject: Subject) -> Result<Post, ApiError> {
    let now = OffsetDateTime::now_utc();
    let account = subject.account;

    let (created_at, removals, rejections): (OffsetDateTime, i64, i64) = sqlx::query_as(
        "SELECT a.created_at,
                (SELECT count(*) FROM prompts p JOIN actors x ON x.id = p.actor_id
                  WHERE x.account_id = a.id AND p.removed_at IS NOT NULL)
              + (SELECT count(*) FROM announcement_comments c LEFT JOIN actors x ON x.id = c.actor_id
                  WHERE coalesce(x.account_id, c.account_id) = a.id AND c.removed_at IS NOT NULL),
                (SELECT count(*) FROM account_notices n
                  WHERE n.account_id = a.id AND n.kind = 'content_rejected')
         FROM accounts a WHERE a.id = $1",
    )
    .bind(account)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::Unauthorized)?;
    let handle: Option<String> = match subject.actor {
        Some(actor) => {
            sqlx::query_scalar("SELECT preferred_username FROM actors WHERE id = $1")
                .bind(actor)
                .fetch_optional(pool)
                .await?
        }
        None => None,
    };

    let mut history = counts(pool, account).await?;
    history.recent_minutes_ago = recent_times(pool, account)
        .await?
        .into_iter()
        .map(|at| minutes_since(at, now))
        .collect();
    if matches!(
        subject.hook,
        Hook::PromptCreated | Hook::PromptEdited | Hook::PromptReported
    ) {
        let text = format!("{} {}", subject.title, subject.body);
        history.own_recent = similar(pool, &text, account, subject.prompt, true, now).await?;
        history.similar_on_board =
            similar(pool, &text, account, subject.prompt, false, now).await?;
        history.most_similar_on_board = history
            .similar_on_board
            .iter()
            .map(|s| s.similarity)
            .fold(0.0, f64::max);
    }

    Ok(Post {
        hook: subject.hook.as_str(),
        kind: subject.hook.kind(),
        title: subject.title,
        body: subject.body,
        tags: subject.tags,
        maturity: subject.maturity,
        previous: subject.previous,
        tag: subject.tag,
        report: subject.report,
        author: Author {
            handle: handle.unwrap_or_default(),
            account_age_days: (now - created_at).as_seconds_f64() / 86_400.0,
            removals,
            rejections,
        },
        history,
    })
}

async fn counts(pool: &PgPool, account: Uuid) -> Result<History, ApiError> {
    let row: (i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "WITH mine AS (
             SELECT p.created_at, p.updated_at FROM prompts p JOIN actors x ON x.id = p.actor_id
             WHERE x.account_id = $1 AND p.updated_at > now() - interval '7 days'
         ), comments AS (
             SELECT c.created_at FROM announcement_comments c LEFT JOIN actors x ON x.id = c.actor_id
             WHERE coalesce(x.account_id, c.account_id) = $1 AND c.created_at > now() - interval '1 day'
         )
         SELECT
             (SELECT count(*) FROM mine WHERE created_at > now() - interval '1 hour'),
             (SELECT count(*) FROM mine WHERE created_at > now() - interval '1 day'),
             (SELECT count(*) FROM mine WHERE created_at > now() - interval '7 days'),
             (SELECT count(*) FROM mine WHERE updated_at > created_at AND updated_at > now() - interval '1 hour'),
             (SELECT count(*) FROM mine WHERE updated_at > created_at AND updated_at > now() - interval '1 day'),
             (SELECT count(*) FROM comments WHERE created_at > now() - interval '1 hour'),
             (SELECT count(*) FROM comments)",
    )
    .bind(account)
    .fetch_one(pool)
    .await?;
    Ok(History {
        prompts_last_hour: row.0,
        prompts_last_day: row.1,
        prompts_last_week: row.2,
        edited_last_hour: row.3,
        edited_last_day: row.4,
        comments_last_hour: row.5,
        comments_last_day: row.6,
        ..History::default()
    })
}

async fn recent_times(pool: &PgPool, account: Uuid) -> Result<Vec<OffsetDateTime>, ApiError> {
    Ok(sqlx::query_scalar(
        "SELECT at FROM (
             SELECT p.created_at AS at FROM prompts p JOIN actors x ON x.id = p.actor_id
             WHERE x.account_id = $1
             UNION ALL
             SELECT c.created_at FROM announcement_comments c LEFT JOIN actors x ON x.id = c.actor_id
             WHERE coalesce(x.account_id, c.account_id) = $1
         ) t ORDER BY at DESC LIMIT $2",
    )
    .bind(account)
    .bind(RECENT_TIMES)
    .fetch_all(pool)
    .await?)
}

/// Recent prompts like `text`: the account's own (`own`), or the board's.
/// The board-wide list uses the trigram index's threshold to stay cheap.
async fn similar(
    pool: &PgPool,
    text: &str,
    account: Uuid,
    exclude: Option<Uuid>,
    own: bool,
    now: OffsetDateTime,
) -> Result<Vec<Similar>, ApiError> {
    let rows: Vec<(String, f32, bool, OffsetDateTime)> = sqlx::query_as(
        "SELECT p.title, similarity(p.title || ' ' || p.body_md, $1), x.account_id = $2, p.created_at
         FROM prompts p JOIN actors x ON x.id = p.actor_id
         WHERE p.created_at > now() - make_interval(days => $3)
           AND p.removed_at IS NULL
           AND ($4::uuid IS NULL OR p.id <> $4)
           AND CASE WHEN $5 THEN x.account_id = $2 ELSE (p.title || ' ' || p.body_md) % $1 END
         ORDER BY 2 DESC LIMIT $6",
    )
    .bind(text)
    .bind(account)
    .bind(SIMILAR_DAYS)
    .bind(exclude)
    .bind(own)
    .bind(if own { OWN_RECENT } else { BOARD_SIMILAR })
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(title, similarity, same_account, at)| Similar {
            title,
            similarity: f64::from(similarity),
            same_account,
            minutes_ago: minutes_since(at, now),
        })
        .collect())
}

/// A made-up post for each hook, for checking a script at upload and for
/// dry runs with nothing else to run on.
pub fn sample(hook: Hook) -> Post {
    let (title, body, tags, maturity) = match hook {
        Hook::CommentCreated => (
            "",
            "Thanks for the update! Looking forward to it.",
            vec![],
            "",
        ),
        Hook::TagRequested => ("Found family", "", vec![], ""),
        _ => (
            "A lighthouse keeper's last night",
            "Looking for someone to write a slow, quiet story about the last night before the lighthouse is automated.",
            vec!["genre:literary".to_owned()],
            "general",
        ),
    };
    Post {
        hook: hook.as_str(),
        kind: hook.kind(),
        title: title.to_owned(),
        body: body.to_owned(),
        tags,
        maturity: maturity.to_owned(),
        previous: (hook == Hook::PromptEdited).then(|| Previous {
            title: title.to_owned(),
            body: "Looking for someone to write a quiet story.".to_owned(),
            tags: vec!["genre:literary".to_owned()],
            maturity: maturity.to_owned(),
        }),
        tag: (hook == Hook::TagRequested).then(|| TagInfo {
            namespace: "theme".to_owned(),
            slug: "found-family".to_owned(),
            display: "Found family".to_owned(),
        }),
        report: (hook == Hook::PromptReported).then(|| ReportInfo {
            reason: "spam".to_owned(),
            details: "Posted the same thing three times.".to_owned(),
        }),
        author: Author {
            handle: "sample_writer".to_owned(),
            account_age_days: 120.0,
            removals: 0,
            rejections: 0,
        },
        history: History {
            prompts_last_hour: 1,
            prompts_last_day: 1,
            prompts_last_week: 3,
            recent_minutes_ago: vec![0.0, 1_440.0, 4_320.0],
            ..History::default()
        },
    }
}
