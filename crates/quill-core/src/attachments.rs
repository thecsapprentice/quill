//! Encrypted attachments (2i; plan: encrypted blob plus key in the
//! message). Each file gets a fresh 256-bit key and is sealed in one shot
//! with AES-256-GCM; the ciphertext is padded to a size bucket so the
//! server learns only the bucket. The key, nonce, SHA-256 of the plaintext
//! and the real size travel inside the MLS message (the note's
//! `EncryptedAttachment`), never to the server.
//!
//! Nothing here looks at what a file is: any file can be attached, and its
//! media type is only a label.

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use quill_proto::limits::blob_bucket;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use quill_proto::keys::{b64, unb64};

use crate::CoreError;

/// The AEAD tag's length.
const TAG: usize = 16;

fn err(msg: impl Into<String>) -> CoreError {
    CoreError::Keys(msg.into())
}

/// An attachment as a note carries it (protocol draft, section 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    /// `EncryptedAttachment`.
    #[serde(rename = "type")]
    pub kind: String,
    /// A label from the file name; nothing checks it.
    pub media_type: String,
    pub name: String,
    /// Where the blob is: `{server}/api/v1/blobs/{id}`.
    pub url: String,
    pub blob_token: String,
    /// `A256GCM`.
    pub enc_alg: String,
    pub key: String,
    pub nonce: String,
    /// `sha-256`.
    pub hash_alg: String,
    /// Of the plaintext.
    pub content_hash: String,
    /// The plaintext's size in bytes.
    pub size: u64,
}

/// The type of an attachment object.
pub const ATTACHMENT_TYPE: &str = "EncryptedAttachment";

/// A file sealed for upload: the padded ciphertext, and what the message
/// must carry to open it.
pub struct Sealed {
    pub blob: Vec<u8>,
    pub key: Zeroizing<[u8; 32]>,
    pub nonce: [u8; 12],
    pub content_hash: [u8; 32],
    pub size: u64,
}

/// Seals `plain`, padding the ciphertext to its bucket under `max` bytes.
pub fn seal(plain: &[u8], max: u64) -> Result<Sealed, CoreError> {
    let mut key = Zeroizing::new([0u8; 32]);
    let mut nonce = [0u8; 12];
    getrandom::fill(key.as_mut()).map_err(|e| err(format!("no randomness: {e}")))?;
    getrandom::fill(&mut nonce).map_err(|e| err(format!("no randomness: {e}")))?;
    seal_with(plain, max, key, nonce)
}

/// [`seal`] with a given key and nonce (for test vectors: a nonce must
/// never be used twice with one key).
pub(crate) fn seal_with(
    plain: &[u8],
    max: u64,
    key: Zeroizing<[u8; 32]>,
    nonce: [u8; 12],
) -> Result<Sealed, CoreError> {
    let size = u64::try_from(plain.len()).map_err(|_| err("the file is too large"))?;
    let bucket = blob_bucket(size + TAG as u64, max).ok_or_else(|| {
        err(format!(
            "the file is too large: attachments are at most {} bytes here",
            max.saturating_sub(TAG as u64)
        ))
    })?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| err("bad key"))?;
    let mut blob = cipher
        .encrypt(&Nonce::from(nonce), plain)
        .map_err(|_| err("encrypting the file failed"))?;
    blob.resize(usize::try_from(bucket).map_err(|_| err("too large"))?, 0);
    Ok(Sealed {
        blob,
        key,
        nonce,
        content_hash: Sha256::digest(plain).into(),
        size,
    })
}

impl Sealed {
    /// The attachment a note carries for this file, once uploaded.
    pub fn attachment(&self, name: &str, url: String, token: String) -> Attachment {
        Attachment {
            kind: ATTACHMENT_TYPE.into(),
            media_type: media_type(name).into(),
            name: name.to_owned(),
            url,
            blob_token: token,
            enc_alg: "A256GCM".into(),
            key: b64(self.key.as_ref()),
            nonce: b64(&self.nonce),
            hash_alg: "sha-256".into(),
            content_hash: b64(&self.content_hash),
            size: self.size,
        }
    }
}

/// Opens a fetched blob: checks its padded size, decrypts, and checks the
/// plaintext against the hash the message carried.
pub fn open(blob: &[u8], a: &Attachment, max: u64) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    if a.enc_alg != "A256GCM" || a.hash_alg != "sha-256" {
        return Err(err("an attachment in a format this client doesn't read"));
    }
    let sealed = a
        .size
        .checked_add(TAG as u64)
        .filter(|n| *n <= max)
        .ok_or_else(|| err("the attachment says it's larger than this server allows"))?;
    let expected = blob_bucket(sealed, max).ok_or_else(|| err("the attachment is too large"))?;
    if u64::try_from(blob.len()).ok() != Some(expected) {
        return Err(err("the attachment's blob isn't the size it should be"));
    }
    let key: Zeroizing<Vec<u8>> = Zeroizing::new(unb64(&a.key).ok_or_else(|| err("bad key"))?);
    let nonce: [u8; 12] = unb64(&a.nonce)
        .and_then(|n| n.try_into().ok())
        .ok_or_else(|| err("bad nonce"))?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| err("bad key"))?;
    let end = usize::try_from(sealed).map_err(|_| err("too large"))?;
    let plain = Zeroizing::new(
        cipher
            .decrypt(&Nonce::from(nonce), &blob[..end])
            .map_err(|_| err("the attachment doesn't decrypt: it was changed or damaged"))?,
    );
    let hash = unb64(&a.content_hash).ok_or_else(|| err("bad hash"))?;
    if Sha256::digest(plain.as_slice()).as_slice() != hash.as_slice() {
        return Err(err("the attachment doesn't match its hash"));
    }
    Ok(plain)
}

/// A media type label from a file name's extension. Only a label: nothing
/// is checked, and anything unknown is `application/octet-stream`.
pub fn media_type(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "md" => "text/markdown",
        _ => "application/octet-stream",
    }
}

/// A safe file name for saving: the base name only, with path separators
/// and control characters replaced.
pub fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_control() || c == ':' { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim_matches(|c| c == '.' || c == ' ').to_owned();
    if cleaned.is_empty() {
        "attachment".into()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: u64 = quill_proto::limits::DEFAULT_MAX_ATTACHMENT_BYTES;

    #[test]
    fn files_seal_to_buckets_and_open_intact() {
        for len in [0usize, 1, 4000, 4080, 4081, 100_000] {
            let plain: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let s = seal(&plain, MAX).unwrap();
            assert!(quill_proto::limits::is_blob_bucket(
                s.blob.len() as u64,
                MAX
            ));
            let a = s.attachment("notes.txt", "u".into(), "t".into());
            assert_eq!(a.media_type, "text/plain");
            assert_eq!(open(&s.blob, &a, MAX).unwrap().as_slice(), plain.as_slice());
        }
        assert!(seal(&vec![0; MAX as usize], MAX).is_err());
    }

    #[test]
    fn a_changed_byte_or_a_lying_message_is_caught() {
        let s = seal(b"The ledger's last page.", MAX).unwrap();
        let a = s.attachment("ledger", "u".into(), "t".into());
        let mut blob = s.blob.clone();
        blob[3] ^= 1;
        assert!(open(&blob, &a, MAX).is_err());
        // Padding is ignored, but the size must match the bucket.
        let mut longer = s.blob.clone();
        longer.extend_from_slice(&[0; 4096]);
        assert!(open(&longer, &a, MAX).is_err());
        let mut wrong = a.clone();
        wrong.content_hash = b64(&[0; 32]);
        assert!(open(&s.blob, &wrong, MAX).is_err());
        let mut huge = a;
        huge.size = MAX;
        assert!(open(&s.blob, &huge, MAX).is_err());
    }

    #[test]
    fn saved_names_stay_in_their_folder() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("C:\\\\x\\\\map.png"), "map.png");
        assert_eq!(safe_name(".."), "attachment");
        assert_eq!(safe_name("a\u{7}b.txt"), "a_b.txt");
        assert_eq!(media_type("Map.PNG"), "image/png");
        assert_eq!(media_type("noext"), "application/octet-stream");
    }
}
