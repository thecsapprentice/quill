//! Test vectors (protocol draft, section 11): generated here from fixed
//! seeds, committed under `test-vectors/`, and checked two ways:
//!
//! 1. regenerating them must reproduce the committed files byte for byte
//!    (Ed25519 signatures are deterministic), and
//! 2. every committed case must give its recorded result through the public
//!    verification functions, independently of how it was generated.
//!
//! To rewrite the files after a deliberate change:
//! `QUILL_WRITE_VECTORS=1 cargo test -p quill-certs`.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ed25519_dalek::SigningKey;
use serde_json::{Value, json};

use super::*;

const ACTOR: &str = "https://writers.example/users/ink_fox";
/// 2026-10-23T00:00:00Z, a UTC day start as clients use.
const DAY: u64 = 1_792_713_600_000;
const NOW: u64 = DAY + 9 * 3_600_000;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn hex(bytes: &[u8]) -> String {
    ::hex::encode(bytes)
}

fn unhex(v: &Value) -> Vec<u8> {
    ::hex::decode(v.as_str().unwrap()).unwrap()
}

fn error_name(e: &CertError) -> &'static str {
    match e {
        CertError::Malformed(_) => "malformed",
        CertError::BadSignature => "bad-signature",
        CertError::BadKey => "bad-key",
        CertError::WrongActor => "wrong-actor",
        CertError::WrongDeviceKey => "wrong-device-key",
        CertError::WrongKeyVersion => "wrong-key-version",
        CertError::UnknownFlags => "unknown-flags",
        CertError::UnknownReason => "unknown-reason",
        CertError::NotYetValid => "not-yet-valid",
        CertError::Expired => "expired",
    }
}

/// Signs arbitrary wire contents, for must-reject cases a well-behaved
/// issuer can't produce.
fn sign_cert_wire(tbs: CertTbsWire, identity: &SigningKey, label: &str) -> Vec<u8> {
    let sig = sign_with_label(identity, label, &tbs.tls_serialize_detached().unwrap()).unwrap();
    CertWire {
        tbs,
        signature: sig.as_slice().into(),
    }
    .tls_serialize_detached()
    .unwrap()
}

fn cert(device: &SigningKey) -> DeviceCertificate {
    DeviceCertificate {
        actor_uri: ACTOR.into(),
        device_key: device.verifying_key().to_bytes(),
        identity_key_version: 1,
        limited: false,
        not_before: DAY,
        not_after: None,
    }
}

fn expectation(
    identity: &SigningKey,
    device: Option<&SigningKey>,
    version: u32,
    now: u64,
) -> Value {
    json!({
        "actorUri": ACTOR,
        "identityKey": hex(identity.verifying_key().as_bytes()),
        "identityKeyVersion": version,
        "deviceKey": device.map(|d| hex(d.verifying_key().as_bytes())),
        "now": now,
    })
}

fn labeled_signatures() -> Value {
    let k = key(1);
    let cases: Vec<Value> = [
        (CERT_LABEL, &b"device certificate contents"[..]),
        (REVOKE_LABEL, &b""[..]),
        ("wr rejoin v1", &[0u8, 1, 2, 255][..]),
    ]
    .iter()
    .map(|(label, content)| {
        json!({
            "privateKeySeed": hex(&k.to_bytes()),
            "publicKey": hex(k.verifying_key().as_bytes()),
            "label": label,
            "content": hex(content),
            "signContent": hex(&sign_content(label, content).unwrap()),
            "signature": hex(&sign_with_label(&k, label, content).unwrap()),
        })
    })
    .collect();
    json!({
        "description": "MLS SignWithLabel (RFC 9420, 5.1.2) with Ed25519: the label is signed as \"MLS 1.0 \" + label. signContent is the TLS-encoded SignContent that is signed. Must reject: verifying any case under a different label.",
        "valid": cases,
    })
}

fn device_certificates() -> Value {
    let id = key(1);
    let other_id = key(9);
    let dev = key(2);
    let valid = |name: &str, c: &DeviceCertificate, now: u64| {
        json!({
            "name": name,
            "fields": {
                "actorUri": c.actor_uri,
                "deviceKey": hex(&c.device_key),
                "identityKeyVersion": c.identity_key_version,
                "limited": c.limited,
                "notBefore": c.not_before,
                "notAfter": c.not_after.unwrap_or(0),
            },
            "certificate": hex(&c.issue(&id).unwrap()),
            "expected": expectation(&id, Some(&dev), c.identity_key_version, now),
        })
    };
    let base = cert(&dev);
    let limited = DeviceCertificate {
        limited: true,
        ..base.clone()
    };
    let dated = DeviceCertificate {
        not_after: Some(DAY + 30 * 86_400_000),
        ..base.clone()
    };
    let good = base.issue(&id).unwrap();

    let reject = |name: &str, certificate: Vec<u8>, expected: Value, error: &str| json!({ "name": name, "certificate": hex(&certificate), "expected": expected, "error": error });
    let exp = |version, now| expectation(&id, Some(&dev), version, now);
    let mut truncated = good.clone();
    truncated.pop();
    let mut trailing = good.clone();
    trailing.push(0);
    // actor_uri's length as a 2-byte varint although it fits in one.
    let mut non_minimal = vec![0x40, u8::try_from(ACTOR.len()).unwrap()];
    non_minimal.extend_from_slice(&good[1..]);
    let flagged = sign_cert_wire(
        CertTbsWire {
            flags: 0x02,
            ..base.tbs()
        },
        &id,
        CERT_LABEL,
    );
    // The neutral element: a valid encoding but of small order.
    let mut small_order = [0u8; 32];
    small_order[0] = 1;
    let weak = sign_cert_wire(
        CertTbsWire {
            device_key: small_order.as_slice().into(),
            ..base.tbs()
        },
        &id,
        CERT_LABEL,
    );
    let invalid = vec![
        reject(
            "wrong label",
            sign_cert_wire(base.tbs(), &id, REVOKE_LABEL),
            exp(1, NOW),
            "bad-signature",
        ),
        reject(
            "signed by another identity key",
            base.issue(&other_id).unwrap(),
            exp(1, NOW),
            "bad-signature",
        ),
        reject(
            "device key differs from the leaf's key",
            good.clone(),
            expectation(&id, Some(&key(3)), 1, NOW),
            "wrong-device-key",
        ),
        reject(
            "for another actor",
            DeviceCertificate {
                actor_uri: "https://writers.example/users/other".into(),
                ..base.clone()
            }
            .issue(&id)
            .unwrap(),
            exp(1, NOW),
            "wrong-actor",
        ),
        reject(
            "wrong identity key version",
            good.clone(),
            exp(2, NOW),
            "wrong-key-version",
        ),
        reject(
            "expired",
            dated.issue(&id).unwrap(),
            exp(1, DAY + 30 * 86_400_000),
            "expired",
        ),
        reject(
            "starts in the future",
            DeviceCertificate {
                not_before: NOW + 86_400_000,
                ..base.clone()
            }
            .issue(&id)
            .unwrap(),
            exp(1, NOW),
            "not-yet-valid",
        ),
        reject("unknown flag bits", flagged, exp(1, NOW), "unknown-flags"),
        reject("truncated", truncated, exp(1, NOW), "malformed"),
        reject("trailing bytes", trailing, exp(1, NOW), "malformed"),
        reject(
            "non-minimal length encoding",
            non_minimal,
            exp(1, NOW),
            "malformed",
        ),
        reject("small-order device key", weak, exp(1, NOW), "bad-key"),
    ];
    json!({
        "description": "DeviceCertificate (protocol draft, section 6): TLS encoding, SignWithLabel(identity key, \"wr device cert v1\", DeviceCertificateTBS). Times are milliseconds since the Unix epoch; notAfter 0 means no expiry. A verifier also allows notBefore up to 10 minutes ahead of now for clock skew.",
        "identityKeySeed": hex(&id.to_bytes()),
        "deviceKeySeed": hex(&dev.to_bytes()),
        "valid": [
            valid("full device", &base, NOW),
            valid("limited device", &limited, NOW),
            valid("with an expiry", &dated, NOW),
        ],
        "invalid": invalid,
    })
}

fn revocations() -> Value {
    let id = key(1);
    let dev = key(2);
    let revocation = |reason| DeviceRevocation {
        actor_uri: ACTOR.into(),
        device_key: dev.verifying_key().to_bytes(),
        revoked_at: NOW,
        reason,
    };
    let valid: Vec<Value> = [
        RevocationReason::Unspecified,
        RevocationReason::Lost,
        RevocationReason::Compromised,
        RevocationReason::Retired,
    ]
    .iter()
    .map(|r| {
        json!({
            "reason": r.as_str(),
            "revocation": hex(&revocation(*r).issue(&id).unwrap()),
        })
    })
    .collect();
    let mut tampered = revocation(RevocationReason::Lost).issue(&id).unwrap();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x01;
    let bad_reason = {
        let tbs = RevokeTbsWire {
            reason: 9,
            ..revocation(RevocationReason::Lost).tbs()
        };
        let sig =
            sign_with_label(&id, REVOKE_LABEL, &tbs.tls_serialize_detached().unwrap()).unwrap();
        RevokeWire {
            tbs,
            signature: sig.as_slice().into(),
        }
        .tls_serialize_detached()
        .unwrap()
    };
    let other_actor = DeviceRevocation {
        actor_uri: "https://writers.example/users/other".into(),
        ..revocation(RevocationReason::Lost)
    }
    .issue(&id)
    .unwrap();
    json!({
        "description": "DeviceRevocation: TLS encoding, SignWithLabel(identity key, \"wr device revoke v1\", DeviceRevocationTBS). Reasons: 0 unspecified, 1 lost, 2 compromised, 3 retired.",
        "actorUri": ACTOR,
        "identityKey": hex(id.verifying_key().as_bytes()),
        "deviceKey": hex(dev.verifying_key().as_bytes()),
        "revokedAt": NOW,
        "valid": valid,
        "invalid": [
            { "name": "bad signature", "revocation": hex(&tampered), "error": "bad-signature" },
            { "name": "unknown reason", "revocation": hex(&bad_reason), "error": "unknown-reason" },
            { "name": "for another actor", "revocation": hex(&other_actor), "error": "wrong-actor" },
        ],
    })
}

fn dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors")
}

/// Regenerates a file's contents and compares (or writes) them.
fn check_file(name: &str, value: &Value) -> Value {
    let path = dir().join(name);
    let text = serde_json::to_string_pretty(value).unwrap() + "\n";
    if std::env::var_os("QUILL_WRITE_VECTORS").is_some() {
        std::fs::create_dir_all(dir()).unwrap();
        std::fs::write(&path, &text).unwrap();
    }
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} missing; set QUILL_WRITE_VECTORS=1", path.display()));
    assert_eq!(
        committed, text,
        "{name} differs from what the code generates"
    );
    serde_json::from_str(&committed).unwrap()
}

#[test]
fn labeled_signature_vectors() {
    let v = check_file("labeled-signature.json", &labeled_signatures());
    for case in v["valid"].as_array().unwrap() {
        let label = case["label"].as_str().unwrap();
        let (pk, content, sig) = (
            unhex(&case["publicKey"]),
            unhex(&case["content"]),
            unhex(&case["signature"]),
        );
        assert_eq!(
            sign_content(label, &content).unwrap(),
            unhex(&case["signContent"])
        );
        verify_with_label(&pk, label, &content, &sig).unwrap();
        assert_eq!(
            verify_with_label(&pk, "wr some other label", &content, &sig),
            Err(CertError::BadSignature)
        );
    }
}

fn expected_of(v: &Value) -> (String, Vec<u8>, u32, Option<Vec<u8>>, u64) {
    (
        v["actorUri"].as_str().unwrap().to_owned(),
        unhex(&v["identityKey"]),
        u32::try_from(v["identityKeyVersion"].as_u64().unwrap()).unwrap(),
        (!v["deviceKey"].is_null()).then(|| unhex(&v["deviceKey"])),
        v["now"].as_u64().unwrap(),
    )
}

fn verify_case(cert: &[u8], e: &Value) -> Result<DeviceCertificate, CertError> {
    let (actor, id, version, dev, now) = expected_of(e);
    DeviceCertificate::verify(
        cert,
        &Expected {
            actor_uri: &actor,
            identity_key: &id,
            identity_key_version: version,
            device_key: dev.as_deref(),
            now_ms: now,
        },
    )
}

#[test]
fn device_certificate_vectors() {
    let v = check_file("device-certificate.json", &device_certificates());
    for case in v["valid"].as_array().unwrap() {
        let cert = verify_case(&unhex(&case["certificate"]), &case["expected"])
            .unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
        let f = &case["fields"];
        assert_eq!(cert.actor_uri, f["actorUri"].as_str().unwrap());
        assert_eq!(cert.device_key.to_vec(), unhex(&f["deviceKey"]));
        assert_eq!(cert.limited, f["limited"].as_bool().unwrap());
        assert_eq!(cert.not_after.unwrap_or(0), f["notAfter"].as_u64().unwrap());
    }
    for case in v["invalid"].as_array().unwrap() {
        let err = verify_case(&unhex(&case["certificate"]), &case["expected"])
            .expect_err(case["name"].as_str().unwrap());
        assert_eq!(error_name(&err), case["error"], "{}: {err}", case["name"]);
    }
}

#[test]
fn revocation_vectors() {
    let v = check_file("device-revocation.json", &revocations());
    let actor = v["actorUri"].as_str().unwrap();
    let id = unhex(&v["identityKey"]);
    for case in v["valid"].as_array().unwrap() {
        let r = DeviceRevocation::verify(&unhex(&case["revocation"]), actor, &id).unwrap();
        assert_eq!(r.reason.as_str(), case["reason"]);
        assert_eq!(r.device_key.to_vec(), unhex(&v["deviceKey"]));
    }
    for case in v["invalid"].as_array().unwrap() {
        let err = DeviceRevocation::verify(&unhex(&case["revocation"]), actor, &id)
            .expect_err(case["name"].as_str().unwrap());
        assert_eq!(error_name(&err), case["error"], "{}", case["name"]);
    }
}

#[test]
fn fingerprints_are_stable_and_short() {
    let f = fingerprint(key(1).verifying_key().as_bytes());
    assert_eq!(f.len(), 8 * 4 + 7);
    assert_eq!(f, fingerprint(key(1).verifying_key().as_bytes()));
    assert_ne!(f, fingerprint(key(2).verifying_key().as_bytes()));
}

#[test]
fn damaged_input_is_refused_never_panics() {
    let id = key(1);
    let dev = key(2);
    let cert = DeviceCertificate {
        limited: true,
        not_after: Some(NOW + 1),
        ..cert(&dev)
    }
    .issue(&id)
    .unwrap();
    let revocation = DeviceRevocation {
        actor_uri: ACTOR.into(),
        device_key: dev.verifying_key().to_bytes(),
        revoked_at: NOW,
        reason: RevocationReason::Lost,
    }
    .issue(&id)
    .unwrap();
    let id_pk = id.verifying_key().to_bytes();
    let expected = Expected {
        actor_uri: ACTOR,
        identity_key: &id_pk,
        identity_key_version: 1,
        device_key: None,
        now_ms: NOW,
    };
    let check = |c: &[u8], r: &[u8]| {
        let _ = DeviceCertificate::verify(c, &expected);
        let _ = DeviceCertificate::decode(c);
        let _ = DeviceRevocation::verify(r, ACTOR, &id_pk);
    };
    // Every truncation, and every single-byte change (all must fail).
    for n in 0..cert.len() {
        assert!(DeviceCertificate::verify(&cert[..n], &expected).is_err());
    }
    for n in 0..revocation.len() {
        assert!(DeviceRevocation::verify(&revocation[..n], ACTOR, &id_pk).is_err());
    }
    for i in 0..cert.len() {
        for bit in 0..8 {
            let mut c = cert.clone();
            c[i] ^= 1 << bit;
            assert!(
                DeviceCertificate::verify(&c, &expected).is_err(),
                "byte {i} bit {bit}"
            );
        }
    }
    // Pseudo-random garbage and splices (xorshift; deterministic).
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for _ in 0..5000 {
        let len = usize::try_from(next() % 300).unwrap();
        let junk: Vec<u8> = (0..len).map(|_| (next() & 0xff) as u8).collect();
        check(&junk, &junk);
        let mut c = cert.clone();
        let at = usize::try_from(next()).unwrap() % c.len();
        c.splice(at..at, junk.iter().copied().take(8));
        check(&c, &c);
    }
}
