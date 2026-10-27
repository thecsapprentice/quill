//! Automated moderation (#43): administrators' scripts, the chains that run
//! them, and the common data they read. Administrators only.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::board::string_enum;

string_enum! {
    /// Where in the board a chain of scripts runs, before the change is
    /// saved.
    Hook {
        PromptCreated => "prompt_created",
        PromptEdited => "prompt_edited",
        /// A comment on an announcement.
        CommentCreated => "comment_created",
        /// A writer used a tag the server doesn't have yet.
        TagRequested => "tag_requested",
        /// A writer reported a prompt.
        PromptReported => "prompt_reported",
    }
}

impl Hook {
    /// What `post` holds at this hook: `prompt`, `comment`, `tag` or
    /// `report`.
    pub fn kind(self) -> &'static str {
        match self {
            Self::PromptCreated | Self::PromptEdited => "prompt",
            Self::CommentCreated => "comment",
            Self::TagRequested => "tag",
            Self::PromptReported => "report",
        }
    }
}

string_enum! {
    /// What a chain does when one of its scripts fails (an error, or a
    /// limit reached).
    OnError {
        /// Skip the script and run the rest of the chain.
        Continue => "continue",
        /// Refuse the submission with a "couldn't be checked" message.
        Reject => "reject",
        /// Let it through and file a report for an administrator.
        Flag => "flag",
    }
}

/// A moderation script, as administrators see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ModerationScript {
    pub id: Uuid,
    pub name: String,
    /// Rhai source.
    pub source: String,
    /// Hex SHA-256 of `source`, as the log records it.
    pub source_hash: String,
    /// Whether it runs at all.
    pub enabled: bool,
    /// Whether its results are only logged, with nothing done.
    pub log_only: bool,
    /// The common data entries it reads.
    #[serde(default)]
    pub data: Vec<String>,
    /// The chains it's in.
    #[serde(default)]
    pub chains: Vec<Hook>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uploaded_by: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// `POST /admin/moderation/scripts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UploadScript {
    pub name: String,
    pub source: String,
    /// Start in log-only mode, to watch what it would do first.
    #[serde(default)]
    pub log_only: bool,
}

/// `PUT /admin/moderation/scripts/{id}`: new source, a new name, or both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ReplaceScript {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// `PATCH /admin/moderation/scripts/{id}`: switch a script on or off, or
/// into or out of log-only mode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SetScriptMode {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_only: Option<bool>,
}

/// One hook's chain: the scripts it runs, in order, and what a failing
/// script means.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ModerationChain {
    pub hook: Hook,
    pub on_error: OnError,
    pub steps: Vec<ChainStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ChainStep {
    pub script_id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub log_only: bool,
}

/// `PUT /admin/moderation/chains/{hook}`: the whole chain, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateChain {
    pub on_error: OnError,
    pub scripts: Vec<Uuid>,
}

/// A template scripts can be started from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ScriptTemplate {
    pub id: String,
    pub name: String,
    pub summary: String,
    /// The chains it's written for.
    pub hooks: Vec<Hook>,
    pub source: String,
    /// The common data entries it reads, created empty if missing.
    #[serde(default)]
    pub data: Vec<String>,
}

/// `POST /admin/moderation/templates/{id}`: a new script from a template.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UseTemplate {
    /// The new script's name; the template's own if left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// `POST /admin/moderation/data/{name}/restore`: undo back to a version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct RestoreData {
    pub version: i32,
}

/// One saved version's content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ModerationDataContent {
    pub name: String,
    pub version: i32,
    pub content: String,
}

/// `POST /admin/moderation/dry-run`: runs scripts without doing anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct DryRun {
    pub hook: Hook,
    /// One script to run; the hook's whole chain if left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<Uuid>,
    /// Run on this existing prompt, with its writer's history. Otherwise
    /// on `title` and `body`, from a made-up writer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<Uuid>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// Common data to use instead of what's saved, to try an edit before
    /// saving it.
    #[serde(default)]
    pub data: Vec<DataOverride>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct DataOverride {
    pub name: String,
    pub content: String,
}

string_enum! {
    /// What a script decided in a dry run.
    DryRunDecision {
        Pass => "pass",
        Reject => "reject",
        Flag => "flag",
        Dismiss => "dismiss",
        /// It failed: an error, or a limit reached.
        Error => "error",
    }
}

/// What one script did in a dry run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct DryRunStep {
    pub script_id: Uuid,
    pub name: String,
    pub decision: DryRunDecision,
    /// The message, reason or error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// What the script printed.
    #[serde(default)]
    pub printed: Vec<String>,
    pub micros: u64,
    pub log_only: bool,
    /// Whether this is the script that would decide for the chain.
    pub decides: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct DryRunResult {
    /// Every script, run whether or not an earlier one decided.
    pub steps: Vec<DryRunStep>,
}

/// What a writer is told when a script refuses without a message.
pub const GENERIC_REFUSAL: &str = "This was refused by the server's moderation rules.";

/// What a writer is told when a script fails and its chain refuses
/// submissions it couldn't check.
pub const UNCHECKED_REFUSAL: &str =
    "This couldn't be checked by the server's moderation rules just now. Please try again later.";

/// Longest common data name.
pub const MAX_DATA_NAME: usize = 64;

/// Whether `name` can name a common data entry: lowercase letters, digits
/// and underscores, 1 to [`MAX_DATA_NAME`] characters.
pub fn valid_data_name(name: &str) -> bool {
    (1..=MAX_DATA_NAME).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// A common data entry: a named string any script can read and none can
/// change. Lists are one entry per line; `#` starts a comment line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ModerationData {
    pub name: String,
    pub description: String,
    pub content: String,
    /// How many times it has been saved; each save is kept in its history.
    pub version: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    /// The administrator who saved it last, if the account still exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<String>,
    /// Names of the scripts that read it. It can't be deleted while any do.
    #[serde(default)]
    pub used_by: Vec<String>,
}

/// One entry in the list of common data, without its content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ModerationDataSummary {
    pub name: String,
    pub description: String,
    pub version: i32,
    /// Size of the content in bytes.
    pub size: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(default)]
    pub used_by: Vec<String>,
}

/// `PUT /admin/moderation/data/{name}`: creates the entry or saves a new
/// version of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SaveModerationData {
    #[serde(default)]
    pub description: String,
    pub content: String,
    /// The version this edit started from. If someone else has saved since,
    /// the save is refused rather than overwriting their change. Leave out
    /// to create an entry, or to save regardless.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_version: Option<i32>,
}

/// One saved version of an entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ModerationDataVersion {
    pub version: i32,
    pub size: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_names() {
        for good in ["blocked_words", "a", "list2", &"x".repeat(64)] {
            assert!(valid_data_name(good), "{good}");
        }
        for bad in ["", "Blocked", "two words", "dash-ed", "é", &"x".repeat(65)] {
            assert!(!valid_data_name(bad), "{bad}");
        }
    }
}
