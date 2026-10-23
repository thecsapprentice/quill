//! The storage interface every backend implements.
//!
//! Deliberately small: named tables of byte keys to byte values, point
//! reads, prefix scans and atomic batches of writes. Everything the client
//! keeps goes through [`super::Store`] on top of this, never through a
//! backend directly, so a backend can be replaced (SQLCipher, or IndexedDB
//! for the web client) by implementing these four methods; the shared tests
//! in this module's `conformance` function check any implementation.

use std::{collections::BTreeMap, sync::RwLock};

use zeroize::Zeroizing;

use super::StoreError;

/// A value read from the store; wiped from memory when dropped.
pub type Secret = Zeroizing<Vec<u8>>;

/// The store's tables. Each later phase adds its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Table {
    /// Store bookkeeping, such as the layout version.
    Meta,
    /// The server address and login token.
    Profile,
}

impl Table {
    pub const ALL: [Self; 2] = [Self::Meta, Self::Profile];

    pub fn name(self) -> &'static str {
        match self {
            Self::Meta => "meta",
            Self::Profile => "profile",
        }
    }
}

/// One write in an atomic batch.
#[derive(Debug, Clone, Copy)]
pub enum Op<'a> {
    Put {
        table: Table,
        key: &'a [u8],
        value: &'a [u8],
    },
    Delete {
        table: Table,
        key: &'a [u8],
    },
}

pub trait Backend: Send + Sync {
    fn get(&self, table: Table, key: &[u8]) -> Result<Option<Secret>, StoreError>;

    /// Every entry whose key starts with `prefix`, in key order.
    fn scan(&self, table: Table, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Secret)>, StoreError>;

    /// Applies all of `ops` or none of them, durably.
    fn apply(&self, ops: &[Op<'_>]) -> Result<(), StoreError>;
}

/// Keeps everything in memory. For tests, and as the reference behaviour.
#[derive(Default)]
pub struct MemoryBackend(RwLock<BTreeMap<(Table, Vec<u8>), Secret>>);

impl MemoryBackend {
    fn poisoned() -> StoreError {
        StoreError::Backend("memory store lock poisoned".into())
    }
}

impl Backend for MemoryBackend {
    fn get(&self, table: Table, key: &[u8]) -> Result<Option<Secret>, StoreError> {
        let map = self.0.read().map_err(|_| Self::poisoned())?;
        Ok(map.get(&(table, key.to_vec())).cloned())
    }

    fn scan(&self, table: Table, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Secret)>, StoreError> {
        let map = self.0.read().map_err(|_| Self::poisoned())?;
        Ok(map
            .range((table, prefix.to_vec())..)
            .take_while(|((t, k), _)| *t == table && k.starts_with(prefix))
            .map(|((_, k), v)| (k.clone(), v.clone()))
            .collect())
    }

    fn apply(&self, ops: &[Op<'_>]) -> Result<(), StoreError> {
        let mut map = self.0.write().map_err(|_| Self::poisoned())?;
        for op in ops {
            match *op {
                Op::Put { table, key, value } => {
                    map.insert((table, key.to_vec()), Zeroizing::new(value.to_vec()));
                }
                Op::Delete { table, key } => {
                    map.remove(&(table, key.to_vec()));
                }
            }
        }
        Ok(())
    }
}

/// Behaviour every backend must share. Called by each backend's tests.
#[cfg(test)]
pub(crate) fn conformance(b: &dyn Backend) {
    use Table::{Meta, Profile};
    let get = |t, k: &[u8]| b.get(t, k).unwrap().map(|v| v.to_vec());

    assert_eq!(get(Profile, b"missing"), None);
    assert!(b.scan(Profile, b"").unwrap().is_empty());

    b.apply(&[
        Op::Put {
            table: Profile,
            key: b"a/2",
            value: b"two",
        },
        Op::Put {
            table: Profile,
            key: b"a/1",
            value: b"one",
        },
        Op::Put {
            table: Profile,
            key: b"b",
            value: b"bee",
        },
        Op::Put {
            table: Meta,
            key: b"a/1",
            value: b"meta",
        },
        Op::Put {
            table: Profile,
            key: b"",
            value: b"empty key",
        },
    ])
    .unwrap();
    assert_eq!(get(Profile, b"a/1"), Some(b"one".to_vec()));
    assert_eq!(get(Meta, b"a/1"), Some(b"meta".to_vec()));
    assert_eq!(get(Profile, b""), Some(b"empty key".to_vec()));

    // Prefix scans: in key order, one table only.
    let keys = |prefix: &[u8]| -> Vec<Vec<u8>> {
        b.scan(Profile, prefix)
            .unwrap()
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    };
    assert_eq!(keys(b"a/"), vec![b"a/1".to_vec(), b"a/2".to_vec()]);
    assert_eq!(
        keys(b""),
        vec![
            b"".to_vec(),
            b"a/1".to_vec(),
            b"a/2".to_vec(),
            b"b".to_vec()
        ]
    );
    assert_eq!(keys(b"c"), Vec::<Vec<u8>>::new());

    // Overwrite and delete in one batch; later ops win.
    b.apply(&[
        Op::Put {
            table: Profile,
            key: b"a/1",
            value: b"uno",
        },
        Op::Delete {
            table: Profile,
            key: b"a/2",
        },
        Op::Delete {
            table: Profile,
            key: b"never-there",
        },
        Op::Put {
            table: Profile,
            key: b"b",
            value: b"first",
        },
        Op::Put {
            table: Profile,
            key: b"b",
            value: b"second",
        },
    ])
    .unwrap();
    assert_eq!(get(Profile, b"a/1"), Some(b"uno".to_vec()));
    assert_eq!(get(Profile, b"a/2"), None);
    assert_eq!(get(Profile, b"b"), Some(b"second".to_vec()));

    // Large values and binary keys.
    let big = vec![0x5Au8; 200_000];
    let key = [0u8, 255, 1, 254];
    b.apply(&[Op::Put {
        table: Profile,
        key: &key,
        value: &big,
    }])
    .unwrap();
    assert_eq!(get(Profile, &key), Some(big));
}

#[cfg(test)]
mod tests {
    #[test]
    fn memory_backend_conforms() {
        super::conformance(&super::MemoryBackend::default());
    }
}
