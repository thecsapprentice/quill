//! Request and response bodies for `/api/v1`.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// `GET /api/v1/instance`: what this server is and what it allows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct InstanceInfo {
    pub domain: String,
    pub software: String,
    pub software_version: String,
    pub protocol_versions: Vec<String>,
    pub registration_open: bool,
    pub email_recovery: bool,
    /// Whether writers may create new tags (perhaps pending approval).
    pub tag_policy: crate::board::TagPolicy,
    /// Maturity cap for anonymous visitors and new accounts.
    pub default_maturity_max: crate::board::Maturity,
    pub limits: Limits,
    /// Whether sessions here take attachments: the administrators' choice.
    #[serde(default = "default_attachments")]
    pub attachments: bool,
}

fn default_attachments() -> bool {
    true
}

/// Limits a client must respect. In a cross-server session a client uses the
/// smaller value of each pair of servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub max_message_bytes: u64,
    pub max_attachment_bytes: u64,
    pub max_envelope_bytes: u64,
    pub request_expiry_days: u32,
    /// How long the server keeps ciphertext, read or not; `None` (`null`)
    /// when it keeps it until every device has read it (2k).
    #[serde(default = "default_retention")]
    pub retention_days: Option<u32>,
    /// Blobs a login may store at once, in bytes.
    #[serde(default = "default_quota")]
    pub account_quota_bytes: u64,
    #[serde(default = "default_rate")]
    pub messages_per_minute: u32,
    /// The largest backup a login may keep.
    #[serde(default = "default_backup")]
    pub max_backup_bytes: u64,
}

fn default_retention() -> Option<u32> {
    Some(crate::limits::DEFAULT_RETENTION_DAYS)
}

fn default_backup() -> u64 {
    crate::limits::DEFAULT_MAX_BACKUP_BYTES
}

fn default_quota() -> u64 {
    crate::limits::DEFAULT_ACCOUNT_QUOTA_BYTES
}

fn default_rate() -> u32 {
    crate::limits::DEFAULT_MESSAGES_PER_MINUTE
}

impl Default for Limits {
    fn default() -> Self {
        use crate::limits::*;
        Self {
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
            max_attachment_bytes: DEFAULT_MAX_ATTACHMENT_BYTES,
            max_envelope_bytes: DEFAULT_MAX_ENVELOPE_BYTES,
            request_expiry_days: DEFAULT_REQUEST_EXPIRY_DAYS,
            retention_days: Some(DEFAULT_RETENTION_DAYS),
            account_quota_bytes: DEFAULT_ACCOUNT_QUOTA_BYTES,
            messages_per_minute: DEFAULT_MESSAGES_PER_MINUTE,
            max_backup_bytes: DEFAULT_MAX_BACKUP_BYTES,
        }
    }
}

/// `POST /api/v1/auth/register` and `POST /api/v1/auth/login`.
///
/// The login name is private to the account and never federated; public
/// identities are personas.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Returned by register and login. The token is shown once and stored only
/// as a hash on the server.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TokenResponse {
    pub token: String,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenResponse")
            .field("token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// A persona: one public ActivityPub actor owned by an account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct Persona {
    pub id: Uuid,
    pub handle: String,
    /// Permanent identifier, signed by device certificates.
    #[serde(default)]
    pub actor_uri: String,
    pub display_name: Option<String>,
    pub summary: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// `POST /api/v1/personas`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreatePersona {
    pub handle: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
}

/// `PATCH /api/v1/personas/{id}`. Absent fields are unchanged; an empty string
/// clears the field.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdatePersona {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// RFC 9457 problem details, used for every error response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Problem {
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Media type for [`Problem`] responses.
pub const PROBLEM_CONTENT_TYPE: &str = "application/problem+json";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_debug_hides_password() {
        let c = Credentials {
            username: "inkfox".into(),
            password: "correct horse battery staple".into(),
        };
        let shown = format!("{c:?}");
        assert!(!shown.contains("battery"));
        assert!(shown.contains("inkfox"));
    }

    #[test]
    fn instance_info_uses_camel_case() {
        let info = InstanceInfo {
            domain: "alpha.test".into(),
            software: "quill".into(),
            software_version: "0.1.0".into(),
            protocol_versions: vec!["0.1".into()],
            registration_open: true,
            email_recovery: false,
            tag_policy: crate::board::TagPolicy::Open,
            default_maturity_max: crate::board::Maturity::Teen,
            limits: Limits::default(),
            attachments: true,
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["limits"]["maxMessageBytes"], 65536);
        assert_eq!(json["protocolVersions"][0], "0.1");
    }
}
