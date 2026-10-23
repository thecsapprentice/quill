//! Phase 2i end-to-end tests: encrypted attachments arrive intact on every
//! device, the server sees only bucketed sizes, blobs need a member login
//! and their token, limits and quotas hold, and blobs go with their
//! messages.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use quill_core::{Client, Store, devices, keys, requests, sessions};
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

fn bytes(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 7 % 256) as u8).collect()
}

async fn attach(store: &Store, client: &Client, id: &str, files: Vec<(String, Vec<u8>)>) -> u64 {
    sessions::post(
        store,
        client,
        id,
        sessions::Draft {
            text: "See attached.",
            files,
            ..sessions::Draft::default()
        },
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_arrives_intact_everywhere_and_the_server_sees_a_bucket() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let (id, (alice, alice_c, _), (bob, bob_c, _)) = pair(&server).await;
    let (phone, phone_c) = linked(&server, "bob", (&bob, &bob_c)).await;
    sessions::sync(&bob, &bob_c).await.unwrap();
    sessions::sync(&phone, &phone_c).await.unwrap();

    let map = bytes(10_000);
    let notes = b"Act one: the keeper.".to_vec();
    let seq = attach(
        &alice,
        &alice_c,
        &id,
        vec![
            ("map.png".into(), map.clone()),
            ("notes.txt".into(), notes.clone()),
        ],
    )
    .await;
    for (s, c) in [(&alice, &alice_c), (&bob, &bob_c), (&phone, &phone_c)] {
        let r = sessions::read(s, c, &id).await.unwrap();
        let m = r.messages.iter().find(|m| m.seq == seq).unwrap();
        assert_eq!(m.attachments.len(), 2);
        assert_eq!(m.attachments[0].media_type, "image/png");
        assert_eq!(
            sessions::file(s, &m.attachments[0].url)
                .unwrap()
                .unwrap()
                .as_slice(),
            map.as_slice()
        );
        assert_eq!(
            sessions::file(s, &m.attachments[1].url)
                .unwrap()
                .unwrap()
                .as_slice(),
            notes.as_slice()
        );
    }
    let pool = server.db().await.unwrap();
    let sizes: Vec<i64> = sqlx::query_scalar("SELECT size FROM blobs ORDER BY size")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(sizes, [4096, 16384]);
    pool.close().await;
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_changed_blob_is_caught_and_blobs_need_a_member_and_the_token() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let (id, (alice, alice_c, _), (bob, bob_c, _)) = pair(&server).await;
    let seq = attach(&alice, &alice_c, &id, vec![("a.bin".into(), bytes(100))]).await;
    let a = sessions::read(&alice, &alice_c, &id)
        .await
        .unwrap()
        .messages
        .into_iter()
        .find(|m| m.seq == seq)
        .unwrap()
        .attachments
        .remove(0);
    // Without the token, with a wrong one, or from outside the session:
    // nothing.
    let max = 10 * 1024 * 1024;
    let mallory = server.account("mallory").await.unwrap();
    for (c, token) in [(&bob_c, "AAAA"), (&mallory, a.blob_token.as_str())] {
        let e = c.fetch_blob(&a.url, token, max).await.unwrap_err();
        assert_eq!(e.status(), Some(404));
    }
    assert!(bob_c.fetch_blob(&a.url, &a.blob_token, max).await.is_ok());

    // The server flips a byte before Bob's device reads it.
    let pool = server.db().await.unwrap();
    sqlx::query("UPDATE blobs SET data = set_byte(data, 5, get_byte(data, 5) # 1)")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let r = sessions::read(&bob, &bob_c, &id).await.unwrap();
    let m = r.messages.iter().find(|m| m.seq == seq).unwrap();
    assert_eq!(m.content, "See attached.");
    assert!(
        sessions::file(&bob, &m.attachments[0].url)
            .unwrap()
            .is_none()
    );
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn limits_quotas_and_rates_hold() {
    let Some(server) = TestServer::start_with(|c| {
        c.limits.max_attachment_bytes = 64 * 1024;
        c.limits.account_quota_bytes = 96 * 1024;
        c.limits.messages_per_minute = 12;
    })
    .await
    .unwrap() else {
        return;
    };
    let (id, (alice, alice_c, _), _) = pair(&server).await;
    // Sizes that aren't buckets, or are over the limit, are refused.
    let e = alice_c.upload_blob(&id, vec![0; 5000]).await.unwrap_err();
    assert_eq!(e.status(), Some(400));
    let e = alice_c
        .upload_blob(&id, vec![0; 128 * 1024])
        .await
        .unwrap_err();
    assert_eq!(e.status(), Some(413));
    // Too big to attach at all.
    assert!(
        sessions::post(
            &alice,
            &alice_c,
            &id,
            sessions::Draft {
                files: vec![("big".into(), bytes(64 * 1024))],
                ..sessions::Draft::default()
            },
        )
        .await
        .is_err()
    );
    // 64 KiB fits; with it stored, another 64 KiB is over the quota.
    alice_c.upload_blob(&id, vec![0; 64 * 1024]).await.unwrap();
    let e = alice_c
        .upload_blob(&id, vec![0; 64 * 1024])
        .await
        .unwrap_err();
    assert_eq!(e.status(), Some(403));
    // Only 12 messages a minute.
    let mut refused = None;
    for i in 0..15 {
        if let Err(e) = sessions::write(&alice, &alice_c, &id, &format!("{i}"), None).await {
            refused = Some(e);
            break;
        }
    }
    assert_eq!(refused.unwrap().status(), Some(429));
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blobs_go_with_their_messages() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let (id, (alice, alice_c, _), (bob, bob_c, _)) = pair(&server).await;
    attach(&alice, &alice_c, &id, vec![("a.bin".into(), bytes(100))]).await;
    // An upload no message ever names.
    alice_c.upload_blob(&id, vec![0; 4096]).await.unwrap();
    let pool = server.db().await.unwrap();
    sqlx::query("UPDATE group_messages SET logged_at = now() - interval '8 days'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE blobs SET uploaded_at = now() - interval '2 days'")
        .execute(&pool)
        .await
        .unwrap();
    let count = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM blobs")
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    // Alice has read it all: the stray blob goes, the attached one stays
    // until Bob's device has it too.
    sessions::read(&alice, &alice_c, &id).await.unwrap();
    assert_eq!(count().await, 1);
    sessions::read(&bob, &bob_c, &id).await.unwrap();
    assert_eq!(count().await, 0);
    pool.close().await;
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn administrators_can_turn_attachments_off() {
    use quill_proto::board::UpdateServerSettings;
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let (id, (alice, alice_c, _), _) = pair(&server).await;
    let boss = server.admin_account("boss").await.unwrap();
    let off = boss
        .admin_update_settings(&UpdateServerSettings {
            attachments: Some(false),
            ..UpdateServerSettings::default()
        })
        .await
        .unwrap();
    assert!(!off.attachments);
    assert!(!alice_c.instance().await.unwrap().attachments);
    let e = alice_c.upload_blob(&id, vec![0; 4096]).await.unwrap_err();
    assert_eq!(e.status(), Some(403));
    let e = sessions::post(
        &alice,
        &alice_c,
        &id,
        sessions::Draft {
            files: vec![("a.bin".into(), bytes(10))],
            ..sessions::Draft::default()
        },
    )
    .await
    .unwrap_err();
    assert!(e.to_string().contains("doesn't take attachments"), "{e}");
    // Writing goes on as before.
    sessions::write(&alice, &alice_c, &id, "Words only.", None)
        .await
        .unwrap();
    let log = boss.admin_log(None, None).await.unwrap();
    assert!(
        log.iter()
            .any(|e| e.detail.as_deref() == Some("attachments in sessions: allowed → not allowed")),
        "{log:?}"
    );
    boss.admin_update_settings(&UpdateServerSettings {
        attachments: Some(true),
        ..UpdateServerSettings::default()
    })
    .await
    .unwrap();
    attach(&alice, &alice_c, &id, vec![("a.bin".into(), bytes(10))]).await;
    server.stop().await.unwrap();
}
