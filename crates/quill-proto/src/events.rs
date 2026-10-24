//! Live delivery (`GET /events`, from 2g): server-sent events telling a
//! login's clients that something changed. Notices carry no content:
//! clients fetch and decrypt as usual.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One notice. Sent as an SSE event named after its `type`, with this
/// JSON as the data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Event {
    /// A message was logged in one of the login's groups (a session or its
    /// personal device group).
    Message { group: String, seq: u64 },
    /// A request is waiting on one of the login's prompts.
    Request { prompt: Uuid },
    /// A session began, ended, or has a Welcome for one of the login's
    /// devices.
    Session { id: String, ended: bool },
    /// A device's KeyPackage pool ran low.
    KeyPackagesLow {
        persona: Uuid,
        device: Uuid,
        available: u32,
    },
    /// Notices were missed (the client fell behind): sync everything.
    Resync,
}

impl Event {
    /// The SSE event name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Message { .. } => "message",
            Self::Request { .. } => "request",
            Self::Session { .. } => "session",
            Self::KeyPackagesLow { .. } => "keyPackagesLow",
            Self::Resync => "resync",
        }
    }
}
