//! The default templates: scripts that ship with the server, which admins
//! copy and edit rather than writing their own. A copy is the admin's from
//! then on; a later server version with a changed template doesn't touch it.

use quill_proto::automod::{ModerationScript, SaveModerationData, UploadScript};
use serde::Serialize;
use uuid::Uuid;

use super::{data, scripts};
use crate::{AppState, error::ApiError};

/// A common data entry a template reads, created empty if it's missing.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateData {
    pub name: &'static str,
    pub description: &'static str,
    /// What a new entry starts with: notes for the admin, as `#` lines.
    pub starts_with: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    /// The chains it's written for.
    pub hooks: &'static [&'static str],
    pub source: &'static str,
    pub data: &'static [TemplateData],
}

const PROMPTS_AND_COMMENTS: &[&str] = &["prompt_created", "prompt_edited", "comment_created"];

pub const TEMPLATES: &[Template] = &[
    Template {
        id: "blocked_words",
        name: "Blocked words and phrases",
        summary: "Refuses posts using any word or phrase on a list, through spacing, repeats and lookalike letters.",
        hooks: PROMPTS_AND_COMMENTS,
        source: include_str!("templates/blocked_words.rhai"),
        data: &[TemplateData {
            name: "blocked_words",
            description: "Words and phrases to refuse, one per line.",
            starts_with: "# Words and phrases to refuse, one per line.\n# Plain spellings are enough: spacing, repeats and lookalike letters are seen through.\n",
        }],
    },
    Template {
        id: "outside_links",
        name: "Links to outside sites",
        summary: "Refuses (or flags) links to any site that isn't on a list.",
        hooks: PROMPTS_AND_COMMENTS,
        source: include_str!("templates/outside_links.rhai"),
        data: &[TemplateData {
            name: "allowed_sites",
            description: "Sites posts may link to, one per line.",
            starts_with: "# Sites posts may link to, one per line, like archiveofourown.org.\n",
        }],
    },
    Template {
        id: "contact_details",
        name: "Phone numbers and email addresses",
        summary: "Refuses posts that share a phone number or an email address.",
        hooks: PROMPTS_AND_COMMENTS,
        source: include_str!("templates/contact_details.rhai"),
        data: &[],
    },
    Template {
        id: "mixed_alphabets",
        name: "Mixed alphabets",
        summary: "Flags words that mix writing systems, a common way past word lists.",
        hooks: PROMPTS_AND_COMMENTS,
        source: include_str!("templates/mixed_alphabets.rhai"),
        data: &[],
    },
    Template {
        id: "new_accounts",
        name: "New accounts",
        summary: "Limits how fast new accounts post, and flags their near copies of others' prompts.",
        hooks: &["prompt_created", "prompt_edited"],
        source: include_str!("templates/new_accounts.rhai"),
        data: &[],
    },
    Template {
        id: "shouting",
        name: "Shouting and repeated characters",
        summary: "Flags posts mostly in capitals, or full of repeated characters.",
        hooks: PROMPTS_AND_COMMENTS,
        source: include_str!("templates/shouting.rhai"),
        data: &[],
    },
    Template {
        id: "duplicates",
        name: "Posting the same prompt again",
        summary: "Refuses a prompt almost the same as one the account posted in the last 30 days.",
        hooks: &["prompt_created"],
        source: include_str!("templates/duplicates.rhai"),
        data: &[],
    },
];

pub fn get(id: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.id == id)
}

/// Copies template `id` into a new script named `name` (the template's
/// name if `None`), creating the common data entries it reads if they
/// don't exist yet.
pub async fn create_from(
    state: &AppState,
    admin: Uuid,
    id: &str,
    name: Option<&str>,
) -> Result<ModerationScript, ApiError> {
    let template = get(id).ok_or(ApiError::NotFound)?;
    for entry in template.data {
        match data::get(&state.pool, entry.name).await {
            Ok(_) => {}
            Err(ApiError::NotFound) => {
                let body = SaveModerationData {
                    description: entry.description.to_owned(),
                    content: entry.starts_with.to_owned(),
                    base_version: None,
                };
                // Someone else creating it at the same moment is fine.
                match data::save(state, admin, entry.name, &body).await {
                    Ok(_) | Err(ApiError::Conflict(_)) => {}
                    Err(e) => return Err(e),
                }
            }
            Err(e) => return Err(e),
        }
    }
    let upload = UploadScript {
        name: name.unwrap_or(template.name).to_owned(),
        source: template.source.to_owned(),
        log_only: false,
    };
    scripts::upload(state, admin, &upload).await
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use quill_proto::automod::Hook;
    use rhai::serde::to_dynamic;

    use super::*;
    use crate::moderation::Limits;
    use crate::moderation::context::{Similar, sample};
    use crate::moderation::data::Snapshot;
    use crate::moderation::engine::{Outcome, Script};

    fn data() -> Snapshot {
        Snapshot::from_entries([
            (
                "blocked_words".to_owned(),
                1,
                "# notes\nspam\nfree crypto\npaypal".to_owned(),
            ),
            (
                "allowed_sites".to_owned(),
                1,
                "quill.example\narchiveofourown.org".to_owned(),
            ),
        ])
    }

    fn run(template: &str, edit: impl FnOnce(&mut crate::moderation::context::Post)) -> Outcome {
        let t = get(template).unwrap();
        let script = Script::compile(t.source, &scripts::VARIABLES, &Limits::default())
            .unwrap_or_else(|e| panic!("{template}: {e:?}"));
        let mut post = sample(Hook::PromptCreated);
        edit(&mut post);
        script
            .run(
                &[("post", to_dynamic(&post).unwrap())],
                &data(),
                Duration::from_millis(500),
            )
            .result
            .unwrap_or_else(|e| panic!("{template}: {e:?}"))
    }

    fn kind(outcome: &Outcome) -> &'static str {
        match outcome {
            Outcome::Pass => "pass",
            Outcome::Reject { .. } => "reject",
            Outcome::Flag { .. } => "flag",
            Outcome::Dismiss { .. } => "dismiss",
        }
    }

    #[test]
    fn every_template_passes_every_sample_post_it_is_for() {
        for t in TEMPLATES {
            let script = Script::compile(t.source, &scripts::VARIABLES, &Limits::default())
                .unwrap_or_else(|e| panic!("{}: {e:?}", t.id));
            for hook in t.hooks {
                let post = to_dynamic(sample(hook.parse().unwrap())).unwrap();
                let run = script.run(&[("post", post)], &data(), Duration::from_millis(500));
                assert_eq!(run.result, Ok(Outcome::Pass), "{} at {hook}", t.id);
            }
            let names: Vec<&str> = script.data_names().iter().map(String::as_str).collect();
            let declared: Vec<&str> = t.data.iter().map(|d| d.name).collect();
            assert_eq!(names, declared, "{}: data it reads", t.id);
        }
    }

    #[test]
    fn blocked_words() {
        assert_eq!(
            kind(&run("blocked_words", |p| p.body = "Get your S P A M here".into())),
            "reject"
        );
        assert_eq!(
            kind(&run("blocked_words", |p| p.title = "p\u{0430}ypal help".into())),
            "reject"
        );
        assert_eq!(
            kind(&run("blocked_words", |p| p.body = "A spammer's lament".into())),
            "pass"
        );
    }

    #[test]
    fn outside_links() {
        assert_eq!(
            kind(&run("outside_links", |p| p.body = "See evil.biz/join".into())),
            "reject"
        );
        assert_eq!(
            kind(&run("outside_links", |p| p.body =
                "Read https://quill.example/rules".into())),
            "pass"
        );
        let sub = run("outside_links", |p| {
            p.body = "My work: https://www.archiveofourown.org/works/1".into()
        });
        assert_eq!(kind(&sub), "pass");
        let lookalike = run("outside_links", |p| {
            p.body = "evilarchiveofourown.org".into()
        });
        assert_eq!(kind(&lookalike), "reject");
    }

    #[test]
    fn contact_details() {
        for body in [
            "Call +1 (555) 010-7788",
            "mail pat@example.com",
            "pat at example dot com",
        ] {
            assert_eq!(
                kind(&run("contact_details", |p| p.body = body.into())),
                "reject",
                "{body}"
            );
        }
        assert_eq!(
            kind(&run("contact_details", |p| p.body =
                "Set in 1984, at dusk.".into())),
            "pass"
        );
    }

    #[test]
    fn mixed_alphabets() {
        assert_eq!(
            kind(&run("mixed_alphabets", |p| p.body =
                "log in to p\u{0430}ypal".into())),
            "flag"
        );
        assert_eq!(
            kind(&run("mixed_alphabets", |p| p.body = "Привет and hello".into())),
            "pass"
        );
    }

    #[test]
    fn new_accounts() {
        let burst = run("new_accounts", |p| {
            p.author.account_age_days = 0.5;
            p.history.prompts_last_hour = 3;
        });
        assert_eq!(kind(&burst), "reject");
        let copy = run("new_accounts", |p| {
            p.author.account_age_days = 1.0;
            p.history.similar_on_board = vec![Similar {
                title: "Theirs".into(),
                similarity: 0.95,
                same_account: false,
                minutes_ago: 30.0,
            }];
        });
        assert_eq!(kind(&copy), "flag");
        let old = run("new_accounts", |p| p.history.prompts_last_hour = 10);
        assert_eq!(kind(&old), "pass");
    }

    #[test]
    fn shouting() {
        let loud = run("shouting", |p| {
            p.body = "PLEASE WRITE WITH ME RIGHT NOW EVERYONE".into()
        });
        assert_eq!(kind(&loud), "flag");
        let stretched = run("shouting", |p| {
            p.body = "noooooooooo!!!!!!!!!!! whyyyyyyyyy".into()
        });
        assert_eq!(kind(&stretched), "flag");
        let short = run("shouting", |p| {
            p.title = "OK".into();
            p.body = "YES!".into();
        });
        assert_eq!(kind(&short), "pass");
    }

    #[test]
    fn duplicates() {
        let again = run("duplicates", |p| {
            p.history.own_recent = vec![Similar {
                title: "Mine".into(),
                similarity: 0.9,
                same_account: true,
                minutes_ago: 60.0,
            }];
        });
        assert_eq!(kind(&again), "reject");
    }
}
