//! Phase 2k end-to-end tests: devices that join a session late get its
//! earlier history from the login's other devices.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use quill_core::{
    Client, Store, devices, keys, requests,
    sessions::{self, Message},
};
use quill_e2e::{TEST_PASSWORD, TestServer};
use quill_proto::{
    api::{CreatePersona, Credentials, Persona},
    board::{CreatePrompt, Maturity, PromptState},
};

async fn writer(client: &Client, handle: &str) -> (Persona, Store) {
    let p = client
        .create_persona(&CreatePersona {
            handle: handle.into(),
            display_name: None,
            summary: None,
        })
        .await
        .unwrap();
    let store = Store::in_memory();
    keys::init(&store, client, &p, false).await.unwrap();
    keys::publish(&store, client, &p, 5).await.unwrap();
    (p, store)
}

async fn linked(server: &TestServer, name: &str, existing: (&Store, &Client)) -> (Store, Client) {
    let store = Store::in_memory();
    let c = server.client();
    let token = c
        .login(&Credentials {
            username: name.into(),
            password: TEST_PASSWORD.into(),
        })
        .await
        .unwrap();
    let client = c.with_token(&token.token);
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

type Side = (Store, Client, Persona);

async fn pair(server: &TestServer) -> (String, Side, Side) {
    let alice_c = server.account("alice").await.unwrap();
    let (fox, alice) = writer(&alice_c, "ink_fox").await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    let p = alice_c
        .create_prompt(&CreatePrompt {
            persona: fox.id,
            title: "Lamps".into(),
            body: "Lamps.".into(),
            maturity: Maturity::General,
            tags: Vec::new(),
            state: Some(PromptState::Open),
        })
        .await
        .unwrap();
    let sent = requests::send(&bob, &bob_c, &wren, p.id, "Pitch.", None)
        .await
        .unwrap();
    requests::inbox(&alice, &alice_c, p.id, false)
        .await
        .unwrap();
    let id = requests::accept(&alice, &alice_c, sent.request.id)
        .await
        .unwrap();
    for (s, c) in [(&alice, &alice_c), (&bob, &bob_c)] {
        assert!(sessions::sync(s, c).await.unwrap().problems.is_empty());
    }
    (id, (alice, alice_c, fox), (bob, bob_c, wren))
}

/// A note as any device of the login should hold it (which device sent
/// it aside).
type Shape = (
    u64,
    String,
    String,
    String,
    String,
    Vec<quill_core::history::Version>,
    bool,
    Vec<quill_core::attachments::Attachment>,
    Option<String>,
);

fn shape(m: &Message) -> Shape {
    (
        m.seq,
        m.id.clone(),
        m.sender.clone(),
        m.channel.clone(),
        m.content.clone(),
        m.versions.clone(),
        m.deleted,
        m.attachments.clone(),
        m.in_reply_to.clone(),
    )
}

async fn shapes(store: &Store, client: &Client, id: &str) -> Vec<Shape> {
    sessions::read(store, client, id)
        .await
        .unwrap()
        .messages
        .iter()
        .map(shape)
        .collect()
}

/// Writes a little of everything: notes in two channels, a reply, an edit,
/// a deletion, a channel description and a file.
async fn write_a_story(id: &str, alice: (&Store, &Client), bob: (&Store, &Client)) -> Vec<u8> {
    let one = sessions::write(alice.0, alice.1, id, "The lamp is lit.", None)
        .await
        .unwrap();
    sessions::name_channel(alice.0, alice.1, id, "world", Some("Places"))
        .await
        .unwrap();
    let two = sessions::post(
        bob.0,
        bob.1,
        id,
        sessions::Draft {
            text: "A map of the coast.",
            channel: Some("world"),
            files: vec![("coast.png".into(), vec![7u8; 6000])],
            reply_to: Some(one),
        },
    )
    .await
    .unwrap();
    sessions::sync(alice.0, alice.1).await.unwrap();
    sessions::edit(bob.0, bob.1, id, two, "A map of the north coast.")
        .await
        .unwrap();
    let three = sessions::write(alice.0, alice.1, id, "Wrong turn.", None)
        .await
        .unwrap();
    sessions::delete(alice.0, alice.1, id, three).await.unwrap();
    sessions::write(bob.0, bob.1, id, "The keeper wakes.", None)
        .await
        .unwrap();
    vec![7u8; 6000]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_added_later_gets_the_history_before_it() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let db = server.db().await.unwrap();
    let (id, (alice, alice_c, _), (bob, bob_c, _)) = pair(&server).await;
    let map = write_a_story(&id, (&alice, &alice_c), (&bob, &bob_c)).await;
    let full = shapes(&bob, &bob_c, &id).await;
    assert_eq!(full.len(), 5, "the pitch and four notes");

    // Bob links a phone; his laptop adds it to the session.
    let (phone, phone_c) = linked(&server, "bob", (&bob, &bob_c)).await;
    sessions::sync(&bob, &bob_c).await.unwrap();
    sessions::sync(&phone, &phone_c).await.unwrap();
    let r = sessions::read(&phone, &phone_c, &id).await.unwrap();
    assert!(r.messages.is_empty(), "it reads only from its Welcome on");
    let (from, to) = r.history_missing.expect("it knows what it lacks");
    assert_eq!(from, 1);
    assert!(to > 5);

    // It asks; the laptop answers; it merges.
    let up = devices::maintain(&phone, &phone_c).await.unwrap();
    assert_eq!((up.history.asked, up.history_problem), (1, None));
    let up = devices::maintain(&bob, &bob_c).await.unwrap();
    assert_eq!((up.history.answered, up.history_problem), (1, None));
    let up = devices::maintain(&phone, &phone_c).await.unwrap();
    assert_eq!((up.history.received, up.history_problem), (1, None));

    let r = sessions::read(&phone, &phone_c, &id).await.unwrap();
    assert_eq!(r.history_missing, None);
    assert_eq!(shapes(&phone, &phone_c, &id).await, full);
    let edited = r
        .messages
        .iter()
        .find(|m| m.content == "A map of the north coast.")
        .unwrap();
    assert_eq!(edited.versions.len(), 2);
    assert_eq!(
        sessions::file(&phone, &edited.attachments[0].url)
            .unwrap()
            .unwrap()
            .as_slice(),
        map.as_slice()
    );
    assert!(r.messages.iter().any(|m| m.deleted));
    let world = sessions::channels(&phone, &phone_c, &id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name == "world")
        .unwrap();
    assert_eq!(world.summary.as_deref(), Some("Places"));

    // The bundle's blobs went once the phone had them; nobody asks again.
    let left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM blobs b JOIN mls_groups g ON g.group_id = b.group_id
         WHERE g.kind = 'personal'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(left, 0);
    for (s, c) in [(&phone, &phone_c), (&bob, &bob_c)] {
        let up = devices::maintain(s, c).await.unwrap();
        assert_eq!(up.history, Default::default());
    }

    // And it carries on as any device does.
    sessions::write(&phone, &phone_c, &id, "From the phone.", None)
        .await
        .unwrap();
    let r = sessions::read(&alice, &alice_c, &id).await.unwrap();
    assert_eq!(r.messages.last().unwrap().content, "From the phone.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_linked_after_a_session_ended_gets_its_history() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let (id, (alice, alice_c, _), (bob, bob_c, _)) = pair(&server).await;
    write_a_story(&id, (&alice, &alice_c), (&bob, &bob_c)).await;
    sessions::leave(&alice, &alice_c, &id).await.unwrap();
    sessions::sync(&bob, &bob_c).await.unwrap();
    let full = shapes(&bob, &bob_c, &id).await;

    let (phone, phone_c) = linked(&server, "bob", (&bob, &bob_c)).await;
    let up = devices::maintain(&phone, &phone_c).await.unwrap();
    assert_eq!(up.history.asked, 1);
    let r = sessions::read(&phone, &phone_c, &id).await.unwrap();
    assert!(r.messages.is_empty());
    assert_eq!(r.history_missing, Some((1, u64::MAX)));

    assert_eq!(
        devices::maintain(&bob, &bob_c)
            .await
            .unwrap()
            .history
            .answered,
        1
    );
    assert_eq!(
        devices::maintain(&phone, &phone_c)
            .await
            .unwrap()
            .history
            .received,
        1
    );
    let r = sessions::read(&phone, &phone_c, &id).await.unwrap();
    assert_eq!(r.history_missing, None);
    assert_eq!(shapes(&phone, &phone_c, &id).await, full);
}

/// Makes every message in groups of `kind` older than the retention
/// period, then sweeps.
async fn age_and_sweep(db: &sqlx::PgPool, kind: &str) {
    sqlx::query(
        "UPDATE group_messages m SET logged_at = now() - interval '31 days'
         FROM mls_groups g WHERE g.group_id = m.group_id AND g.kind = $1",
    )
    .bind(kind)
    .execute(db)
    .await
    .unwrap();
    let swept = quill_server::retention::sweep(db, 30).await.unwrap();
    assert!(swept.aged > 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_away_too_long_comes_back_through_another_device() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let db = server.db().await.unwrap();
    let (id, (alice, alice_c, _), (bob, bob_c, _)) = pair(&server).await;
    let (phone, phone_c) = linked(&server, "bob", (&bob, &bob_c)).await;
    sessions::sync(&bob, &bob_c).await.unwrap();
    sessions::sync(&phone, &phone_c).await.unwrap();
    sessions::write(&alice, &alice_c, &id, "Before the phone went away.", None)
        .await
        .unwrap();
    sessions::sync(&phone, &phone_c).await.unwrap();

    // The phone sits in a drawer while the story goes on, and the server
    // deletes everything by age.
    write_a_story(&id, (&alice, &alice_c), (&bob, &bob_c)).await;
    for (s, c) in [(&alice, &alice_c), (&bob, &bob_c)] {
        sessions::sync(s, c).await.unwrap();
    }
    age_and_sweep(&db, "session").await;

    // Bob's laptop takes the idle phone out and puts it back in afresh.
    let s = sessions::find(&bob_c, &id).await.unwrap();
    let persona = s.members.iter().find_map(|m| m.persona).unwrap();
    let phone_device = keys::load_device(&phone, persona)
        .unwrap()
        .unwrap()
        .id
        .unwrap();
    assert_eq!(s.stale_devices, vec![phone_device]);
    let r = sessions::sync(&bob, &bob_c).await.unwrap();
    assert_eq!((r.removed, r.added), (1, 1), "{r:?}");
    assert!(
        sessions::find(&bob_c, &id)
            .await
            .unwrap()
            .stale_devices
            .is_empty()
    );

    // Back from the drawer: it can't follow the old log, joins afresh, and
    // gets what it missed from the laptop.
    let up = devices::maintain(&phone, &phone_c).await.unwrap();
    assert!(
        up.sessions.problems.is_empty(),
        "{:?}",
        up.sessions.problems
    );
    assert_eq!(up.history.asked, 1);
    let r = sessions::read(&phone, &phone_c, &id).await.unwrap();
    assert!(r.history_missing.is_some());
    devices::maintain(&bob, &bob_c).await.unwrap();
    devices::maintain(&phone, &phone_c).await.unwrap();
    let r = sessions::read(&phone, &phone_c, &id).await.unwrap();
    assert_eq!(r.history_missing, None);
    assert_eq!(
        shapes(&phone, &phone_c, &id).await,
        shapes(&bob, &bob_c, &id).await
    );
    sessions::write(&phone, &phone_c, &id, "Back from the drawer.", None)
        .await
        .unwrap();
    let r = sessions::read(&alice, &alice_c, &id).await.unwrap();
    assert_eq!(r.messages.last().unwrap().content, "Back from the drawer.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lone_device_away_too_long_comes_back_through_the_other_writer() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let db = server.db().await.unwrap();
    let (id, (alice, alice_c, _), (bob, bob_c, _)) = pair(&server).await;
    sessions::write(&bob, &bob_c, &id, "Bob, before.", None)
        .await
        .unwrap();
    sessions::sync(&alice, &alice_c).await.unwrap();
    // Bob is away; Alice writes on; the server deletes it all by age.
    sessions::write(&alice, &alice_c, &id, "Alice, while Bob was away.", None)
        .await
        .unwrap();
    sessions::sync(&alice, &alice_c).await.unwrap();
    age_and_sweep(&db, "session").await;

    // Bob's only device can't follow: it asks Alice's client to let it
    // back in, replacing its own old place.
    let r = sessions::list(&bob, &bob_c).await.unwrap();
    assert_eq!(r[0].standing, sessions::Standing::Rejoining);
    let e = sessions::write(&bob, &bob_c, &id, "Too soon.", None)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("let back in"), "{e}");
    let r = sessions::sync(&alice, &alice_c).await.unwrap();
    assert_eq!(r.rejoined, 1, "{r:?}");
    assert!(
        sessions::find(&alice_c, &id)
            .await
            .unwrap()
            .rejoins
            .is_empty()
    );

    let r = sessions::list(&bob, &bob_c).await.unwrap();
    assert_eq!(r[0].standing, sessions::Standing::Joined);
    // What was deleted while he was away is gone for him (he has no other
    // device to ask); what he had stays, and he writes on.
    let r = sessions::read(&bob, &bob_c, &id).await.unwrap();
    assert!(r.messages.iter().any(|m| m.content == "Bob, before."));
    assert!(!r.messages.iter().any(|m| m.content.contains("while Bob")));
    assert!(r.history_missing.is_some());
    sessions::write(&bob, &bob_c, &id, "Bob, back.", None)
        .await
        .unwrap();
    let r = sessions::read(&alice, &alice_c, &id).await.unwrap();
    assert_eq!(r.messages.last().unwrap().content, "Bob, back.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_away_from_the_device_group_too_long_is_linked_again() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let db = server.db().await.unwrap();
    let (id, (alice, alice_c, _), (bob, bob_c, _)) = pair(&server).await;
    let (phone, phone_c) = linked(&server, "bob", (&bob, &bob_c)).await;
    sessions::sync(&bob, &bob_c).await.unwrap();
    sessions::sync(&phone, &phone_c).await.unwrap();
    // The laptop announces itself again after the phone last read.
    devices::list(&bob, &bob_c).await.unwrap();
    sessions::write(&bob, &bob_c, &id, "From the laptop.", None)
        .await
        .unwrap();
    age_and_sweep(&db, "personal").await;

    let up = devices::maintain(&phone, &phone_c).await.unwrap();
    assert!(up.caught_up.stale);
    let e = devices::list(&phone, &phone_c).await.unwrap_err();
    assert!(e.to_string().contains("link it again"), "{e}");
    // Its sessions carry on meanwhile.
    sessions::write(&phone, &phone_c, &id, "Still here.", None)
        .await
        .unwrap();

    // Linked again from the laptop, it keeps its keys and sessions.
    let pending = devices::start_link(&phone, &phone_c).await.unwrap();
    devices::approve(&bob, &bob_c, &pending.code.parse().unwrap())
        .await
        .unwrap();
    devices::wait_for_link(
        &phone,
        &phone_c,
        &pending.link_id,
        Duration::from_millis(50),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    devices::finish_link(&phone, &phone_c).await.unwrap();
    assert!(
        !devices::maintain(&phone, &phone_c)
            .await
            .unwrap()
            .caught_up
            .stale
    );
    let listed = devices::list(&bob, &bob_c).await.unwrap();
    assert_eq!(listed.len(), 3, "the laptop, the old entry and the phone");

    // Removing the old entry doesn't revoke the persona devices the phone
    // still uses.
    let fresh = devices::list(&phone, &phone_c).await.unwrap();
    let phone_id = fresh.iter().find(|d| d.this_device).unwrap().id;
    let old = listed
        .iter()
        .find(|d| !d.this_device && d.id != phone_id)
        .unwrap()
        .id;
    let removed = devices::remove(&bob, &bob_c, old, quill_certs::RevocationReason::Lost)
        .await
        .unwrap();
    assert!(removed.revoked.is_empty());
    sessions::write(&phone, &phone_c, &id, "Linked again.", None)
        .await
        .unwrap();
    let r = sessions::read(&alice, &alice_c, &id).await.unwrap();
    assert_eq!(r.messages.last().unwrap().content, "Linked again.");
}
