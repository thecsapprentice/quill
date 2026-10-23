//! Phase 2g end-to-end tests: clock order, edits and deletions with every
//! version kept, gaps from withheld messages, padding, live delivery and
//! retention.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use quill_core::{Client, Store, keys, requests, sessions};
use quill_e2e::TestServer;
use quill_proto::{
    api::{CreatePersona, Persona},
    board::{CreatePrompt, Maturity, PromptState},
    groups::MessagesQuery,
};

macro_rules! server {
    () => {
        match TestServer::start().await.unwrap() {
            Some(s) => s,
            None => return,
        }
    };
}

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

/// Alice and Bob in a session, both synced. Returns the session id.
async fn pair(server: &TestServer) -> (String, (Store, Client, Persona), (Store, Client, Persona)) {
    let alice_c = server.account("alice").await.unwrap();
    let (fox, alice) = writer(&alice_c, "ink_fox").await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    let p = alice_c
        .create_prompt(&CreatePrompt {
            persona: fox.id,
            title: "The lighthouse ledger".into(),
            body: "A keeper finds a ledger.".into(),
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
        let r = sessions::sync(s, c).await.unwrap();
        assert!(r.problems.is_empty(), "{r:?}");
    }
    (id, (alice, alice_c, fox), (bob, bob_c, wren))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn edits_and_deletions_keep_every_version_and_replies_sort_after() {
    let server = server!();
    let (id, (alice, alice_c, fox), (bob, bob_c, wren)) = pair(&server).await;
    let first = sessions::write(&alice, &alice_c, &id, "The tide came in.", None)
        .await
        .unwrap();
    let reply = sessions::write(&bob, &bob_c, &id, "Early?", None)
        .await
        .unwrap();
    sessions::edit(&alice, &alice_c, &id, first, "The tide came in early.")
        .await
        .unwrap();
    // Only its author can change a message.
    assert!(
        sessions::edit(&bob, &bob_c, &id, first, "Mine now.")
            .await
            .is_err()
    );
    sessions::delete(&bob, &bob_c, &id, reply).await.unwrap();

    for (s, c) in [(&alice, &alice_c), (&bob, &bob_c)] {
        let r = sessions::read(s, c, &id).await.unwrap();
        assert!(r.gaps.is_empty(), "{:?}", r.gaps);
        let shown: Vec<(u64, &str, bool, bool)> = r
            .messages
            .iter()
            .map(|m| (m.seq, m.content.as_str(), m.edited(), m.deleted))
            .collect();
        assert_eq!(
            shown,
            [
                (1, "Pitch.", false, false),
                (first, "The tide came in early.", true, false),
                (reply, "Early?", true, true),
            ]
        );
        // The reply sorts after what it answers.
        assert!(r.messages[2].hlc > r.messages[1].hlc);
        assert_eq!(r.messages[1].sender, fox.actor_uri);
        assert_eq!(r.messages[2].sender, wren.actor_uri);
    }
    let v = sessions::versions(&bob, &bob_c, &id, first).await.unwrap();
    assert_eq!(
        v.versions
            .iter()
            .map(|v| v.content.clone())
            .collect::<Vec<_>>(),
        [
            Some("The tide came in.".to_owned()),
            Some("The tide came in early.".to_owned())
        ]
    );
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_withheld_message_shows_as_a_gap_and_sizes_are_padded() {
    let server = server!();
    let (id, (alice, alice_c, _), (bob, bob_c, _)) = pair(&server).await;
    let short = sessions::write(&bob, &bob_c, &id, "Hi.", None)
        .await
        .unwrap();
    let long = sessions::write(&bob, &bob_c, &id, "Hello.", None)
        .await
        .unwrap();
    let withheld = sessions::write(&bob, &bob_c, &id, "You won't see this.", None)
        .await
        .unwrap();
    sessions::write(&bob, &bob_c, &id, "Or did you?", None)
        .await
        .unwrap();

    // Messages of different lengths fill the same padded size.
    let log = alice_c
        .group_messages(&id, &MessagesQuery::default())
        .await
        .unwrap();
    let size = |seq: u64| {
        log.messages
            .iter()
            .find(|m| m.seq == seq)
            .unwrap()
            .message
            .len()
    };
    assert_eq!(size(short), size(long));

    // The server drops one of Bob's messages before Alice reads it.
    let pool = server.db().await.unwrap();
    sqlx::query("DELETE FROM group_messages WHERE seq = $1")
        .bind(i64::try_from(withheld).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let r = sessions::read(&alice, &alice_c, &id).await.unwrap();
    assert_eq!(r.gaps.len(), 1);
    assert_eq!(r.gaps[0].missing.len(), 1);
    assert!(r.messages.iter().any(|m| m.content == "Or did you?"));
    server.stop().await.unwrap();
}

/// The next notice, within a few seconds.
async fn next(events: &mut quill_core::client::EventStream) -> quill_proto::events::Event {
    tokio::time::timeout(std::time::Duration::from_secs(5), events.next())
        .await
        .expect("no notice in time")
        .unwrap()
        .expect("stream closed")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notices_arrive_live_and_carry_no_content() {
    use quill_proto::events::Event;
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let (fox, _alice) = writer(&alice_c, "ink_fox").await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    let mut alice_events = alice_c.events().await.unwrap();
    let mut bob_events = bob_c.events().await.unwrap();
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
    // Alice published 5 packages: one claim leaves her pool low.
    requests::send(&bob, &bob_c, &wren, p.id, "Pitch.", None)
        .await
        .unwrap();
    let mut seen = vec![next(&mut alice_events).await, next(&mut alice_events).await];
    seen.sort_by_key(|e| e.name());
    assert!(
        matches!(&seen[0], Event::KeyPackagesLow { persona, available: 4, .. } if *persona == fox.id),
        "{seen:?}"
    );
    assert_eq!(seen[1], Event::Request { prompt: p.id });
    // Bob hears nothing about Alice's queue.
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(300), bob_events.next())
            .await
            .is_err()
    );
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn following_a_session_shows_messages_as_they_arrive() {
    let server = server!();
    let (id, (alice, alice_c, _), (bob, bob_c, _)) = pair(&server).await;
    let mut events = alice_c.events().await.unwrap();
    sessions::write(&bob, &bob_c, &id, "Live.", None)
        .await
        .unwrap();
    match next(&mut events).await {
        quill_proto::events::Event::Message { group, .. } => assert_eq!(group, id),
        other => panic!("{other:?}"),
    }

    let follower = {
        let id = id.clone();
        tokio::spawn(async move {
            let mut got = Vec::new();
            sessions::follow(&alice, &alice_c, &id, |_, fresh| {
                got.extend(fresh.iter().map(|m| m.content.clone()));
                !got.iter().any(|c| c == "Second.")
            })
            .await
            .unwrap();
            got
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    sessions::write(&bob, &bob_c, &id, "Second.", None)
        .await
        .unwrap();
    // `follow` only reads again when a notice arrives, so finishing at all
    // shows the notice came; the timeout only guards against hanging.
    let got = tokio::time::timeout(std::time::Duration::from_secs(10), follower)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, ["Pitch.", "Live.", "Second."]);
    server.stop().await.unwrap();
}

/// Links a new device from an existing one.
async fn linked(server: &TestServer, name: &str, existing: (&Store, &Client)) -> (Store, Client) {
    let store = Store::in_memory();
    let c = server.client();
    let token = c
        .login(&quill_proto::api::Credentials {
            username: name.into(),
            password: quill_e2e::TEST_PASSWORD.into(),
        })
        .await
        .unwrap();
    let client = c.with_token(&token.token);
    let pending = quill_core::devices::start_link(&store, &client)
        .await
        .unwrap();
    quill_core::devices::approve(existing.0, existing.1, &pending.code.parse().unwrap())
        .await
        .unwrap();
    quill_core::devices::wait_for_link(
        &store,
        &client,
        &pending.link_id,
        std::time::Duration::from_millis(50),
        std::time::Duration::from_secs(5),
    )
    .await
    .unwrap();
    quill_core::devices::finish_link(&store, &client)
        .await
        .unwrap();
    (store, client)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acknowledged_messages_are_deleted_only_once_every_device_has_them() {
    let server = server!();
    let (id, (alice, alice_c, _), (bob, bob_c, _)) = pair(&server).await;
    // Bob links a phone, which doesn't read the session yet.
    let (phone, phone_c) = linked(&server, "bob", (&bob, &bob_c)).await;
    for text in ["One.", "Two.", "Three."] {
        sessions::write(&alice, &alice_c, &id, text, None)
            .await
            .unwrap();
    }
    let pool = server.db().await.unwrap();
    let gid = quill_proto::keys::unb64(&id).unwrap();
    // Everything so far is older than the grace period.
    sqlx::query(
        "UPDATE group_messages SET logged_at = now() - interval '8 days' WHERE group_id = $1",
    )
    .bind(&gid)
    .execute(&pool)
    .await
    .unwrap();
    let old = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM group_messages
             WHERE group_id = $1 AND logged_at < now() - interval '1 day'",
        )
        .bind(&gid)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    assert_eq!(old().await, 4);
    // Alice and Bob's laptop have read it all; the phone holds it.
    sessions::read(&alice, &alice_c, &id).await.unwrap();
    sessions::read(&bob, &bob_c, &id).await.unwrap();
    assert_eq!(old().await, 4);
    // Bob's laptop added the phone; once it has read to the end, the old
    // messages go.
    let r = sessions::sync(&phone, &phone_c).await.unwrap();
    assert_eq!(r.joined, 1, "{r:?}");
    assert_eq!(old().await, 0);
    // New ones stay within the grace period, even read by everyone.
    let last = alice_c.sessions().await.unwrap()[0].last_seq;
    let next = sessions::write(&bob, &bob_c, &id, "Four.", None)
        .await
        .unwrap();
    // Numbering carries on past what was deleted.
    assert_eq!(next, last + 1);
    for (s, c) in [(&alice, &alice_c), (&phone, &phone_c)] {
        let r = sessions::read(s, c, &id).await.unwrap();
        assert_eq!(r.messages.last().unwrap().content, "Four.");
    }
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM group_messages WHERE group_id = $1")
        .bind(&gid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(left >= 1);
    pool.close().await;
    server.stop().await.unwrap();
}
