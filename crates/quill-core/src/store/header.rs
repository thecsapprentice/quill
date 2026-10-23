//! The store file's header: which key unlocks it, and the data key wrapped
//! under that key.
//!
//! The header occupies the first [`HEADER_REGION`] bytes of the file and is
//! kept twice, in two slots. Every update writes both slots, one after the
//! other with a sync between, so a crash mid-update leaves at least one
//! intact slot. Each slot carries a SHA-256 checksum to tell a torn slot from
//! a wrong key, and a sequence number to tell the newer slot.
//!
//! Slot layout (big-endian integers):
//!
//! | Bytes | Field |
//! | --- | --- |
//! | 8 | magic `QUILLSDB` |
//! | 4 | format version (1) |
//! | 16 | file id (random; binds pages to this file) |
//! | 8 | sequence number |
//! | 1 | key source: 1 keychain, 2 passphrase |
//! | 12 | Argon2id memory (KiB), iterations, parallelism (passphrase only) |
//! | 16 | Argon2id salt (passphrase only) |
//! | 24 + 32 + 16 | XChaCha20-Poly1305 nonce, wrapped data key, tag |
//! | 32 | SHA-256 of all the above |
//!
//! The wrapped data key's associated data is every byte before the nonce,
//! so changing any field breaks the unwrap.

use chacha20poly1305::{AeadInOut, KeyInit, Tag, XChaCha20Poly1305, XNonce};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::StoreError;

/// Bytes reserved at the start of the file for the header.
pub(crate) const HEADER_REGION: u64 = 4096;
const SLOT_OFFSETS: [u64; 2] = [0, 2048];

const MAGIC: &[u8; 8] = b"QUILLSDB";
const VERSION: u32 = 1;
/// Bytes covered by the wrapped key's associated data.
const AAD_LEN: usize = 8 + 4 + 16 + 8 + 1 + 12 + 16;
const WRAP_LEN: usize = 24 + 32 + 16;
const BODY_LEN: usize = AAD_LEN + WRAP_LEN;
pub(crate) const SLOT_LEN: usize = BODY_LEN + 32;

/// Where the key that unlocks the store comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    /// A random key kept in the operating system's keychain.
    Keychain,
    /// A key derived from a passphrase with Argon2id.
    Passphrase,
}

impl KeySource {
    fn byte(self) -> u8 {
        match self {
            Self::Keychain => 1,
            Self::Passphrase => 2,
        }
    }

    fn from_byte(b: u8) -> Option<Self> {
        match b {
            1 => Some(Self::Keychain),
            2 => Some(Self::Passphrase),
            _ => None,
        }
    }
}

/// Argon2id cost parameters for a passphrase-protected store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory in KiB.
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl KdfParams {
    /// RFC 9106's second recommended option: 64 MiB, 3 passes, 4 lanes.
    pub const DEFAULT: Self = Self {
        memory_kib: 64 * 1024,
        iterations: 3,
        parallelism: 4,
    };

    /// Cheap parameters for tests only.
    pub const INSECURE_FOR_TESTS: Self = Self {
        memory_kib: 64,
        iterations: 1,
        parallelism: 1,
    };

    const NONE: Self = Self {
        memory_kib: 0,
        iterations: 0,
        parallelism: 0,
    };

    /// Refuses costs a tampered header could use to exhaust the machine.
    fn check(self) -> Result<(), StoreError> {
        let ok = (8..=1024 * 1024).contains(&self.memory_kib)
            && (1..=20).contains(&self.iterations)
            && (1..=16).contains(&self.parallelism);
        if ok {
            Ok(())
        } else {
            Err(StoreError::Corrupt(
                "unreasonable passphrase key-derivation parameters".into(),
            ))
        }
    }

    /// Derives the 32-byte key-encryption key from a passphrase.
    pub(crate) fn derive(
        self,
        passphrase: &str,
        salt: &[u8; 16],
    ) -> Result<Zeroizing<[u8; 32]>, StoreError> {
        self.check()?;
        let params =
            argon2::Params::new(self.memory_kib, self.iterations, self.parallelism, Some(32))
                .map_err(|e| StoreError::Corrupt(format!("key-derivation parameters: {e}")))?;
        let argon =
            argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        let mut out = Zeroizing::new([0u8; 32]);
        argon
            .hash_password_into(passphrase.as_bytes(), salt, out.as_mut())
            .map_err(|e| StoreError::Corrupt(format!("key derivation failed: {e}")))?;
        Ok(out)
    }
}

/// One header slot, parsed.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Header {
    pub file_id: [u8; 16],
    pub seq: u64,
    pub source: KeySource,
    pub kdf: KdfParams,
    pub salt: [u8; 16],
    /// Nonce, wrapped data key and tag.
    pub wrapped: [u8; WRAP_LEN],
}

impl std::fmt::Debug for Header {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Header")
            .field("seq", &self.seq)
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl Header {
    /// A header wrapping `data_key` under `kek`.
    pub fn seal(
        file_id: [u8; 16],
        seq: u64,
        source: KeySource,
        kdf: Option<(KdfParams, [u8; 16])>,
        kek: &[u8; 32],
        data_key: &[u8; 32],
    ) -> Result<Self, StoreError> {
        let (kdf, salt) = kdf.unwrap_or((KdfParams::NONE, [0; 16]));
        let mut header = Self {
            file_id,
            seq,
            source,
            kdf,
            salt,
            wrapped: [0; WRAP_LEN],
        };
        let aad = header.aad();
        let mut nonce = [0u8; 24];
        super::random(&mut nonce)?;
        let mut body = Zeroizing::new(*data_key);
        let tag = XChaCha20Poly1305::new(kek.into())
            .encrypt_inout_detached(&XNonce::from(nonce), &aad, body.as_mut().into())
            .map_err(|_| StoreError::Corrupt("could not wrap the data key".into()))?;
        header.wrapped[..24].copy_from_slice(&nonce);
        header.wrapped[24..56].copy_from_slice(body.as_ref());
        header.wrapped[56..].copy_from_slice(&tag);
        Ok(header)
    }

    /// The data key, if `kek` is the right key.
    pub fn unwrap_key(&self, kek: &[u8; 32]) -> Result<Zeroizing<[u8; 32]>, StoreError> {
        let aad = self.aad();
        let nonce = XNonce::try_from(&self.wrapped[..24])
            .map_err(|_| StoreError::Corrupt("bad nonce".into()))?;
        let tag = Tag::try_from(&self.wrapped[56..])
            .map_err(|_| StoreError::Corrupt("bad tag".into()))?;
        let mut key = Zeroizing::new([0u8; 32]);
        key.copy_from_slice(&self.wrapped[24..56]);
        XChaCha20Poly1305::new(kek.into())
            .decrypt_inout_detached(&nonce, &aad, key.as_mut().into(), &tag)
            .map_err(|_| StoreError::WrongKey)?;
        Ok(key)
    }

    fn aad(&self) -> [u8; AAD_LEN] {
        let mut out = [0u8; AAD_LEN];
        let mut w = Writer(&mut out, 0);
        w.put(MAGIC);
        w.put(&VERSION.to_be_bytes());
        w.put(&self.file_id);
        w.put(&self.seq.to_be_bytes());
        w.put(&[self.source.byte()]);
        w.put(&self.kdf.memory_kib.to_be_bytes());
        w.put(&self.kdf.iterations.to_be_bytes());
        w.put(&self.kdf.parallelism.to_be_bytes());
        w.put(&self.salt);
        out
    }

    pub fn encode(&self) -> [u8; SLOT_LEN] {
        let mut out = [0u8; SLOT_LEN];
        out[..AAD_LEN].copy_from_slice(&self.aad());
        out[AAD_LEN..BODY_LEN].copy_from_slice(&self.wrapped);
        let sum = Sha256::digest(&out[..BODY_LEN]);
        out[BODY_LEN..].copy_from_slice(&sum);
        out
    }

    /// Parses one slot. `Ok(None)` means torn or empty; errors mean it's a
    /// complete slot this version can't use.
    pub fn decode(slot: &[u8]) -> Result<Option<Self>, StoreError> {
        if slot.len() < SLOT_LEN || &slot[..8] != MAGIC {
            return Ok(None);
        }
        if Sha256::digest(&slot[..BODY_LEN])[..] != slot[BODY_LEN..SLOT_LEN] {
            return Ok(None);
        }
        let u32_at = |i: usize| {
            let mut b = [0u8; 4];
            b.copy_from_slice(&slot[i..i + 4]);
            u32::from_be_bytes(b)
        };
        let version = u32_at(8);
        if version != VERSION {
            return Err(StoreError::UnsupportedVersion(version));
        }
        let mut file_id = [0u8; 16];
        file_id.copy_from_slice(&slot[12..28]);
        let mut seq = [0u8; 8];
        seq.copy_from_slice(&slot[28..36]);
        let source = KeySource::from_byte(slot[36])
            .ok_or_else(|| StoreError::Corrupt("unknown key source".into()))?;
        let kdf = KdfParams {
            memory_kib: u32_at(37),
            iterations: u32_at(41),
            parallelism: u32_at(45),
        };
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&slot[49..65]);
        let mut wrapped = [0u8; WRAP_LEN];
        wrapped.copy_from_slice(&slot[AAD_LEN..BODY_LEN]);
        Ok(Some(Self {
            file_id,
            seq: u64::from_be_bytes(seq),
            source,
            kdf,
            salt,
            wrapped,
        }))
    }

    /// The newest intact slot of a header region.
    pub fn read_region(region: &[u8]) -> Result<Self, StoreError> {
        let mut found: Vec<Self> = Vec::new();
        for offset in SLOT_OFFSETS {
            let start = usize::try_from(offset).unwrap_or(usize::MAX);
            if let Some(slot) = region.get(start..)
                && let Some(h) = Self::decode(slot)?
            {
                found.push(h);
            }
        }
        if found.len() == 2 && found[0].file_id != found[1].file_id {
            return Err(StoreError::Corrupt("header slots disagree".into()));
        }
        found
            .into_iter()
            .max_by_key(|h| h.seq)
            .ok_or(StoreError::NotAStore)
    }

    /// The slot offsets in the order to write them: the one not holding the
    /// newest intact header first, so it survives until the new one is down.
    pub fn write_order(region: &[u8]) -> [u64; 2] {
        let seq_at = |offset: u64| {
            let start = usize::try_from(offset).unwrap_or(usize::MAX);
            region
                .get(start..)
                .and_then(|s| Self::decode(s).ok().flatten())
                .map(|h| h.seq)
        };
        match (seq_at(SLOT_OFFSETS[0]), seq_at(SLOT_OFFSETS[1])) {
            (Some(a), Some(b)) if a > b => [SLOT_OFFSETS[1], SLOT_OFFSETS[0]],
            (Some(_), None) => [SLOT_OFFSETS[1], SLOT_OFFSETS[0]],
            _ => SLOT_OFFSETS,
        }
    }
}

struct Writer<'a>(&'a mut [u8], usize);

impl Writer<'_> {
    fn put(&mut self, bytes: &[u8]) {
        self.0[self.1..self.1 + bytes.len()].copy_from_slice(bytes);
        self.1 += bytes.len();
    }
}
