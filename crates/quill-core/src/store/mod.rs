//! The encrypted local store: everything this client keeps on disk.
//!
//! One file, unlocked by a key from the OS keychain or from a passphrase.
//! Layers, top to bottom:
//!
//! - [`Store`]: typed methods (the profile now; keys, trust pins and MLS
//!   state in later phases). Nothing else reads or writes the file.
//! - [`kv::Backend`]: named tables of byte keys and values, with atomic
//!   batches. Any backend that passes `kv::conformance` will do.
//! - `redb_kv`: the backend, on redb (a pure-Rust embedded database).
//! - `pagefile`: redb's storage, a file of 4 KiB pages each sealed with
//!   XChaCha20-Poly1305 and written atomically.
//! - `header`: the file's first 4 KiB, holding the random data key wrapped
//!   under the keychain key or the Argon2id passphrase key, twice over.
//!
//! The data key never changes; changing the passphrase or moving to the
//! keychain rewraps it. That keeps a key change instant, but a copy of the
//! file taken earlier still opens with the old passphrase.

mod header;
pub mod kv;
mod pagefile;
mod redb_kv;
pub mod vault;

use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use header::{HEADER_REGION, Header};
pub use header::{KdfParams, KeySource};
use kv::{Backend, MemoryBackend, Op, Table};
use pagefile::PageFile;
use redb_kv::RedbBackend;
use vault::Vault;
use zeroize::Zeroizing;

/// Environment variable that overrides the store file location.
pub const STORE_ENV: &str = "QUILL_STORE";

/// Default store path: `$QUILL_STORE`, else the platform data directory
/// (e.g. `~/.local/share/quill/store.quill` on Linux).
pub fn default_path() -> Result<PathBuf, StoreError> {
    if let Some(p) = std::env::var_os(STORE_ENV) {
        return Ok(PathBuf::from(p));
    }
    let dirs = directories::ProjectDirs::from("", "", "quill").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no home directory; set QUILL_STORE",
        )
    })?;
    Ok(dirs.data_dir().join("store.quill"))
}

/// Version of the table layout inside the store.
const LAYOUT: u8 = 1;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("store file: {0}")]
    Io(#[from] io::Error),
    #[error("{0} already exists")]
    AlreadyExists(PathBuf),
    #[error("the store is open in another quill process")]
    InUse,
    #[error("not a Quill store file")]
    NotAStore,
    #[error("store format version {0} is not supported by this client")]
    UnsupportedVersion(u32),
    #[error("the store is damaged: {0}")]
    Corrupt(String),
    #[error("wrong passphrase, or the keychain holds a different key")]
    WrongKey,
    #[error("this store is protected by {}", describe(*.0))]
    WrongUnlock(KeySource),
    #[error("the store's key isn't in this computer's keychain")]
    MissingKey,
    #[error("keychain: {0}")]
    Keychain(String),
    #[error("store database: {0}")]
    Backend(String),
}

fn describe(source: KeySource) -> &'static str {
    match source {
        KeySource::Keychain => "the system keychain",
        KeySource::Passphrase => "a passphrase",
    }
}

pub(crate) fn random(buf: &mut [u8]) -> Result<(), StoreError> {
    getrandom::fill(buf).map_err(|e| StoreError::Io(io::Error::other(e.to_string())))
}

fn random_key() -> Result<Zeroizing<[u8; 32]>, StoreError> {
    let mut k = Zeroizing::new([0u8; 32]);
    random(k.as_mut())?;
    Ok(k)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The page key, derived from the data key and bound to this file.
fn page_key(data_key: &[u8; 32], file_id: &[u8; 16]) -> Zeroizing<[u8; 32]> {
    let mut out = Zeroizing::new([0u8; 32]);
    // 32 bytes is always a valid HKDF-SHA256 output length.
    let _ = hkdf::Hkdf::<sha2::Sha256>::new(Some(file_id), data_key)
        .expand(b"quill store pages v1", out.as_mut());
    out
}

/// How a new store, or a store changing its key, is protected.
pub enum KeySetup<'a> {
    Keychain(&'a dyn Vault),
    Passphrase { passphrase: &'a str, kdf: KdfParams },
}

/// How to unlock an existing store.
pub enum Unlock<'a> {
    Keychain(&'a dyn Vault),
    Passphrase(&'a str),
}

/// What [`Store::status`] reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreStatus {
    /// `None` for an in-memory store.
    pub path: Option<PathBuf>,
    pub key_source: Option<KeySource>,
    pub file_bytes: u64,
}

struct OnDisk {
    path: PathBuf,
    pages: Arc<PageFile>,
    file_id: [u8; 16],
    data_key: Zeroizing<[u8; 32]>,
    header: Header,
}

pub struct Store {
    kv: Box<dyn Backend>,
    disk: Option<OnDisk>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.disk.as_ref().map(|d| &d.path))
            .finish_non_exhaustive()
    }
}

/// The keychain entry name for one header generation.
fn keychain_name(file_id: &[u8; 16], seq: u64) -> String {
    format!("store-{}-{seq}", hex(file_id))
}

/// Locks the file for this process, failing if another process has it.
fn lock(file: &File) -> Result<(), StoreError> {
    match file.try_lock() {
        Ok(()) => Ok(()),
        Err(fs::TryLockError::WouldBlock) => Err(StoreError::InUse),
        Err(fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

fn read_region(file: &mut File) -> Result<Vec<u8>, StoreError> {
    let mut region = vec![0u8; usize::try_from(HEADER_REGION).unwrap_or(4096)];
    file.seek(SeekFrom::Start(0))?;
    match file.read_exact(&mut region) {
        Ok(()) => Ok(region),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Err(StoreError::NotAStore),
        Err(e) => Err(e.into()),
    }
}

/// Writes `header` to both slots, the stale one first, syncing after each.
fn write_header(file: &mut File, header: &Header) -> Result<(), StoreError> {
    let region = read_region(file)?;
    let bytes = header.encode();
    for offset in Header::write_order(&region) {
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(&bytes)?;
        file.sync_data()?;
    }
    Ok(())
}

/// A new key-encryption key, its source, and Argon2id settings if any.
type NewKek = (
    Zeroizing<[u8; 32]>,
    KeySource,
    Option<(KdfParams, [u8; 16])>,
);

/// The key-encryption key for a setup, and the header fields it needs.
/// For the keychain, the new key is filed under the header's generation.
fn new_kek(setup: &KeySetup<'_>, file_id: &[u8; 16], seq: u64) -> Result<NewKek, StoreError> {
    match setup {
        KeySetup::Keychain(vault) => {
            let kek = random_key()?;
            vault.set(&keychain_name(file_id, seq), kek.as_ref())?;
            Ok((kek, KeySource::Keychain, None))
        }
        KeySetup::Passphrase { passphrase, kdf } => {
            let mut salt = [0u8; 16];
            random(&mut salt)?;
            let kek = kdf.derive(passphrase, &salt)?;
            Ok((kek, KeySource::Passphrase, Some((*kdf, salt))))
        }
    }
}

impl Store {
    /// A store that lives only in memory, for tests.
    pub fn in_memory() -> Self {
        Self {
            kv: Box::new(MemoryBackend::default()),
            disk: None,
        }
    }

    /// Creates a new store file at `path`.
    pub fn create(path: &Path, setup: KeySetup<'_>) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut options = File::options();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(StoreError::AlreadyExists(path.to_owned()));
            }
            Err(e) => return Err(e.into()),
        };
        lock(&file)?;
        let mut file_id = [0u8; 16];
        random(&mut file_id)?;
        let data_key = random_key()?;
        let (kek, source, kdf) = new_kek(&setup, &file_id, 1)?;
        let header = Header::seal(file_id, 1, source, kdf, &kek, &data_key)?;
        file.write_all(&vec![0u8; usize::try_from(HEADER_REGION).unwrap_or(4096)])?;
        write_header(&mut file, &header)?;
        let store = Self::finish_open(path, file, file_id, data_key, header)?;
        store.kv.apply(&[Op::Put {
            table: Table::Meta,
            key: b"layout",
            value: &[LAYOUT],
        }])?;
        Ok(store)
    }

    /// How the store at `path` is protected, without unlocking it.
    pub fn key_source(path: &Path) -> Result<KeySource, StoreError> {
        let mut file = File::open(path)?;
        Ok(Header::read_region(&read_region(&mut file)?)?.source)
    }

    /// Deletes the store at `path` and, for a keychain store, its key in
    /// `vault`. The file must not be open. A missing file is already gone.
    pub fn destroy(path: &Path, vault: Option<&dyn Vault>) -> Result<(), StoreError> {
        let mut file = match File::options().read(true).write(true).open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        lock(&file)?;
        let header = Header::read_region(&read_region(&mut file)?)?;
        if let (KeySource::Keychain, Some(vault)) = (header.source, vault) {
            vault.delete(&keychain_name(&header.file_id, header.seq))?;
        }
        drop(file);
        fs::remove_file(path)?;
        Ok(())
    }

    /// Opens and unlocks the store at `path`.
    pub fn open(path: &Path, unlock: Unlock<'_>) -> Result<Self, StoreError> {
        let mut file = File::options().read(true).write(true).open(path)?;
        lock(&file)?;
        let header = Header::read_region(&read_region(&mut file)?)?;
        let kek = match (header.source, unlock) {
            (KeySource::Keychain, Unlock::Keychain(vault)) => {
                let secret = vault
                    .get(&keychain_name(&header.file_id, header.seq))?
                    .ok_or(StoreError::MissingKey)?;
                let mut kek = Zeroizing::new([0u8; 32]);
                if secret.len() != 32 {
                    return Err(StoreError::WrongKey);
                }
                kek.copy_from_slice(&secret);
                kek
            }
            (KeySource::Passphrase, Unlock::Passphrase(p)) => header.kdf.derive(p, &header.salt)?,
            (source, _) => return Err(StoreError::WrongUnlock(source)),
        };
        let data_key = header.unwrap_key(&kek)?;
        let file_id = header.file_id;
        let store = Self::finish_open(path, file, file_id, data_key, header)?;
        match store.kv.get(Table::Meta, b"layout")? {
            Some(v) if v.as_slice() == [LAYOUT] => Ok(store),
            Some(v) => Err(StoreError::UnsupportedVersion(
                v.first().copied().map_or(0, u32::from),
            )),
            // Creation was interrupted after the header: finish it.
            None => {
                store.kv.apply(&[Op::Put {
                    table: Table::Meta,
                    key: b"layout",
                    value: &[LAYOUT],
                }])?;
                Ok(store)
            }
        }
    }

    fn finish_open(
        path: &Path,
        file: File,
        file_id: [u8; 16],
        data_key: Zeroizing<[u8; 32]>,
        header: Header,
    ) -> Result<Self, StoreError> {
        let pages = Arc::new(PageFile::new(
            file,
            &page_key(&data_key, &file_id),
            file_id,
        )?);
        let kv = RedbBackend::open(Arc::clone(&pages))?;
        Ok(Self {
            kv: Box::new(kv),
            disk: Some(OnDisk {
                path: path.to_owned(),
                pages,
                file_id,
                data_key,
                header,
            }),
        })
    }

    /// Protects the store with a new passphrase or a keychain key. `old`
    /// is the keychain the old key is in, to remove it from.
    pub fn change_key(
        &mut self,
        setup: KeySetup<'_>,
        old: Option<&dyn Vault>,
    ) -> Result<(), StoreError> {
        let Some(disk) = self.disk.as_mut() else {
            return Ok(());
        };
        let old_header = disk.header.clone();
        let seq = old_header.seq + 1;
        let (kek, source, kdf) = new_kek(&setup, &disk.file_id, seq)?;
        let header = Header::seal(disk.file_id, seq, source, kdf, &kek, &disk.data_key)?;
        disk.pages
            .with_file(|f| write_header(f, &header).map_err(io::Error::other))?;
        disk.header = header;
        if let (KeySource::Keychain, Some(vault)) = (old_header.source, old) {
            vault.delete(&keychain_name(&disk.file_id, old_header.seq))?;
        }
        Ok(())
    }

    pub fn status(&self) -> Result<StoreStatus, StoreError> {
        Ok(match &self.disk {
            Some(d) => StoreStatus {
                path: Some(d.path.clone()),
                key_source: Some(d.header.source),
                file_bytes: fs::metadata(&d.path)?.len(),
            },
            None => StoreStatus {
                path: None,
                key_source: None,
                file_bytes: 0,
            },
        })
    }

    // ---- Raw access for the other client modules -------------------------

    pub(crate) fn get(&self, table: Table, key: &[u8]) -> Result<Option<kv::Secret>, StoreError> {
        self.kv.get(table, key)
    }

    pub(crate) fn apply(&self, ops: &[Op<'_>]) -> Result<(), StoreError> {
        self.kv.apply(ops)
    }

    pub(crate) fn scan(
        &self,
        table: Table,
        prefix: &[u8],
    ) -> Result<Vec<(Vec<u8>, kv::Secret)>, StoreError> {
        self.kv.scan(table, prefix)
    }

    /// How many entries a table holds (tests).
    #[cfg(test)]
    pub(crate) fn scan_count(&self, table: Table) -> usize {
        self.kv.scan(table, b"").map(|v| v.len()).unwrap_or(0)
    }

    // ---- Profile: the server and login token ----------------------------

    fn text(&self, table: Table, key: &[u8]) -> Result<Option<Zeroizing<String>>, StoreError> {
        self.kv
            .get(table, key)?
            .map(|v| {
                String::from_utf8(v.to_vec())
                    .map(Zeroizing::new)
                    .map_err(|_| StoreError::Corrupt("text value is not UTF-8".into()))
            })
            .transpose()
    }

    /// The server this client talks to.
    pub fn server(&self) -> Result<Option<String>, StoreError> {
        Ok(self
            .text(Table::Profile, b"server")?
            .map(|s| s.as_str().to_owned()))
    }

    /// Sets the server. A different server forgets the login token, which
    /// only the server that issued it accepts.
    pub fn set_server(&self, url: &str) -> Result<(), StoreError> {
        if self.server()?.as_deref() == Some(url) {
            return Ok(());
        }
        self.kv.apply(&[
            Op::Put {
                table: Table::Profile,
                key: b"server",
                value: url.as_bytes(),
            },
            Op::Delete {
                table: Table::Profile,
                key: b"token",
            },
        ])
    }

    /// The login token, if logged in.
    pub fn token(&self) -> Result<Option<Zeroizing<String>>, StoreError> {
        self.text(Table::Profile, b"token")
    }

    pub fn set_token(&self, token: Option<&str>) -> Result<(), StoreError> {
        self.kv.apply(&[match token {
            Some(t) => Op::Put {
                table: Table::Profile,
                key: b"token",
                value: t.as_bytes(),
            },
            None => Op::Delete {
                table: Table::Profile,
                key: b"token",
            },
        }])
    }
}

#[cfg(test)]
mod tests;
