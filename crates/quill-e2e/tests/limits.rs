//! Size limits at their edges (2m, `test-vectors/limits.json`): an MLS
//! message exactly at the instance's limit gets past the size check, and
//! one byte more is refused with 413 before anything else is looked at.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use quill_e2e::TestServer;
use quill_proto::keys::b64;
use serde_json::{Value, json};

#[tokio::test]
async fn messages_at_the_limit_pass_the_size_check_and_one_byte_more_doesnt() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let vectors: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test-vectors/limits.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let client = server.account("alice").await.unwrap();
    let info = client.instance().await.unwrap();
    assert_eq!(
        vectors["defaults"]["maxMessageBytes"].as_u64(),
        Some(quill_proto::limits::DEFAULT_MAX_MESSAGE_BYTES)
    );
    let max = usize::try_from(info.limits.max_message_bytes).unwrap();
    let token = {
        let r = reqwest::Client::new()
            .post(format!("{}/api/v1/auth/login", server.url()))
            .json(&json!({ "username": "alice", "password": quill_e2e::TEST_PASSWORD }))
            .send()
            .await
            .unwrap();
        r.json::<Value>().await.unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let post = |len: usize| {
        let url = format!("{}/api/v1/groups/{}/messages", server.url(), b64(&[7; 32]));
        let token = token.clone();
        async move {
            reqwest::Client::new()
                .post(url)
                .bearer_auth(token)
                .json(&json!({ "message": b64(&vec![0u8; len]) }))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }
    };
    // At the limit, the size check passes and the next check answers
    // instead: zeros aren't an MLS message.
    assert_eq!(post(max).await, 400);
    assert_eq!(post(max + 1).await, 413);
    server.stop().await.unwrap();
}
