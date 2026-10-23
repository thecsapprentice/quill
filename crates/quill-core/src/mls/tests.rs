// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ed25519_dalek::SigningKey;
use openmls::prelude::{
    GroupId, MlsGroup, MlsGroupCreateConfig, MlsGroupJoinConfig, MlsMessageIn,
    ProcessedMessageContent, StagedWelcome,
};
use tls_codec::Deserialize as _;

use super::*;
use crate::{
    keys::{now_ms, test_device},
    store::{KdfParams, KeySetup, Unlock, kv::Table},
};

const ALICE: &str = "https://writers.example/users/ink_fox";
const BOB: &str = "https://writers.example/users/salt_wren";

fn identity(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn expect<'a>(actor: &'a str, id: &'a [u8], device: &'a [u8]) -> PackageExpect<'a> {
    PackageExpect {
        actor_uri: actor,
        identity_key: id,
        identity_key_version: 1,
        device_key: device,
        now_ms: now_ms(),
    }
}

fn now_secs() -> u64 {
    now_ms() / 1000
}

#[test]
fn key_packages_carry_a_certificate_and_check_out() {
    let store = Store::in_memory();
    let id = identity(1);
    let dev = test_device(&id, 1, ALICE);
    let packages = make_key_packages(&store, ALICE, &dev, 3, false, now_secs()).unwrap();
    let last = make_key_packages(&store, ALICE, &dev, 1, true, now_secs()).unwrap();
    assert_eq!(packages.len(), 3);
    let id_pk = id.verifying_key().to_bytes();
    let dev_pk = dev.public();
    for (bytes, last_resort) in packages
        .iter()
        .map(|p| (p, false))
        .chain([(&last[0], true)])
    {
        check_key_package(bytes, &expect(ALICE, &id_pk, &dev_pk)).unwrap();
        // The server's structural parser agrees with OpenMLS's encoding.
        let info = quill_mls_envelope::key_package::parse(bytes).unwrap();
        assert_eq!(info.cipher_suite, 1);
        assert_eq!(info.identity, ALICE.as_bytes());
        assert_eq!(info.signature_key, dev_pk);
        assert!(info.leaf_extensions.contains(&DEVICE_CERT_EXTENSION));
        assert_eq!(info.last_resort, last_resort);
        // Day-aligned lifetimes: no telling timestamps.
        assert_eq!(info.not_before % DAY_SECS, 0);
        assert_eq!(
            info.not_after - info.not_before,
            (KEY_PACKAGE_DAYS + 1) * DAY_SECS
        );
    }
    // Each package's private keys were saved, in one batch per call.
    assert!(store.scan_count(Table::Mls) >= 4);
}

#[test]
fn claimed_packages_that_arent_ours_are_refused() {
    let store = Store::in_memory();
    let id = identity(1);
    let id_pk = id.verifying_key().to_bytes();
    let dev = test_device(&id, 1, ALICE);
    let dev_pk = dev.public();
    let good = make_key_packages(&store, ALICE, &dev, 1, false, now_secs())
        .unwrap()
        .remove(0);

    let refused = |bytes: &[u8], e: PackageExpect<'_>, why: &str| {
        let err = check_key_package(bytes, &e).unwrap_err().to_string();
        assert!(err.contains(why), "{why}: {err}");
    };
    // Expected for someone else, another device, another identity key.
    refused(&good, expect(BOB, &id_pk, &dev_pk), "another actor");
    let other_dev = test_device(&id, 1, ALICE);
    refused(
        &good,
        expect(ALICE, &id_pk, &other_dev.public()),
        "this device's key",
    );
    let other_id = identity(2).verifying_key().to_bytes();
    refused(
        &good,
        expect(ALICE, &other_id, &dev_pk),
        "device certificate",
    );
    // Tampered anywhere.
    for i in [10, good.len() / 2, good.len() - 5] {
        let mut bad = good.clone();
        bad[i] ^= 0x01;
        assert!(check_key_package(&bad, &expect(ALICE, &id_pk, &dev_pk)).is_err());
    }
    // A device carrying another device's certificate (a server swapping
    // certificates between leaves).
    let mut borrowed = test_device(&id, 1, ALICE);
    borrowed.certificate = other_dev.certificate.clone();
    let swapped = make_key_packages(&store, ALICE, &borrowed, 1, false, now_secs())
        .unwrap()
        .remove(0);
    refused(
        &swapped,
        expect(ALICE, &id_pk, &borrowed.public()),
        "device certificate",
    );
    // An uncertified device (certificate from an attacker's identity key).
    let rogue = test_device(&identity(9), 1, ALICE);
    let uncertified = make_key_packages(&store, ALICE, &rogue, 1, false, now_secs())
        .unwrap()
        .remove(0);
    refused(
        &uncertified,
        expect(ALICE, &id_pk, &rogue.public()),
        "device certificate",
    );
    // Not a KeyPackage at all.
    refused(
        b"\x00\x01\x00\x05junk",
        expect(ALICE, &id_pk, &dev_pk),
        "refused",
    );
}

#[test]
fn a_failed_batch_leaves_no_mls_state() {
    let store = Store::in_memory();
    let provider = Provider::new(&store);
    let result: Result<(), CoreError> = provider.atomically(|p| {
        use openmls_traits::storage::StorageProvider as _;
        // Any write OpenMLS might make, then a failure.
        p.storage()
            .write_own_leaf_index(
                &GroupId::from_slice(b"g"),
                &openmls::prelude::LeafNodeIndex::new(3),
            )
            .unwrap();
        Err(CoreError::Keys("boom".into()))
    });
    assert!(result.is_err());
    assert_eq!(store.scan_count(Table::Mls), 0);
}

fn welcome_of(message: MlsMessageOut) -> openmls::prelude::Welcome {
    let bytes = message.tls_serialize_detached().unwrap();
    match MlsMessageIn::tls_deserialize_exact(bytes)
        .unwrap()
        .extract()
    {
        MlsMessageBodyIn::Welcome(w) => w,
        _ => panic!("not a Welcome"),
    }
}

fn protocol(message: MlsMessageOut) -> openmls::prelude::ProtocolMessage {
    let bytes = message.tls_serialize_detached().unwrap();
    MlsMessageIn::tls_deserialize_exact(bytes)
        .unwrap()
        .try_into_protocol_message()
        .unwrap()
}

fn text(group: &mut MlsGroup, provider: &Provider<'_>, message: MlsMessageOut) -> Vec<u8> {
    let processed = provider
        .atomically(|p| {
            group
                .process_message(p, protocol(message))
                .map_err(|e| mls_err("process", e))
        })
        .unwrap();
    match processed.into_content() {
        ProcessedMessageContent::ApplicationMessage(m) => m.into_bytes(),
        _ => panic!("not an application message"),
    }
}

/// A whole two-person conversation through the store-backed storage,
/// surviving both stores being closed and reopened mid-way.
#[test]
fn a_group_runs_on_the_encrypted_store_and_survives_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let (a_path, b_path) = (dir.path().join("a"), dir.path().join("b"));
    let setup = || KeySetup::Passphrase {
        passphrase: "p",
        kdf: KdfParams::INSECURE_FOR_TESTS,
    };
    let (a_id, b_id) = (identity(1), identity(2));
    let a_dev = test_device(&a_id, 1, ALICE);
    let b_dev = test_device(&b_id, 1, BOB);
    let group_id;
    {
        let a_store = Store::create(&a_path, setup()).unwrap();
        let b_store = Store::create(&b_path, setup()).unwrap();
        let (a, b) = (Provider::new(&a_store), Provider::new(&b_store));

        // Bob publishes a package; Alice claims and checks it.
        let package = make_key_packages(&b_store, BOB, &b_dev, 1, false, now_secs())
            .unwrap()
            .remove(0);
        let b_id_pk = b_id.verifying_key().to_bytes();
        let checked = check_key_package(&package, &expect(BOB, &b_id_pk, &b_dev.public())).unwrap();

        let config = MlsGroupCreateConfig::builder()
            .ciphersuite(CIPHERSUITE)
            .capabilities(capabilities())
            .with_leaf_node_extensions(
                Extensions::single(Extension::Unknown(
                    DEVICE_CERT_EXTENSION,
                    UnknownExtension(a_dev.certificate.clone()),
                ))
                .unwrap(),
            )
            .unwrap()
            .use_ratchet_tree_extension(true)
            .build();
        let signer = DeviceSigner(a_dev.signing_key());
        let mut alice = a
            .atomically(|p| {
                MlsGroup::new(p, &signer, &config, credential(ALICE, &a_dev))
                    .map_err(|e| mls_err("new group", e))
            })
            .unwrap();
        group_id = alice.group_id().clone();
        let (_, welcome, _) = a
            .atomically(|p| {
                let out = alice
                    .add_members(p, &signer, std::slice::from_ref(&checked))
                    .map_err(|e| mls_err("add", e))?;
                alice
                    .merge_pending_commit(p)
                    .map_err(|e| mls_err("merge", e))?;
                Ok(out)
            })
            .unwrap();

        let join = MlsGroupJoinConfig::builder()
            .use_ratchet_tree_extension(true)
            .build();
        let mut bob = b
            .atomically(|p| {
                StagedWelcome::new_from_welcome(p, &join, welcome_of(welcome), None)
                    .and_then(|s| s.into_group(p))
                    .map_err(|e| mls_err("join", e))
            })
            .unwrap();
        let hello = a
            .atomically(|p| {
                alice
                    .create_message(p, &signer, b"the ledger, chapter one")
                    .map_err(|e| mls_err("send", e))
            })
            .unwrap();
        assert_eq!(text(&mut bob, &b, hello), b"the ledger, chapter one");
    }

    // Both stores closed; reopen and carry on in both directions.
    let a_store = Store::open(&a_path, Unlock::Passphrase("p")).unwrap();
    let b_store = Store::open(&b_path, Unlock::Passphrase("p")).unwrap();
    let (a, b) = (Provider::new(&a_store), Provider::new(&b_store));
    let mut alice = MlsGroup::load(a.storage(), &group_id).unwrap().unwrap();
    let mut bob = MlsGroup::load(b.storage(), &group_id).unwrap().unwrap();
    let reply = b
        .atomically(|p| {
            bob.create_message(p, &DeviceSigner(b_dev.signing_key()), b"chapter two")
                .map_err(|e| mls_err("send", e))
        })
        .unwrap();
    assert_eq!(text(&mut alice, &a, reply), b"chapter two");
    let more = a
        .atomically(|p| {
            alice
                .create_message(p, &DeviceSigner(a_dev.signing_key()), b"chapter three")
                .map_err(|e| mls_err("send", e))
        })
        .unwrap();
    assert_eq!(text(&mut bob, &b, more), b"chapter three");

    // And none of it is readable in the files.
    drop((a, b));
    drop((a_store, b_store));
    for path in [&a_path, &b_path] {
        let raw = std::fs::read(path).unwrap();
        for needle in [&b"chapter"[..], b"ink_fox", b"salt_wren", b"GroupState"] {
            assert!(!raw.windows(needle.len()).any(|w| w == needle));
        }
    }
}
