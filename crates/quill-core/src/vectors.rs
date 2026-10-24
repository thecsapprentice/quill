//! Test vectors (protocol draft, section 11) for the client's formats:
//! device linking, backups, attachments and the ordering clock. Generated
//! here from fixed inputs, committed under `test-vectors/`, and checked two
//! ways: regenerating them must reproduce the committed files byte for
//! byte, and every committed case must give its recorded result through
//! the functions the client uses.
//!
//! `age` encryption is randomized, so the backup file's sealed example is
//! kept as committed while it still opens to the expected plaintext; the
//! generator makes a new one only when it doesn't.
//!
//! To rewrite the files after a deliberate change:
//! `QUILL_WRITE_VECTORS=1 cargo test -p quill-core vectors`.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use quill_proto::keys::{b64, unb64};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{
    attachments::{self, Attachment},
    backup::{
        self,
        cipher::{AgeCrate, BackupCipher},
    },
    devices::{self, LinkCode},
    history,
    hlc::Hlc,
    words,
};

fn dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors")
}

fn writing() -> bool {
    std::env::var_os("QUILL_WRITE_VECTORS").is_some()
}

fn committed(name: &str) -> Option<Value> {
    let text = std::fs::read_to_string(dir().join(name)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Regenerates a file's contents and compares (or writes) them.
fn check_file(name: &str, value: &Value) -> Value {
    let path = dir().join(name);
    let text = serde_json::to_string_pretty(value).unwrap() + "\n";
    if writing() {
        std::fs::write(&path, &text).unwrap();
    }
    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} missing; set QUILL_WRITE_VECTORS=1", path.display()));
    assert_eq!(on_disk, text, "{name} differs from what the code generates");
    serde_json::from_str(&on_disk).unwrap()
}

fn hex(bytes: &[u8]) -> String {
    ::hex::encode(bytes)
}

fn unhex(v: &Value) -> Vec<u8> {
    ::hex::decode(v.as_str().unwrap()).unwrap()
}

/// Bytes standing in for a serialized KeyPackage: the fingerprint is over
/// whatever bytes the mailbox holds, so any fixed bytes show the rule.
fn package(seed: u8) -> Vec<u8> {
    (0..300u16)
        .map(|i| u8::try_from((i * 7 + u16::from(seed)) % 256).unwrap())
        .collect()
}

// ---- Device linking -------------------------------------------------------------------------

fn linking() -> Value {
    let pkg = package(1);
    let other = package(2);
    let hosts = ["writers.example", "writers.example:8443"];
    let valid: Vec<Value> = hosts
        .iter()
        .map(|host| {
            let link = devices::pending_link(host, "2c8f0a1b9d3e4f5a6b7c8d9e0f1a2b3c".into(), &pkg);
            json!({
                "host": host,
                "linkId": link.link_id,
                "keyPackage": hex(&pkg),
                "fingerprint": hex(&Sha256::digest(&pkg)),
                "qr": link.code,
                "words": link.words.join(" "),
            })
        })
        .collect();
    let good = devices::pending_link(hosts[0], "2c8f0a1b9d3e4f5a6b7c8d9e0f1a2b3c".into(), &pkg);
    let other_link =
        devices::pending_link(hosts[0], "2c8f0a1b9d3e4f5a6b7c8d9e0f1a2b3c".into(), &other);
    let typed = format!("{} {}", good.link_id, good.words.join(" "));
    let typed_loud = format!(
        "  {}   {}  ",
        good.link_id,
        good.words.join("  ").to_uppercase()
    );
    let mut wrong_words = good.words.clone();
    wrong_words.swap(0, 1);
    json!({
        "description": "Device linking (protocol draft, section 6). The new device shows quill-link:1:<host>:<linkId>:<fp>, where fp is SHA-256 of the serialized KeyPackage (the MLSMessage bytes posted to /link), base64url without padding. The host may carry a port, so a reader splits the rest from the right. The typed form is the link id and 6 words: the first 66 bits of the SHA-256, 11 bits per word, big-endian, BIP-39 English list; any case and spacing. A device approving the link must refuse a KeyPackage whose fingerprint doesn't match.",
        "valid": valid,
        "typed": [typed, typed_loud],
        "mismatched": {
            "name": "the mailbox holds another KeyPackage",
            "code": good.code,
            "typed": typed,
            "keyPackage": hex(&other),
            "itsQr": other_link.code,
        },
        "invalid": {
            "wrongWords": format!("{} {}", good.link_id, wrong_words.join(" ")),
            "malformed": [
                "quill-link:1:writers.example:abc",
                "quill-link:1:writers.example::".to_owned() + &b64(&Sha256::digest(&pkg)),
                "quill-link:1:h:id:short",
                "quill-link:2:writers.example:id:".to_owned() + &b64(&Sha256::digest(&pkg)),
                format!("{} {}", good.link_id, good.words[..5].join(" ")),
                String::new(),
            ],
        },
    })
}

#[test]
fn linking_vectors() {
    let v = check_file("linking.json", &linking());
    for case in v["valid"].as_array().unwrap() {
        let pkg = unhex(&case["keyPackage"]);
        assert_eq!(hex(&Sha256::digest(&pkg)), case["fingerprint"]);
        let code: LinkCode = case["qr"].as_str().unwrap().parse().unwrap();
        assert_eq!(code.link_id, case["linkId"]);
        assert!(code.matches(&pkg));
        assert_eq!(words::words(&Sha256::digest(&pkg)).join(" "), case["words"]);
    }
    let pkg = unhex(&v["valid"][0]["keyPackage"]);
    for typed in v["typed"].as_array().unwrap() {
        let code: LinkCode = typed.as_str().unwrap().parse().unwrap();
        assert!(code.matches(&pkg), "{typed}");
    }
    let m = &v["mismatched"];
    let other = unhex(&m["keyPackage"]);
    for field in ["code", "typed"] {
        let code: LinkCode = m[field].as_str().unwrap().parse().unwrap();
        assert!(!code.matches(&other), "{field}");
    }
    let wrong: LinkCode = v["invalid"]["wrongWords"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(!wrong.matches(&pkg));
    for bad in v["invalid"]["malformed"].as_array().unwrap() {
        assert!(bad.as_str().unwrap().parse::<LinkCode>().is_err(), "{bad}");
    }
}

// ---- Backup ---------------------------------------------------------------------------------

/// The BIP-39 reference vectors for 128-bit entropy (from the Trezor
/// reference implementation), plus one of ours.
const MNEMONICS: [&str; 5] = [
    "00000000000000000000000000000000",
    "7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f",
    "80808080808080808080808080808080",
    "ffffffffffffffffffffffffffffffff",
    "000102030405060708090a0b0c0d0e0f",
];

fn entropy(h: &str) -> [u8; 16] {
    ::hex::decode(h).unwrap().try_into().unwrap()
}

/// The plaintext of the sealed example: the backup's real format, with a
/// few made-up rows.
fn backup_plaintext() -> Vec<u8> {
    let mut tables = BTreeMap::new();
    tables.insert(
        "identities".into(),
        vec![[b64(b"persona id"), b64(b"identity record")]],
    );
    tables.insert(
        "pins".into(),
        vec![[b64(b"actor uri"), b64(b"pinned identity key")]],
    );
    tables.insert("verified".into(), Vec::new());
    tables.insert(
        "messages".into(),
        vec![[b64(b"session note key"), b64(b"{\"seq\":1}")]],
    );
    tables.insert("files".into(), Vec::new());
    serde_json::to_vec(&backup::Contents {
        format: backup::FORMAT.into(),
        version: backup::VERSION,
        created_ms: 1_792_746_000_000,
        tables,
    })
    .unwrap()
}

fn backups() -> Value {
    let codes: Vec<Value> = MNEMONICS
        .iter()
        .map(|h| {
            let e = entropy(h);
            json!({
                "entropy": h,
                "words": words::mnemonic(&e).join(" "),
                "passphrase": *backup::passphrase(&e),
            })
        })
        .collect();
    let code = entropy(MNEMONICS[4]);
    let passphrase = backup::passphrase(&code);
    let plain = backup_plaintext();
    // Keep the committed file while it still opens to this plaintext.
    let sealed = committed("backup.json")
        .and_then(|v| unb64(v["sealed"]["file"].as_str()?))
        .filter(|f| {
            AgeCrate
                .open(&passphrase, f)
                .is_ok_and(|p| p.as_slice() == plain.as_slice())
        })
        .unwrap_or_else(|| {
            assert!(
                writing(),
                "backup.json's sealed file no longer opens; set QUILL_WRITE_VECTORS=1"
            );
            AgeCrate.seal(&passphrase, &plain).unwrap()
        });
    // The last word carries the checksum's 4 bits: the same entropy with
    // one checksum bit flipped.
    let mut bytes = code.to_vec();
    bytes.push(Sha256::digest(code)[0] ^ 0x10);
    let bad_checksum = words::take(&bytes, 12);
    json!({
        "description": "Backups (protocol draft, section 6, as built in 2j). The recovery code is 128 bits of entropy written as a BIP-39 mnemonic: the entropy and the first 4 bits of its SHA-256, 11 bits per word, big-endian, English list. Reading accepts any case and spacing. The age passphrase is the 12 words, lowercase, joined by single spaces. A backup is an age v1 file with one scrypt recipient (work factor 15; readers allow up to 20) over the plaintext: UTF-8 JSON {format: \"quill-backup\", version: 1, createdMs, tables}, where tables maps the client store's identities, pins, verified, messages and files tables to lists of [key, value] in base64url (the rows' contents are the client's own). sealed.file is base64url; it is randomized, so a new implementation checks it opens rather than reproducing it.",
        "codes": codes,
        "acceptedSpellings": [
            words::mnemonic(&code).join(" ").to_uppercase(),
            format!("  {}  ", words::mnemonic(&code).join("   ")),
        ],
        "invalidCodes": [
            { "name": "bad checksum", "words": bad_checksum.join(" ") },
            { "name": "11 words", "words": words::mnemonic(&code)[..11].join(" ") },
            { "name": "13 words", "words": format!("{} abandon", words::mnemonic(&code).join(" ")) },
            { "name": "a word not on the list", "words": format!("{} zebrafish", words::mnemonic(&code)[..11].join(" ")) },
        ],
        "sealed": {
            "entropy": MNEMONICS[4],
            "plaintext": String::from_utf8(plain).unwrap(),
            "file": b64(&sealed),
            "wrongCode": MNEMONICS[0],
        },
    })
}

#[test]
fn backup_vectors() {
    let v = check_file("backup.json", &backups());
    for case in v["codes"].as_array().unwrap() {
        let e = entropy(case["entropy"].as_str().unwrap());
        assert_eq!(words::mnemonic(&e).join(" "), case["words"]);
        assert_eq!(
            words::from_mnemonic(case["words"].as_str().unwrap()),
            Some(e)
        );
        assert_eq!(*backup::passphrase(&e), case["passphrase"]);
    }
    // The BIP-39 reference vectors.
    assert_eq!(
        v["codes"][0]["words"],
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    );
    assert_eq!(
        v["codes"][1]["words"],
        "legal winner thank year wave sausage worth useful legal winner thank yellow"
    );
    assert_eq!(
        v["codes"][2]["words"],
        "letter advice cage absurd amount doctor acoustic avoid letter advice cage above"
    );
    assert_eq!(
        v["codes"][3]["words"],
        "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong"
    );
    let s = &v["sealed"];
    let code = entropy(s["entropy"].as_str().unwrap());
    for spelling in v["acceptedSpellings"].as_array().unwrap() {
        assert_eq!(words::from_mnemonic(spelling.as_str().unwrap()), Some(code));
    }
    for case in v["invalidCodes"].as_array().unwrap() {
        assert_eq!(
            words::from_mnemonic(case["words"].as_str().unwrap()),
            None,
            "{}",
            case["name"]
        );
    }
    let file = unb64(s["file"].as_str().unwrap()).unwrap();
    assert!(file.starts_with(b"age-encryption.org/v1\n-> scrypt "));
    let plain = AgeCrate.open(&backup::passphrase(&code), &file).unwrap();
    assert_eq!(
        plain.as_slice(),
        s["plaintext"].as_str().unwrap().as_bytes()
    );
    let contents: backup::Contents = serde_json::from_slice(&plain).unwrap();
    assert_eq!(
        (contents.format.as_str(), contents.version),
        (backup::FORMAT, backup::VERSION)
    );
    let wrong = entropy(s["wrongCode"].as_str().unwrap());
    assert!(AgeCrate.open(&backup::passphrase(&wrong), &file).is_err());
    let mut damaged = file.clone();
    let n = damaged.len() - 20;
    damaged[n] ^= 1;
    assert!(AgeCrate.open(&backup::passphrase(&code), &damaged).is_err());
    assert!(
        AgeCrate
            .open(&backup::passphrase(&code), &file[..file.len() - 1])
            .is_err()
    );
}

// ---- Attachments ----------------------------------------------------------------------------

const MAX: u64 = quill_proto::limits::DEFAULT_MAX_ATTACHMENT_BYTES;

fn plaintext(len: usize) -> Vec<u8> {
    (0..len).map(|i| u8::try_from(i % 251).unwrap()).collect()
}

fn sealed_case(len: usize, seed: u8) -> (attachments::Sealed, Attachment) {
    let key = Zeroizing::new([seed; 32]);
    let nonce = [seed.wrapping_add(1); 12];
    let s = attachments::seal_with(&plaintext(len), MAX, key, nonce).unwrap();
    let a = s.attachment(
        "notes.txt",
        "https://writers.example/api/v1/blobs/0192f3d0-2222-7000-8000-000000000001".into(),
        "blob-token".into(),
    );
    (s, a)
}

fn attachment_vectors_value() -> Value {
    let valid: Vec<Value> = [(0usize, 1u8), (23, 2), (4080, 3), (4081, 4)]
        .iter()
        .map(|(len, seed)| {
            let (s, a) = sealed_case(*len, *seed);
            let tag_end = *len + 16;
            json!({
                "plaintextLength": len,
                "plaintext": "bytes i % 251 for i in 0..plaintextLength",
                "key": a.key,
                "nonce": a.nonce,
                "contentHash": a.content_hash,
                "size": a.size,
                "blobLength": s.blob.len(),
                "sealed": hex(&s.blob[..tag_end]),
                "blobSha256": hex(&Sha256::digest(&s.blob)),
                "attachment": serde_json::to_value(&a).unwrap(),
            })
        })
        .collect();
    json!({
        "description": "Encrypted attachments (protocol draft, section 5). The file is sealed in one shot with AES-256-GCM (no associated data) under a fresh 256-bit key and 96-bit nonce; the ciphertext and 16-byte tag are padded with zeros to the size bucket (see limits.json), and that is the blob uploaded. The message carries key, nonce and contentHash (SHA-256 of the plaintext), base64url, and size (the plaintext's length). A reader checks the blob's length is the bucket for size + 16, decrypts the first size + 16 bytes, and checks the hash. sealed is the ciphertext and tag before padding; blobSha256 covers the whole padded blob. Keys and nonces here are fixed for the vectors only: a real key is random and used once.",
        "maxAttachmentBytes": MAX,
        "valid": valid,
        "invalid": [
            { "name": "a byte of the tag changed", "change": "flip bit 0 of the blob's byte plaintextLength + 15 in valid[1]" },
            { "name": "a byte of the ciphertext changed", "change": "flip bit 0 of the blob's byte 3 in valid[1]" },
            { "name": "the hash doesn't match", "change": "contentHash of 32 zero bytes in valid[1]" },
            { "name": "the blob isn't its bucket", "change": "append 4096 zero bytes to valid[1]'s blob" },
            { "name": "a size the server doesn't allow", "change": "size set to maxAttachmentBytes in valid[1]" },
            { "name": "an unknown algorithm", "change": "encAlg set to A128GCM in valid[1]" },
        ],
    })
}

/// The blob of a valid case, rebuilt from the vector.
fn blob_of(case: &Value) -> Vec<u8> {
    let mut blob = unhex(&case["sealed"]);
    blob.resize(
        usize::try_from(case["blobLength"].as_u64().unwrap()).unwrap(),
        0,
    );
    blob
}

#[test]
fn attachment_vectors() {
    let v = check_file("attachment.json", &attachment_vectors_value());
    for case in v["valid"].as_array().unwrap() {
        let blob = blob_of(case);
        assert_eq!(hex(&Sha256::digest(&blob)), case["blobSha256"]);
        let a: Attachment = serde_json::from_value(case["attachment"].clone()).unwrap();
        let len = usize::try_from(case["plaintextLength"].as_u64().unwrap()).unwrap();
        assert_eq!(
            attachments::open(&blob, &a, MAX).unwrap().as_slice(),
            plaintext(len).as_slice()
        );
    }
    let case = &v["valid"][1];
    let a: Attachment = serde_json::from_value(case["attachment"].clone()).unwrap();
    let blob = blob_of(case);
    let len = usize::try_from(a.size).unwrap();
    let flip = |i: usize| {
        let mut b = blob.clone();
        b[i] ^= 1;
        b
    };
    assert!(attachments::open(&flip(len + 15), &a, MAX).is_err());
    assert!(attachments::open(&flip(3), &a, MAX).is_err());
    let mut longer = blob.clone();
    longer.extend_from_slice(&[0; 4096]);
    assert!(attachments::open(&longer, &a, MAX).is_err());
    for change in [
        Attachment {
            content_hash: b64(&[0; 32]),
            ..a.clone()
        },
        Attachment {
            size: MAX,
            ..a.clone()
        },
        Attachment {
            enc_alg: "A128GCM".into(),
            ..a.clone()
        },
    ] {
        assert!(attachments::open(&blob, &change, MAX).is_err());
    }
    assert_eq!(v["invalid"].as_array().unwrap().len(), 6);
}

// ---- Ordering clock -------------------------------------------------------------------------

fn h(s: &str) -> Hlc {
    s.parse().unwrap()
}

fn clocks() -> Value {
    let parse_ok: Vec<Value> = [
        "1760861400123-0004",
        "5-0000",
        "0-0000",
        "9999999999999999-9999",
    ]
    .iter()
    .map(|s| {
        let c = h(s);
        json!({ "value": s, "ms": c.ms, "counter": c.counter })
    })
    .collect();
    let send: Vec<Value> = [
        ("0-0000", 1_000_000u64, "a fresh clock takes the wall clock"),
        ("1000000-0000", 1_000_500, "the wall clock moved on: counter back to 0"),
        ("1000000-0003", 1_000_000, "the same millisecond: the counter goes up"),
        ("1000000-0003", 990_000, "the wall clock is behind what was seen: the counter goes up"),
        ("7-9999", 7, "a full counter borrows the next millisecond"),
    ]
    .iter()
    .map(|(last, now, why)| {
        json!({ "last": last, "nowMs": now, "result": Hlc::send(h(last), *now).to_string(), "why": why })
    })
    .collect();
    let ahead = crate::hlc::MAX_AHEAD_MS;
    let read: Vec<Value> = [
        ("0-0000", "1000000-0000".to_owned(), 990_000u64, "a remote value ahead of the local clock moves it"),
        ("1000000-0005", "1000000-0002".to_owned(), 1_000_000, "a value behind it doesn't"),
        ("0-0000", format!("{}-0000", 1_000_000 + ahead), 1_000_000, "at most a day ahead of the wall clock is taken"),
        ("0-0000", format!("{}-0000", 1_000_000 + ahead + 1), 1_000_000, "further ahead is ignored"),
    ]
    .iter()
    .map(|(last, remote, now, why)| {
        json!({ "last": last, "remote": remote, "nowMs": now, "result": Hlc::read(h(last), h(remote), *now).to_string(), "why": why })
    })
    .collect();
    // Shuffled; `order` gives them in display order.
    let notes: [(&str, &str, u64); 7] = [
        ("1000-0001", "https://b.example/users/bo", 3),
        ("1000-0000", "https://b.example/users/bo", 9),
        ("1000-0001", "https://a.example/users/al", 4),
        ("1000-0001", "https://b.example/users/bo", 2),
        ("999-0009", "https://z.example/users/zed", 1),
        ("garbage", "https://z.example/users/zed", 5),
        ("1000-0001", "https://a.example/users/Al", 7),
    ];
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by(|a, b| {
        let (x, y) = (notes[*a], notes[*b]);
        history::display_key(x.0, x.1, x.2).cmp(&history::display_key(y.0, y.1, y.2))
    });
    json!({
        "description": "The hybrid logical clock (protocol draft, section 5): `ms-counter`, the wall clock in milliseconds since the Unix epoch (1-16 ASCII digits) and a 4-digit counter. Sending: if the wall clock is past the last value, (now, 0); otherwise the last value with the counter up one, a full counter (9999) carrying into the next millisecond. Reading a value: the clock becomes the larger of the two, unless the value is more than 24 hours ahead of the wall clock, when it is ignored. Display order sorts by clock (ms, then counter; a malformed value counts as 0-0000), then the sender's actor URI by its UTF-8 bytes, then authorSeq. `order` lists the notes' indices in display order.",
        "parse": { "valid": parse_ok, "invalid": ["", "1-1", "1-00001", "-0001", "x-0001", "1760861400123", "1-00a1", "+1-0001", "1 -0001", "12345678901234567-0000", "1-０００１"] },
        "send": send,
        "read": read,
        "sort": {
            "notes": notes.iter().map(|(c, s, n)| json!({ "hlc": c, "sender": s, "authorSeq": n })).collect::<Vec<_>>(),
            "order": order,
        },
    })
}

#[test]
fn clock_vectors() {
    let v = check_file("ordering-clock.json", &clocks());
    for case in v["parse"]["valid"].as_array().unwrap() {
        let c = h(case["value"].as_str().unwrap());
        assert_eq!(
            (c.ms, u64::from(c.counter)),
            (
                case["ms"].as_u64().unwrap(),
                case["counter"].as_u64().unwrap()
            )
        );
        assert_eq!(c.to_string(), case["value"]);
    }
    for bad in v["parse"]["invalid"].as_array().unwrap() {
        assert!(bad.as_str().unwrap().parse::<Hlc>().is_err(), "{bad}");
    }
    for case in v["send"].as_array().unwrap() {
        let got = Hlc::send(
            h(case["last"].as_str().unwrap()),
            case["nowMs"].as_u64().unwrap(),
        );
        assert_eq!(got.to_string(), case["result"], "{}", case["why"]);
    }
    for case in v["read"].as_array().unwrap() {
        let got = Hlc::read(
            h(case["last"].as_str().unwrap()),
            h(case["remote"].as_str().unwrap()),
            case["nowMs"].as_u64().unwrap(),
        );
        assert_eq!(got.to_string(), case["result"], "{}", case["why"]);
    }
    let notes = v["sort"]["notes"].as_array().unwrap();
    let mut order: Vec<usize> = (0..notes.len()).collect();
    let key = |i: usize| {
        let n = &notes[i];
        history::display_key(
            n["hlc"].as_str().unwrap(),
            n["sender"].as_str().unwrap(),
            n["authorSeq"].as_u64().unwrap(),
        )
    };
    order.sort_by_key(|i| key(*i));
    assert_eq!(serde_json::to_value(order).unwrap(), v["sort"]["order"]);
}

#[test]
fn the_mls_padding_is_the_one_published() {
    let limits = committed("limits.json").expect("limits.json");
    assert_eq!(
        limits["defaults"]["mlsPaddingBytes"].as_u64(),
        Some(u64::try_from(devices::PADDING).unwrap())
    );
}
