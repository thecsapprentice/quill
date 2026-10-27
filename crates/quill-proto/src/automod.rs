//! Automated moderation (#43): administrators' scripts, the chains that run
//! them, and the common data they read. Administrators only.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

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
