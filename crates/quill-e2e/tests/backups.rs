//! Phase 2j end-to-end tests: backup and restore with the recovery code,
//! rejoining sessions through the other writer, and refusing rejoin
//! requests the persona didn't sign.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use quill_core::{
    Client, Store,
    backup::{self, cipher::BackupCipher},
    keys, requests,
    sessions::{self, Standing},
};
use quill_e2e::{TEST_PASSWORD, TestServer};
use quill_proto::{
    api::{CreatePersona, Credentials, Persona},
    board::{CreatePrompt, Maturity, PromptState},
    keys::b64,
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

async fn login(server: &TestServer, name: &str) -> Client {
    let c = server.client();
    let token = c
        .login(&Credentials {
            username: name.into(),
            password: TEST_PASSWORD.into(),
        })
        .await
        .unwrap();
    c.with_token(&token.token)
}

async fn session(alice: (&Store, &Client, &Persona), bob: (&Store, &Client, &Persona)) -> String {
    let p = alice
        .1
        .create_prompt(&CreatePrompt {
            persona: alice.2.id,
            title: "Lamps".into(),
            body: "Lamps.".into(),
            maturity: Maturity::General,
            tags: Vec::new(),
            state: Some(PromptState::Open),
        })
        .await
        .unwrap();
    let sent = requests::send(bob.0, bob.1, bob.2, p.id, "Pitch.", None)
        .await
        .unwrap();
    requests::inbox(alice.0, alice.1, p.id, false)
        .await
        .unwrap();
    let id = requests::accept(alice.0, alice.1, sent.request.id)
        .await
        .unwrap();
    for (s, c) in [(alice.0, alice.1), (bob.0, bob.1)] {
        assert!(sessions::sync(s, c).await.unwrap().problems.is_empty());
    }
    id
}

async fn texts(store: &Store, client: &Client, id: &str) -> Vec<String> {
    sessions::read(store, client, id)
        .await
        .unwrap()
        .messages
        .into_iter()
        .map(|m| m.content)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lost_device_comes_back_from_the_recovery_code() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let alice_c = server.account("alice").await.unwrap();
    let (fox, laptop) = writer(&alice_c, "ink_fox").await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    let id = session((&laptop, &alice_c, &fox), (&bob, &bob_c, &wren)).await;
    sessions::write(&laptop, &alice_c, &id, "Before the loss.", None)
        .await
        .unwrap();
    sessions::post(
        &bob,
        &bob_c,
        &id,
        sessions::Draft {
            text: "A map.",
            files: vec![("map.txt".into(), b"North is up.".to_vec())],
            ..sessions::Draft::default()
        },
    )
    .await
    .unwrap();
    sessions::read(&laptop, &alice_c, &id).await.unwrap();
    let verified = sessions::confirm_verified(&laptop, &alice_c, &id)
        .await
        .unwrap();
    assert!(verified.verified);

    let created = backup::create(&laptop, &alice_c, false).await.unwrap();
    let code = created.code.unwrap().join(" ");
    assert_eq!(code.split(' ').count(), 12);
    // Again: refreshed with the same code, nothing new to show.
    let again = backup::create(&laptop, &alice_c, false).await.unwrap();
    assert!(again.code.is_none());
    assert_ne!(again.info.generation, created.info.generation);
    // The file is a standalone age file that opens with the words.
    let mut file = Vec::new();
    for part in 0..again.info.parts {
        file.extend_from_slice(&alice_c.backup_part(part).await.unwrap());
    }
    assert!(file.starts_with(b"age-encryption.org/v1\n"));
    assert!(backup::cipher::AgeCrate.open(&code, &file).is_ok());

    // The laptop is lost. A new device signs in and restores.
    let fresh = Store::in_memory();
    let fresh_c = login(&server, "alice").await;
    let mut words: Vec<&str> = code.split(' ').collect();
    words.swap(0, 1);
    assert!(
        backup::restore(&fresh, &fresh_c, &words.join(" "))
            .await
            .is_err()
    );
    let restored = backup::restore(&fresh, &fresh_c, &code.to_uppercase())
        .await
        .unwrap();
    assert_eq!(restored.personas, ["ink_fox"]);
    assert_eq!((restored.revoked, restored.rejoins), (1, 1));
    // History is back before the session lets it in.
    let before = texts(&fresh, &fresh_c, &id).await;
    assert_eq!(before, ["Pitch.", "Before the loss.", "A map."]);
    let r = sessions::read(&fresh, &fresh_c, &id).await.unwrap();
    let map = &r.messages[2].attachments[0];
    assert_eq!(
        sessions::file(&fresh, &map.url)
            .unwrap()
            .unwrap()
            .as_slice(),
        b"North is up."
    );
    // The verification mark came back too.
    assert!(sessions::verified(&fresh, &r.session).unwrap());

    // Bob's client lets the restored device in and the lost one out.
    let r = sessions::sync(&bob, &bob_c).await.unwrap();
    assert_eq!(r.rejoined, 1, "{r:?}");
    let r = sessions::sync(&fresh, &fresh_c).await.unwrap();
    assert_eq!(r.joined, 1, "{r:?}");
    sessions::write(&bob, &bob_c, &id, "Welcome back.", None)
        .await
        .unwrap();
    assert_eq!(
        texts(&fresh, &fresh_c, &id).await.last().unwrap(),
        "Welcome back."
    );
    sessions::write(&fresh, &fresh_c, &id, "Glad to be.", None)
        .await
        .unwrap();
    assert_eq!(
        texts(&bob, &bob_c, &id).await.last().unwrap(),
        "Glad to be."
    );
    // The lost laptop was removed and reads nothing new.
    sessions::sync(&laptop, &alice_c).await.unwrap();
    let standing = sessions::list(&laptop, &alice_c).await.unwrap()[0]
        .standing
        .clone();
    assert!(matches!(standing, Standing::Broken(_)), "{standing:?}");
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejoin_request_must_be_signed_by_the_persona() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let alice_c = server.account("alice").await.unwrap();
    let (fox, laptop) = writer(&alice_c, "ink_fox").await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    let id = session((&laptop, &alice_c, &fox), (&bob, &bob_c, &wren)).await;
    let gid = quill_proto::keys::unb64(&id).unwrap();

    // A request signed with some other key: the server refuses it.
    let forger = ed25519_dalek::SigningKey::from_bytes(&[9; 32]);
    let forged = quill_certs::RejoinRequest {
        actor_uri: fox.actor_uri.clone(),
        group_id: gid.clone(),
        key_package: vec![0, 1, 0, 5],
        certificate: vec![1],
        remove_keys: Vec::new(),
        created_at: keys::now_ms(),
    }
    .issue(&forger)
    .unwrap();
    let e = alice_c.send_rejoin(&id, b64(&forged)).await.unwrap_err();
    assert_eq!(e.status(), Some(400));
    // Slipped in by the server itself, Bob's client ignores it.
    let pool = server.db().await.unwrap();
    sqlx::query(
        "INSERT INTO session_rejoins (id, group_id, actor_id, request) VALUES ($1, $2, $3, $4)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(&gid)
    .bind(fox.id)
    .bind(&forged)
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    let r = sessions::sync(&bob, &bob_c).await.unwrap();
    assert_eq!(r.rejoined, 0, "{r:?}");
    assert!(r.problems.is_empty(), "{r:?}");
    // And no backup means nothing to restore.
    let fresh = Store::in_memory();
    let fresh_c = login(&server, "alice").await;
    let words = quill_core::words::mnemonic(&[1; 16]).join(" ");
    let e = backup::restore(&fresh, &fresh_c, &words).await.unwrap_err();
    assert!(e.to_string().contains("no backup"), "{e}");
    server.stop().await.unwrap();
}
