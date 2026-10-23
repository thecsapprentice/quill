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
