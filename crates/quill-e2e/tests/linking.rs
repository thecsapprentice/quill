//! Phase 2d end-to-end tests: linking devices through the personal device
//! group, staying in sync, removing devices, and keeping pools full.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use quill_certs::RevocationReason;
use quill_core::{
    Client, Store,
    devices::{self, LinkCode},
    keys::{self, DeviceStatus},
};
use quill_e2e::TestServer;
use quill_proto::{
    api::{CreatePersona, Persona},
    keys::b64,
};

macro_rules! server {
    () => {
        match TestServer::start().await.unwrap() {
            Some(s) => s,
            None => return,
        }
    };
}

use quill_e2e::TEST_PASSWORD as PASSWORD;

/// Another signed-in client for the same login, as a second computer has.
async fn same_login(server: &TestServer, name: &str) -> Client {
    let client = server.client();
    let token = client
        .login(&quill_proto::api::Credentials {
            username: name.into(),
            password: PASSWORD.into(),
        })
        .await
        .unwrap();
    client.with_token(&token.token)
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

/// Links `new` from `existing`, the way a user would: start, approve with
/// the code, compare the words, finish.
async fn link(new: (&Store, &Client), existing: (&Store, &Client)) -> Vec<&'static str> {
    let pending = devices::start_link(new.0, new.1).await.unwrap();
    let code: LinkCode = pending.code.parse().unwrap();
    let approved = devices::approve(existing.0, existing.1, &code)
        .await
        .unwrap();
    let words = devices::wait_for_link(
        new.0,
        new.1,
        &pending.link_id,
        Duration::from_millis(50),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(words, approved.confirm, "confirmation words differ");
    devices::finish_link(new.0, new.1).await.unwrap();
    words
}

fn ids(list: &[devices::ListedDevice]) -> Vec<uuid::Uuid> {
    let mut v: Vec<_> = list.iter().map(|d| d.id).collect();
    v.sort();
    v
}

#[tokio::test]
async fn linked_devices_share_personas_and_stay_in_sync() {
    let server = server!();
    let laptop_c = server.account("alice").await.unwrap();
    let pen = persona(&laptop_c, "ink_fox").await;
    let wren = persona(&laptop_c, "salt_wren").await;
    let laptop = Store::in_memory();
    for p in [&pen, &wren] {
        keys::init(&laptop, &laptop_c, p, false).await.unwrap();
        keys::publish(&laptop, &laptop_c, p, 5).await.unwrap();
    }
    // A pin Alice's laptop holds for someone else, to be carried over.
    let bob_c = server.account("bob").await.unwrap();
    let bob_pen = persona(&bob_c, "bob_pen").await;
    let bob = Store::in_memory();
    keys::init(&bob, &bob_c, &bob_pen, false).await.unwrap();
    let bob_keys = laptop_c.actor_keys("bob_pen").await.unwrap();
    let checked = keys::check_published(&bob_keys, &bob_pen.actor_uri, keys::now_ms()).unwrap();
    let (bk, bv) = checked.identity.unwrap();
    keys::check_pin(&laptop, &bob_pen.actor_uri, &bk, bv).unwrap();

    // Desktop links from the laptop.
    let desktop = Store::in_memory();
    let desktop_c = same_login(&server, "alice").await;
    link((&desktop, &desktop_c), (&laptop, &laptop_c)).await;
    for p in [&pen, &wren] {
        let ours = keys::load_identity(&desktop, p.id).unwrap().unwrap();
        let theirs = keys::load_identity(&laptop, p.id).unwrap().unwrap();
        assert_eq!(ours.public(), theirs.public());
        assert!(
            keys::load_device(&desktop, p.id)
                .unwrap()
                .unwrap()
                .id
                .is_some()
        );
    }
    assert_eq!(
        keys::check_pin(&desktop, &bob_pen.actor_uri, &bk, bv).unwrap(),
        keys::PinStatus::Matches
    );
    // Others now find both devices, both certified and with packages.
    let claim = keys::claim(&bob, &bob_c, "ink_fox", &pen.actor_uri)
        .await
        .unwrap();
    assert_eq!(claim.packages.len(), 2, "{:?}", claim.refused);
    assert!(claim.refused.is_empty() && claim.missing.is_empty());

    // A third device links from the desktop; the laptop catches up.
    let phone = Store::in_memory();
    let phone_c = same_login(&server, "alice").await;
    link((&phone, &phone_c), (&desktop, &desktop_c)).await;
    let lists = [
        devices::list(&laptop, &laptop_c).await.unwrap(),
        devices::list(&desktop, &desktop_c).await.unwrap(),
        devices::list(&phone, &phone_c).await.unwrap(),
    ];
    assert_eq!(lists[0].len(), 3);
    assert!(lists.iter().all(|l| ids(l) == ids(&lists[0])));
    // Everyone knows everyone's persona devices.
    for l in &lists {
        assert!(l.iter().all(|d| d.personas.len() == 2), "{l:?}");
        assert_eq!(l.iter().filter(|d| d.this_device).count(), 1);
    }
    let claim = keys::claim(&bob, &bob_c, "salt_wren", &wren.actor_uri)
        .await
        .unwrap();
    assert_eq!(claim.packages.len(), 3);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn the_code_catches_a_swapped_key_package_and_typed_words_work() {
    let server = server!();
    let laptop_c = server.account("alice").await.unwrap();
    let pen = persona(&laptop_c, "ink_fox").await;
    let laptop = Store::in_memory();
    keys::init(&laptop, &laptop_c, &pen, false).await.unwrap();

    // The server swaps the new device's KeyPackage for one of its own.
    let desktop = Store::in_memory();
    let desktop_c = same_login(&server, "alice").await;
    let pending = devices::start_link(&desktop, &desktop_c).await.unwrap();
    let evil = Store::in_memory();
    let evil_c = same_login(&server, "alice").await;
    let evil_link = devices::start_link(&evil, &evil_c).await.unwrap();
    let pool = server.db().await.unwrap();
    sqlx::query(
        "UPDATE link_mailbox SET key_package =
             (SELECT key_package FROM link_mailbox WHERE link_id = $2)
         WHERE link_id = $1",
    )
    .bind(quill_proto::keys::unb64(&pending.link_id).unwrap())
    .bind(quill_proto::keys::unb64(&evil_link.link_id).unwrap())
    .execute(&pool)
    .await
    .unwrap();
    for code in [
        pending.code.clone(),
        format!("{} {}", pending.link_id, pending.words.join(" ")),
    ] {
        let err = devices::approve(&laptop, &laptop_c, &code.parse().unwrap())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("doesn't match"), "{err}");
    }
    // Nothing was granted to anyone.
    assert!(keys::load_identity(&evil, pen.id).unwrap().is_none());

    // A fresh, untampered link approved with the typed words works.
    let other = Store::in_memory();
    let other_c = same_login(&server, "alice").await;
    let pending = devices::start_link(&other, &other_c).await.unwrap();
    let typed = format!(
        "{}  {}",
        pending.link_id,
        pending.words.join(" ").to_uppercase()
    );
    let approved = devices::approve(&laptop, &laptop_c, &typed.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(approved.granted, 1);
    let words = devices::wait_for_link(
        &other,
        &other_c,
        &pending.link_id,
        Duration::from_millis(50),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(words, approved.confirm);
    pool.close().await;
    server.stop().await.unwrap();
}

#[tokio::test]
async fn the_words_catch_a_group_the_server_made_up() {
    let server = server!();
    let laptop_c = server.account("alice").await.unwrap();
    let pen = persona(&laptop_c, "ink_fox").await;
    let laptop = Store::in_memory();
    keys::init(&laptop, &laptop_c, &pen, false).await.unwrap();
    let desktop = Store::in_memory();
    let desktop_c = same_login(&server, "alice").await;
    let pending = devices::start_link(&desktop, &desktop_c).await.unwrap();

    // The server (playing the part of another login it controls) adds the
    // new device's real KeyPackage to a group of its own...
    let mallory_c = server.account("mallory").await.unwrap();
    let mallory = Store::in_memory();
    let pool = server.db().await.unwrap();
    let fake_id = [9u8; 16];
    sqlx::query(
        "INSERT INTO link_mailbox (link_id, account_id, key_package, expires_at)
         SELECT $2, (SELECT id FROM accounts WHERE login_name = 'mallory'), key_package, expires_at
         FROM link_mailbox WHERE link_id = $1",
    )
    .bind(quill_proto::keys::unb64(&pending.link_id).unwrap())
    .bind(fake_id.as_slice())
    .execute(&pool)
    .await
    .unwrap();
    let fake_code = pending.code.replace(&pending.link_id, &b64(&fake_id));
    devices::approve(&mallory, &mallory_c, &fake_code.parse().unwrap())
        .await
        .unwrap();
    // ...the real laptop approves the real link...
    let approved = devices::approve(&laptop, &laptop_c, &pending.code.parse().unwrap())
        .await
        .unwrap();
    // ...and the server hands the new device its own group's Welcome.
    sqlx::query(
        "UPDATE link_mailbox SET (welcome, from_seq) =
             (SELECT welcome, from_seq FROM link_mailbox WHERE link_id = $2)
         WHERE link_id = $1",
    )
    .bind(quill_proto::keys::unb64(&pending.link_id).unwrap())
    .bind(fake_id.as_slice())
    .execute(&pool)
    .await
    .unwrap();
    let words = devices::wait_for_link(
        &desktop,
        &desktop_c,
        &pending.link_id,
        Duration::from_millis(50),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    // The words differ from what the laptop shows: the user stops here.
    assert_ne!(words, approved.confirm);
    devices::abandon_link(&desktop).unwrap();
    assert!(keys::load_identity(&desktop, pen.id).unwrap().is_none());
    pool.close().await;
    server.stop().await.unwrap();
}

#[tokio::test]
async fn two_devices_adding_at_once_both_get_through() {
    let server = server!();
    let laptop_c = server.account("alice").await.unwrap();
    let pen = persona(&laptop_c, "ink_fox").await;
    let laptop = Store::in_memory();
    keys::init(&laptop, &laptop_c, &pen, false).await.unwrap();
    let desktop = Store::in_memory();
    let desktop_c = same_login(&server, "alice").await;
    link((&desktop, &desktop_c), (&laptop, &laptop_c)).await;

    // Two new devices, approved at the same moment from different devices:
    // one commit loses the epoch race, catches up and retries.
    let (a, b) = (Store::in_memory(), Store::in_memory());
    let (a_c, b_c) = (
        same_login(&server, "alice").await,
        same_login(&server, "alice").await,
    );
    let pa = devices::start_link(&a, &a_c).await.unwrap();
    let pb = devices::start_link(&b, &b_c).await.unwrap();
    let (ca, cb): (LinkCode, LinkCode) = (pa.code.parse().unwrap(), pb.code.parse().unwrap());
    let (ra, rb) = tokio::join!(
        devices::approve(&laptop, &laptop_c, &ca),
        devices::approve(&desktop, &desktop_c, &cb),
    );
    let (ra, rb) = (ra.unwrap(), rb.unwrap());
    for (store, client, pending, approved) in [(&a, &a_c, &pa, &ra), (&b, &b_c, &pb, &rb)] {
        let words = devices::wait_for_link(
            store,
            client,
            &pending.link_id,
            Duration::from_millis(50),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(words, approved.confirm);
        devices::finish_link(store, client).await.unwrap();
    }
    let lists = [
        devices::list(&laptop, &laptop_c).await.unwrap(),
        devices::list(&desktop, &desktop_c).await.unwrap(),
        devices::list(&a, &a_c).await.unwrap(),
        devices::list(&b, &b_c).await.unwrap(),
    ];
    assert_eq!(lists[0].len(), 4);
    assert!(lists.iter().all(|l| ids(l) == ids(&lists[0])));
    server.stop().await.unwrap();
}

#[tokio::test]
async fn a_removed_device_loses_its_keys_and_its_persona_devices() {
    let server = server!();
    let laptop_c = server.account("alice").await.unwrap();
    let pen = persona(&laptop_c, "ink_fox").await;
    let laptop = Store::in_memory();
    keys::init(&laptop, &laptop_c, &pen, false).await.unwrap();
    let desktop = Store::in_memory();
    let desktop_c = same_login(&server, "alice").await;
    link((&desktop, &desktop_c), (&laptop, &laptop_c)).await;
    let desktop_id = devices::list(&desktop, &desktop_c)
        .await
        .unwrap()
        .into_iter()
        .find(|d| d.this_device)
        .unwrap()
        .id;
    let desktop_pen = keys::load_device(&desktop, pen.id)
        .unwrap()
        .unwrap()
        .id
        .unwrap();

    // Can't remove oneself; can remove the other.
    let me = devices::list(&laptop, &laptop_c)
        .await
        .unwrap()
        .into_iter()
        .find(|d| d.this_device)
        .unwrap()
        .id;
    assert!(
        devices::remove(&laptop, &laptop_c, me, RevocationReason::Lost)
            .await
            .is_err()
    );
    let removed = devices::remove(&laptop, &laptop_c, desktop_id, RevocationReason::Lost)
        .await
        .unwrap();
    assert_eq!(removed.revoked, vec![desktop_pen]);

    // The directory shows its persona device revoked.
    let public = laptop_c.actor_keys("ink_fox").await.unwrap();
    let checked = keys::check_published(&public, &pen.actor_uri, keys::now_ms()).unwrap();
    let status = &checked
        .devices
        .iter()
        .find(|d| d.id == desktop_pen)
        .unwrap()
        .status;
    assert_eq!(status, &DeviceStatus::Revoked(RevocationReason::Lost));

    // The laptop sends something after; the desktop catches up, finds it
    // was removed, and its keys are gone.
    let third = Store::in_memory();
    let third_c = same_login(&server, "alice").await;
    link((&third, &third_c), (&laptop, &laptop_c)).await;
    let caught = devices::catch_up(&desktop, &desktop_c).await.unwrap();
    assert!(caught.removed);
    assert!(keys::load_identity(&desktop, pen.id).unwrap().is_none());
    assert!(keys::load_device(&desktop, pen.id).unwrap().is_none());
    assert_eq!(devices::list(&laptop, &laptop_c).await.unwrap().len(), 2);

    // It can be linked again, as a new device.
    link((&desktop, &desktop_c), (&laptop, &laptop_c)).await;
    assert!(keys::load_identity(&desktop, pen.id).unwrap().is_some());
    let list = devices::list(&laptop, &laptop_c).await.unwrap();
    assert_eq!(list.len(), 3);
    assert!(!list.iter().any(|d| d.id == desktop_id));
    server.stop().await.unwrap();
}

#[tokio::test]
async fn upkeep_refills_low_pools() {
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    let pen = persona(&alice, "ink_fox").await;
    let store = Store::in_memory();
    keys::init(&store, &alice, &pen, false).await.unwrap();
    keys::publish(&store, &alice, &pen, keys::POOL_TARGET)
        .await
        .unwrap();
    let bob = server.account("bob").await.unwrap();
    let dev = keys::load_device(&store, pen.id)
        .unwrap()
        .unwrap()
        .id
        .unwrap();

    // Above the threshold: nothing to do.
    for _ in 0..5 {
        bob.claim_key_packages("ink_fox").await.unwrap();
    }
    let up = devices::maintain(&store, &alice).await.unwrap();
    assert!(up.refilled.is_empty());
    assert_eq!(
        alice
            .key_package_status(pen.id, dev)
            .await
            .unwrap()
            .available,
        15
    );
    // Below it: back to the target.
    for _ in 0..6 {
        bob.claim_key_packages("ink_fox").await.unwrap();
    }
    let up = devices::maintain(&store, &alice).await.unwrap();
    assert_eq!(up.refilled, vec![("ink_fox".to_owned(), 11)]);
    assert_eq!(
        alice
            .key_package_status(pen.id, dev)
            .await
            .unwrap()
            .available,
        keys::POOL_TARGET
    );
    server.stop().await.unwrap();
}

/// The words a new device shows, as a person would type them on the device
/// already in use, with the link found from the login's waiting links.
#[tokio::test]
async fn approving_with_the_six_words_alone_finds_the_waiting_link() {
    let server = server!();
    let laptop_c = server.account("alice").await.unwrap();
    let pen = persona(&laptop_c, "ink_fox").await;
    let laptop = Store::in_memory();
    keys::init(&laptop, &laptop_c, &pen, false).await.unwrap();

    let desktop = Store::in_memory();
    let desktop_c = same_login(&server, "alice").await;
    // Nothing waiting yet.
    assert_eq!(
        devices::waiting(&laptop_c).await.unwrap(),
        devices::Waiting::None
    );
    let typed: LinkCode = "one two three four five six".parse().unwrap();
    let none = devices::approve(&laptop, &laptop_c, &typed)
        .await
        .unwrap_err();
    assert!(
        none.to_string().contains("no new device is waiting"),
        "{none}"
    );

    let pending = devices::start_link(&desktop, &desktop_c).await.unwrap();
    assert_eq!(
        devices::waiting(&laptop_c).await.unwrap(),
        devices::Waiting::One(pending.link_id.clone())
    );
    // Another login never sees it.
    let bob_c = server.account("bob").await.unwrap();
    assert_eq!(
        devices::waiting(&bob_c).await.unwrap(),
        devices::Waiting::None
    );

    // Wrong words stop, and link nothing.
    let wrong: LinkCode = "abandon ability able about above absent".parse().unwrap();
    assert!(devices::approve(&laptop, &laptop_c, &wrong).await.is_err());

    let words: LinkCode = pending.words.join(" ").parse().unwrap();
    let approved = devices::approve(&laptop, &laptop_c, &words).await.unwrap();
    let confirm = devices::wait_for_link(
        &desktop,
        &desktop_c,
        &pending.link_id,
        Duration::from_millis(50),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(confirm, approved.confirm);
    devices::finish_link(&desktop, &desktop_c).await.unwrap();
    // Answered: no longer waiting.
    assert_eq!(
        devices::waiting(&laptop_c).await.unwrap(),
        devices::Waiting::None
    );
}

/// Two links waiting at once: words alone approve neither.
#[tokio::test]
async fn two_waiting_links_are_not_approved_by_words_alone() {
    let server = server!();
    let laptop_c = server.account("alice").await.unwrap();
    let pen = persona(&laptop_c, "ink_fox").await;
    let laptop = Store::in_memory();
    keys::init(&laptop, &laptop_c, &pen, false).await.unwrap();
    let (a, b) = (Store::in_memory(), Store::in_memory());
    let (ac, bc) = (
        same_login(&server, "alice").await,
        same_login(&server, "alice").await,
    );
    let pa = devices::start_link(&a, &ac).await.unwrap();
    devices::start_link(&b, &bc).await.unwrap();
    assert_eq!(
        devices::waiting(&laptop_c).await.unwrap(),
        devices::Waiting::Several(2)
    );
    let words: LinkCode = pa.words.join(" ").parse().unwrap();
    let refused = devices::approve(&laptop, &laptop_c, &words)
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("someone else may be"),
        "{refused}"
    );
    // The whole code still works: it names its link.
    let code: LinkCode = pa.code.parse().unwrap();
    devices::approve(&laptop, &laptop_c, &code).await.unwrap();
}
