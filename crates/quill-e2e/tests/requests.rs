//! Phase 2e end-to-end tests: collaboration requests. A request reaches only
//! the author's devices; held, refused and declined requests all look like
//! "waiting" to the requester; one waiting request per login per prompt;
//! acceptance makes the group a session; forged groups are caught.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use quill_core::{Client, CoreError, Store, devices, keys, requests};
use quill_e2e::{TEST_PASSWORD as PASSWORD, TestServer};
use quill_proto::{
    api::{CreatePersona, Persona},
    board::{CreatePrompt, Maturity, Prompt, PromptState, UpdatePrompt},
    groups::{MessagesQuery, PostMessage},
    keys::b64,
    requests::{RequestStatus, SendRequest},
};
use uuid::Uuid;

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

/// A persona with keys on a fresh device.
async fn writer(client: &Client, handle: &str) -> (Persona, Store) {
    let p = persona(client, handle).await;
    let store = Store::in_memory();
    keys::init(&store, client, &p, false).await.unwrap();
    keys::publish(&store, client, &p, 5).await.unwrap();
    (p, store)
}

async fn prompt(client: &Client, persona: Uuid, state: PromptState) -> Prompt {
    client
        .create_prompt(&CreatePrompt {
            persona,
            title: "The lighthouse ledger".into(),
            body: "A keeper finds a ledger of ships that never sailed.".into(),
            maturity: Maturity::General,
            tags: Vec::new(),
            state: Some(state),
        })
        .await
        .unwrap()
}

async fn set_state(client: &Client, prompt: Uuid, state: PromptState) {
    client
        .update_prompt(
            prompt,
            &UpdatePrompt {
                state: Some(state),
                ..UpdatePrompt::default()
            },
        )
        .await
        .unwrap();
}

async fn own_status(client: &Client, id: Uuid) -> RequestStatus {
    requests::list(client)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.id == id)
        .unwrap()
        .status
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

async fn link(new: (&Store, &Client), existing: (&Store, &Client)) {
    let pending = devices::start_link(new.0, new.1).await.unwrap();
    devices::approve(existing.0, existing.1, &pending.code.parse().unwrap())
        .await
        .unwrap();
    devices::wait_for_link(
        new.0,
        new.1,
        &pending.link_id,
        Duration::from_millis(50),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    devices::finish_link(new.0, new.1).await.unwrap();
}

/// An `MLSMessage` holding a `PrivateMessage` with this header and junk
/// encrypted fields. Content types: 1 application, 3 commit.
fn message(group: &[u8], epoch: u64, content_type: u8) -> String {
    let mut m = vec![0, 1, 0, 2];
    m.push(u8::try_from(group.len()).unwrap());
    m.extend_from_slice(group);
    m.extend_from_slice(&epoch.to_be_bytes());
    m.push(content_type);
    m.extend_from_slice(&[0]);
    m.extend_from_slice(&[4, 1, 2, 3, 4]);
    m.extend_from_slice(&[3, 9, 9, 9]);
    b64(&m)
}

const PITCH: &str = "I'd write the keeper.\n\nShe should never open the ledger's last page.";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_reaches_the_author_who_accepts_it() {
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let fox = persona(&alice_c, "ink_fox").await;
    // No keys yet: the prompt can't take requests.
    let p = prompt(&alice_c, fox.id, PromptState::Open).await;
    assert!(!p.accepting_sessions);
    let alice = Store::in_memory();
    keys::init(&alice, &alice_c, &fox, false).await.unwrap();
    keys::publish(&alice, &alice_c, &fox, 5).await.unwrap();
    assert!(alice_c.prompt(p.id).await.unwrap().accepting_sessions);

    // Bob writes from two computers; both go into the group.
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    let bob_phone = Store::in_memory();
    let bob_phone_c = same_login(&server, "bob").await;
    link((&bob_phone, &bob_phone_c), (&bob, &bob_c)).await;

    let sent = requests::send(&bob, &bob_c, &wren, p.id, PITCH, None)
        .await
        .unwrap();
    assert_eq!((sent.author_devices, sent.own_devices), (1, 1));
    assert!(sent.skipped.is_empty());
    assert_eq!(
        own_status(&bob_c, sent.request.id).await,
        RequestStatus::Waiting
    );
    // The requester can't read the queue or decide for the author.
    assert_eq!(
        status(&bob_c.request_queue(p.id, false).await.unwrap_err()),
        404
    );
    assert_eq!(
        status(&bob_c.accept_request(sent.request.id).await.unwrap_err()),
        404
    );

    let inbox = requests::inbox(&alice, &alice_c, p.id, false)
        .await
        .unwrap();
    assert_eq!(inbox.held, 0);
    assert_eq!(inbox.requests.len(), 1);
    let r = &inbox.requests[0];
    assert_eq!(r.id, sent.request.id);
    assert_eq!(r.requester.handle, "quiet_wren");
    assert_eq!(r.pitch.as_deref(), Ok(PITCH));
    // Read again: from this device's note (MLS decrypts only once).
    let again = requests::inbox(&alice, &alice_c, p.id, false)
        .await
        .unwrap();
    assert_eq!(again.requests[0].pitch.as_deref(), Ok(PITCH));
    assert_eq!(
        alice_c.request_status(r.id).await.unwrap().status,
        RequestStatus::Pending
    );

    let group = requests::accept(&alice, &alice_c, r.id).await.unwrap();
    assert_eq!(own_status(&bob_c, r.id).await, RequestStatus::Session);
    // Upkeep keeps both sides' groups and notes the session.
    assert_eq!(
        devices::maintain(&bob, &bob_c)
            .await
            .unwrap()
            .requests_dropped,
        0
    );
    assert_eq!(
        requests::session_group(&bob, r.id).unwrap().as_deref(),
        Some(group.as_str())
    );
    assert_eq!(
        requests::session_group(&alice, r.id).unwrap().as_deref(),
        Some(group.as_str())
    );

    // The session's log starts with the pitch; both logins can use it, and
    // nobody else can.
    for c in [&alice_c, &bob_c] {
        let log = c
            .group_messages(&group, &MessagesQuery::default())
            .await
            .unwrap();
        assert_eq!(log.messages.len(), 1);
        assert_eq!(log.messages[0].seq, 1);
    }
    let gid = quill_proto::keys::unb64(&group).unwrap();
    let posted = bob_c
        .post_group_message(
            &group,
            &PostMessage {
                message: message(&gid, 1, 1),
            },
        )
        .await
        .unwrap();
    assert_eq!(posted.seq, 2);
    let mallory = server.account("mallory").await.unwrap();
    assert_eq!(
        status(
            &mallory
                .group_messages(&group, &MessagesQuery::default())
                .await
                .unwrap_err()
        ),
        404
    );
    // Accepted is final.
    assert_eq!(
        status(&bob_c.withdraw_request(r.id).await.unwrap_err()),
        409
    );
    assert_eq!(
        status(&alice_c.decline_request(r.id).await.unwrap_err()),
        409
    );
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declines_are_silent_and_one_request_waits_per_login() {
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let (fox, alice) = writer(&alice_c, "ink_fox").await;
    let p = prompt(&alice_c, fox.id, PromptState::Open).await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    // Bob's second persona, with its keys on another computer of his.
    let (gull, bob_gull) = writer(&bob_c, "grey_gull").await;

    let first = requests::send(&bob, &bob_c, &wren, p.id, PITCH, Some(3))
        .await
        .unwrap();
    // A second request from any of the login's personas waits its turn.
    let e = requests::send(&bob_gull, &bob_c, &gull, p.id, PITCH, None)
        .await
        .unwrap_err();
    assert_eq!(status(&e), 409);

    // Alice declines without reading it: Bob still sees "waiting", and
    // still can't send another.
    requests::decline(&alice, &alice_c, first.request.id)
        .await
        .unwrap();
    assert_eq!(
        own_status(&bob_c, first.request.id).await,
        RequestStatus::Waiting
    );
    assert_eq!(
        bob_c.request_status(first.request.id).await.unwrap().status,
        RequestStatus::Waiting
    );
    assert_eq!(
        alice_c
            .request_status(first.request.id)
            .await
            .unwrap()
            .status,
        RequestStatus::Declined
    );
    let e = requests::send(&bob, &bob_c, &wren, p.id, PITCH, None)
        .await
        .unwrap_err();
    assert_eq!(status(&e), 409);
    assert!(
        requests::inbox(&alice, &alice_c, p.id, false)
            .await
            .unwrap()
            .requests
            .is_empty()
    );

    // Withdrawing frees the slot.
    requests::withdraw(&bob, &bob_c, first.request.id)
        .await
        .unwrap();
    let second = requests::send(&bob_gull, &bob_c, &gull, p.id, PITCH, None)
        .await
        .unwrap();
    // Alice reads it, then declines: her device drops the group.
    let inbox = requests::inbox(&alice, &alice_c, p.id, false)
        .await
        .unwrap();
    assert_eq!(inbox.requests[0].pitch.as_deref(), Ok(PITCH));
    requests::decline(&alice, &alice_c, second.request.id)
        .await
        .unwrap();
    assert!(
        requests::session_group(&alice, second.request.id)
            .unwrap()
            .is_none()
    );

    // Expiry: "no response", and upkeep drops Bob's group.
    let pool = server.db().await.unwrap();
    sqlx::query("UPDATE session_requests SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        own_status(&bob_c, second.request.id).await,
        RequestStatus::NoResponse
    );
    assert_eq!(
        devices::maintain(&bob_gull, &bob_c)
            .await
            .unwrap()
            .requests_dropped,
        1
    );
    // The slot is free again; an expired request can be dismissed.
    requests::send(&bob, &bob_c, &wren, p.id, PITCH, None)
        .await
        .unwrap();
    requests::withdraw(&bob_gull, &bob_c, second.request.id)
        .await
        .unwrap();
    assert_eq!(requests::list(&bob_c).await.unwrap().len(), 1);
    // No ciphertext outlives a decision.
    let left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM session_requests WHERE state <> 'pending' AND (welcome IS NOT NULL OR pitch IS NOT NULL)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(left, 0);
    pool.close().await;
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn busy_prompts_hold_closed_ones_refuse_and_deleting_expires() {
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let (fox, alice) = writer(&alice_c, "ink_fox").await;
    let p = prompt(&alice_c, fox.id, PromptState::Busy).await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    let carol_c = server.account("carol").await.unwrap();
    let (moth, carol) = writer(&carol_c, "paper_moth").await;
    let dave_c = server.account("dave").await.unwrap();
    let (heron, dave) = writer(&dave_c, "slow_heron").await;

    // Busy: held, out of the queue until released.
    let held = requests::send(&bob, &bob_c, &wren, p.id, PITCH, None)
        .await
        .unwrap();
    let inbox = requests::inbox(&alice, &alice_c, p.id, false)
        .await
        .unwrap();
    assert!(inbox.requests.is_empty());
    assert_eq!(inbox.held, 1);
    assert_eq!(
        alice_c
            .request_status(held.request.id)
            .await
            .unwrap()
            .status,
        RequestStatus::Held
    );
    let inbox = requests::inbox(&alice, &alice_c, p.id, true).await.unwrap();
    assert_eq!((inbox.requests.len(), inbox.held), (1, 0));
    assert_eq!(inbox.requests[0].pitch.as_deref(), Ok(PITCH));

    // Closed: refused, silently. Carol sees what anyone sees.
    set_state(&alice_c, p.id, PromptState::Closed).await;
    let refused = requests::send(&carol, &carol_c, &moth, p.id, PITCH, None)
        .await
        .unwrap();
    assert_eq!(
        own_status(&carol_c, refused.request.id).await,
        RequestStatus::Waiting
    );
    let inbox = requests::inbox(&alice, &alice_c, p.id, true).await.unwrap();
    assert_eq!((inbox.requests.len(), inbox.held), (1, 0));
    assert_eq!(
        status(
            &alice_c
                .request_status(refused.request.id)
                .await
                .unwrap_err()
        ),
        404
    );
    assert_eq!(
        status(
            &alice_c
                .decline_request(refused.request.id)
                .await
                .unwrap_err()
        ),
        404
    );
    // A refused request still takes the slot.
    let e = requests::send(&carol, &carol_c, &moth, p.id, PITCH, None)
        .await
        .unwrap_err();
    assert_eq!(status(&e), 409);

    // Reopening releases what was held while busy.
    set_state(&alice_c, p.id, PromptState::Busy).await;
    let dave_req = requests::send(&dave, &dave_c, &heron, p.id, PITCH, None)
        .await
        .unwrap();
    set_state(&alice_c, p.id, PromptState::Open).await;
    let inbox = requests::inbox(&alice, &alice_c, p.id, false)
        .await
        .unwrap();
    assert_eq!((inbox.requests.len(), inbox.held), (2, 0));

    // Deleting the prompt expires every waiting request at once.
    alice_c.delete_prompt(p.id).await.unwrap();
    for (c, id) in [
        (&bob_c, held.request.id),
        (&carol_c, refused.request.id),
        (&dave_c, dave_req.request.id),
    ] {
        let r = requests::list(c)
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.id == id)
            .unwrap();
        assert_eq!((r.status, r.prompt), (RequestStatus::NoResponse, None));
    }
    // Alice's device drops the groups it joined.
    assert_eq!(
        devices::maintain(&alice, &alice_c)
            .await
            .unwrap()
            .requests_dropped,
        2
    );
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_authors_client_catches_a_forged_group() {
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let (fox, alice) = writer(&alice_c, "ink_fox").await;
    let p = prompt(&alice_c, fox.id, PromptState::Open).await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, _bob) = writer(&bob_c, "quiet_wren").await;
    let mallory_c = server.account("mallory").await.unwrap();
    let (moth, mallory) = writer(&mallory_c, "grey_moth").await;

    // Mallory builds a request; the server then says it's from Bob.
    let sent = requests::send(&mallory, &mallory_c, &moth, p.id, PITCH, None)
        .await
        .unwrap();
    let pool = server.db().await.unwrap();
    sqlx::query("UPDATE session_requests SET requester_actor_id = $1 WHERE id = $2")
        .bind(wren.id)
        .bind(sent.request.id)
        .execute(&pool)
        .await
        .unwrap();
    let inbox = requests::inbox(&alice, &alice_c, p.id, false)
        .await
        .unwrap();
    assert_eq!(inbox.requests[0].requester.handle, "quiet_wren");
    let why = inbox.requests[0].pitch.clone().unwrap_err();
    assert!(why.contains("neither yours nor the requester's"), "{why}");
    // Nothing was kept, so it can't be accepted from here.
    assert!(
        requests::accept(&alice, &alice_c, sent.request.id)
            .await
            .is_err()
    );
    pool.close().await;

    // A device linked after the request was sent can't read it.
    let desktop = Store::in_memory();
    let desktop_c = same_login(&server, "alice").await;
    link((&desktop, &desktop_c), (&alice, &alice_c)).await;
    let bob2_c = server.account("bob2").await.unwrap();
    let (gull, bob2) = writer(&bob2_c, "grey_gull").await;
    let p2 = prompt(&alice_c, fox.id, PromptState::Open).await;
    // A request sent now reaches both of Alice's devices.
    let sent = requests::send(&bob2, &bob2_c, &gull, p2.id, PITCH, None)
        .await
        .unwrap();
    assert_eq!(sent.author_devices, 2);
    for (s, c) in [(&alice, &alice_c), (&desktop, &desktop_c)] {
        let inbox = requests::inbox(s, c, p2.id, false).await.unwrap();
        assert_eq!(inbox.requests[0].pitch.as_deref(), Ok(PITCH));
    }
    let phone = Store::in_memory();
    let phone_c = same_login(&server, "alice").await;
    link((&phone, &phone_c), (&desktop, &desktop_c)).await;
    let inbox = requests::inbox(&phone, &phone_c, p2.id, false)
        .await
        .unwrap();
    let why = inbox.requests[0].pitch.clone().unwrap_err();
    assert!(why.contains("wasn't sent to this device"), "{why}");
    server.stop().await.unwrap();
}

#[tokio::test]
async fn the_server_checks_what_it_files() {
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let fox = persona(&alice_c, "ink_fox").await;
    let p = prompt(&alice_c, fox.id, PromptState::Open).await;
    let bob_c = server.account("bob").await.unwrap();
    let wren = persona(&bob_c, "quiet_wren").await;
    let gid = [5u8; 32];
    let welcome = b64(&[0, 1, 0, 3, 1, 2, 3]);
    let good = || SendRequest {
        persona: wren.id,
        prompt: p.id,
        welcome: welcome.clone(),
        pitch: message(&gid, 1, 1),
        expires_in_days: None,
    };
    for (bad, code) in [
        (
            SendRequest {
                welcome: b64(&[0, 1, 0, 2, 1]),
                ..good()
            },
            400,
        ),
        (
            SendRequest {
                pitch: message(&gid, 1, 3),
                ..good()
            },
            400,
        ),
        (
            SendRequest {
                pitch: message(&[1u8; 8], 1, 1),
                ..good()
            },
            400,
        ),
        (
            SendRequest {
                expires_in_days: Some(31),
                ..good()
            },
            400,
        ),
        (
            SendRequest {
                expires_in_days: Some(0),
                ..good()
            },
            400,
        ),
        (
            SendRequest {
                persona: fox.id,
                ..good()
            },
            404,
        ),
        (
            SendRequest {
                prompt: Uuid::new_v4(),
                ..good()
            },
            404,
        ),
    ] {
        let e = bob_c.send_request(&bad).await.unwrap_err();
        assert_eq!(status(&e), code, "{bad:?}");
    }
    // Not to one's own prompt.
    let e = alice_c
        .send_request(&SendRequest {
            persona: fox.id,
            ..good()
        })
        .await
        .unwrap_err();
    assert_eq!(status(&e), 400);

    let sent = bob_c.send_request(&good()).await.unwrap();
    // The same group id can't be filed twice.
    let carol_c = server.account("carol").await.unwrap();
    let moth = persona(&carol_c, "paper_moth").await;
    let e = carol_c
        .send_request(&SendRequest {
            persona: moth.id,
            ..good()
        })
        .await
        .unwrap_err();
    assert_eq!(status(&e), 409);
    // Only the requester withdraws; only the author decides or reads.
    assert_eq!(
        status(&alice_c.withdraw_request(sent.id).await.unwrap_err()),
        404
    );
    assert_eq!(
        status(&carol_c.decline_request(sent.id).await.unwrap_err()),
        404
    );
    assert_eq!(
        status(&carol_c.request_status(sent.id).await.unwrap_err()),
        404
    );
    let queue = alice_c.request_queue(p.id, false).await.unwrap();
    assert_eq!(queue.requests.len(), 1);
    assert_eq!(queue.requests[0].requester.actor_uri, wren.actor_uri);
    // Accepting registers the group at the pitch's epoch.
    let state = alice_c.accept_request(sent.id).await.unwrap();
    assert_eq!((state.epoch, state.last_seq), (1, 1));
    assert_eq!(
        status(&alice_c.accept_request(sent.id).await.unwrap_err()),
        409
    );
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_the_authors_persona_means_no_response() {
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let (fox, _alice) = writer(&alice_c, "ink_fox").await;
    let p = prompt(&alice_c, fox.id, PromptState::Open).await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    let sent = requests::send(&bob, &bob_c, &wren, p.id, PITCH, None)
        .await
        .unwrap();
    alice_c.delete_persona(fox.id).await.unwrap();
    let r = requests::list(&bob_c)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.id == sent.request.id)
        .unwrap();
    assert_eq!((r.status, r.prompt), (RequestStatus::NoResponse, None));
    assert_eq!(
        bob_c.request_status(sent.request.id).await.unwrap().status,
        RequestStatus::NoResponse
    );
    assert_eq!(
        devices::maintain(&bob, &bob_c)
            .await
            .unwrap()
            .requests_dropped,
        1
    );
    requests::withdraw(&bob, &bob_c, sent.request.id)
        .await
        .unwrap();
    server.stop().await.unwrap();
}
