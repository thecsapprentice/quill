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
