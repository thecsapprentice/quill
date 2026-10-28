//! The prompt form's tag picker: reading the form, and turning the tags it
//! names into chips, or into messages explaining why they can't be added.
//!
//! Nothing here creates tags; that happens only when the prompt is saved.

use quill_board_ui::{ChipKind, TagChip};
use quill_proto::{
    board::{Tag, TagOrigin, TagPolicy, TagStatus},
    limits::PROMPT_MAX_TAGS,
    validate,
};

use crate::{
    AppState,
    error::ApiError,
    prompts::{chip_color, load_namespace, load_tags, resolve_tags},
    server_settings,
};

/// The prompt form as sent. `tag` repeats (one per ticked chip), which the
/// standard form extractor can't read, so the body is parsed here.
#[derive(Debug, Default)]
pub struct PromptPost {
    pub csrf: Option<String>,
    pub persona: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub maturity: Option<String>,
    pub state: Option<String>,
    /// Ticked chips.
    pub tags: Vec<String>,
    /// The "add tags" field.
    pub add: String,
    /// A tag chosen from "Browse tags".
    pub pick: Option<String>,
    /// `save` posts the prompt; anything else (Add, a browsed tag, or Enter
    /// in a field) only updates the form.
    pub op: Option<String>,
}

impl PromptPost {
    pub fn parse(body: &[u8]) -> Self {
        let mut post = Self::default();
        for (key, value) in url::form_urlencoded::parse(body) {
            let value = value.into_owned();
            match key.as_ref() {
                "csrf" => post.csrf = Some(value),
                "persona" => post.persona = Some(value),
                "title" => post.title = Some(value),
                "body" => post.body = Some(value),
                "maturity" => post.maturity = Some(value),
                "state" => post.state = Some(value),
                "tag" => post.tags.push(value),
                "add" => post.add = value,
                "pick" => post.pick = Some(value),
                "op" => post.op = Some(value),
                _ => {}
            }
        }
        post
    }

    pub fn saving(&self) -> bool {
        self.op.as_deref() == Some("save")
    }

    /// The Preview button: show the text as the board would.
    pub fn previewing(&self) -> bool {
        self.op.as_deref() == Some("preview")
    }

    /// Every tag named: kept chips, then typed ones, then a browsed one.
    pub fn tag_values(&self) -> Vec<String> {
        self.tags
            .iter()
            .cloned()
            .chain(split_tags(&self.add))
            .chain(self.pick.clone())
            .collect()
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect::<String>() + ".")
        .unwrap_or_default()
}

/// Splits the "add tags" field on commas.
pub fn split_tags(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The result of reading the named tags.
#[derive(Debug, Default)]
pub struct Picked {
    pub chips: Vec<TagChip>,
    /// Why some tags couldn't be added.
    pub errors: Vec<String>,
    /// Typed tags that couldn't be added, to put back in the field.
    pub unadded: Vec<String>,
}

/// Turns tag names into chips, in order and without duplicates, checking
/// each against the server's tags, tag types and "new tags" setting.
pub async fn preview(state: &AppState, values: &[String]) -> Result<Picked, ApiError> {
    let mut picked = Picked::default();
    let mut specs = Vec::new();
    for raw in values {
        match validate::tag_spec(raw) {
            Ok(spec) => specs.push((raw.clone(), spec)),
            Err(e) => {
                // The message names the tag already.
                picked.errors.push(capitalize(&e.to_string()));
                picked.unadded.push(raw.clone());
            }
        }
    }
    let keys: Vec<(String, String)> = specs
        .iter()
        .map(|(_, s)| (s.namespace.clone(), s.slug.clone()))
        .collect();
    let mut conn = state.pool.acquire().await?;
    let known = resolve_tags(&mut conn, &keys, false).await?;
    let ids: Vec<i64> = known.values().copied().collect();
    let tags = load_tags(&state.pool, &ids).await?;
    let mut policy: Option<TagPolicy> = None;
    let mut seen: Vec<String> = Vec::new();

    for (raw, spec) in specs {
        let key = (spec.namespace.clone(), spec.slug.clone());
        let chip = match known.get(&key).and_then(|id| tags.get(id)) {
            // Hidden tags stay hidden: shown as typed, not by their name or
            // colour on the server.
            Some(tag) if tag.status == TagStatus::Hidden => TagChip {
                value: format!("{}:{}", spec.namespace, spec.slug),
                tag: Tag {
                    namespace: spec.namespace.clone(),
                    slug: spec.slug.clone(),
                    display: spec.display.clone(),
                    origin: TagOrigin::User,
                    status: TagStatus::Visible,
                    color: None,
                },
                kind: ChipKind::Existing,
            },
            Some(tag) => TagChip {
                value: tag.key(),
                tag: tag.clone(),
                kind: ChipKind::Existing,
            },
            None => {
                let Ok(ns) = load_namespace(&mut conn, &spec.namespace, false).await else {
                    picked.errors.push(format!(
                        "\"{raw}\": there's no tag type \"{}\" on this server.",
                        spec.namespace
                    ));
                    picked.unadded.push(raw);
                    continue;
                };
                let policy = match policy {
                    Some(p) => p,
                    None => *policy.insert(server_settings::tag_policy(&mut conn).await?),
                };
                if ns.locked || !policy.allows_new_tags() {
                    let why = if ns.locked {
                        format!("new {} tags can't be created", ns.display)
                    } else {
                        "this server doesn't accept new tags".to_owned()
                    };
                    picked
                        .errors
                        .push(format!("\"{raw}\" isn't a tag on this server, and {why}."));
                    picked.unadded.push(raw);
                    continue;
                }
                TagChip {
                    // The display name keeps the writer's capitalization.
                    value: format!("{}:{}", spec.namespace, spec.display),
                    tag: Tag {
                        namespace: spec.namespace.clone(),
                        slug: spec.slug.clone(),
                        display: spec.display.clone(),
                        origin: TagOrigin::User,
                        status: TagStatus::Visible,
                        color: chip_color(
                            ns.color.as_ref().map(|c| c.background.clone()),
                            ns.color.and_then(|c| c.text),
                        ),
                    },
                    kind: if policy == TagPolicy::Approval {
                        ChipKind::NewNeedsApproval
                    } else {
                        ChipKind::New
                    },
                }
            }
        };
        let identity = chip.tag.key();
        if seen.contains(&identity) {
            continue;
        }
        if picked.chips.len() >= PROMPT_MAX_TAGS {
            picked.errors.push(format!(
                "A prompt can have at most {PROMPT_MAX_TAGS} tags, so \"{raw}\" wasn't added."
            ));
            picked.unadded.push(raw);
            continue;
        }
        seen.push(identity);
        picked.chips.push(chip);
    }
    Ok(picked)
}

/// Chips for a prompt's current tags, as its author sees them.
pub fn chips_for(tags: &[Tag]) -> Vec<TagChip> {
    tags.iter()
        .map(|t| TagChip {
            value: t.key(),
            tag: t.clone(),
            kind: ChipKind::Existing,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repeated_fields() {
        let post = PromptPost::parse(
            b"csrf=t&tag=genre%3Ahorror&tag=theme%3ASea&add=format%3Aprose%2C+%2C+length%3Ashort&pick=genre%3Amystery&op=add&title=A+%26+B",
        );
        assert_eq!(post.title.as_deref(), Some("A & B"));
        assert!(!post.saving());
        assert_eq!(
            post.tag_values(),
            vec![
                "genre:horror",
                "theme:Sea",
                "format:prose",
                "length:short",
                "genre:mystery"
            ]
        );
        assert!(PromptPost::parse(b"op=save").saving());
        // A form sent without pressing a button (Enter, older browsers) only
        // updates the form.
        assert!(!PromptPost::parse(b"title=x").saving());
    }
}
