//! Phase 2k end-to-end tests, the server's side: deleting by age, `410
//! Gone` for readers who fell behind, and blobs in the login's device
//! group. Messages are hand-built with junk ciphertext, as in `groups.rs`.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use quill_e2e::TestServer;
use quill_proto::{
    groups::{CreateGroup, MessagesQuery, PostMessage},
    keys::b64,
    limits::BLOB_MIN_BUCKET,
};
use quill_server::retention;

macro_rules! server {
    () => {
        match TestServer::start().await.unwrap() {
            Some(s) => s,
            None => return,
        }
    };
}

/// An application `PrivateMessage` for `group` at epoch 0, junk inside.
fn message(group: &[u8]) -> String {
    let mut m = vec![0, 1, 0, 2];
    m.push(u8::try_from(group.len()).unwrap());
    m.extend_from_slice(group);
    m.extend_from_slice(&0u64.to_be_bytes());
    m.push(1);
    m.extend_from_slice(&[0]);
    m.extend_from_slice(&[4, 1, 2, 3, 4]);
    m.extend_from_slice(&[3, 9, 9, 9]);
    b64(&m)
}

fn after(n: u64) -> MessagesQuery {
    MessagesQuery {
        after: Some(n),
        limit: None,
    }
}

async fn age(db: &sqlx::PgPool, group: &[u8], seqs: &[i64], days: i32) {
    sqlx::query(
        "UPDATE group_messages SET logged_at = now() - make_interval(days => $3)
         WHERE group_id = $1 AND seq = ANY($2)",
    )
    .bind(group)
    .bind(seqs)
    .bind(days)
    .execute(db)
    .await
    .unwrap();
}

#[tokio::test]
async fn old_messages_go_and_readers_from_before_them_get_gone() {
    let server = server!();
    let db = server.db().await.unwrap();
    let alice = server.account("alice").await.unwrap();
    let gid = [3u8; 32];
    let g = b64(&gid);
    alice
        .create_group(&CreateGroup {
            group_id: g.clone(),
            kind: "personal".into(),
        })
        .await
        .unwrap();
    for _ in 0..3 {
        alice
            .post_group_message(
                &g,
                &PostMessage {
                    message: message(&gid),
                    blobs: Vec::new(),
                },
            )
            .await
            .unwrap();
    }
    // Inside the retention period, nothing goes.
    age(&db, &gid, &[1, 2], 29).await;
    let swept = retention::sweep(&db, Some(30)).await.unwrap();
    assert_eq!(swept.aged, 0);
    assert_eq!(
        alice
            .group_messages(&g, &after(0))
            .await
            .unwrap()
            .messages
            .len(),
        3
    );

    age(&db, &gid, &[1, 2], 31).await;
    // With deleting by age turned off (QUILL_RETENTION_DAYS=-1), everything
    // stays however old.
    assert_eq!(retention::sweep(&db, None).await.unwrap().aged, 0);
    assert_eq!(
        alice
            .group_messages(&g, &after(0))
            .await
            .unwrap()
            .messages
            .len(),
        3
    );
    assert_eq!(retention::sweep(&db, Some(30)).await.unwrap().aged, 2);
    let pruned: i64 =
        sqlx::query_scalar("SELECT pruned_through FROM mls_groups WHERE group_id = $1")
            .bind(gid.as_slice())
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(pruned, 2);
    // A reader that last read before the deleted part can't follow.
    for from in [0, 1] {
        let e = alice.group_messages(&g, &after(from)).await.unwrap_err();
        assert_eq!(e.status(), Some(410), "{e}");
    }
    let rest = alice.group_messages(&g, &after(2)).await.unwrap();
    assert_eq!(rest.messages.len(), 1);
    assert_eq!(rest.messages[0].seq, 3);
    // Numbering carries on past what's gone.
    let posted = alice
        .post_group_message(
            &g,
            &PostMessage {
                message: message(&gid),
                blobs: Vec::new(),
            },
        )
        .await
        .unwrap();
    assert_eq!(posted.seq, 4);
}

#[tokio::test]
async fn the_device_group_takes_blobs_only_its_login_can_use() {
    let server = server!();
    let db = server.db().await.unwrap();
    let alice = server.account("alice").await.unwrap();
    let mallory = server.account("mallory").await.unwrap();
    let gid = [5u8; 32];
    let g = b64(&gid);
    alice
        .create_group(&CreateGroup {
            group_id: g.clone(),
            kind: "personal".into(),
        })
        .await
        .unwrap();
    let blob = vec![9u8; usize::try_from(BLOB_MIN_BUCKET).unwrap()];
    // Someone else's device group takes nothing.
    let e = mallory
        .upload_group_blob(&g, blob.clone())
        .await
        .unwrap_err();
    assert_eq!(e.status(), Some(404), "{e}");

    // Turning attachments off is about sessions; the device group still
    // takes them.
    sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('attachments', 'false')
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
    )
    .execute(&db)
    .await
    .unwrap();
    let up = alice.upload_group_blob(&g, blob.clone()).await.unwrap();
    let url = alice.blob_url(up.id).unwrap();
    let max = alice.instance().await.unwrap().limits.max_attachment_bytes;
    assert_eq!(alice.fetch_blob(&url, &up.token, max).await.unwrap(), blob);
    for e in [
        mallory.fetch_blob(&url, &up.token, max).await.unwrap_err(),
        alice
            .fetch_blob(&url, &b64(&[0; 32]), max)
            .await
            .unwrap_err(),
    ] {
        assert_eq!(e.status(), Some(404), "{e}");
    }
    let e = mallory.delete_blob(&url, &up.token).await.unwrap_err();
    assert_eq!(e.status(), Some(404), "{e}");

    // Carried by a message, it lasts as long as the message does...
    alice
        .post_group_message(
            &g,
            &PostMessage {
                message: message(&gid),
                blobs: vec![up.id],
            },
        )
        .await
        .unwrap();
    let second = alice.upload_group_blob(&g, blob.clone()).await.unwrap();
    alice
        .post_group_message(
            &g,
            &PostMessage {
                message: message(&gid),
                blobs: vec![second.id],
            },
        )
        .await
        .unwrap();
    age(&db, &gid, &[1], 31).await;
    let swept = retention::sweep(&db, Some(30)).await.unwrap();
    assert_eq!((swept.aged, swept.blobs), (1, 1));
    let e = alice.fetch_blob(&url, &up.token, max).await.unwrap_err();
    assert_eq!(e.status(), Some(404), "{e}");
    // ...or until the login deletes it, once read.
    let url2 = alice.blob_url(second.id).unwrap();
    alice.delete_blob(&url2, &second.token).await.unwrap();
    let e = alice
        .fetch_blob(&url2, &second.token, max)
        .await
        .unwrap_err();
    assert_eq!(e.status(), Some(404), "{e}");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM blobs")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn a_server_keeping_messages_regardless_of_age_says_so() {
    let Some(server) = TestServer::start_with(|c| c.limits.retention_days = None)
        .await
        .unwrap()
    else {
        return;
    };
    let info = server.client().instance().await.unwrap();
    assert_eq!(info.limits.retention_days, None);
    let raw: serde_json::Value = reqwest::get(format!("{}/api/v1/instance", server.url()))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(raw["limits"]["retentionDays"].is_null(), "{raw}");
}
