//! Encrypted backups (2j; plan: recovery when no device is left).
//!
//! A backup holds every persona's identity key, the trust pins and
//! verification marks, and each session's history as this device keeps it
//! (notes with every version, channels, attachments). Never MLS state: a
//! restored device joins as a new device. It's sealed with a recovery code,
//! 128 random bits shown once as 12 BIP-39 words, which the server never
//! sees; the device keeps the code in its encrypted store so it can
//! refresh the backup later.

pub mod cipher;

use quill_certs::RevocationReason;
use quill_proto::{
    keys::{b64, unb64},
    limits::BACKUP_PART_BYTES,
    sessions::BackupInfo,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    Client, CoreError, Store, keys,
    store::kv::{Op, Table},
    words,
};
use cipher::{AgeCrate, BackupCipher};

/// What a backup carries: these tables, whole.
const TABLES: [Table; 5] = [
    Table::Identities,
    Table::Pins,
    Table::Verified,
    Table::Messages,
    Table::Files,
];
const FORMAT: &str = "quill-backup";
const VERSION: u32 = 1;
const K_CODE: &[u8] = b"code";

fn err(msg: impl Into<String>) -> CoreError {
    CoreError::Keys(msg.into())
}

/// The backup's plaintext: JSON (the protocol draft's CBOR is noted as a
/// difference there), tables as lists of `[key, value]` in base64url.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Contents {
    format: String,
    version: u32,
    created_ms: u64,
    tables: BTreeMap<String, Vec<[String; 2]>>,
}

/// The passphrase a code gives: its 12 words, lowercase, single spaces.
fn passphrase(entropy: &[u8; 16]) -> Zeroizing<String> {
    Zeroizing::new(words::mnemonic(entropy).join(" "))
}

fn stored_code(store: &Store) -> Result<Option<Zeroizing<[u8; 16]>>, CoreError> {
    Ok(store
        .get(Table::Backup, K_CODE)?
        .and_then(|v| <[u8; 16]>::try_from(v.as_slice()).ok())
        .map(Zeroizing::new))
}

fn save_code(store: &Store, entropy: &[u8; 16]) -> Result<(), CoreError> {
    Ok(store.apply(&[Op::Put {
        table: Table::Backup,
        key: K_CODE,
        value: entropy,
    }])?)
}

/// What [`create`] did.
#[derive(Debug, Clone)]
pub struct Created {
    /// The recovery code's words when a new code was made: shown once.
    pub code: Option<Vec<&'static str>>,
    pub info: BackupInfo,
}

/// Makes a backup of this device and uploads it, replacing the login's
/// previous one. Uses the code this device keeps, or a new one (when it
/// has none, or `new_code`).
pub async fn create(store: &Store, client: &Client, new_code: bool) -> Result<Created, CoreError> {
    let (entropy, fresh) = match stored_code(store)? {
        Some(code) if !new_code => (code, false),
        _ => {
            let mut e = Zeroizing::new([0u8; 16]);
            getrandom::fill(e.as_mut()).map_err(|e| err(format!("no randomness: {e}")))?;
            (e, true)
        }
    };
    let mut tables = BTreeMap::new();
    for t in TABLES {
        let rows: Vec<[String; 2]> = store
            .scan(t, b"")?
            .into_iter()
            .map(|(k, v)| [b64(&k), b64(&v)])
            .collect();
        tables.insert(t.name().to_owned(), rows);
    }
    let plain = Zeroizing::new(
        serde_json::to_vec(&Contents {
            format: FORMAT.into(),
            version: VERSION,
            created_ms: keys::now_ms(),
            tables,
        })
        .map_err(|_| err("encoding the backup"))?,
    );
    let sealed = AgeCrate.seal(&passphrase(&entropy), &plain)?;
    let max = client.instance().await?.limits.max_backup_bytes;
    if sealed.len() as u64 > max {
        return Err(err(format!(
            "the backup would be {} MiB; this server keeps at most {} MiB",
            sealed.len() / (1024 * 1024) + 1,
            max / (1024 * 1024)
        )));
    }
    let generation = Uuid::new_v4();
    let chunks: Vec<&[u8]> = sealed
        .chunks(usize::try_from(BACKUP_PART_BYTES).unwrap_or(usize::MAX))
        .collect();
    for (i, chunk) in chunks.iter().enumerate() {
        client
            .upload_backup_part(
                generation,
                u32::try_from(i).unwrap_or(u32::MAX),
                chunk.to_vec(),
            )
            .await?;
    }
    let info = client
        .commit_backup(generation, u32::try_from(chunks.len()).unwrap_or(u32::MAX))
        .await?;
    // Kept only once the backup it opens is safely stored.
    save_code(store, &entropy)?;
    Ok(Created {
        code: fresh.then(|| words::mnemonic(&entropy)),
        info,
    })
}

/// Whether this device keeps a recovery code (and so can refresh the
/// backup without showing a new one).
pub fn has_code(store: &Store) -> Result<bool, CoreError> {
    Ok(stored_code(store)?.is_some())
}

/// What [`restore`] did.
#[derive(Debug, Clone, Default)]
pub struct Restored {
    /// Personas whose keys came back, set up on this device.
    pub personas: Vec<String>,
    /// The personas' other devices revoked.
    pub revoked: usize,
    /// Sessions this device asked to rejoin.
    pub rejoins: usize,
}

/// Restores the login's backup on this (new) device from the 12 words.
/// It replaces every other device of the login: their persona devices are
/// revoked, the device group starts again, and this device asks to rejoin
/// each session.
pub async fn restore(store: &Store, client: &Client, typed: &str) -> Result<Restored, CoreError> {
    let entropy = Zeroizing::new(words::from_mnemonic(typed).ok_or_else(|| {
        err("those aren't the 12 words of a recovery code: a word is misspelt, missing or out of order")
    })?);
    if !store.scan(Table::Identities, b"")?.is_empty() {
        return Err(err(
            "this device already holds persona keys: restore on a new device (a new store)",
        ));
    }
    let info = client
        .backup_info()
        .await?
        .ok_or_else(|| err("this login has no backup on the server"))?;
    let mut sealed = Vec::new();
    for part in 0..info.parts {
        sealed.extend_from_slice(&client.backup_part(part).await?);
    }
    let plain = AgeCrate.open(&passphrase(&entropy), &sealed)?;
    let contents: Contents =
        serde_json::from_slice(&plain).map_err(|_| err("the backup's contents can't be read"))?;
    if contents.format != FORMAT || contents.version != VERSION {
        return Err(err("a backup from a newer or different client"));
    }
    let mut rows: Vec<(Table, Vec<u8>, Zeroizing<Vec<u8>>)> = Vec::new();
    for t in TABLES {
        for [k, v] in contents
            .tables
            .get(t.name())
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            let (Some(k), Some(v)) = (unb64(k), unb64(v)) else {
                return Err(err("the backup's contents are damaged"));
            };
            rows.push((t, k, Zeroizing::new(v)));
        }
    }
    let ops: Vec<Op<'_>> = rows
        .iter()
        .map(|(table, key, value)| Op::Put {
            table: *table,
            key,
            value,
        })
        .collect();
    store.apply(&ops)?;
    save_code(store, &entropy)?;

    // The old devices are gone: the device group starts again.
    match client.drop_device_group().await {
        Ok(()) => {}
        Err(e) if e.status() == Some(404) => {}
        Err(e) => return Err(e),
    }
    let mut report = Restored::default();
    for persona in client.personas().await? {
        if keys::load_identity(store, persona.id)?.is_none() {
            continue;
        }
        keys::init(store, client, &persona, false).await?;
        keys::publish(store, client, &persona, keys::POOL_TARGET).await?;
        let mine = keys::load_device(store, persona.id)?.and_then(|d| d.id);
        let published = keys::check_published(
            &client.actor_keys(&persona.handle).await?,
            &persona.actor_uri,
            keys::now_ms(),
        )?;
        for d in published.devices {
            if Some(d.id) == mine || !matches!(d.status, keys::DeviceStatus::Valid { .. }) {
                continue;
            }
            keys::revoke(store, client, &persona, d.id, RevocationReason::Lost).await?;
            report.revoked += 1;
        }
        report.personas.push(persona.handle);
    }
    report.rejoins = crate::sessions::rejoin_all(store, client).await?;
    Ok(report)
}
