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
    activity::{Activity, Object},
    attachments::{self, Attachment},
    backup::{
        self,
        cipher::{AgeCrate, BackupCipher},
    },
    devices::{
        self, BundlePart, GrantedIdentity, GrantedPin, HistoryBundle, LinkCode, Payload,
        PersonaDevice,
    },
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

use ruzstd::encoding::CompressionLevel as Level;

fn compress(data: &[u8], level: Level) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(data, level)
}

fn entropy(h: &str) -> [u8; 16] {
    ::hex::decode(h).unwrap().try_into().unwrap()
}

/// The JSON inside the sealed example: the backup's real format, with a
/// few made-up rows.
fn backup_json(version: u32) -> Vec<u8> {
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
        version,
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
    let json = backup_json(backup::VERSION);
    let opens_to_json =
        |plain: &[u8]| backup::unpack(plain).is_ok_and(|c| serde_json::to_vec(&c).unwrap() == json);
    // Keep the committed file while it still opens to this JSON.
    let sealed = committed("backup.json")
        .and_then(|v| unb64(v["sealed"]["file"].as_str()?))
        .filter(|f| {
            AgeCrate
                .open(&passphrase, f)
                .is_ok_and(|p| opens_to_json(&p))
        })
        .unwrap_or_else(|| {
            assert!(
                writing(),
                "backup.json's sealed file no longer opens; set QUILL_WRITE_VECTORS=1"
            );
            let contents = backup::unpack(&compress(&json, Level::Fastest)).unwrap();
            AgeCrate
                .seal(&passphrase, &backup::pack(&contents).unwrap())
                .unwrap()
        });
    // The last word carries the checksum's 4 bits: the same entropy with
    // one checksum bit flipped.
    let mut bytes = code.to_vec();
    bytes.push(Sha256::digest(code)[0] ^ 0x10);
    let bad_checksum = words::take(&bytes, 12);
    json!({
        "description": "Backups (protocol draft, section 6, as built in 2j). The recovery code is 128 bits of entropy written as a BIP-39 mnemonic: the entropy and the first 4 bits of its SHA-256, 11 bits per word, big-endian, English list. Reading accepts any case and spacing. The age passphrase is the 12 words, lowercase, joined by single spaces. A backup is an age v1 file with one scrypt recipient (work factor 15; readers allow up to 20) over the plaintext: one zstd frame (RFC 8878) holding UTF-8 JSON {format: \"quill-backup\", version: 2, createdMs, tables}, where tables maps the client store's identities, pins, verified, messages and files tables to lists of [key, value] in base64url (the rows' contents are the client's own). Readers decompress at most 1 GiB. Compressors differ, so any valid frame must be read (uncompressedFrame stores the same JSON without compression). sealed.file is base64url; it is randomized, so a new implementation checks it opens rather than reproducing it. invalidPlaintexts must be refused after the file opens.",
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
            "json": String::from_utf8(json.clone()).unwrap(),
            "file": b64(&sealed),
            "wrongCode": MNEMONICS[0],
        },
        "uncompressedFrame": hex(&compress(&json, Level::Uncompressed)),
        "invalidPlaintexts": [
            { "name": "JSON that isn't compressed (version 1)", "plaintext": hex(&backup_json(1)) },
            { "name": "a compressed version 1", "plaintext": hex(&compress(&backup_json(1), Level::Fastest)) },
            { "name": "a truncated frame", "plaintext": hex(&{
                let mut f = compress(&json, Level::Fastest);
                f.truncate(f.len() / 2);
                f
            }) },
        ],
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
    let json = s["json"].as_str().unwrap().as_bytes();
    let contents = backup::unpack(&plain).unwrap();
    assert_eq!(serde_json::to_vec(&contents).unwrap(), json);
    assert_eq!(
        (contents.format.as_str(), contents.version),
        (backup::FORMAT, backup::VERSION)
    );
    let raw = backup::unpack(&unhex(&v["uncompressedFrame"])).unwrap();
    assert_eq!(serde_json::to_vec(&raw).unwrap(), json);
    for case in v["invalidPlaintexts"].as_array().unwrap() {
        assert!(
            backup::unpack(&unhex(&case["plaintext"])).is_err(),
            "{}",
            case["name"]
        );
    }
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

// ---- JSON objects and their schemas ---------------------------------------------------------

const SCHEMAS: [&str; 4] = [
    "activity.json",
    "encrypted-attachment.json",
    "device-payload.json",
    "history-rows.json",
];
const SCHEMA_BASE: &str = "https://quill.example/ns/wr/schemas/1/";

fn schema_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas")
}

/// The schemas, compiled: `name → index`.
fn compiled() -> (boon::Schemas, BTreeMap<&'static str, boon::SchemaIndex>) {
    let mut compiler = boon::Compiler::new();
    for name in SCHEMAS {
        let text = std::fs::read_to_string(schema_dir().join(name)).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["$id"], format!("{SCHEMA_BASE}{name}"), "{name}'s $id");
        compiler
            .add_resource(&format!("{SCHEMA_BASE}{name}"), value)
            .unwrap();
    }
    let mut schemas = boon::Schemas::new();
    let indexes = SCHEMAS
        .iter()
        .map(|name| {
            let i = compiler
                .compile(&format!("{SCHEMA_BASE}{name}"), &mut schemas)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            (*name, i)
        })
        .collect();
    (schemas, indexes)
}

/// Whether the client's own reader takes `object` as a `schema` object.
fn reader_takes(schema: &str, object: &Value) -> bool {
    match schema {
        "activity.json" => serde_json::from_value::<Activity>(object.clone()).is_ok(),
        "encrypted-attachment.json" => serde_json::from_value::<Attachment>(object.clone()).is_ok(),
        "device-payload.json" => serde_json::from_value::<Payload>(object.clone()).is_ok(),
        "history-rows.json" => serde_json::from_value::<history::Rows>(object.clone()).is_ok(),
        other => panic!("no reader for {other}"),
    }
}

fn urn(n: u8) -> String {
    format!("urn:uuid:0192f3d0-1111-7222-8333-4444555566{n:02x}")
}

fn uuid(n: u8) -> uuid::Uuid {
    format!("0192f3d0-2222-7000-8000-0000000000{n:02x}")
        .parse()
        .unwrap()
}

/// An activity the client makes, with fixed ids in place of random ones.
fn fixed(mut a: Activity, n: u8) -> Value {
    a.id = urn(n);
    if let Object::Note(note) = &mut a.object
        && a.kind == "Create"
    {
        note.id = urn(n + 1);
    }
    serde_json::to_value(a).unwrap()
}

fn attachment_example() -> Attachment {
    sealed_case(23, 2).1
}

fn with(mut v: Value, path: &[&str], value: Value) -> Value {
    let mut at = &mut v;
    for p in &path[..path.len() - 1] {
        at = &mut at[*p];
    }
    at[path[path.len() - 1]] = value;
    v
}

fn without(mut v: Value, path: &[&str]) -> Value {
    let mut at = &mut v;
    for p in &path[..path.len() - 1] {
        at = &mut at[*p];
    }
    at.as_object_mut().unwrap().remove(path[path.len() - 1]);
    v
}

fn json_objects() -> Value {
    let clock = h("1792746000123-0000");
    let note = fixed(
        Activity::create("The ledger's last page was still wet.", "story", clock, 1),
        1,
    );
    let note_id = urn(2);
    let reply = fixed(
        Activity::create("See planning.", "planning", h("1792746000200-0000"), 2)
            .replying_to(&note_id),
        3,
    );
    let with_file = fixed(
        Activity::create("A map.", "story", h("1792746000300-0000"), 3)
            .with_attachments(vec![attachment_example()]),
        5,
    );
    let update = fixed(
        Activity::update(
            &note_id,
            "The ledger's last page was dry.",
            "story",
            h("1792746000400-0000"),
            4,
        ),
        7,
    );
    let delete = fixed(
        Activity::delete(&note_id, "story", h("1792746000500-0000"), 5),
        8,
    );
    let named = fixed(
        Activity::channel("Create", "world", None, h("1792746000600-0000"), 6),
        9,
    );
    let described = fixed(
        Activity::channel(
            "Update",
            "world",
            Some("Places and people"),
            h("1792746000700-0000"),
            7,
        ),
        10,
    );
    let no_channel = without(note.clone(), &["channel"]);

    let request = serde_json::to_value(Payload::HistoryRequest {
        session: b64(&[7; 32]),
        from: 1,
        to: u64::MAX,
    })
    .unwrap();
    let bundle = serde_json::to_value(Payload::HistoryBundle(HistoryBundle {
        for_device: uuid(1),
        session: b64(&[7; 32]),
        from: 1,
        to: 40,
        parts: vec![
            BundlePart {
                file: None,
                blob: attachment_example(),
            },
            BundlePart {
                file: Some(attachment_example().url),
                blob: attachment_example(),
            },
        ],
        last: true,
    }))
    .unwrap();
    let grant = serde_json::to_value(Payload::DeviceGrant {
        identities: vec![GrantedIdentity {
            persona: uuid(2),
            key_version: 1,
            key: b64(&[1; 32]),
        }],
        pins: vec![GrantedPin {
            actor_uri: "https://writers.example/users/bo".into(),
            key: b64(&[3; 32]),
            key_version: 2,
        }],
        devices: vec![devices::DeviceAnnouncement {
            device: uuid(3),
            personas: vec![PersonaDevice {
                persona: uuid(2),
                device: uuid(4),
            }],
            name: Some("Study desktop".into()),
            since: Some(1_790_000_000),
        }],
    })
    .unwrap();
    let device_keys = serde_json::to_value(Payload::DeviceKeys {
        device: uuid(3),
        personas: vec![PersonaDevice {
            persona: uuid(2),
            device: uuid(4),
        }],
        name: Some("Study desktop".into()),
        since: Some(1_790_000_000),
    })
    .unwrap();

    let rows = serde_json::to_value(history::Rows {
        notes: vec![history::Message {
            seq: 12,
            id: urn(2),
            sender: "https://writers.example/users/ink_fox".into(),
            device: b64(&[5; 32]),
            channel: "story".into(),
            hlc: "1792746000123-0000".into(),
            author_seq: 1,
            content: "The ledger's last page was dry.".into(),
            versions: vec![
                history::Version {
                    seq: 12,
                    hlc: "1792746000123-0000".into(),
                    content: Some("The ledger's last page was still wet.".into()),
                },
                history::Version {
                    seq: 15,
                    hlc: "1792746000400-0000".into(),
                    content: Some("The ledger's last page was dry.".into()),
                },
            ],
            deleted: false,
            own: false,
            in_reply_to: None,
            attachments: Vec::new(),
        }],
        channels: vec![history::ChannelInfo {
            name: "world".into(),
            summary: Some("Places and people".into()),
            versions: vec![history::Version {
                seq: 13,
                hlc: "1792746000700-0000".into(),
                content: Some("Places and people".into()),
            }],
            notes: 0,
            unseen: 0,
        }],
        seen: vec![(b64(&[5; 32]), [1u64, 2, 4].into_iter().collect())],
    })
    .unwrap();

    let case = |schema: &str, name: &str, object: Value| json!({ "schema": schema, "name": name, "object": object });
    let reject = |schema: &str, name: &str, object: Value, why: &str| {
        let reader = reader_takes(schema, &object);
        json!({ "schema": schema, "name": name, "object": object, "readerTakes": reader, "why": why })
    };
    let attachment = serde_json::to_value(attachment_example()).unwrap();
    json!({
        "description": "JSON objects (protocol draft, sections 5 and 11) against the schemas in schemas/. Every `valid` object is what the client produces and passes its schema; `unknownTerms` adds terms nobody defined, which schema and reader both accept; `invalid` objects fail the schema. readerTakes says whether the client's reader still parses one (then it discards it or reads it leniently, as `why` says); a reader must never take an object missing a required field.",
        "valid": [
            case("activity.json", "a note", note.clone()),
            case("activity.json", "a reply in another channel", reply),
            case("activity.json", "a note with a file", with_file),
            case("activity.json", "an edit", update),
            case("activity.json", "a deletion", delete.clone()),
            case("activity.json", "naming a channel", named),
            case("activity.json", "describing a channel", described.clone()),
            case("activity.json", "no channel (story)", no_channel),
            case("encrypted-attachment.json", "an attachment", attachment.clone()),
            case("device-payload.json", "a history request for an ended session", request.clone()),
            case("device-payload.json", "a history bundle", bundle.clone()),
            case("device-payload.json", "a device grant", grant.clone()),
            case("device-payload.json", "device keys", device_keys),
            case("history-rows.json", "rows", rows.clone()),
        ],
        "unknownTerms": [
            case("activity.json", "on the activity and the note", with(with(note.clone(), &["wr:mood"], json!("tense")), &["object", "summary"], json!("ignored"))),
            case("encrypted-attachment.json", "an image's size", with(with(attachment.clone(), &["width"], json!(1200)), &["height"], json!(800))),
            case("device-payload.json", "a later field", with(request.clone(), &["priority"], json!(3))),
            case("history-rows.json", "a later table", with(rows.clone(), &["reactions"], json!([]))),
        ],
        "invalid": [
            reject("activity.json", "no object", without(note.clone(), &["object"]), "required"),
            reject("activity.json", "no id", without(note.clone(), &["id"]), "required"),
            reject("activity.json", "no @context", without(note.clone(), &["@context"]), "required"),
            reject("activity.json", "a note without content", without(note.clone(), &["object", "content"]), "required"),
            reject("activity.json", "a channel without a name", without(described.clone(), &["object", "name"]), "required"),
            reject("activity.json", "a Read activity", with(note.clone(), &["type"], json!("Read")), "never sent; a reader discards it"),
            reject("activity.json", "a malformed clock", with(note.clone(), &["hlc"], json!("12-1")), "a reader reads it as 0-0000"),
            reject("activity.json", "a malformed channel", with(note.clone(), &["channel"], json!("Story")), "a reader files the note under story"),
            reject("activity.json", "authorSeq 0", with(note.clone(), &["authorSeq"], json!(0)), "counts start at 1"),
            reject("activity.json", "a deletion carrying a note", with(delete, &["object"], note["object"].clone()), "a reader deletes the note it names"),
            reject("activity.json", "an id that isn't a urn:uuid", with(note.clone(), &["id"], json!("https://writers.example/notes/1")), "ids inside sessions are urn:uuid"),
            reject("encrypted-attachment.json", "no key", without(attachment.clone(), &["key"]), "required"),
            reject("encrypted-attachment.json", "another cipher", with(attachment.clone(), &["encAlg"], json!("A128GCM")), "a reader refuses to open it"),
            reject("encrypted-attachment.json", "a 16-byte key", with(attachment.clone(), &["key"], json!(b64(&[1; 16]))), "a reader refuses to open it"),
            reject("device-payload.json", "a bundle without last", without(bundle.clone(), &["last"]), "required"),
            reject("device-payload.json", "a type nobody defined", with(request.clone(), &["type"], json!("settings")), "a reader skips it"),
            reject("device-payload.json", "a request without to", without(request, &["to"]), "required"),
            reject("device-payload.json", "an identity key that isn't 32 bytes", with(grant, &["identities"], json!([{ "persona": uuid(2), "keyVersion": 1, "key": b64(&[1; 31]) }])), "a reader refuses the grant"),
            reject("device-payload.json", "11 parts", with(bundle.clone(), &["parts"], Value::Array(vec![bundle["parts"][0].clone(); 11])), "senders split bundles into messages of at most 10"),
            reject("history-rows.json", "a note without versions", with(rows.clone(), &["notes"], json!([without(rows["notes"][0].clone(), &["versions"])])), "required"),
        ],
    })
}

#[test]
fn json_object_vectors() {
    let v = check_file("json-objects.json", &json_objects());
    let (schemas, index) = compiled();
    let valid = |case: &Value| {
        let i = index[case["schema"].as_str().unwrap()];
        schemas
            .validate(&case["object"], i)
            .map_err(|e| e.to_string())
    };
    for group in ["valid", "unknownTerms"] {
        for case in v[group].as_array().unwrap() {
            valid(case).unwrap_or_else(|e| panic!("{group} {}: {e}", case["name"]));
            assert!(
                reader_takes(case["schema"].as_str().unwrap(), &case["object"]),
                "the reader refuses {group} {}",
                case["name"]
            );
        }
    }
    for case in v["invalid"].as_array().unwrap() {
        assert!(valid(case).is_err(), "the schema takes {}", case["name"]);
        let reader = reader_takes(case["schema"].as_str().unwrap(), &case["object"]);
        assert_eq!(reader, case["readerTakes"], "{}", case["name"]);
        if case["why"] == "required" {
            assert!(
                !reader,
                "a reader takes {} missing a required field",
                case["name"]
            );
        }
    }
}
