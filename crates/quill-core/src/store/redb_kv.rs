//! The storage interface on redb, over the page-encrypted file.

use std::sync::Arc;

use redb::{Database, ReadableDatabase, TableDefinition, TableError};
use zeroize::Zeroizing;

use super::{
    StoreError,
    kv::{Backend, Op, Secret, Table},
    pagefile::PageFile,
};

fn def(table: Table) -> TableDefinition<'static, &'static [u8], &'static [u8]> {
    TableDefinition::new(table.name())
}

fn db_err(e: impl std::fmt::Display) -> StoreError {
    StoreError::Backend(e.to_string())
}

/// Lets the store keep a handle on the file redb owns, for header updates.
#[derive(Debug)]
pub(crate) struct SharedPageFile(pub Arc<PageFile>);

impl redb::StorageBackend for SharedPageFile {
    fn len(&self) -> Result<u64, std::io::Error> {
        self.0.len()
    }
    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), std::io::Error> {
        self.0.read(offset, out)
    }
    fn set_len(&self, len: u64) -> Result<(), std::io::Error> {
        self.0.set_len(len)
    }
    fn sync_data(&self) -> Result<(), std::io::Error> {
        self.0.sync_data()
    }
    fn write(&self, offset: u64, data: &[u8]) -> Result<(), std::io::Error> {
        self.0.write(offset, data)
    }
}

pub(crate) struct RedbBackend {
    db: Database,
}

impl RedbBackend {
    pub fn open(file: Arc<PageFile>) -> Result<Self, StoreError> {
        Self::open_backend(SharedPageFile(file))
    }

    pub(crate) fn open_backend(backend: impl redb::StorageBackend) -> Result<Self, StoreError> {
        let db = Database::builder()
            // redb's page cache holds plaintext; keep it modest.
            .set_cache_size(8 * 1024 * 1024)
            .create_with_backend(backend)
            .map_err(db_err)?;
        Ok(Self { db })
    }
}

impl Backend for RedbBackend {
    fn get(&self, table: Table, key: &[u8]) -> Result<Option<Secret>, StoreError> {
        let txn = self.db.begin_read().map_err(db_err)?;
        let t = match txn.open_table(def(table)) {
            Ok(t) => t,
            Err(TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(db_err(e)),
        };
        Ok(t.get(key)
            .map_err(db_err)?
            .map(|v| Zeroizing::new(v.value().to_vec())))
    }

    fn scan(&self, table: Table, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Secret)>, StoreError> {
        let txn = self.db.begin_read().map_err(db_err)?;
        let t = match txn.open_table(def(table)) {
            Ok(t) => t,
            Err(TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(e) => return Err(db_err(e)),
        };
        let mut out = Vec::new();
        for entry in t.range(prefix..).map_err(db_err)? {
            let (k, v) = entry.map_err(db_err)?;
            if !k.value().starts_with(prefix) {
                break;
            }
            out.push((k.value().to_vec(), Zeroizing::new(v.value().to_vec())));
        }
        Ok(out)
    }

    fn apply(&self, ops: &[Op<'_>]) -> Result<(), StoreError> {
        let txn = self.db.begin_write().map_err(db_err)?;
        for op in ops {
            match *op {
                Op::Put { table, key, value } => {
                    txn.open_table(def(table))
                        .map_err(db_err)?
                        .insert(key, value)
                        .map_err(db_err)?;
                }
                Op::Delete { table, key } => {
                    txn.open_table(def(table))
                        .map_err(db_err)?
                        .remove(key)
                        .map_err(db_err)?;
                }
            }
        }
        txn.commit().map_err(db_err)
    }
}
