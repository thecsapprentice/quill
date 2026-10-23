//! Device certificates and revocations (protocol draft, section 6).
//!
//! A persona's **identity key** certifies each of its **device keys**; a
//! device key is the MLS signature key of that device's leaves (from 2c).
//! Both structures are TLS-encoded like MLS itself and signed with MLS's
//! `SignWithLabel` (RFC 9420, section 5.1.2) under project labels, so a
//! signature made for one purpose can't be replayed as another.
//!
//! Verification is strict: lengths must be minimal (tls_codec's `mls`
//! feature), there may be no trailing bytes, unknown flag bits are refused,
//! keys must be valid and not of small order, and signatures are checked
//! with Ed25519's strict verification.
//!
//! The server links this crate to check what clients upload; only clients
//! enable the `sign` feature to issue certificates and revocations.

#![forbid(unsafe_code)]

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use tls_codec::{
    Deserialize as _, Serialize as _, TlsDeserialize, TlsSerialize, TlsSize, VLByteSlice, VLBytes,
};

/// Label for device certificates (signed as `"MLS 1.0 wr device cert v1"`).
pub const CERT_LABEL: &str = "wr device cert v1";
/// Label for device revocations.
pub const REVOKE_LABEL: &str = "wr device revoke v1";
/// Label of the signature on a rejoin request (protocol draft, section 6).
pub const REJOIN_LABEL: &str = "wr rejoin v1";
/// Prefix MLS's `SignWithLabel` puts before every label.
const MLS_PREFIX: &[u8] = b"MLS 1.0 ";

/// How far in the future a certificate's start may be, for clock skew.
pub const MAX_CLOCK_SKEW_MS: u64 = 10 * 60 * 1000;

/// Bit 0 of a certificate's flags: a limited device (no identity key).
const FLAG_LIMITED: u8 = 0x01;

/// Length of an Ed25519 public key.
pub const KEY_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CertError {
    #[error("malformed: {0}")]
    Malformed(String),
    #[error("signature does not verify")]
    BadSignature,
    #[error("not a valid Ed25519 public key")]
    BadKey,
    #[error("issued for a different actor")]
    WrongActor,
    #[error("certifies a different device key")]
    WrongDeviceKey,
    #[error("signed by a different identity key version")]
    WrongKeyVersion,
    #[error("unknown flag bits")]
    UnknownFlags,
    #[error("unknown revocation reason")]
    UnknownReason,
    #[error("not valid yet")]
    NotYetValid,
    #[error("expired")]
    Expired,
}

impl From<tls_codec::Error> for CertError {
    fn from(e: tls_codec::Error) -> Self {
        Self::Malformed(e.to_string())
    }
}

#[derive(TlsSerialize, TlsSize)]
struct SignContent<'a> {
    label: VLByteSlice<'a>,
    content: VLByteSlice<'a>,
}

/// The bytes `SignWithLabel(key, label, content)` signs: a TLS-encoded
/// `SignContent` with the label prefixed by `"MLS 1.0 "`.
pub fn sign_content(label: &str, content: &[u8]) -> Result<Vec<u8>, CertError> {
    let mut full = MLS_PREFIX.to_vec();
    full.extend_from_slice(label.as_bytes());
    Ok(SignContent {
        label: VLByteSlice(&full),
        content: VLByteSlice(content),
    }
    .tls_serialize_detached()?)
}

/// A public key that is a valid curve point and not of small order.
pub fn public_key(bytes: &[u8]) -> Result<VerifyingKey, CertError> {
    let array: [u8; KEY_LEN] = bytes.try_into().map_err(|_| CertError::BadKey)?;
    let key = VerifyingKey::from_bytes(&array).map_err(|_| CertError::BadKey)?;
    if key.is_weak() {
        return Err(CertError::BadKey);
    }
    Ok(key)
}

/// `VerifyWithLabel`, with Ed25519 strict verification.
pub fn verify_with_label(
    key: &[u8],
    label: &str,
    content: &[u8],
    signature: &[u8],
) -> Result<(), CertError> {
    let key = public_key(key)?;
    let signature = Signature::from_slice(signature).map_err(|_| CertError::BadSignature)?;
    key.verify_strict(&sign_content(label, content)?, &signature)
        .map_err(|_| CertError::BadSignature)
}

/// `SignWithLabel` with an Ed25519 key.
#[cfg(feature = "sign")]
pub fn sign_with_label(
    key: &ed25519_dalek::SigningKey,
    label: &str,
    content: &[u8],
) -> Result<[u8; 64], CertError> {
    use ed25519_dalek::Signer;
    Ok(key.sign(&sign_content(label, content)?).to_bytes())
}

/// A short, human-comparable fingerprint of an identity key: the first 16
/// bytes of SHA-256 over a label and the key, as 8 groups of 4 hex digits.
pub fn fingerprint(key: &[u8]) -> String {
    let digest = Sha256::new()
        .chain_update(b"wr identity fingerprint v1")
        .chain_update(key)
        .finalize();
    digest[..16]
        .chunks(2)
        .map(|c| format!("{:02x}{:02x}", c[0], c[1]))
        .collect::<Vec<_>>()
        .join(" ")
}

fn key_array(bytes: &[u8]) -> Result<[u8; KEY_LEN], CertError> {
    bytes
        .try_into()
        .map_err(|_| CertError::Malformed("key must be 32 bytes".into()))
}

fn utf8(bytes: &[u8]) -> Result<String, CertError> {
    String::from_utf8(bytes.to_vec())
        .map_err(|_| CertError::Malformed("actor URI is not UTF-8".into()))
}

// ---- Device certificate -----------------------------------------------------

#[derive(Clone, TlsSerialize, TlsDeserialize, TlsSize)]
struct CertTbsWire {
    actor_uri: VLBytes,
    device_key: VLBytes,
    identity_key_version: u32,
    flags: u8,
    not_before: u64,
    not_after: u64,
}

#[derive(TlsSerialize, TlsDeserialize, TlsSize)]
struct CertWire {
    tbs: CertTbsWire,
    signature: VLBytes,
}

/// A device certificate's contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCertificate {
    /// The persona's actor URI, exactly as signed.
    pub actor_uri: String,
    /// The device's Ed25519 public key.
    pub device_key: [u8; KEY_LEN],
    /// Which version of the persona's identity key signed it.
    pub identity_key_version: u32,
    /// A limited device holds no identity key.
    pub limited: bool,
    /// Milliseconds since the Unix epoch.
    pub not_before: u64,
    /// `None` means no expiry.
    pub not_after: Option<u64>,
}

/// What a certificate must match to be accepted.
#[derive(Debug, Clone, Copy)]
pub struct Expected<'a> {
    pub actor_uri: &'a str,
    pub identity_key: &'a [u8],
    pub identity_key_version: u32,
    /// The leaf's signature key, when checking a certificate for a leaf.
    pub device_key: Option<&'a [u8]>,
    /// Now, in milliseconds since the Unix epoch.
    pub now_ms: u64,
}

impl DeviceCertificate {
    #[cfg(any(feature = "sign", test))]
    fn tbs(&self) -> CertTbsWire {
        CertTbsWire {
            actor_uri: self.actor_uri.as_bytes().into(),
            device_key: self.device_key.as_slice().into(),
            identity_key_version: self.identity_key_version,
            flags: if self.limited { FLAG_LIMITED } else { 0 },
            not_before: self.not_before,
            not_after: self.not_after.unwrap_or(0),
        }
    }

    /// Signs the certificate with the persona's identity key.
    #[cfg(feature = "sign")]
    pub fn issue(&self, identity: &ed25519_dalek::SigningKey) -> Result<Vec<u8>, CertError> {
        let tbs = self.tbs();
        let signature = sign_with_label(identity, CERT_LABEL, &tbs.tls_serialize_detached()?)?;
        Ok(CertWire {
            tbs,
            signature: signature.as_slice().into(),
        }
        .tls_serialize_detached()?)
    }

    /// Decodes without checking the signature: for display only.
    pub fn decode(bytes: &[u8]) -> Result<Self, CertError> {
        Ok(Self::decode_wire(bytes)?.0)
    }

    fn decode_wire(bytes: &[u8]) -> Result<(Self, CertWire), CertError> {
        let wire = CertWire::tls_deserialize_exact(bytes)?;
        let t = &wire.tbs;
        if t.flags & !FLAG_LIMITED != 0 {
            return Err(CertError::UnknownFlags);
        }
        if t.not_after != 0 && t.not_after <= t.not_before {
            return Err(CertError::Malformed("ends before it starts".into()));
        }
        let cert = Self {
            actor_uri: utf8(t.actor_uri.as_slice())?,
            device_key: key_array(t.device_key.as_slice())?,
            identity_key_version: t.identity_key_version,
            limited: t.flags & FLAG_LIMITED != 0,
            not_before: t.not_before,
            not_after: (t.not_after != 0).then_some(t.not_after),
        };
        Ok((cert, wire))
    }

    /// Checks a certificate: its signature under the identity key, then
    /// that it is for this actor, device and key version, and in date.
    pub fn verify(bytes: &[u8], expected: &Expected<'_>) -> Result<Self, CertError> {
        let (cert, wire) = Self::decode_wire(bytes)?;
        verify_with_label(
            expected.identity_key,
            CERT_LABEL,
            &wire.tbs.tls_serialize_detached()?,
            wire.signature.as_slice(),
        )?;
        public_key(&cert.device_key)?;
        if cert.actor_uri != expected.actor_uri {
            return Err(CertError::WrongActor);
        }
        if cert.identity_key_version != expected.identity_key_version {
            return Err(CertError::WrongKeyVersion);
        }
        if let Some(leaf) = expected.device_key
            && leaf != cert.device_key
        {
            return Err(CertError::WrongDeviceKey);
        }
        if cert.not_before > expected.now_ms.saturating_add(MAX_CLOCK_SKEW_MS) {
            return Err(CertError::NotYetValid);
        }
        if cert.not_after.is_some_and(|end| expected.now_ms >= end) {
            return Err(CertError::Expired);
        }
        Ok(cert)
    }
}

// ---- Device revocation -------------------------------------------------------

/// Why a device was revoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevocationReason {
    Unspecified,
    Lost,
    Compromised,
    Retired,
}

impl RevocationReason {
    pub fn code(self) -> u8 {
        match self {
            Self::Unspecified => 0,
            Self::Lost => 1,
            Self::Compromised => 2,
            Self::Retired => 3,
        }
    }

    pub fn from_code(code: u8) -> Result<Self, CertError> {
        Ok(match code {
            0 => Self::Unspecified,
            1 => Self::Lost,
            2 => Self::Compromised,
            3 => Self::Retired,
            _ => return Err(CertError::UnknownReason),
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::Lost => "lost",
            Self::Compromised => "compromised",
            Self::Retired => "retired",
        }
    }
}

impl std::str::FromStr for RevocationReason {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "unspecified" => Ok(Self::Unspecified),
            "lost" => Ok(Self::Lost),
            "compromised" => Ok(Self::Compromised),
            "retired" => Ok(Self::Retired),
            _ => Err(format!(
                "unknown reason {s:?}: use lost, compromised, retired or unspecified"
            )),
        }
    }
}

#[derive(TlsSerialize, TlsDeserialize, TlsSize)]
struct RevokeTbsWire {
    actor_uri: VLBytes,
    device_key: VLBytes,
    revoked_at: u64,
    reason: u8,
}

#[derive(TlsSerialize, TlsDeserialize, TlsSize)]
struct RevokeWire {
    tbs: RevokeTbsWire,
    signature: VLBytes,
}

/// A device revocation's contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRevocation {
    pub actor_uri: String,
    pub device_key: [u8; KEY_LEN],
    /// Milliseconds since the Unix epoch.
    pub revoked_at: u64,
    pub reason: RevocationReason,
}

impl DeviceRevocation {
    #[cfg(any(feature = "sign", test))]
    fn tbs(&self) -> RevokeTbsWire {
        RevokeTbsWire {
            actor_uri: self.actor_uri.as_bytes().into(),
            device_key: self.device_key.as_slice().into(),
            revoked_at: self.revoked_at,
            reason: self.reason.code(),
        }
    }

    /// Signs the revocation with the persona's identity key.
    #[cfg(feature = "sign")]
    pub fn issue(&self, identity: &ed25519_dalek::SigningKey) -> Result<Vec<u8>, CertError> {
        let tbs = self.tbs();
        let signature = sign_with_label(identity, REVOKE_LABEL, &tbs.tls_serialize_detached()?)?;
        Ok(RevokeWire {
            tbs,
            signature: signature.as_slice().into(),
        }
        .tls_serialize_detached()?)
    }

    /// Checks a revocation's signature under the identity key and that it is
    /// for this actor.
    pub fn verify(bytes: &[u8], actor_uri: &str, identity_key: &[u8]) -> Result<Self, CertError> {
        let wire = RevokeWire::tls_deserialize_exact(bytes)?;
        let t = &wire.tbs;
        let reason = RevocationReason::from_code(t.reason)?;
        let revocation = Self {
            actor_uri: utf8(t.actor_uri.as_slice())?,
            device_key: key_array(t.device_key.as_slice())?,
            revoked_at: t.revoked_at,
            reason,
        };
        verify_with_label(
            identity_key,
            REVOKE_LABEL,
            &t.tls_serialize_detached()?,
            wire.signature.as_slice(),
        )?;
        if revocation.actor_uri != actor_uri {
            return Err(CertError::WrongActor);
        }
        Ok(revocation)
    }
}

// ---- Rejoin requests ---------------------------------------------------------

#[derive(TlsSerialize, TlsDeserialize, TlsSize)]
struct RejoinTbsWire {
    actor_uri: VLBytes,
    group_id: VLBytes,
    key_package: VLBytes,
    certificate: VLBytes,
    remove_keys: Vec<VLBytes>,
    created_at: u64,
}

#[derive(TlsSerialize, TlsDeserialize, TlsSize)]
struct RejoinWire {
    tbs: RejoinTbsWire,
    signature: VLBytes,
}

/// A restored device asking to be let back into a session: signed with the
/// persona's identity key, so the other writer's client can add a device
/// that isn't its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejoinRequest {
    pub actor_uri: String,
    pub group_id: Vec<u8>,
    /// The restored device's KeyPackage (an `MLSMessage`).
    pub key_package: Vec<u8>,
    /// Its device certificate.
    pub certificate: Vec<u8>,
    /// Device keys of the persona's lost devices, to remove from the group.
    pub remove_keys: Vec<[u8; KEY_LEN]>,
    /// Milliseconds since the Unix epoch.
    pub created_at: u64,
}

/// The most devices a rejoin request can ask to remove.
pub const MAX_REJOIN_REMOVALS: usize = 64;

impl RejoinRequest {
    #[cfg(any(feature = "sign", test))]
    fn tbs(&self) -> RejoinTbsWire {
        RejoinTbsWire {
            actor_uri: self.actor_uri.as_bytes().into(),
            group_id: self.group_id.as_slice().into(),
            key_package: self.key_package.as_slice().into(),
            certificate: self.certificate.as_slice().into(),
            remove_keys: self
                .remove_keys
                .iter()
                .map(|k| k.as_slice().into())
                .collect(),
            created_at: self.created_at,
        }
    }

    /// Signs the request with the persona's identity key.
    #[cfg(feature = "sign")]
    pub fn issue(&self, identity: &ed25519_dalek::SigningKey) -> Result<Vec<u8>, CertError> {
        if self.remove_keys.len() > MAX_REJOIN_REMOVALS {
            return Err(CertError::Malformed("too many devices to remove".into()));
        }
        let tbs = self.tbs();
        let signature = sign_with_label(identity, REJOIN_LABEL, &tbs.tls_serialize_detached()?)?;
        Ok(RejoinWire {
            tbs,
            signature: signature.as_slice().into(),
        }
        .tls_serialize_detached()?)
    }

    /// Checks a request's signature under the identity key, and that it's
    /// from this actor for this group.
    pub fn verify(
        bytes: &[u8],
        actor_uri: &str,
        group_id: &[u8],
        identity_key: &[u8],
    ) -> Result<Self, CertError> {
        let wire = RejoinWire::tls_deserialize_exact(bytes)?;
        let t = &wire.tbs;
        if t.remove_keys.len() > MAX_REJOIN_REMOVALS {
            return Err(CertError::Malformed("too many devices to remove".into()));
        }
        let request = Self {
            actor_uri: utf8(t.actor_uri.as_slice())?,
            group_id: t.group_id.as_slice().to_vec(),
            key_package: t.key_package.as_slice().to_vec(),
            certificate: t.certificate.as_slice().to_vec(),
            remove_keys: t
                .remove_keys
                .iter()
                .map(|k| key_array(k.as_slice()))
                .collect::<Result<_, _>>()?,
            created_at: t.created_at,
        };
        verify_with_label(
            identity_key,
            REJOIN_LABEL,
            &t.tls_serialize_detached()?,
            wire.signature.as_slice(),
        )?;
        if request.actor_uri != actor_uri {
            return Err(CertError::WrongActor);
        }
        if request.group_id != group_id {
            return Err(CertError::Malformed(
                "the request is for another group".into(),
            ));
        }
        Ok(request)
    }
}

#[cfg(test)]
mod vectors;
