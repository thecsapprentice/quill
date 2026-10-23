//! Phase 2f end-to-end tests: every device of both writers joins a session
//! and reads it; a writer can only add their own devices; revoked devices
//! are removed; leaving ends the session.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use quill_certs::RevocationReason;
use quill_core::{
    Client, CoreError, Store, devices, keys, requests,
    sessions::{self, Message, Standing},
};
use quill_e2e::{TEST_PASSWORD as PASSWORD, TestServer};
use quill_proto::{
    api::{CreatePersona, Persona},
    board::{CreatePrompt, Maturity, PromptState},
};

macro_rules! server {
    () => {
        match TestServer::start().await.unwrap() {
            Some(s) => s,
            None => return,
        }
    };
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

async fn writer(client: &Client, handle: &str) -> (Persona, Store) {
    let p = persona(client, handle).await;
    let store = Store::in_memory();
    keys::init(&store, client, &p, false).await.unwrap();
    keys::publish(&store, client, &p, 5).await.unwrap();
    (p, store)
}

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

/// Links a new device from an existing one; returns its store and client.
async fn linked(server: &TestServer, name: &str, existing: (&Store, &Client)) -> (Store, Client) {
    let store = Store::in_memory();
    let client = same_login(server, name).await;
    let pending = devices::start_link(&store, &client).await.unwrap();
    devices::approve(existing.0, existing.1, &pending.code.parse().unwrap())
        .await
        .unwrap();
    devices::wait_for_link(
        &store,
        &client,
        &pending.link_id,
        Duration::from_millis(50),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    devices::finish_link(&store, &client).await.unwrap();
    (store, client)
}

const PITCH: &str = "I'd write the keeper.";

/// Alice's prompt; Bob's request, read and accepted on Alice's `reader`.
/// Returns the session id.
async fn session(alice: (&Store, &Client, &Persona), bob: (&Store, &Client, &Persona)) -> String {
    let p = alice
        .1
        .create_prompt(&CreatePrompt {
            persona: alice.2.id,
            title: "The lighthouse ledger".into(),
            body: "A keeper finds a ledger.".into(),
            maturity: Maturity::General,
            tags: Vec::new(),
            state: Some(PromptState::Open),
        })
        .await
        .unwrap();
    let sent = requests::send(bob.0, bob.1, bob.2, p.id, PITCH, None)
        .await
        .unwrap();
    requests::inbox(alice.0, alice.1, p.id, false)
        .await
        .unwrap();
    requests::accept(alice.0, alice.1, sent.request.id)
        .await
        .unwrap()
}

async fn sync(store: &Store, client: &Client) -> sessions::SyncReport {
    let r = sessions::sync(store, client).await.unwrap();
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    r
}

async fn texts(store: &Store, client: &Client, id: &str) -> Vec<String> {
    sessions::read(store, client, id)
        .await
        .unwrap()
        .messages
        .into_iter()
        .map(|m: Message| m.content)
        .collect()
}

async fn standing(store: &Store, client: &Client, id: &str) -> Standing {
    sessions::list(store, client)
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.session.id == id)
        .unwrap()
        .standing
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_device_of_both_writers_joins_and_reads() {
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let (fox, laptop) = writer(&alice_c, "ink_fox").await;
    // Linked before the request, but never reads the pitch.
    let (desktop, desktop_c) = linked(&server, "alice", (&laptop, &alice_c)).await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    // In the request's Welcome, never opened.
    let (phone, phone_c) = linked(&server, "bob", (&bob, &bob_c)).await;

    let id = session((&laptop, &alice_c, &fox), (&bob, &bob_c, &wren)).await;
    // Linked after the request: added by one of Alice's devices.
    let (tablet, tablet_c) = linked(&server, "alice", (&laptop, &alice_c)).await;
    assert_eq!(standing(&tablet, &tablet_c, &id).await, Standing::Waiting);

    let r = sync(&laptop, &alice_c).await;
    assert_eq!(r.added, 1);
    assert_eq!(sync(&desktop, &desktop_c).await.joined, 1);
    assert_eq!(sync(&tablet, &tablet_c).await.joined, 1);
    sync(&bob, &bob_c).await;
    assert_eq!(sync(&phone, &phone_c).await.joined, 1);
    // Nothing left to do anywhere.
    for (s, c) in [(&laptop, &alice_c), (&desktop, &desktop_c), (&bob, &bob_c)] {
        let r = sync(s, c).await;
        assert_eq!((r.joined, r.added, r.removed), (0, 0, 0));
    }

    sessions::write(&bob, &bob_c, &id, "The tide came in early.", None)
        .await
        .unwrap();
    sessions::write(&tablet, &tablet_c, &id[..10], "Then the lamp failed.", None)
        .await
        .unwrap();
    sessions::write(
        &phone,
        &phone_c,
        &id,
        "Who rang the bell?",
        Some("planning"),
    )
    .await
    .unwrap();
    let all = [
        PITCH,
        "The tide came in early.",
        "Then the lamp failed.",
        "Who rang the bell?",
    ];
    // Every device that was in the request reads the pitch too.
    for (s, c) in [
        (&laptop, &alice_c),
        (&desktop, &desktop_c),
        (&bob, &bob_c),
        (&phone, &phone_c),
    ] {
        assert_eq!(texts(s, c, &id).await, all);
    }
    // The tablet reads from when it joined.
    assert_eq!(texts(&tablet, &tablet_c, &id).await, &all[1..]);
    let on_laptop = sessions::read(&laptop, &alice_c, &id)
        .await
        .unwrap()
        .messages;
    assert_eq!(on_laptop[3].channel, "planning");
    assert_eq!(on_laptop[1].sender, wren.actor_uri);
    assert_eq!(on_laptop[2].sender, fox.actor_uri);
    assert!(!on_laptop[2].own);
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn racing_adds_settle_and_revoked_devices_are_removed() {
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let (fox, laptop) = writer(&alice_c, "ink_fox").await;
    let (desktop, desktop_c) = linked(&server, "alice", (&laptop, &alice_c)).await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    let id = session((&laptop, &alice_c, &fox), (&bob, &bob_c, &wren)).await;
    sync(&laptop, &alice_c).await;
    sync(&desktop, &desktop_c).await;
    sync(&bob, &bob_c).await;

    // Both of Alice's devices see the new tablet missing at once.
    let (tablet, tablet_c) = linked(&server, "alice", (&laptop, &alice_c)).await;
    let (a, b) = tokio::join!(
        sessions::sync(&laptop, &alice_c),
        sessions::sync(&desktop, &desktop_c)
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert!(
        a.problems.is_empty() && b.problems.is_empty(),
        "{a:?} {b:?}"
    );
    assert_eq!(a.added + b.added, 1);
    assert_eq!(sync(&tablet, &tablet_c).await.joined, 1);
    sessions::write(&bob, &bob_c, &id, "Still there?", None)
        .await
        .unwrap();
    for (s, c) in [
        (&laptop, &alice_c),
        (&desktop, &desktop_c),
        (&tablet, &tablet_c),
    ] {
        assert_eq!(texts(s, c, &id).await.last().unwrap(), "Still there?");
    }

    // The tablet is removed from Alice's devices: its persona device is
    // revoked, and her next sync takes it out of the session.
    let tablet_device = keys::load_device(&tablet, fox.id)
        .unwrap()
        .unwrap()
        .id
        .unwrap();
    let account_device = devices::list(&laptop, &alice_c)
        .await
        .unwrap()
        .into_iter()
        .find(|d| d.personas.iter().any(|p| p.device == tablet_device))
        .unwrap()
        .id;
    devices::remove(&laptop, &alice_c, account_device, RevocationReason::Lost)
        .await
        .unwrap();
    assert_eq!(sync(&laptop, &alice_c).await.removed, 1);
    sessions::write(&bob, &bob_c, &id, "After the tablet left.", None)
        .await
        .unwrap();
    assert_eq!(
        texts(&desktop, &desktop_c, &id).await.last().unwrap(),
        "After the tablet left."
    );
    sync(&tablet, &tablet_c).await;
    assert!(matches!(
        standing(&tablet, &tablet_c, &id).await,
        Standing::Broken(why) if why.contains("removed")
    ));
    let held = sessions::read(&tablet, &tablet_c, &id)
        .await
        .unwrap()
        .messages;
    assert!(!held.iter().any(|m| m.content == "After the tablet left."));
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_writer_cannot_add_the_other_writers_devices() {
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let (fox, laptop) = writer(&alice_c, "ink_fox").await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    let id = session((&laptop, &alice_c, &fox), (&bob, &bob_c, &wren)).await;
    sync(&laptop, &alice_c).await;
    sync(&bob, &bob_c).await;

    // Alice links a tablet; before she adds it, Bob's client adds it
    // itself (a device of Alice's, so it would pass a membership check).
    let (_tablet, _tablet_c) = linked(&server, "alice", (&laptop, &alice_c)).await;
    sessions::testing::forge_add(&bob, &bob_c, &id, "ink_fox", &fox.actor_uri)
        .await
        .unwrap();
    let r = sessions::sync(&laptop, &alice_c).await.unwrap();
    assert_eq!(r.problems.len(), 1);
    assert!(
        r.problems[0].1.contains("isn't one of the committer's own"),
        "{r:?}"
    );
    assert!(matches!(
        standing(&laptop, &alice_c, &id).await,
        Standing::Broken(_)
    ));
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_server_cannot_swap_the_other_writer() {
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let (fox, laptop) = writer(&alice_c, "ink_fox").await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    let carol_c = server.account("carol").await.unwrap();
    let (moth, _carol) = writer(&carol_c, "paper_moth").await;
    let id = session((&laptop, &alice_c, &fox), (&bob, &bob_c, &wren)).await;
    sync(&laptop, &alice_c).await;
    sync(&bob, &bob_c).await;
    sessions::write(&bob, &bob_c, &id, "From Bob.", None)
        .await
        .unwrap();
    // The server now says Alice's partner is Carol.
    let pool = server.db().await.unwrap();
    sqlx::query("UPDATE session_members SET actor_id = $1 WHERE actor_id = $2")
        .bind(moth.id)
        .bind(wren.id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let r = sessions::sync(&laptop, &alice_c).await.unwrap();
    assert_eq!(r.problems.len(), 1, "{r:?}");
    assert!(r.problems[0].1.contains("neither writer"), "{r:?}");
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leaving_ends_the_session_for_both() {
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let (fox, laptop) = writer(&alice_c, "ink_fox").await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    let id = session((&laptop, &alice_c, &fox), (&bob, &bob_c, &wren)).await;
    sync(&laptop, &alice_c).await;
    sync(&bob, &bob_c).await;
    sessions::write(&laptop, &alice_c, &id, "Last words.", None)
        .await
        .unwrap();

    sessions::leave(&bob, &bob_c, &id).await.unwrap();
    // Bob read to the end before leaving.
    assert_eq!(texts(&bob, &bob_c, &id).await, [PITCH, "Last words."]);
    let e: CoreError = sessions::write(&laptop, &alice_c, &id, "Hello?", None)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("ended"), "{e}");
    let listed = sessions::list(&laptop, &alice_c).await.unwrap();
    assert!(listed[0].session.ended);
    assert_eq!(texts(&laptop, &alice_c, &id).await, [PITCH, "Last words."]);
    // Nobody can post to its log any more.
    let gid = quill_proto::keys::unb64(&id).unwrap();
    let mut m = vec![0, 1, 0, 2, 32];
    m.extend_from_slice(&gid);
    m.extend_from_slice(&5u64.to_be_bytes());
    m.extend_from_slice(&[1, 0, 4, 1, 2, 3, 4, 3, 9, 9, 9]);
    let e = alice_c
        .post_group_message(
            &id,
            &quill_proto::groups::PostMessage {
                message: quill_proto::keys::b64(&m),
                blobs: Vec::new(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.status(), Some(403));

    // Deleting a persona ends its sessions too.
    let carol_c = server.account("carol").await.unwrap();
    let (moth, carol) = writer(&carol_c, "paper_moth").await;
    let id2 = session((&laptop, &alice_c, &fox), (&carol, &carol_c, &moth)).await;
    sync(&laptop, &alice_c).await;
    carol_c.delete_persona(moth.id).await.unwrap();
    let listed = sessions::list(&laptop, &alice_c).await.unwrap();
    let s = listed.iter().find(|l| l.session.id == id2).unwrap();
    assert!(s.session.ended && s.partner().is_none());
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_added_through_its_last_resort_package_updates_its_key() {
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let (fox, laptop) = writer(&alice_c, "ink_fox").await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    let id = session((&laptop, &alice_c, &fox), (&bob, &bob_c, &wren)).await;
    sync(&laptop, &alice_c).await;
    sync(&bob, &bob_c).await;

    let (tablet, tablet_c) = linked(&server, "alice", (&laptop, &alice_c)).await;
    let tablet_device = keys::load_device(&tablet, fox.id)
        .unwrap()
        .unwrap()
        .id
        .unwrap();
    let pool = server.db().await.unwrap();
    sqlx::query("DELETE FROM key_packages WHERE device_id = $1 AND NOT last_resort")
        .bind(tablet_device)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert_eq!(sync(&laptop, &alice_c).await.added, 1);
    let before = alice_c.sessions().await.unwrap()[0].epoch;
    assert_eq!(sync(&tablet, &tablet_c).await.joined, 1);
    // The tablet's key update moved the group on an epoch.
    assert_eq!(alice_c.sessions().await.unwrap()[0].epoch, before + 1);
    // Everyone follows it.
    sessions::write(&tablet, &tablet_c, &id, "Fresh keys.", None)
        .await
        .unwrap();
    for (s, c) in [(&laptop, &alice_c), (&bob, &bob_c)] {
        assert_eq!(texts(s, c, &id).await.last().unwrap(), "Fresh keys.");
    }
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn safety_numbers_match_and_a_swapped_key_changes_them() {
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let (fox, laptop) = writer(&alice_c, "ink_fox").await;
    let (desktop, desktop_c) = linked(&server, "alice", (&laptop, &alice_c)).await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    let id = session((&laptop, &alice_c, &fox), (&bob, &bob_c, &wren)).await;
    for (s, c) in [(&laptop, &alice_c), (&desktop, &desktop_c), (&bob, &bob_c)] {
        sync(s, c).await;
    }
    // Everyone ends up at the same epoch before comparing.
    for (s, c) in [(&laptop, &alice_c), (&desktop, &desktop_c), (&bob, &bob_c)] {
        sync(s, c).await;
    }
    let a = sessions::safety_numbers(&laptop, &alice_c, &id)
        .await
        .unwrap();
    let d = sessions::safety_numbers(&desktop, &desktop_c, &id)
        .await
        .unwrap();
    let b = sessions::safety_numbers(&bob, &bob_c, &id).await.unwrap();
    assert_eq!(a.identity.len(), 12);
    assert_eq!(
        (&a.identity, a.epoch, &a.check),
        (&b.identity, b.epoch, &b.check)
    );
    assert_eq!((&d.identity, &d.check), (&b.identity, &b.check));
    assert!(!b.verified);
    assert!(
        sessions::confirm_verified(&bob, &bob_c, &id)
            .await
            .unwrap()
            .verified
    );
    let listed = sessions::list(&bob, &bob_c).await.unwrap();
    assert!(sessions::verified(&bob, &listed[0].session).unwrap());

    // Alice's identity key is replaced: Bob's client stops at the changed
    // pin, and once he accepts it the numbers differ and the mark is gone.
    let fresh = Store::in_memory();
    keys::init(&fresh, &alice_c, &fox, true).await.unwrap();
    let e = sessions::safety_numbers(&bob, &bob_c, &id)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("changed since it was pinned"), "{e}");
    let published = keys::check_published(
        &bob_c.actor_keys("ink_fox").await.unwrap(),
        &fox.actor_uri,
        keys::now_ms(),
    )
    .unwrap();
    let (k, v) = published.identity.unwrap();
    keys::accept_pin(&bob, &fox.actor_uri, &k, v).unwrap();
    let listed = sessions::list(&bob, &bob_c).await.unwrap();
    assert!(!sessions::verified(&bob, &listed[0].session).unwrap());
    let fresh_number = sessions::identity_number(
        (fox.actor_uri.as_str(), k.as_slice()),
        (
            wren.actor_uri.as_str(),
            {
                let id = keys::load_identity(&bob, wren.id).unwrap().unwrap();
                id.public().to_vec()
            }
            .as_slice(),
        ),
    );
    assert_ne!(fresh_number, b.identity);
    server.stop().await.unwrap();
}
