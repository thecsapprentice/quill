//! How a backup is sealed: behind one small interface, so the format's
//! implementation can be swapped without the rest of the code changing.
//!
//! Backups are `age` v1 files with the scrypt passphrase recipient, the
//! recovery words as the passphrase (protocol draft, section 6), so they
//! open with the standard `age` tools too. Today the `age` crate does the
//! work ([`AgeCrate`]); our own implementation of the same format could
//! replace it (decided 2026-10-23).

use std::io::{Read, Write};

use age::secrecy::SecretString;
use zeroize::Zeroizing;

use crate::CoreError;

/// scrypt's work factor (log2 N) for new backups. The passphrase is a
/// random 128-bit code, not something a person chose, so it needs no
/// stretching to resist guessing; this keeps memory use modest (32 MiB)
/// for the web client to come.
pub const WORK_FACTOR: u8 = 15;
/// The most a backup may ask of a device opening it (log2 N, 512 MiB).
pub const MAX_WORK_FACTOR: u8 = 20;

fn err(msg: impl Into<String>) -> CoreError {
    CoreError::Keys(msg.into())
}

/// Seals and opens backup files with a passphrase.
pub trait BackupCipher {
    fn seal(&self, passphrase: &str, plain: &[u8]) -> Result<Vec<u8>, CoreError>;
    /// Opens a file sealed with `passphrase`. A wrong passphrase and a
    /// damaged file both fail.
    fn open(&self, passphrase: &str, sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, CoreError>;
}

/// The `age` crate's implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct AgeCrate;

impl BackupCipher for AgeCrate {
    fn seal(&self, passphrase: &str, plain: &[u8]) -> Result<Vec<u8>, CoreError> {
        let mut recipient = age::scrypt::Recipient::new(SecretString::from(passphrase.to_owned()));
        recipient.set_work_factor(WORK_FACTOR);
        let encryptor =
            age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
                .map_err(|e| err(format!("sealing the backup: {e}")))?;
        let mut out = Vec::with_capacity(plain.len() + 256);
        let mut writer = encryptor
            .wrap_output(&mut out)
            .map_err(|e| err(format!("sealing the backup: {e}")))?;
        writer
            .write_all(plain)
            .and_then(|()| writer.finish().map(|_| ()))
            .map_err(|e| err(format!("sealing the backup: {e}")))?;
        Ok(out)
    }

    fn open(&self, passphrase: &str, sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, CoreError> {
        let mut identity = age::scrypt::Identity::new(SecretString::from(passphrase.to_owned()));
        identity.set_max_work_factor(MAX_WORK_FACTOR);
        let decryptor = age::Decryptor::new(sealed)
            .map_err(|e| err(format!("the backup can't be read: {e}")))?;
        let mut reader = decryptor
            .decrypt(std::iter::once(&identity as &dyn age::Identity))
            .map_err(|_| err("the backup doesn't open with that recovery code"))?;
        let mut plain = Zeroizing::new(Vec::new());
        reader
            .read_to_end(&mut plain)
            .map_err(|_| err("the backup is damaged"))?;
        Ok(plain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_files_round_trip_and_refuse_wrong_codes_and_damage() {
        let c = AgeCrate;
        let plain = b"identity keys and history".repeat(5000);
        let sealed = c.seal("abandon ability able", &plain).unwrap();
        assert!(sealed.starts_with(b"age-encryption.org/v1\n-> scrypt "));
        assert_eq!(
            c.open("abandon ability able", &sealed).unwrap().as_slice(),
            plain.as_slice()
        );
        assert!(c.open("abandon ability ably", &sealed).is_err());
        let mut damaged = sealed.clone();
        let n = damaged.len() - 10;
        damaged[n] ^= 1;
        assert!(c.open("abandon ability able", &damaged).is_err());
        assert!(
            c.open("abandon ability able", &sealed[..sealed.len() - 5])
                .is_err()
        );
    }
}
