//! Error codes (#76) end to end: the server's refusals carry the code
//! documented in `docs/error-codes.md`, as the `code` member of the
//! problem reply, and a client reads it back.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use quill_core::{Client, CoreError};
use quill_e2e::{TEST_PASSWORD as PASSWORD, TestServer};
use quill_proto::{
    api::{CreatePersona, Credentials, Persona},
    board::{CreatePrompt, Maturity, Prompt, PromptState},
    codes::{self, ErrorCode},
    keys::b64,
    requests::SendRequest,
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

/// The code on a refusal; panics if the error isn't a problem reply.
fn code(err: &CoreError) -> ErrorCode {
    match err {
        CoreError::Api(problem) => ErrorCode(problem.code.expect("the reply has a code")),
        other => panic!("not a problem reply: {other:?}"),
    }
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

fn new_prompt(persona: Uuid, title: &str) -> CreatePrompt {
    CreatePrompt {
        persona,
        title: title.into(),
        body: "A keeper finds a ledger of ships that never sailed.".into(),
        maturity: Maturity::General,
        tags: Vec::new(),
        state: Some(PromptState::Open),
    }
}

async fn prompt(client: &Client, persona: Uuid) -> Prompt {
    client
        .create_prompt(&new_prompt(persona, "The lighthouse ledger"))
        .await
        .unwrap()
}

/// An `MLSMessage` holding an application `PrivateMessage` with junk
/// encrypted fields, as `tests/requests.rs` builds it.
fn pitch(group: &[u8]) -> String {
    let mut m = vec![0, 1, 0, 2];
    m.push(u8::try_from(group.len()).unwrap());
    m.extend_from_slice(group);
    m.extend_from_slice(&1u64.to_be_bytes());
    m.push(1);
    m.extend_from_slice(&[0]);
    m.extend_from_slice(&[4, 1, 2, 3, 4]);
    m.extend_from_slice(&[3, 9, 9, 9]);
    b64(&m)
}

#[tokio::test]
async fn a_wrong_password_is_wrong_login() {
    let server = server!();
    server.account("alice").await.unwrap();
    let e = server
        .client()
        .login(&Credentials {
            username: "alice".into(),
            password: format!("{PASSWORD}-not"),
        })
        .await
        .unwrap_err();
    assert_eq!(code(&e), codes::WRONG_LOGIN);
}

#[tokio::test]
async fn an_unknown_prompt_is_not_found() {
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let e = alice_c.prompt(Uuid::new_v4()).await.unwrap_err();
    assert_eq!(code(&e), codes::NOT_FOUND);
}

#[tokio::test]
async fn a_validation_failure_carries_its_check_code() {
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let fox = persona(&alice_c, "ink_fox").await;
    let e = alice_c
        .create_prompt(&new_prompt(fox.id, "   "))
        .await
        .unwrap_err();
    assert_eq!(code(&e), codes::REQUIRED_MISSING);
}

#[tokio::test]
async fn a_request_on_your_own_prompt_is_refused_with_its_code() {
    let server = server!();
    let alice_c = server.account("alice").await.unwrap();
    let fox = persona(&alice_c, "ink_fox").await;
    let p = prompt(&alice_c, fox.id).await;
    let e = alice_c
        .send_request(&SendRequest {
            persona: fox.id,
            prompt: p.id,
            welcome: b64(&[0, 1, 0, 3, 1, 2, 3]),
            pitch: pitch(&[5u8; 32]),
            expires_in_days: None,
        })
        .await
        .unwrap_err();
    assert_eq!(code(&e), codes::REQUEST_OWN_PROMPT);
}
