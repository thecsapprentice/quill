//! Phase 2d end-to-end tests: the server's side of group message logs (the
//! epoch sequencer) and the link mailbox. The server reads only clear
//! headers, so these tests post hand-built messages with junk ciphertext.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use quill_core::CoreError;
use quill_e2e::TestServer;
use quill_proto::{
    groups::{CreateGroup, LinkReply, MessagesQuery, PostMessage, StartLink},
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

fn status(err: &CoreError) -> u16 {
    err.status().unwrap_or_default()
}

/// An `MLSMessage` holding a `PrivateMessage` with this header and junk
/// encrypted fields. Content types: 1 application, 3 commit.
fn message(group: &[u8], epoch: u64, content_type: u8) -> String {
    let mut m = vec![0, 1, 0, 2];
    m.push(u8::try_from(group.len()).unwrap());
    m.extend_from_slice(group);
    m.extend_from_slice(&epoch.to_be_bytes());
    m.push(content_type);
    m.extend_from_slice(&[0]); // authenticated_data: empty
    m.extend_from_slice(&[4, 1, 2, 3, 4]); // encrypted_sender_data
    m.extend_from_slice(&[3, 9, 9, 9]); // ciphertext
    b64(&m)
}

fn post(m: String) -> PostMessage {
    PostMessage {
        message: m,
        blobs: Vec::new(),
    }
}

#[tokio::test]
async fn the_first_commit_for_an_epoch_wins() {
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    let mallory = server.account("mallory").await.unwrap();
    let gid = [7u8; 32];
    let g = b64(&gid);
    let create = CreateGroup {
        group_id: g.clone(),
        kind: "personal".into(),
    };
    assert!(alice.device_group().await.unwrap().is_none());
    let state = alice.create_group(&create).await.unwrap();
    assert_eq!((state.epoch, state.last_seq), (0, 0));
    // One personal group per login.
    let again = CreateGroup {
        group_id: b64(&[8u8; 32]),
        kind: "personal".into(),
    };
    assert_eq!(status(&alice.create_group(&again).await.unwrap_err()), 409);
    assert_eq!(alice.device_group().await.unwrap().unwrap().group_id, g);

    // Application message at epoch 0, then a commit: epoch 1.
    let p = alice
        .post_group_message(&g, &post(message(&gid, 0, 1)))
        .await
        .unwrap();
    assert_eq!((p.seq, p.epoch), (1, 0));
    let p = alice
        .post_group_message(&g, &post(message(&gid, 0, 3)))
        .await
        .unwrap();
    assert_eq!((p.seq, p.epoch), (2, 1));
    // A second commit for epoch 0 lost the race; so does anything else for it.
    for ct in [3, 1] {
        let e = alice
            .post_group_message(&g, &post(message(&gid, 0, ct)))
            .await
            .unwrap_err();
        assert_eq!(status(&e), 409);
    }
    // A message from the future is refused too.
    let e = alice
        .post_group_message(&g, &post(message(&gid, 5, 1)))
        .await
        .unwrap_err();
    assert_eq!(status(&e), 409);
    alice
        .post_group_message(&g, &post(message(&gid, 1, 1)))
        .await
        .unwrap();

    // Malformed, for another group, or not a PrivateMessage.
    for bad in [
        b64(b"junk"),
        message(&[9u8; 32], 1, 1),
        b64(&[0, 1, 0, 1, 0, 0]),
    ] {
        let e = alice.post_group_message(&g, &post(bad)).await.unwrap_err();
        assert_eq!(status(&e), 400);
    }
    // Other logins can't see or use the group.
    let e = mallory
        .post_group_message(&g, &post(message(&gid, 1, 1)))
        .await
        .unwrap_err();
    assert_eq!(status(&e), 404);
    let e = mallory
        .group_messages(&g, &MessagesQuery::default())
        .await
        .unwrap_err();
    assert_eq!(status(&e), 404);

    // Reading in pages, in order.
    let page = alice
        .group_messages(
            &g,
            &MessagesQuery {
                after: Some(0),
                limit: Some(2),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        page.messages.iter().map(|m| m.seq).collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(page.more);
    let rest = alice
        .group_messages(
            &g,
            &MessagesQuery {
                after: Some(2),
                limit: Some(2),
            },
        )
        .await
        .unwrap();
    assert_eq!(rest.messages.iter().map(|m| m.seq).collect::<Vec<_>>(), [3]);
    assert!(!rest.more);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn the_link_mailbox_is_private_answered_once_and_short_lived() {
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    let mallory = server.account("mallory").await.unwrap();
    // A real KeyPackage to leave in the mailbox: any device's will do.
    let store = quill_core::Store::in_memory();
    let pending = quill_core::devices::start_link(&store, &alice)
        .await
        .unwrap();
    let id = pending.link_id.clone();

    // Junk isn't accepted as a KeyPackage.
    let e = alice
        .start_link(&StartLink {
            key_package: b64(b"junk"),
        })
        .await
        .unwrap_err();
    assert_eq!(status(&e), 400);
    // Only the same login sees or answers it.
    assert_eq!(status(&mallory.read_link(&id).await.unwrap_err()), 404);
    let reply = LinkReply {
        welcome: b64(&[0, 1, 0, 3, 1, 2, 3]),
        from_seq: 1,
    };
    assert_eq!(
        status(&mallory.reply_link(&id, &reply).await.unwrap_err()),
        404
    );
    // A reply must be a Welcome, and there's only one.
    let not_welcome = LinkReply {
        welcome: b64(&[0, 1, 0, 2, 1]),
        from_seq: 1,
    };
    assert_eq!(
        status(&alice.reply_link(&id, &not_welcome).await.unwrap_err()),
        400
    );
    alice.reply_link(&id, &reply).await.unwrap();
    assert_eq!(
        status(&alice.reply_link(&id, &reply).await.unwrap_err()),
        404
    );
    // Reading the reply empties the mailbox.
    let read = alice.read_link(&id).await.unwrap();
    assert!(read.welcome.is_some());
    assert_eq!(status(&alice.read_link(&id).await.unwrap_err()), 404);

    // Expired mailboxes are gone.
    let other = quill_core::Store::in_memory();
    let late = quill_core::devices::start_link(&other, &alice)
        .await
        .unwrap();
    let pool = server.db().await.unwrap();
    sqlx::query("UPDATE link_mailbox SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        status(&alice.read_link(&late.link_id).await.unwrap_err()),
        404
    );
    pool.close().await;
    server.stop().await.unwrap();
}
