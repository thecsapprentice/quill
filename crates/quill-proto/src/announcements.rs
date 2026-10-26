//! Announcements: news the server's administrators post on the board.
//!
//! They are posted as the server, not as a persona, so readers see "the
//! administrators" rather than one person. They stay on this server; they
//! aren't federated. Writers comment on them as personas.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::board::{PromptAuthor, ReportReason};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct Announcement {
    pub id: Uuid,
    pub title: String,
    /// Markdown source.
    pub body: String,
    /// Sanitized HTML rendered by the server from `body`.
    pub body_html: String,
    /// Whether writers may comment on it.
    pub comments_open: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Later than `created_at` once it has been edited.
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl Announcement {
    /// Whether the title or text changed after posting.
    pub fn edited(&self) -> bool {
        self.updated_at > self.created_at
    }
}

/// `POST /admin/announcements`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateAnnouncement {
    pub title: String,
    /// Markdown.
    pub body: String,
    /// Whether writers may comment. Defaults to true.
    #[serde(default = "yes")]
    pub comments_open: bool,
}

fn yes() -> bool {
    true
}

/// `PATCH /admin/announcements/{id}`. Absent fields are unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateAnnouncement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comments_open: Option<bool>,
}

/// `GET /announcements` query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "openapi",
    derive(utoipa::IntoParams),
    into_params(parameter_in = Query)
)]
#[serde(rename_all = "camelCase")]
pub struct AnnouncementQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// One page of announcements, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AnnouncementPage {
    pub items: Vec<Announcement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

// ---- Comments ----------------------------------------------------------------

/// A writer's comment on an announcement: flat, under the announcement,
/// oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: Uuid,
    /// The announcement it's on.
    pub announcement: Uuid,
    /// The persona that wrote it.
    pub author: PromptAuthor,
    /// Markdown source.
    pub body: String,
    /// Sanitized HTML rendered by the server from `body`.
    pub body_html: String,
    /// Set only in its author's view (and administrators' inbox) once an
    /// administrator removed it.
    #[serde(default)]
    pub removed: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// `POST /announcements/{id}/comments`. Comments can't be edited, so a
/// reported comment stays as it was reported; the author can delete it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateComment {
    /// Persona (owned by the caller) the comment is written as.
    pub persona: Uuid,
    /// Markdown.
    pub body: String,
}

/// `GET /announcements/{id}/comments` query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "openapi",
    derive(utoipa::IntoParams),
    into_params(parameter_in = Query)
)]
#[serde(rename_all = "camelCase")]
pub struct CommentQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// One page of comments, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CommentPage {
    pub items: Vec<Comment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// `POST /comments/{id}/report`. Anyone may report, signed in or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ReportComment {
    pub reason: ReportReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

/// `POST /admin/comments/{id}/remove`: an administrator removes a comment
/// without a report. The author is told the reason's category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct RemoveComment {
    pub reason: ReportReason,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_are_open_unless_closed() {
        let a: CreateAnnouncement = serde_json::from_str(r#"{"title":"t","body":"b"}"#).unwrap();
        assert!(a.comments_open);
        let b: CreateAnnouncement =
            serde_json::from_str(r#"{"title":"t","body":"b","commentsOpen":false}"#).unwrap();
        assert!(!b.comments_open);
    }
}
