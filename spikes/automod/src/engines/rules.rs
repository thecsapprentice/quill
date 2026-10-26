//! Rule files: no language at all, just TOML that lists conditions and what
//! to do. The first rule whose conditions all hold decides.

use serde::Deserialize;

use super::{Engine, Prepared};
use crate::context::{Action, Post};
use crate::text;

pub struct Rules;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    rule: Vec<Rule>,
}

/// Every condition that is set must hold for the rule to apply.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    #[allow(dead_code)]
    name: String,
    /// Any of these words or phrases, through spacing, repeats and lookalikes.
    any_words: Option<Vec<String>>,
    /// A regular expression, checked against the title and the body.
    pattern: Option<String>,
    /// A link to any site not on this list.
    links_except: Option<Vec<String>>,
    /// A word that mixes writing systems, like Latin with a Cyrillic letter.
    mixed_scripts: Option<bool>,
    account_younger_than_days: Option<f64>,
    prompts_last_hour_at_least: Option<i64>,
    similar_on_board_at_least: Option<f64>,

    then: Then,
    message: Option<String>,
    reason: Option<String>,
    category: Option<String>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Then {
    Reject,
    Flag,
}

struct Script(RuleFile);

impl Rule {
    fn applies(&self, post: &Post) -> Result<bool, String> {
        let all = format!("{}\n{}", post.title, post.body);
        if let Some(words) = &self.any_words
            && text::find_words(words, &all)?.is_empty()
        {
            return Ok(false);
        }
        if let Some(pattern) = &self.pattern
            && !text::matches(pattern, &all)?
        {
            return Ok(false);
        }
        if let Some(allowed) = &self.links_except
            && !text::links(&all).iter().any(|l| !allowed.iter().any(|a| l.host == *a))
        {
            return Ok(false);
        }
        if self.mixed_scripts == Some(true) && text::mixed_script_words(&all).is_empty() {
            return Ok(false);
        }
        if self.account_younger_than_days.is_some_and(|d| post.author.account_age_days >= d) {
            return Ok(false);
        }
        if self.prompts_last_hour_at_least.is_some_and(|n| post.history.prompts_last_hour < n) {
            return Ok(false);
        }
        if self.similar_on_board_at_least.is_some_and(|s| post.history.most_similar_on_board < s) {
            return Ok(false);
        }
        Ok(true)
    }

    fn action(&self) -> Action {
        match self.then {
            Then::Reject => Action::Reject { message: self.message.clone().unwrap_or_default() },
            Then::Flag => Action::Flag {
                reason: self.reason.clone().unwrap_or_default(),
                category: self.category.clone().unwrap_or_else(|| "other".to_owned()),
            },
        }
    }
}

impl Engine for Rules {
    fn name(&self) -> &'static str {
        "Rule file"
    }

    fn extension(&self) -> &'static str {
        "toml"
    }

    fn prepare(&self, source: &str) -> Result<Box<dyn Prepared>, String> {
        let file: RuleFile = toml::from_str(source).map_err(|e| e.to_string())?;
        // Check patterns now, so a bad one is refused at upload.
        for rule in &file.rule {
            if let Some(pattern) = &rule.pattern {
                text::matches(pattern, "")?;
            }
        }
        Ok(Box::new(Script(file)))
    }
}

impl Prepared for Script {
    fn run(&self, post: &Post) -> Result<Action, String> {
        for rule in &self.0.rule {
            if rule.applies(post)? {
                return Ok(rule.action());
            }
        }
        Ok(Action::Pass)
    }
}
