//! Phase 2b end-to-end tests: identity keys per persona, device
//! certificates and the server's device directory.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use ed25519_dalek::SigningKey;
use quill_certs::{DeviceCertificate, DeviceRevocation, RevocationReason};
use quill_core::{
    Client, CoreError, Store,
    keys::{self, DeviceStatus, PinStatus},
};
use quill_e2e::TestServer;
use quill_proto::{
    api::{CreatePersona, Persona},
    keys::{RegisterDevice, RevokeDevice, SetIdentityKey, b64, unb64},
};
use uuid::Uuid;

macro_rules! server {
    () => {
        match TestServer::start().await.unwrap() {
            Some(s) => s,
            None => return,
        }
    };
}

fn status(err: &CoreError) -> u16 {
    err.status().unwrap_or_default()
}

async fn persona(client: &Client, handle: &str) -> Persona {
    client
        .create_persona(&CreatePersona {
            handle: handle.into(),
            display_name: None,
            summary: None,
        })
        .await
        .unwrap()
}

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn cert(identity: &SigningKey, actor_uri: &str, device: &SigningKey, version: u32) -> String {
    b64(&DeviceCertificate {
        actor_uri: actor_uri.into(),
        device_key: device.verifying_key().to_bytes(),
        identity_key_version: version,
        limited: false,
        not_before: keys::day_start(keys::now_ms()),
        not_after: None,
    }
    .issue(identity)
    .unwrap())
}

fn statuses(c: &keys::CheckedKeys) -> Vec<DeviceStatus> {
    c.devices.iter().map(|d| d.status.clone()).collect()
}

#[tokio::test]
async fn a_persona_publishes_keys_that_others_can_check() {
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    let pen = persona(&alice, "ink_fox").await;
    assert_eq!(pen.actor_uri, "https://localhost/users/ink_fox");
    let store = Store::in_memory();

    // Nothing published yet.
    let public = server.client().actor_keys("ink_fox").await.unwrap();
    assert!(public.identity_key.is_none() && public.devices.is_empty());

    let out = keys::init(&store, &alice, &pen, false).await.unwrap();
    assert!(out.new_identity && out.new_device);
    assert_eq!(out.identity_version, 1);
    // Running it again changes nothing.
    let again = keys::init(&store, &alice, &pen, false).await.unwrap();
    assert!(!again.new_identity && !again.new_device);
    assert_eq!(again.device, out.device);

    // Anyone, signed in or not, can fetch and check them.
    let public = server.client().actor_keys("INK_FOX").await.unwrap();
    let checked = keys::check_published(&public, &pen.actor_uri, keys::now_ms()).unwrap();
    assert_eq!(checked.fingerprint().unwrap(), out.fingerprint);
    assert_eq!(
        statuses(&checked),
        vec![DeviceStatus::Valid { limited: false }]
    );
    // Asked for one persona, given another: refused.
    assert!(keys::check_published(&public, "https://localhost/users/other", 0).is_err());
    assert_eq!(
        status(&server.client().actor_keys("nobody").await.unwrap_err()),
        404
    );

    // Its own key is pinned already.
    let (k, v) = checked.identity.clone().unwrap();
    assert_eq!(
        keys::check_pin(&store, &pen.actor_uri, &k, v).unwrap(),
        PinStatus::Matches
    );
    // Another writer pins it on first sight.
    let bob_store = Store::in_memory();
    assert_eq!(
        keys::check_pin(&bob_store, &pen.actor_uri, &k, v).unwrap(),
        PinStatus::New
    );
    server.stop().await.unwrap();
}

#[tokio::test]
async fn a_second_device_cant_take_over_a_persona_by_accident() {
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    let pen = persona(&alice, "ink_fox").await;
    let laptop = Store::in_memory();
    let desktop = Store::in_memory();
    let first = keys::init(&laptop, &alice, &pen, false).await.unwrap();

    let err = keys::init(&desktop, &alice, &pen, false).await.unwrap_err();
    assert!(err.to_string().contains("another device"), "{err}");

    // Replacing the key on purpose: version 2; the laptop's device needs
    // certifying again and the laptop can no longer act for the persona.
    let second = keys::init(&desktop, &alice, &pen, true).await.unwrap();
    assert_eq!(second.identity_version, 2);
    assert_ne!(second.fingerprint, first.fingerprint);
    let public = alice.persona_keys(pen.id).await.unwrap();
    let checked = keys::check_published(&public, &pen.actor_uri, keys::now_ms()).unwrap();
    let mut s = statuses(&checked);
    s.sort_by_key(|x| format!("{x:?}"));
    assert_eq!(
        s,
        vec![
            DeviceStatus::Superseded,
            DeviceStatus::Valid { limited: false }
        ]
    );
    assert!(keys::init(&laptop, &alice, &pen, false).await.is_err());

    // Other writers who pinned the first key are told it changed.
    let bob = Store::in_memory();
    let fp1 = unb64(&b64(&laptop_public(&laptop, pen.id))).unwrap();
    keys::check_pin(&bob, &pen.actor_uri, &fp1, 1).unwrap();
    let (k2, v2) = checked.identity.unwrap();
    assert!(matches!(
        keys::check_pin(&bob, &pen.actor_uri, &k2, v2).unwrap(),
        PinStatus::Changed {
            pinned_version: 1,
            ..
        }
    ));
    server.stop().await.unwrap();
}

fn laptop_public(store: &Store, persona: Uuid) -> [u8; 32] {
    keys::load_identity(store, persona)
        .unwrap()
        .unwrap()
        .public()
}

#[tokio::test]
async fn the_server_refuses_anything_that_wouldnt_verify() {
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    let pen = persona(&alice, "ink_fox").await;
    let other = persona(&alice, "salt_wren").await;
    let mallory = server.account("mallory").await.unwrap();
    let id = key(1);
    let set = |k: &SigningKey, v| SetIdentityKey {
        public_key: b64(k.verifying_key().as_bytes()),
        key_version: v,
    };
    let reg = |c: String| RegisterDevice { certificate: c };

    // A device before any identity key.
    let early = alice
        .register_device(pen.id, &reg(cert(&id, &pen.actor_uri, &key(2), 1)))
        .await
        .unwrap_err();
    assert_eq!(status(&early), 409);
    alice.set_identity_key(pen.id, &set(&id, 1)).await.unwrap();
    // Same key again is fine; a lower or equal version for a new key isn't;
    // nor is a small-order key, nor someone else's persona.
    alice.set_identity_key(pen.id, &set(&id, 1)).await.unwrap();
    for (k, v, code) in [(key(3), 1, 409), (key(3), 0, 400)] {
        let e = alice
            .set_identity_key(pen.id, &set(&k, v))
            .await
            .unwrap_err();
        assert_eq!(status(&e), code);
    }
    let weak = SetIdentityKey {
        public_key: b64(&{
            let mut k = [0u8; 32];
            k[0] = 1;
            k
        }),
        key_version: 2,
    };
    assert_eq!(
        status(&alice.set_identity_key(pen.id, &weak).await.unwrap_err()),
        400
    );
    assert_eq!(
        status(
            &mallory
                .set_identity_key(pen.id, &set(&key(4), 5))
                .await
                .unwrap_err()
        ),
        404
    );
    // One identity key on two personas would link them.
    assert_eq!(
        status(
            &alice
                .set_identity_key(other.id, &set(&id, 1))
                .await
                .unwrap_err()
        ),
        409
    );

    // Bad certificates.
    let forged = cert(&key(9), &pen.actor_uri, &key(2), 1);
    let wrong_actor = cert(&id, &other.actor_uri, &key(2), 1);
    let wrong_version = cert(&id, &pen.actor_uri, &key(2), 2);
    for c in [
        "not base64!".to_owned(),
        b64(b"garbage"),
        forged,
        wrong_actor,
        wrong_version,
    ] {
        let e = alice.register_device(pen.id, &reg(c)).await.unwrap_err();
        assert_eq!(status(&e), 400, "{e}");
    }
    let device = alice
        .register_device(pen.id, &reg(cert(&id, &pen.actor_uri, &key(2), 1)))
        .await
        .unwrap();
    // The same device key for another persona would link them.
    alice
        .set_identity_key(other.id, &set(&key(5), 1))
        .await
        .unwrap();
    let e = alice
        .register_device(other.id, &reg(cert(&key(5), &other.actor_uri, &key(2), 1)))
        .await
        .unwrap_err();
    assert_eq!(status(&e), 409);

    // Revocations: signed by the identity key, for this device, once.
    let revocation = |signer: &SigningKey, device_key: &SigningKey, reason| RevokeDevice {
        revocation: b64(&DeviceRevocation {
            actor_uri: pen.actor_uri.clone(),
            device_key: device_key.verifying_key().to_bytes(),
            revoked_at: keys::now_ms(),
            reason,
        }
        .issue(signer)
        .unwrap()),
    };
    for (signer, dk) in [(key(9), key(2)), (id.clone(), key(7))] {
        let e = alice
            .revoke_device(
                pen.id,
                device.id,
                &revocation(&signer, &dk, RevocationReason::Lost),
            )
            .await
            .unwrap_err();
        assert_eq!(status(&e), 400);
    }
    let lost = revocation(&id, &key(2), RevocationReason::Lost);
    let revoked = alice.revoke_device(pen.id, device.id, &lost).await.unwrap();
    assert!(revoked.revocation.is_some());
    alice.revoke_device(pen.id, device.id, &lost).await.unwrap();
    let e = alice
        .revoke_device(
            pen.id,
            device.id,
            &revocation(&id, &key(2), RevocationReason::Retired),
        )
        .await
        .unwrap_err();
    assert_eq!(status(&e), 409);
    // A revoked device can't come back, even under a new identity key.
    alice
        .set_identity_key(pen.id, &set(&key(6), 2))
        .await
        .unwrap();
    let e = alice
        .register_device(pen.id, &reg(cert(&key(6), &pen.actor_uri, &key(2), 2)))
        .await
        .unwrap_err();
    assert_eq!(status(&e), 409);

    // And the database itself won't un-revoke.
    let pool = server.db().await.unwrap();
    assert!(
        sqlx::query("UPDATE devices SET revocation = NULL")
            .execute(&pool)
            .await
            .is_err()
    );
    pool.close().await;
    server.stop().await.unwrap();
}

#[tokio::test]
async fn clients_catch_forgeries_the_server_lets_through() {
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    let pen = persona(&alice, "ink_fox").await;
    let store = Store::in_memory();
    keys::init(&store, &alice, &pen, false).await.unwrap();
    let bob = Store::in_memory();
    let public = server.client().actor_keys("ink_fox").await.unwrap();
    let checked = keys::check_published(&public, &pen.actor_uri, keys::now_ms()).unwrap();
    let (k, v) = checked.identity.clone().unwrap();
    keys::check_pin(&bob, &pen.actor_uri, &k, v).unwrap();

    // A malicious server adds a device of its own, with a certificate made
    // by a key it controls, straight into its database.
    let pool = server.db().await.unwrap();
    let fake = DeviceCertificate {
        actor_uri: pen.actor_uri.clone(),
        device_key: key(8).verifying_key().to_bytes(),
        identity_key_version: 1,
        limited: false,
        not_before: 0,
        not_after: None,
    }
    .issue(&key(9))
    .unwrap();
    sqlx::query(
        "INSERT INTO devices (id, actor_id, device_key, key_version, limited, certificate)
         VALUES ($1, $2, $3, 1, false, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(pen.id)
    .bind(key(8).verifying_key().as_bytes().as_slice())
    .bind(&fake)
    .execute(&pool)
    .await
    .unwrap();
    let public = server.client().actor_keys("ink_fox").await.unwrap();
    let checked = keys::check_published(&public, &pen.actor_uri, keys::now_ms()).unwrap();
    let invalid: Vec<_> = checked
        .devices
        .iter()
        .filter(|d| matches!(d.status, DeviceStatus::Invalid(_)))
        .collect();
    assert_eq!(invalid.len(), 1);
    assert_eq!(invalid[0].device_key, key(8).verifying_key().to_bytes());

    // Or it swaps the identity key itself (and re-signs its fake device):
    // every real device stops verifying, and the pin catches the swap.
    sqlx::query("UPDATE actors SET identity_key = $2 WHERE id = $1")
        .bind(pen.id)
        .bind(key(9).verifying_key().as_bytes().as_slice())
        .execute(&pool)
        .await
        .unwrap();
    let public = server.client().actor_keys("ink_fox").await.unwrap();
    let checked = keys::check_published(&public, &pen.actor_uri, keys::now_ms()).unwrap();
    let (k, v) = checked.identity.clone().unwrap();
    assert!(matches!(
        keys::check_pin(&bob, &pen.actor_uri, &k, v).unwrap(),
        PinStatus::Changed { .. }
    ));
    assert!(
        checked
            .devices
            .iter()
            .any(|d| matches!(d.status, DeviceStatus::Invalid(_)))
    );
    pool.close().await;
    server.stop().await.unwrap();
}

#[tokio::test]
async fn revoking_this_device_forgets_it_and_init_makes_a_new_one() {
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    let pen = persona(&alice, "ink_fox").await;
    let store = Store::in_memory();
    let first = keys::init(&store, &alice, &pen, false).await.unwrap();
    keys::revoke(
        &store,
        &alice,
        &pen,
        first.device,
        RevocationReason::Retired,
    )
    .await
    .unwrap();
    assert!(keys::load_device(&store, pen.id).unwrap().is_none());
    let public = alice.persona_keys(pen.id).await.unwrap();
    let checked = keys::check_published(&public, &pen.actor_uri, keys::now_ms()).unwrap();
    assert_eq!(
        statuses(&checked),
        vec![DeviceStatus::Revoked(RevocationReason::Retired)]
    );

    let second = keys::init(&store, &alice, &pen, false).await.unwrap();
    assert!(!second.new_identity && second.new_device);
    assert_ne!(second.device, first.device);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn keys_dont_link_a_logins_personas() {
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    let a = persona(&alice, "ink_fox").await;
    let b = persona(&alice, "salt_wren").await;
    let store = Store::in_memory();
    keys::init(&store, &alice, &a, false).await.unwrap();
    keys::init(&store, &alice, &b, false).await.unwrap();

    let ka = server.client().actor_keys("ink_fox").await.unwrap();
    let kb = server.client().actor_keys("salt_wren").await.unwrap();
    assert_ne!(ka.identity_key, kb.identity_key);
    assert_ne!(ka.devices[0].device_key, kb.devices[0].device_key);
    for k in [&ka, &kb] {
        let d = &k.devices[0];
        // Random ids, not time-ordered ones.
        assert_eq!(d.id.get_version_num(), 4);
        // Certificates start at a UTC day boundary.
        let cert = DeviceCertificate::decode(&unb64(&d.certificate).unwrap()).unwrap();
        assert_eq!(cert.not_before % 86_400_000, 0);
    }
    // The published documents carry no times at all.
    let json = serde_json::to_value(&ka).unwrap();
    let text = json.to_string();
    for field in ["created", "published", "updated", "At\""] {
        assert!(!text.contains(field), "{field} in {text}");
    }
    server.stop().await.unwrap();
}
