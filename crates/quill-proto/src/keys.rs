//! Identity keys, devices and the public key directory (`/api/v1`).
//!
//! Keys and certificates travel as base64url without padding (protocol
//! draft, section 1). This module only moves bytes; checking them is
//! `quill-certs`' job.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Encodes bytes as base64url without padding.
pub fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Decodes base64url without padding.
pub fn unb64(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text).ok()
}

/// A persona's permanent actor URI, which certificates sign.
pub fn actor_uri(domain: &str, handle: &str) -> String {
    format!("https://{domain}/users/{handle}")
}

/// `PUT /personas/{id}/identity-key`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetIdentityKey {
    /// Ed25519 public key, base64url.
    pub public_key: String,
    /// 1 for the first key; each replacement must be higher.
    pub key_version: u32,
}

/// `POST /personas/{id}/devices`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterDevice {
    /// A `DeviceCertificate`, base64url.
    pub certificate: String,
}

/// `POST /personas/{id}/devices/{device}/revoke`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeDevice {
    /// A `DeviceRevocation`, base64url.
    pub revocation: String,
}

/// One of a persona's devices, as the directory publishes it. There are
/// deliberately no timestamps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    /// Random; not ordered by time.
    pub id: Uuid,
    /// Ed25519 public key, base64url.
    pub device_key: String,
    /// The identity key version its certificate is signed with. Lower than
    /// the persona's current version means it needs certifying again.
    pub key_version: u32,
    pub limited: bool,
    /// `DeviceCertificate`, base64url.
    pub certificate: String,
    /// `DeviceRevocation`, base64url, once revoked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revocation: Option<String>,
}

/// `GET /actors/{handle}/keys` (public) and the owner's views.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonaKeys {
    pub actor_uri: String,
    pub handle: String,
    /// Absent until the persona's first client sets it up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_version: Option<u32>,
    #[serde(default)]
    pub devices: Vec<DeviceInfo>,
}

/// `POST /personas/{id}/devices/{device}/key-packages`: KeyPackages as
/// `MLSMessage`s, base64url.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadKeyPackages {
    /// One-time packages, added to the pool.
    #[serde(default)]
    pub key_packages: Vec<String>,
    /// A last-resort package, replacing the device's current one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_resort: Option<String>,
}

/// What's left in a device's pool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyPackageStatus {
    /// Unexpired one-time packages.
    pub available: u32,
    /// When the last-resort package expires (seconds since the Unix epoch),
    /// if there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_resort_until: Option<u64>,
}

/// One device's package in a claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimedKeyPackage {
    pub device: Uuid,
    /// `MLSMessage` holding a KeyPackage, base64url.
    pub key_package: String,
    /// The device's last-resort package (its one-time ones ran out).
    pub last_resort: bool,
}

/// `POST /actors/{handle}/key-packages/claim?device=`: limits the claim to
/// one device (2f, adding a device to a session).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<Uuid>,
}

/// `POST /actors/{handle}/key-packages/claim`: one package per current
/// device of the persona.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyPackageClaim {
    pub actor_uri: String,
    pub handle: String,
    pub packages: Vec<ClaimedKeyPackage>,
}
