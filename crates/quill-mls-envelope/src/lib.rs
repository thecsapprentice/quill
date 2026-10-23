//! MLS framing-header reader for the delivery service.
//!
//! The server needs three cleartext fields from each `PrivateMessage` to route
//! and sequence it: `group_id`, `epoch` and `content_type` (RFC 9420,
//! section 6.3). This crate will parse exactly those, and nothing else.
//!
//! It also reads the public structure of a KeyPackage ([`key_package`]),
//! so the server can check an upload belongs to the device it's filed
//! under. It verifies no signatures: clients do that.
//!
//! Contract (enforced by `scripts/check-server-deps.sh`): this crate must never
//! depend on an MLS crypto provider, an HPKE implementation, or `quill-core`.
//! It is implemented in Phase 2.

/// The MLS wire formats (RFC 9420, section 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireFormat {
    PublicMessage,
    PrivateMessage,
    Welcome,
    GroupInfo,
    KeyPackage,
}

impl WireFormat {
    /// Maps the 16-bit `WireFormat` code point to a known format.
    pub fn from_code(code: u16) -> Option<Self> {
        match code {
            0x0001 => Some(Self::PublicMessage),
            0x0002 => Some(Self::PrivateMessage),
            0x0003 => Some(Self::Welcome),
            0x0004 => Some(Self::GroupInfo),
            0x0005 => Some(Self::KeyPackage),
            _ => None,
        }
    }
}

pub mod key_package;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_codes() {
        assert_eq!(WireFormat::from_code(2), Some(WireFormat::PrivateMessage));
        assert_eq!(WireFormat::from_code(0), None);
        assert_eq!(WireFormat::from_code(0xF000), None);
    }
}
