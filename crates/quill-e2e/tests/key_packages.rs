//! Phase 2c end-to-end tests: KeyPackage pools, claims, and the checks a
//! claimer makes on what the server hands out.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use quill_core::{Client, CoreError, Store, keys};
use quill_e2e::TestServer;
use quill_proto::{
    api::{CreatePersona, Persona},
    keys::{UploadKeyPackages, b64, unb64},
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

/// A signed-in writer with a persona whose keys are set up on `store`.
async fn writer(server: &TestServer, name: &str, handle: &str, store: &Store) -> (Client, Persona) {
    let client = server.account(name).await.unwrap();
    let persona = client
        .create_persona(&CreatePersona {
            handle: handle.into(),
            display_name: None,
            summary: None,
        })
        .await
        .unwrap();
    keys::init(store, &client, &persona, false).await.unwrap();
    (client, persona)
}

fn device(store: &Store, persona: &Persona) -> Uuid {
    keys::load_device(store, persona.id)
        .unwrap()
        .unwrap()
        .id
        .unwrap()
}

#[tokio::test]
async fn the_pool_drains_to_the_last_resort_package_and_refills() {
    let server = server!();
    let a_store = Store::in_memory();
    let (alice, pen) = writer(&server, "alice", "ink_fox", &a_store).await;
    let b_store = Store::in_memory();
    let (bob, _) = writer(&server, "bob", "salt_wren", &b_store).await;

    let out = keys::publish(&a_store, &alice, &pen, 3).await.unwrap();
    assert_eq!((out.uploaded, out.available), (3, 3));
    assert!(out.last_resort_renewed && out.last_resort_until.is_some());
    // Nothing to do when full.
    let again = keys::publish(&a_store, &alice, &pen, 3).await.unwrap();
    assert_eq!((again.uploaded, again.last_resort_renewed), (0, false));

    let dev = device(&a_store, &pen);
    for left in (0..3).rev() {
        let claim = keys::claim(&b_store, &bob, "ink_fox", &pen.actor_uri)
            .await
            .unwrap();
        assert_eq!(claim.packages.len(), 1, "{:?}", claim.refused);
        assert_eq!(claim.packages[0].device, dev);
        assert!(!claim.packages[0].last_resort);
        assert!(claim.refused.is_empty() && claim.missing.is_empty());
        assert_eq!(
            alice
                .key_package_status(pen.id, dev)
                .await
                .unwrap()
                .available,
            left
        );
    }
    // Empty pool: the last-resort package, again and again.
    for _ in 0..2 {
        let claim = keys::claim(&b_store, &bob, "ink_fox", &pen.actor_uri)
            .await
            .unwrap();
        assert!(claim.packages[0].last_resort);
    }
    // One-time packages are really consumed: all three claimed were different.
    let refill = keys::publish(&a_store, &alice, &pen, 2).await.unwrap();
    assert_eq!((refill.uploaded, refill.available), (2, 2));
    assert!(!refill.last_resort_renewed);

    // Claiming needs a login, and a persona with keys.
    let anon = server
        .client()
        .claim_key_packages("ink_fox")
        .await
        .unwrap_err();
    assert!(matches!(anon, CoreError::NotLoggedIn));
    let raw = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/actors/ink_fox/key-packages/claim",
            server.url()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(raw.status().as_u16(), 401);
    let shy = bob
        .create_persona(&CreatePersona {
            handle: "no_keys".into(),
            display_name: None,
            summary: None,
        })
        .await
        .unwrap();
    assert!(!shy.actor_uri.is_empty());
    assert_eq!(
        status(&bob.claim_key_packages("no_keys").await.unwrap_err()),
        409
    );
    server.stop().await.unwrap();
}

#[tokio::test]
async fn the_server_refuses_packages_that_dont_belong_to_the_device() {
    let server = server!();
    let a_store = Store::in_memory();
    let (alice, pen) = writer(&server, "alice", "ink_fox", &a_store).await;
    let b_store = Store::in_memory();
    let (bob, wren) = writer(&server, "bob", "salt_wren", &b_store).await;
    let a_dev = device(&a_store, &pen);
    let b_dev = device(&b_store, &wren);

    // Real packages to misuse, taken from claims.
    keys::publish(&b_store, &bob, &wren, 2).await.unwrap();
    keys::publish(&a_store, &alice, &pen, 2).await.unwrap();
    let bobs = bob_package(&alice, "salt_wren").await;
    let alices = bob_package(&bob, "ink_fox").await;
    let upload = |one_time: Vec<String>, last_resort: Option<String>| UploadKeyPackages {
        key_packages: one_time,
        last_resort,
    };
    for (body, code) in [
        (upload(vec![b64(b"junk")], None), 400),
        // Bob's package filed under Alice's device.
        (upload(vec![bobs.clone()], None), 400),
        // A one-time package in the last-resort slot.
        (upload(vec![], Some(alices.clone())), 400),
    ] {
        let e = alice
            .upload_key_packages(pen.id, a_dev, &body)
            .await
            .unwrap_err();
        assert_eq!(status(&e), code, "{e}");
    }
    // Someone else's persona or device.
    let e = bob
        .upload_key_packages(pen.id, a_dev, &upload(vec![alices.clone()], None))
        .await
        .unwrap_err();
    assert_eq!(status(&e), 404);
    let e = alice
        .upload_key_packages(pen.id, b_dev, &upload(vec![alices.clone()], None))
        .await
        .unwrap_err();
    assert_eq!(status(&e), 404);
    // The pool has a ceiling.
    let many = vec![alices.clone(); 101];
    let e = alice
        .upload_key_packages(pen.id, a_dev, &upload(many, None))
        .await
        .unwrap_err();
    assert_eq!(status(&e), 409);
    server.stop().await.unwrap();
}

/// One package of `handle`'s, claimed as `client`, still encoded.
async fn bob_package(client: &Client, handle: &str) -> String {
    client.claim_key_packages(handle).await.unwrap().packages[0]
        .key_package
        .clone()
}

#[tokio::test]
async fn claimers_refuse_packages_a_malicious_server_substitutes() {
    let server = server!();
    let a_store = Store::in_memory();
    let (alice, pen) = writer(&server, "alice", "ink_fox", &a_store).await;
    let b_store = Store::in_memory();
    let (bob, wren) = writer(&server, "bob", "salt_wren", &b_store).await;
    keys::publish(&b_store, &bob, &wren, 1).await.unwrap();
    let a_dev = device(&a_store, &pen);

    // The server files Bob's package (a valid package, but Bob's) under
    // Alice's device, straight into its database.
    let bobs = unb64(&bob_package(&alice, "salt_wren").await).unwrap();
    let pool = server.db().await.unwrap();
    sqlx::query(
        "INSERT INTO key_packages (device_id, data, last_resort, not_after)
         VALUES ($1, $2, false, 4102444800)",
    )
    .bind(a_dev)
    .bind(&bobs)
    .execute(&pool)
    .await
    .unwrap();
    let claim = keys::claim(&b_store, &bob, "ink_fox", &pen.actor_uri)
        .await
        .unwrap();
    assert!(claim.packages.is_empty());
    assert_eq!(claim.refused.len(), 1);
    assert!(
        claim.refused[0].1.contains("another actor"),
        "{:?}",
        claim.refused
    );

    // Or garbage.
    sqlx::query(
        "INSERT INTO key_packages (device_id, data, last_resort, not_after)
         VALUES ($1, $2, false, 4102444800)",
    )
    .bind(a_dev)
    .bind(b"not a key package".as_slice())
    .execute(&pool)
    .await
    .unwrap();
    let claim = keys::claim(&b_store, &bob, "ink_fox", &pen.actor_uri)
        .await
        .unwrap();
    assert!(claim.packages.is_empty() && claim.refused.len() == 1);

    // Expired packages are never handed out.
    sqlx::query("DELETE FROM key_packages")
        .execute(&pool)
        .await
        .unwrap();
    keys::publish(&a_store, &alice, &pen, 1).await.unwrap();
    sqlx::query("UPDATE key_packages SET not_after = 1 WHERE NOT last_resort")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        alice
            .key_package_status(pen.id, a_dev)
            .await
            .unwrap()
            .available,
        0
    );
    let claim = keys::claim(&b_store, &bob, "ink_fox", &pen.actor_uri)
        .await
        .unwrap();
    assert!(claim.packages[0].last_resort);
    pool.close().await;
    server.stop().await.unwrap();
}

#[tokio::test]
async fn a_changed_identity_key_stops_claims_and_old_devices_drop_out() {
    let server = server!();
    let laptop = Store::in_memory();
    let (alice, pen) = writer(&server, "alice", "ink_fox", &laptop).await;
    keys::publish(&laptop, &alice, &pen, 2).await.unwrap();
    let b_store = Store::in_memory();
    let (bob, _) = writer(&server, "bob", "salt_wren", &b_store).await;
    let first = keys::claim(&b_store, &bob, "ink_fox", &pen.actor_uri)
        .await
        .unwrap();
    assert_eq!(first.packages.len(), 1);

    // Alice replaces the identity key from another store.
    let desktop = Store::in_memory();
    keys::init(&desktop, &alice, &pen, true).await.unwrap();
    keys::publish(&desktop, &alice, &pen, 2).await.unwrap();
    let err = keys::claim(&b_store, &bob, "ink_fox", &pen.actor_uri)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("changed"), "{err}");

    // Once Bob accepts the new key, only the new device's package comes,
    // and the laptop (certified by the old key) can't publish any more.
    let public = bob.actor_keys("ink_fox").await.unwrap();
    let checked = keys::check_published(&public, &pen.actor_uri, keys::now_ms()).unwrap();
    let (k, v) = checked.identity.unwrap();
    keys::accept_pin(&b_store, &pen.actor_uri, &k, v).unwrap();
    let claim = keys::claim(&b_store, &bob, "ink_fox", &pen.actor_uri)
        .await
        .unwrap();
    assert_eq!(claim.packages.len(), 1);
    assert_eq!(claim.packages[0].device, device(&desktop, &pen));
    let err = keys::publish(&laptop, &alice, &pen, 2).await.unwrap_err();
    assert_eq!(status(&err), 409);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn claims_are_rate_limited_per_account() {
    let server = server!();
    let a_store = Store::in_memory();
    let (alice, pen) = writer(&server, "alice", "ink_fox", &a_store).await;
    keys::publish(&a_store, &alice, &pen, 1).await.unwrap();
    let bob = server.account("bob").await.unwrap();
    let mut limited = false;
    for _ in 0..61 {
        match bob.claim_key_packages("ink_fox").await {
            Ok(_) => {}
            Err(e) if status(&e) == 429 => {
                limited = true;
                break;
            }
            Err(e) => panic!("{e}"),
        }
    }
    assert!(limited);
    // Others aren't affected.
    alice.claim_key_packages("ink_fox").await.unwrap();
    server.stop().await.unwrap();
}
