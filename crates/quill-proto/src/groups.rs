//! MLS groups' message logs and device linking (`/api/v1`, from 2d).
//! MLS objects travel as base64url `MLSMessage`s; the server reads only
//! their clear headers.

use serde::{Deserialize, Serialize};

/// `POST /groups`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGroup {
    /// The MLS group id, base64url.
    pub group_id: String,
    /// `personal`. Session groups are registered by accepting a request.
    pub kind: String,
}

/// A group's place in its log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupState {
    pub group_id: String,
    /// The epoch the next commit must be for.
    pub epoch: u64,
    /// The last message's sequence number (0 if none).
    pub last_seq: u64,
}

/// `POST /groups/{id}/messages`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostMessage {
    /// An `MLSMessage` holding a `PrivateMessage`, base64url.
    pub message: String,
    /// Blobs the message carries (2i): they're kept as long as it is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blobs: Vec<uuid::Uuid>,
}

/// Where the server put a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Posted {
    pub seq: u64,
    /// The group's epoch after it (one higher than before for a commit).
    pub epoch: u64,
}

/// One logged message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoggedMessage {
    pub seq: u64,
    pub message: String,
}

/// `GET /groups/{id}/messages?after=&limit=`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Messages {
    pub messages: Vec<LoggedMessage>,
    /// More after the last one returned.
    pub more: bool,
}

/// `POST /link`: a new device's KeyPackage for the personal group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartLink {
    pub key_package: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkStarted {
    /// 128 random bits, base64url.
    pub link_id: String,
    pub expires_in_secs: u64,
}

/// `GET /link/{id}`. Once the reply has been fetched, the mailbox is gone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkMailbox {
    pub key_package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcome: Option<String>,
    /// Where the new device starts reading the personal group's log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_seq: Option<u64>,
}

/// `POST /link/{id}/reply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkReply {
    /// An `MLSMessage` holding a `Welcome`, base64url.
    pub welcome: String,
    pub from_seq: u64,
}
