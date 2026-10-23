use super::*;
use vault::MemoryVault;

const KDF: KdfParams = KdfParams::INSECURE_FOR_TESTS;

fn pass(p: &str) -> KeySetup<'_> {
    KeySetup::Passphrase {
        passphrase: p,
        kdf: KDF,
    }
}

fn store_path() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("store.quill");
    (dir, path)
}

#[test]
fn passphrase_store_round_trips_and_refuses_wrong_unlocks() {
    let (_dir, path) = store_path();
    let store = Store::create(&path, pass("correct horse battery")).unwrap();
    store.set_server("https://writers.example").unwrap();
    store.set_token(Some("qt_secret")).unwrap();
    drop(store);

    assert_eq!(Store::key_source(&path).unwrap(), KeySource::Passphrase);
    let store = Store::open(&path, Unlock::Passphrase("correct horse battery")).unwrap();
    assert_eq!(
        store.server().unwrap().as_deref(),
        Some("https://writers.example")
    );
    assert_eq!(
        store.token().unwrap().as_deref().map(String::as_str),
        Some("qt_secret")
    );
    drop(store);

    assert!(matches!(
        Store::open(&path, Unlock::Passphrase("wrong horse")),
        Err(StoreError::WrongKey)
    ));
    let vault = MemoryVault::default();
    assert!(matches!(
        Store::open(&path, Unlock::Keychain(&vault)),
        Err(StoreError::WrongUnlock(KeySource::Passphrase))
    ));
    assert!(matches!(
        Store::create(&path, pass("again")),
        Err(StoreError::AlreadyExists(_))
    ));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn keychain_store_needs_its_key() {
    let (_dir, path) = store_path();
    let vault = MemoryVault::default();
    let store = Store::create(&path, KeySetup::Keychain(&vault)).unwrap();
    store.set_server("https://writers.example").unwrap();
    drop(store);
    assert_eq!(vault.len(), 1);
    assert_eq!(Store::key_source(&path).unwrap(), KeySource::Keychain);

    let store = Store::open(&path, Unlock::Keychain(&vault)).unwrap();
    assert!(store.server().unwrap().is_some());
    drop(store);
    // Another computer's keychain doesn't have it.
    let elsewhere = MemoryVault::default();
    assert!(matches!(
        Store::open(&path, Unlock::Keychain(&elsewhere)),
        Err(StoreError::MissingKey)
    ));
    assert!(matches!(
        Store::open(&path, Unlock::Passphrase("anything")),
        Err(StoreError::WrongUnlock(KeySource::Keychain))
    ));
}

#[test]
fn changing_the_server_forgets_the_token() {
    let store = Store::in_memory();
    store.set_server("https://a.example").unwrap();
    store.set_token(Some("t")).unwrap();
    store.set_server("https://a.example").unwrap();
    assert!(store.token().unwrap().is_some());
    store.set_server("https://b.example").unwrap();
    assert!(store.token().unwrap().is_none());
    store.set_token(Some("u")).unwrap();
    store.set_token(None).unwrap();
    assert!(store.token().unwrap().is_none());
}

#[test]
fn one_process_at_a_time() {
    let (_dir, path) = store_path();
    let store = Store::create(&path, pass("p")).unwrap();
    assert!(matches!(
        Store::open(&path, Unlock::Passphrase("p")),
        Err(StoreError::InUse)
    ));
    drop(store);
    Store::open(&path, Unlock::Passphrase("p")).unwrap();
}

#[test]
fn key_changes_rewrap_without_losing_data() {
    let (_dir, path) = store_path();
    let vault = MemoryVault::default();
    let mut store = Store::create(&path, pass("first")).unwrap();
    store.set_token(Some("keep me")).unwrap();

    // Passphrase to passphrase: the old one stops working.
    store.change_key(pass("second"), None).unwrap();
    drop(store);
    assert!(matches!(
        Store::open(&path, Unlock::Passphrase("first")),
        Err(StoreError::WrongKey)
    ));
    let mut store = Store::open(&path, Unlock::Passphrase("second")).unwrap();

    // To the keychain, then a new keychain key: old entries are removed.
    store.change_key(KeySetup::Keychain(&vault), None).unwrap();
    store
        .change_key(KeySetup::Keychain(&vault), Some(&vault))
        .unwrap();
    assert_eq!(vault.len(), 1);
    drop(store);
    let mut store = Store::open(&path, Unlock::Keychain(&vault)).unwrap();
    assert_eq!(
        store.status().unwrap().key_source,
        Some(KeySource::Keychain)
    );

    // Back to a passphrase: the keychain is emptied.
    store.change_key(pass("third"), Some(&vault)).unwrap();
    assert!(vault.is_empty());
    drop(store);
    let store = Store::open(&path, Unlock::Passphrase("third")).unwrap();
    assert_eq!(
        store.token().unwrap().as_deref().map(String::as_str),
        Some("keep me")
    );
}

#[test]
fn the_file_holds_no_plaintext() {
    let (_dir, path) = store_path();
    let store = Store::create(&path, pass("p")).unwrap();
    store.set_server("https://canary-server.example").unwrap();
    store.set_token(Some("qt_CANARY_TOKEN_CANARY")).unwrap();
    drop(store);
    let raw = fs::read(&path).unwrap();
    for needle in [
        &b"canary-server"[..],
        b"CANARY_TOKEN",
        b"profile",
        b"server",
        b"token",
        b"layout",
    ] {
        assert!(
            !raw.windows(needle.len()).any(|w| w == needle),
            "{} found in the store file",
            String::from_utf8_lossy(needle)
        );
    }
}

#[test]
fn a_torn_header_slot_is_survived() {
    let (_dir, path) = store_path();
    let store = Store::create(&path, pass("p")).unwrap();
    store.set_token(Some("t")).unwrap();
    drop(store);
    // Damage the first slot; the second still opens the store.
    let mut raw = fs::read(&path).unwrap();
    raw[40] ^= 0xFF;
    fs::write(&path, &raw).unwrap();
    Store::open(&path, Unlock::Passphrase("p")).unwrap();
    // Both damaged: not a store any more.
    raw[2048 + 40] ^= 0xFF;
    fs::write(&path, &raw).unwrap();
    assert!(matches!(
        Store::open(&path, Unlock::Passphrase("p")),
        Err(StoreError::NotAStore)
    ));
}

#[test]
fn altered_data_is_refused_not_misread() {
    let (_dir, path) = store_path();
    let store = Store::create(&path, pass("p")).unwrap();
    store.set_token(Some("t")).unwrap();
    drop(store);
    let mut raw = fs::read(&path).unwrap();
    // Flip a byte in every ciphertext slot: no page authenticates.
    let header = usize::try_from(HEADER_REGION).unwrap();
    for i in (header + 100..raw.len()).step_by(4144) {
        raw[i] ^= 0x01;
    }
    fs::write(&path, &raw).unwrap();
    let result = Store::open(&path, Unlock::Passphrase("p")).and_then(|s| s.token());
    assert!(result.is_err());
}

#[test]
fn other_files_are_not_stores() {
    let (dir, _) = store_path();
    let path = dir.path().join("notes.txt");
    fs::write(&path, b"just some notes").unwrap();
    assert!(matches!(
        Store::open(&path, Unlock::Passphrase("p")),
        Err(StoreError::NotAStore)
    ));
    fs::write(&path, vec![7u8; 10_000]).unwrap();
    assert!(matches!(
        Store::key_source(&path),
        Err(StoreError::NotAStore)
    ));
}

#[test]
fn redb_backend_conforms_and_persists() {
    let (_dir, path) = store_path();
    let store = Store::create(&path, pass("p")).unwrap();
    kv::conformance(&*store.kv);
    drop(store);
    let store = Store::open(&path, Unlock::Passphrase("p")).unwrap();
    let big = store.kv.get(Table::Profile, &[0, 255, 1, 254]).unwrap();
    assert_eq!(big.map(|v| v.len()), Some(200_000));
}

/// Simulates power loss: after `budget` page writes, every later write and
/// sync is silently dropped, as if the machine stopped.
#[derive(Debug)]
struct CrashAfter {
    inner: Arc<PageFile>,
    budget: std::sync::atomic::AtomicI64,
}

impl redb::StorageBackend for CrashAfter {
    fn len(&self) -> Result<u64, io::Error> {
        self.inner.len()
    }
    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), io::Error> {
        self.inner.read(offset, out)
    }
    fn set_len(&self, len: u64) -> Result<(), io::Error> {
        if self.budget.load(std::sync::atomic::Ordering::SeqCst) <= 0 {
            return Err(io::Error::other("crashed"));
        }
        self.inner.set_len(len)
    }
    fn sync_data(&self) -> Result<(), io::Error> {
        if self.budget.load(std::sync::atomic::Ordering::SeqCst) <= 0 {
            return Err(io::Error::other("crashed"));
        }
        self.inner.sync_data()
    }
    fn write(&self, offset: u64, data: &[u8]) -> Result<(), io::Error> {
        if self
            .budget
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst)
            <= 0
        {
            return Err(io::Error::other("crashed"));
        }
        self.inner.write(offset, data)
    }
}

#[test]
fn a_crash_mid_commit_leaves_the_old_or_the_new_data() {
    use std::sync::atomic::AtomicI64;
    let (_dir, path) = store_path();
    let store = Store::create(&path, pass("p")).unwrap();
    store.set_token(Some("old")).unwrap();
    drop(store);

    let mut outcomes = (0, 0);
    for budget in 0..40 {
        // Open the file by hand, as Store::open does, but with the crash
        // injector between redb and the page file.
        let mut file = File::options().read(true).write(true).open(&path).unwrap();
        let header = Header::read_region(&read_region(&mut file).unwrap()).unwrap();
        let kek = header.kdf.derive("p", &header.salt).unwrap();
        let data_key = header.unwrap_key(&kek).unwrap();
        let pages = Arc::new(
            PageFile::new(file, &page_key(&data_key, &header.file_id), header.file_id).unwrap(),
        );
        let backend = RedbBackend::open_backend(CrashAfter {
            inner: pages,
            budget: AtomicI64::new(budget),
        });
        if let Ok(kv) = backend {
            let big = vec![b'x'; 50_000];
            let _ = kv.apply(&[
                Op::Put {
                    table: Table::Profile,
                    key: b"token",
                    value: b"new",
                },
                Op::Put {
                    table: Table::Profile,
                    key: b"bulk",
                    value: &big,
                },
            ]);
        }

        // After the "power loss" the store must open, with all or nothing.
        let store = Store::open(&path, Unlock::Passphrase("p")).unwrap();
        let token = store.token().unwrap().map(|t| t.as_str().to_owned());
        let bulk = store.kv.get(Table::Profile, b"bulk").unwrap();
        match token.as_deref() {
            Some("old") => {
                assert!(bulk.is_none(), "budget {budget}: half a commit");
                outcomes.0 += 1;
            }
            Some("new") => {
                assert_eq!(bulk.map(|b| b.len()), Some(50_000), "budget {budget}");
                outcomes.1 += 1;
                // Reset for the next round.
                store
                    .kv
                    .apply(&[
                        Op::Put {
                            table: Table::Profile,
                            key: b"token",
                            value: b"old",
                        },
                        Op::Delete {
                            table: Table::Profile,
                            key: b"bulk",
                        },
                    ])
                    .unwrap();
            }
            other => panic!("budget {budget}: token {other:?}"),
        }
    }
    // Both outcomes happened, so the crash points span the commit.
    assert!(outcomes.0 > 0 && outcomes.1 > 0, "{outcomes:?}");
}
