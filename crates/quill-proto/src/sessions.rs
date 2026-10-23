//! Sessions (`/api/v1`, from 2f): accepted requests' MLS groups. A
//! session's id is its group id, base64url; its log is the group's
//! (`/groups/{id}/messages`).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A persona taking part in a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMember {
    pub handle: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub actor_uri: String,
    /// Set when the persona is the caller's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<Uuid>,
}

/// A Welcome left for one of the caller's devices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceWelcome {
    /// The persona device (directory id) it's for.
    pub device: Uuid,
    /// An `MLSMessage` holding the Welcome, base64url.
    pub welcome: String,
    /// Where the device starts reading the log.
    pub from_seq: u64,
}

/// One of the login's sessions, in `GET /sessions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// The group id, base64url.
    pub id: String,
    /// The request it began as.
    pub request: Uuid,
    /// Absent once the prompt is gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<Uuid>,
    /// Its personas still taking part.
    pub members: Vec<SessionMember>,
    /// The epoch the next commit must be for.
    pub epoch: u64,
    pub last_seq: u64,
    pub ended: bool,
    /// The request's Welcome, for devices that were in it.
    pub welcome: String,
    /// Welcomes left for the caller's devices added since.
    #[serde(default)]
    pub device_welcomes: Vec<DeviceWelcome>,
}

/// `POST /sessions/{id}/welcomes`: a Welcome for one of the caller's
/// persona's devices, just added to the group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaveWelcome {
    pub device: Uuid,
    pub welcome: String,
    pub from_seq: u64,
}

/// `POST /sessions/{id}/ack`: how far one of the caller's devices has read
/// the session's log. Only the server sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ack {
    pub device: Uuid,
    pub seq: u64,
}

/// The answer to `POST /sessions/{id}/blobs` (2i).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobUploaded {
    pub id: Uuid,
    /// The capability token for fetching it, base64url. The server keeps
    /// only its hash.
    pub token: String,
}

/// The header carrying a blob's capability token on `GET /blobs/{id}`.
pub const BLOB_TOKEN_HEADER: &str = "blob-token";

/// One session's share of the server's storage (2i). Sizes are of
/// ciphertext: nothing here says what was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStorage {
    /// The session id (group id, base64url).
    pub id: String,
    /// The caller's persona and the other writer's, in a login's own view;
    /// absent in an administrator's, which doesn't link personas to logins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partner: Option<String>,
    pub ended: bool,
    /// Messages still stored, and their bytes.
    pub messages: u64,
    pub message_bytes: u64,
    /// Attachments still stored, and their (padded) bytes.
    pub files: u64,
    pub file_bytes: u64,
    /// The part of `file_bytes` this login uploaded (it counts against its
    /// quota).
    pub own_file_bytes: u64,
}

/// `GET /account/storage`, and an administrator's view of one login
/// (`GET /admin/storage/{account}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStorage {
    /// Everything stored for the login's sessions and device group.
    pub total_bytes: u64,
    /// Attachments the login uploaded, against its quota.
    pub quota_used_bytes: u64,
    pub quota_bytes: u64,
    /// The personal device group's log.
    pub device_sync_bytes: u64,
    /// The login's encrypted backup (2j), counted apart from the quota.
    #[serde(default)]
    pub backup_bytes: u64,
    pub sessions: Vec<SessionStorage>,
}

/// One login in `GET /admin/storage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStorageSummary {
    pub account: Uuid,
    /// The private login name, as administrators see it elsewhere.
    pub login: String,
    pub sessions: u64,
    /// Messages in its sessions (shared sessions count for both logins).
    pub message_bytes: u64,
    /// Attachments it uploaded.
    pub file_bytes: u64,
    pub device_sync_bytes: u64,
    pub quota_bytes: u64,
}

/// `GET /admin/storage`: every login with anything stored, largest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStorage {
    /// Everything stored, counted once.
    pub total_bytes: u64,
    pub message_bytes: u64,
    pub file_bytes: u64,
    pub accounts: Vec<AccountStorageSummary>,
}

/// The login's committed backup (`GET /backup`, 2j).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub generation: Uuid,
    pub parts: u32,
    pub bytes: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
}

/// `POST /backup/{generation}/commit`: the new backup is complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitBackup {
    pub parts: u32,
}
