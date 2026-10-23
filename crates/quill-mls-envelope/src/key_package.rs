//! Reads the public fields of an MLS KeyPackage (RFC 9420, sections 10 and
//! 7.2), wrapped as an `MLSMessage`, without any cryptography.
//!
//! The server uses this to file uploaded packages under the right device
//! and to drop expired ones. A package that parses here may still have bad
//! signatures; claimers' clients check those.

use std::io::Read;

use tls_codec::{Deserialize, VLBytes};

/// MLS 1.0.
pub const MLS10: u16 = 1;
/// `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`.
pub const CIPHERSUITE_X25519_AES128GCM_ED25519: u16 = 1;
const WIRE_FORMAT_KEY_PACKAGE: u16 = 5;
const CREDENTIAL_BASIC: u16 = 1;
const LEAF_SOURCE_KEY_PACKAGE: u8 = 1;
const EXTENSION_LAST_RESORT: u16 = 0x000A;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyPackageError {
    #[error("malformed KeyPackage: {0}")]
    Malformed(String),
    #[error("not an MLS 1.0 KeyPackage")]
    NotKeyPackage,
    #[error("unsupported credential type")]
    UnsupportedCredential,
}

impl From<tls_codec::Error> for KeyPackageError {
    fn from(e: tls_codec::Error) -> Self {
        Self::Malformed(e.to_string())
    }
}

/// The public fields the server needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPackageInfo {
    pub cipher_suite: u16,
    /// The leaf's signature key.
    pub signature_key: Vec<u8>,
    /// The basic credential's identity.
    pub identity: Vec<u8>,
    /// Seconds since the Unix epoch.
    pub not_before: u64,
    pub not_after: u64,
    /// The leaf's extension types, in order.
    pub leaf_extensions: Vec<u16>,
    /// Whether the package carries the last-resort extension.
    pub last_resort: bool,
}

fn u8_(r: &mut &[u8]) -> Result<u8, KeyPackageError> {
    Ok(u8::tls_deserialize(r)?)
}
fn u16_(r: &mut &[u8]) -> Result<u16, KeyPackageError> {
    Ok(u16::tls_deserialize(r)?)
}
fn u64_(r: &mut &[u8]) -> Result<u64, KeyPackageError> {
    Ok(u64::tls_deserialize(r)?)
}
fn bytes(r: &mut &[u8]) -> Result<Vec<u8>, KeyPackageError> {
    Ok(VLBytes::tls_deserialize(r)?.into())
}

/// Extension types in an `Extension extensions<V>` list.
fn extension_types(r: &mut &[u8]) -> Result<Vec<u16>, KeyPackageError> {
    let list = bytes(r)?;
    let mut inner = list.as_slice();
    let mut types = Vec::new();
    while !inner.is_empty() {
        types.push(u16_(&mut inner)?);
        bytes(&mut inner)?;
    }
    Ok(types)
}

/// Parses an `MLSMessage` holding a KeyPackage. All bytes must be used.
pub fn parse(message: &[u8]) -> Result<KeyPackageInfo, KeyPackageError> {
    let mut r = message;
    if u16_(&mut r)? != MLS10 || u16_(&mut r)? != WIRE_FORMAT_KEY_PACKAGE {
        return Err(KeyPackageError::NotKeyPackage);
    }
    // KeyPackage
    if u16_(&mut r)? != MLS10 {
        return Err(KeyPackageError::NotKeyPackage);
    }
    let cipher_suite = u16_(&mut r)?;
    let _init_key = bytes(&mut r)?;
    // LeafNode
    let _encryption_key = bytes(&mut r)?;
    let signature_key = bytes(&mut r)?;
    if u16_(&mut r)? != CREDENTIAL_BASIC {
        return Err(KeyPackageError::UnsupportedCredential);
    }
    let identity = bytes(&mut r)?;
    // Capabilities: versions, cipher suites, extensions, proposals, credentials.
    for _ in 0..5 {
        bytes(&mut r)?;
    }
    if u8_(&mut r)? != LEAF_SOURCE_KEY_PACKAGE {
        return Err(KeyPackageError::Malformed(
            "leaf node is not from a KeyPackage".into(),
        ));
    }
    let not_before = u64_(&mut r)?;
    let not_after = u64_(&mut r)?;
    let leaf_extensions = extension_types(&mut r)?;
    let _leaf_signature = bytes(&mut r)?;
    // KeyPackage extensions and signature.
    let package_extensions = extension_types(&mut r)?;
    let _signature = bytes(&mut r)?;
    let mut rest = Vec::new();
    r.read_to_end(&mut rest)
        .map_err(|e| KeyPackageError::Malformed(e.to_string()))?;
    if !rest.is_empty() {
        return Err(KeyPackageError::Malformed("trailing bytes".into()));
    }
    Ok(KeyPackageInfo {
        cipher_suite,
        signature_key,
        identity,
        not_before,
        not_after,
        leaf_extensions,
        last_resort: package_extensions.contains(&EXTENSION_LAST_RESORT),
    })
}
