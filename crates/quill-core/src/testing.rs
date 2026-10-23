//! Tools for tests (feature `test-tools`); not built into clients.

use serde_json::Value;

use crate::{CoreError, Store, store::kv::Table, words};

/// A secret this device holds, for tests that check none of them ever
/// reaches the server (the Phase 2 gate, 2l).
#[derive(Debug, Clone)]
pub struct Secret {
    /// What it is, for the failure message.
    pub what: String,
    pub bytes: Vec<u8>,
}

/// OpenMLS state entries holding private keys or secrets, by label.
const MLS_SECRET_LABELS: [&str; 7] = [
    "KeyPackage",
    "EncryptionKeyPair",
    "EpochKeyPairs",
    "EpochSecrets",
    "MessageSecrets",
    "ResumptionPsk",
    "Psk",
];

/// Every byte string in `v` of 16 bytes or more that sits under an object
/// key naming a private key or a secret.
fn secret_bytes(v: &Value, under_secret: bool, out: &mut Vec<Vec<u8>>) {
    match v {
        Value::Array(items) => {
            let bytes: Option<Vec<u8>> = items
                .iter()
                .map(|i| i.as_u64().and_then(|n| u8::try_from(n).ok()))
                .collect();
            match bytes {
                Some(b) if under_secret && b.len() >= 16 => out.push(b),
                Some(_) => {}
                None => {
                    for i in items {
                        secret_bytes(i, under_secret, out);
                    }
                }
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                let k = k.to_ascii_lowercase();
                let secret = under_secret || k.contains("private") || k.contains("secret");
                secret_bytes(v, secret, out);
            }
        }
        _ => {}
    }
}

/// Every long-term secret and key this store holds: each persona's identity
/// key, this device's persona device keys and account device key, the
/// recovery code (as bits and as words), and OpenMLS's private keys and
/// epoch secrets.
pub fn secrets(store: &Store) -> Result<Vec<Secret>, CoreError> {
    let mut out = Vec::new();
    for (k, v) in store.scan(Table::Identities, b"")? {
        if let Some(seed) = v.get(4..36) {
            out.push(Secret {
                what: format!("identity key of persona {}", hex(&k)),
                bytes: seed.to_vec(),
            });
        }
    }
    for (k, v) in store.scan(Table::Devices, b"")? {
        if let Some(seed) = v.get(16..48) {
            out.push(Secret {
                what: format!("device key for persona {}", hex(&k)),
                bytes: seed.to_vec(),
            });
        }
    }
    if let Some(v) = store.get(Table::Account, b"device")?
        && let Some(seed) = v.get(16..48)
    {
        out.push(Secret {
            what: "account device key".into(),
            bytes: seed.to_vec(),
        });
    }
    if let Some(v) = store.get(Table::Backup, b"code")?
        && let Ok(entropy) = <[u8; 16]>::try_from(v.as_slice())
    {
        out.push(Secret {
            what: "recovery code bits".into(),
            bytes: entropy.to_vec(),
        });
        out.push(Secret {
            what: "recovery code words".into(),
            bytes: words::mnemonic(&entropy).join(" ").into_bytes(),
        });
    }
    for (k, v) in store.scan(Table::Mls, b"")? {
        let Some(label) = MLS_SECRET_LABELS
            .iter()
            .find(|l| k.starts_with(l.as_bytes()))
        else {
            continue;
        };
        let Ok(json) = serde_json::from_slice::<Value>(&v) else {
            continue;
        };
        let mut found = Vec::new();
        // Psk and resumption entries are secret throughout.
        secret_bytes(&json, matches!(*label, "Psk" | "ResumptionPsk"), &mut found);
        for b in found {
            out.push(Secret {
                what: format!("OpenMLS {label}"),
                bytes: b,
            });
        }
    }
    Ok(out)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
