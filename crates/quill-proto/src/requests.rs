//! Collaboration requests (`/api/v1`, from 2e). A request carries an MLS
//! `Welcome` and the encrypted pitch, both base64url `MLSMessage`s; the
//! server reads only the pitch's clear header.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// The most days a request can wait for an answer.
pub const MAX_EXPIRY_DAYS: u32 = 30;

/// `POST /requests`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SendRequest {
    /// The requesting persona (the caller's).
    pub persona: Uuid,
    pub prompt: Uuid,
    /// An `MLSMessage` holding the `Welcome`.
    pub welcome: String,
    /// An `MLSMessage` holding the pitch, a `PrivateMessage` in the group.
    pub pitch: String,
    /// Days to wait for an answer, at most [`MAX_EXPIRY_DAYS`] (the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_days: Option<u32>,
}

/// The answer to `POST /requests`: the same whatever the server did with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct RequestSent {
    pub id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

/// A request's status as one side sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub enum RequestStatus {
    /// Requester's view of anything undecided, declined or refused.
    Waiting,
    /// Either side: accepted; the group is now a session.
    Session,
    /// Requester's view after expiry.
    NoResponse,
    /// Author's view: in the queue.
    Pending,
    /// Author's view: arrived while the prompt was busy.
    Held,
    /// Author's view: declined.
    Declined,
    /// Author's view: expired.
    Expired,
}

/// `GET /requests/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct RequestState {
    pub id: Uuid,
    pub status: RequestStatus,
}

/// One of the login's own requests, in `GET /requests`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct OutgoingRequest {
    pub id: Uuid,
    /// Absent once the prompt is deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<Uuid>,
    /// The requesting persona.
    pub persona: Uuid,
    /// `Waiting`, `Session` or `NoResponse`.
    pub status: RequestStatus,
    pub group_id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

/// A persona as a request shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct RequestPersona {
    pub handle: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub actor_uri: String,
}

/// A request in the author's queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct IncomingRequest {
    pub id: Uuid,
    pub prompt: Uuid,
    pub requester: RequestPersona,
    pub group_id: String,
    pub welcome: String,
    pub pitch: String,
    #[serde(with = "time::serde::rfc3339")]
    pub received_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

/// `GET /prompts/{id}/requests?state=`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "openapi",
    derive(utoipa::IntoParams),
    into_params(parameter_in = Query)
)]
#[serde(rename_all = "camelCase")]
pub struct QueueQuery {
    /// `pending` (the default) or `held`. Reading held requests releases
    /// them to pending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// The author's queue for one prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    pub requests: Vec<IncomingRequest>,
    /// Held requests still waiting to be released.
    pub held: u32,
}
