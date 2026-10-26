//! What a script is given (a cut-down version of #43's context) and what it
//! can return, plus the sample posts every engine is run against.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct Post {
    pub hook: &'static str,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub maturity: &'static str,
    pub author: Author,
    pub history: History,
}

#[derive(Debug, Clone, Serialize)]
pub struct Author {
    pub handle: String,
    pub account_age_days: f64,
    pub removals: i64,
}

/// Counted per account, across its personas.
#[derive(Debug, Clone, Default, Serialize)]
pub struct History {
    pub prompts_last_hour: i64,
    pub prompts_last_day: i64,
    /// The closest prompt on the board from any account, last 30 days.
    pub most_similar_on_board: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum Action {
    Pass,
    Reject { message: String },
    Flag { reason: String, category: String },
}

impl Action {
    /// Engines hand back JSON; nothing returned at all means pass.
    pub fn from_json(value: serde_json::Value) -> Result<Self, String> {
        if value.is_null() {
            return Ok(Action::Pass);
        }
        serde_json::from_value(value).map_err(|e| format!("script returned something that isn't an action: {e}"))
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Action::Pass => "pass",
            Action::Reject { .. } => "reject",
            Action::Flag { .. } => "flag",
        }
    }
}

/// A sample post and what each sample script should do with it.
pub struct Fixture {
    pub name: &'static str,
    pub post: Post,
    /// (sample, expected action kind); samples not listed expect "pass".
    pub expect: &'static [(&'static str, &'static str)],
}

pub const SAMPLES: [&str; 4] = ["words", "links", "lookalikes", "newcomer"];

fn post(title: &str, body: &str) -> Post {
    Post {
        hook: "prompt_created",
        title: title.to_owned(),
        body: body.to_owned(),
        tags: vec!["fandom:original".to_owned()],
        maturity: "general",
        author: Author { handle: "writer".to_owned(), account_age_days: 400.0, removals: 0 },
        history: History { prompts_last_hour: 0, prompts_last_day: 1, most_similar_on_board: 0.2 },
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture {
            name: "ordinary prompt",
            post: post(
                "A lighthouse keeper's last night",
                "Looking for someone to write a slow, quiet story about the last night \
                 before the lighthouse is automated. See https://quill.example/rules first.",
            ),
            expect: &[],
        },
        Fixture {
            name: "spaced-out spam",
            post: post("Great deal", "Get your S P A M here, cheaper than anywhere!"),
            expect: &[("words", "reject")],
        },
        Fixture {
            name: "phone and outside link",
            post: post("Write with me", "Text me on +1 (555) 010-7788 or visit chat-elsewhere.biz/join"),
            expect: &[("links", "reject")],
        },
        Fixture {
            name: "Cyrillic lookalike",
            post: post("Account notice", "Your p\u{0430}yp\u{0430}l account is locked, log in to fix it"),
            expect: &[("words", "reject"), ("lookalikes", "flag")],
        },
        Fixture {
            name: "new account burst",
            post: Post {
                author: Author { handle: "newbie".to_owned(), account_age_days: 0.5, removals: 0 },
                history: History { prompts_last_hour: 4, prompts_last_day: 4, most_similar_on_board: 0.3 },
                ..post("Prompt number five", "Another idea: two rivals stuck in a lift.")
            },
            expect: &[("newcomer", "reject")],
        },
        Fixture {
            name: "new account copy",
            post: Post {
                author: Author { handle: "fresh".to_owned(), account_age_days: 1.0, removals: 0 },
                history: History { prompts_last_hour: 1, prompts_last_day: 1, most_similar_on_board: 0.97 },
                ..post("A lighthouse keeper's last night", "Looking for someone to write a slow, quiet story.")
            },
            expect: &[("newcomer", "flag")],
        },
    ]
}
