//! Reads the clear header of an MLS `PrivateMessage` (RFC 9420, 6.3.1):
//! the three fields the delivery service needs to route and sequence it.
//! Everything else is ciphertext and stays that way.

use std::io::Read;

use tls_codec::{Deserialize, VLBytes};

use crate::key_package::MLS10;

const WIRE_FORMAT_PRIVATE_MESSAGE: u16 = 2;

/// What a message carries, as far as the header says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    Application,
    Proposal,
    Commit,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HeaderError {
    #[error("malformed message: {0}")]
    Malformed(String),
    #[error("not an MLS 1.0 PrivateMessage")]
    NotPrivateMessage,
}

impl From<tls_codec::Error> for HeaderError {
    fn from(e: tls_codec::Error) -> Self {
        Self::Malformed(e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub group_id: Vec<u8>,
    pub epoch: u64,
    pub content_type: ContentType,
}

/// Parses an `MLSMessage` holding a `PrivateMessage`. All bytes must be
/// used, and the ciphertext fields must be well formed (not checked
/// further: they're encrypted).
pub fn parse(message: &[u8]) -> Result<Header, HeaderError> {
    let mut r = message;
    if u16::tls_deserialize(&mut r)? != MLS10
        || u16::tls_deserialize(&mut r)? != WIRE_FORMAT_PRIVATE_MESSAGE
    {
        return Err(HeaderError::NotPrivateMessage);
    }
    let group_id: Vec<u8> = VLBytes::tls_deserialize(&mut r)?.into();
    let epoch = u64::tls_deserialize(&mut r)?;
    let content_type = match u8::tls_deserialize(&mut r)? {
        1 => ContentType::Application,
        2 => ContentType::Proposal,
        3 => ContentType::Commit,
        other => {
            return Err(HeaderError::Malformed(format!(
                "unknown content type {other}"
            )));
        }
    };
    for _ in 0..3 {
        // authenticated_data, encrypted_sender_data, ciphertext
        VLBytes::tls_deserialize(&mut r)?;
    }
    let mut rest = Vec::new();
    r.read_to_end(&mut rest)
        .map_err(|e| HeaderError::Malformed(e.to_string()))?;
    if !rest.is_empty() {
        return Err(HeaderError::Malformed("trailing bytes".into()));
    }
    if group_id.is_empty() {
        return Err(HeaderError::Malformed("empty group id".into()));
    }
    Ok(Header {
        group_id,
        epoch,
        content_type,
    })
}
