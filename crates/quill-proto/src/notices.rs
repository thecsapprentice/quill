//! Account notices: what administrators did to an account's prompts and
//! tags, readable only by that account.
//!
//! Notices are typed records with structured fields; clients write the
//! sentences. Later moderation (warnings, messages, appeals) adds kinds.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::board::ReportReason;

crate::board::string_enum! {
    /// What happened.
    NoticeKind {
        /// A prompt was removed from the board after a report.
        PromptRemoved => "prompt_removed",
        /// A tag on the account's prompts was deleted.
        TagDeleted => "tag_deleted",
        /// A new tag the account used wasn't approved, so it was removed.
        TagRejected => "tag_rejected",
        /// A tag was merged into another; the prompts carry that one now.
        TagMerged => "tag_merged",
        /// A tag stays on the prompts but only administrators see it.
        TagHidden => "tag_hidden",
        /// A hidden tag is visible again.
        TagShown => "tag_shown",
        /// A tag's display name changed.
        TagRenamed => "tag_renamed",
        /// A new tag the account used was approved.
        TagApproved => "tag_approved",
    }
}

/// Kind-specific details. Absent fields don't apply to the kind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoticeFields {
    /// The tag acted on, as `namespace:slug`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// The tag's display name at the time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_name: Option<String>,
    /// For a merge: the tag the prompts carry now, as `namespace:slug`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub into: Option<String>,
    /// For a rename: the old and new display names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// For a removal after a report: the report's category. Never who
    /// reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<ReportReason>,
}

/// One of the account's prompts as it was when the action was taken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoticePrompt {
    pub id: Uuid,
    pub title: String,
    /// The persona it was posted as.
    pub handle: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountNotice {
    pub id: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub kind: NoticeKind,
    #[serde(default)]
    pub fields: NoticeFields,
    /// The account's affected prompts.
    #[serde(default)]
    pub prompts: Vec<NoticePrompt>,
    pub read: bool,
}
