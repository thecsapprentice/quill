//! Phase 0 end-to-end tests: instance info, accounts, tokens and personas.

use quill_core::CoreError;
use quill_e2e::{TEST_PASSWORD, TestServer};
use quill_proto::api::{CreatePersona, Credentials, UpdatePersona};

macro_rules! server {
    () => {
        match TestServer::start().await.unwrap() {
            Some(s) => s,
            None => return,
        }
    };
}

fn creds(username: &str, password: &str) -> Credentials {
    Credentials {
        username: username.into(),
        password: password.into(),
    }
}

fn status(err: &CoreError) -> u16 {
    err.status().unwrap_or_default()
}

#[tokio::test]
async fn instance_reports_limits_and_versions() {
    let server = server!();
    let info = server.client().instance().await.unwrap();
    assert_eq!(info.software, "quill");
    assert_eq!(info.protocol_versions, vec!["0.1"]);
    assert_eq!(info.limits.max_message_bytes, 65536);
    assert!(!info.email_recovery);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn register_login_logout() {
    let server = server!();
    let client = server.client();

    let first = client
        .register(&creds("InkFox", TEST_PASSWORD))
        .await
        .unwrap();
    assert!(first.token.starts_with("qt_"));

    // Login names are case-insensitive.
    let second = client.login(&creds("inkfox", TEST_PASSWORD)).await.unwrap();
    assert_ne!(first.token, second.token);

    let authed = server.client().with_token(second.token.clone());
    authed.personas().await.unwrap();
    authed.logout().await.unwrap();
    let err = authed.personas().await.unwrap_err();
    assert_eq!(status(&err), 401);

    // The first token is still valid: logout only revokes the token used.
    server
        .client()
        .with_token(first.token)
        .personas()
        .await
        .unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn registration_rules() {
    let server = server!();
    let client = server.client();

    client
        .register(&creds("taken", TEST_PASSWORD))
        .await
        .unwrap();
    let dup = client
        .register(&creds("TAKEN", TEST_PASSWORD))
        .await
        .unwrap_err();
    assert_eq!(status(&dup), 409);

    let weak = client.register(&creds("weak", "short")).await.unwrap_err();
    assert_eq!(status(&weak), 400);

    let bad_name = client
        .register(&creds("ink-fox", TEST_PASSWORD))
        .await
        .unwrap_err();
    assert_eq!(status(&bad_name), 400);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn closed_registration_is_refused() {
    let Some(server) = TestServer::start_with(|c| c.registration_open = false)
        .await
        .unwrap()
    else {
        return;
    };
    let info = server.client().instance().await.unwrap();
    assert!(!info.registration_open);
    let err = server
        .client()
        .register(&creds("someone", TEST_PASSWORD))
        .await
        .unwrap_err();
    assert_eq!(status(&err), 403);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn login_failures_are_indistinguishable() {
    let server = server!();
    let client = server.client();
    client
        .register(&creds("realuser", TEST_PASSWORD))
        .await
        .unwrap();

    let wrong_password = client
        .login(&creds("realuser", "not the right password"))
        .await
        .unwrap_err();
    let no_account = client
        .login(&creds("nobodyhere", TEST_PASSWORD))
        .await
        .unwrap_err();
    let invalid_name = client
        .login(&creds("not a valid name", TEST_PASSWORD))
        .await
        .unwrap_err();

    match (wrong_password, no_account, invalid_name) {
        (CoreError::Api(a), CoreError::Api(b), CoreError::Api(c)) => {
            assert_eq!(a, b);
            assert_eq!(b, c);
            assert_eq!(a.status, 401);
        }
        other => panic!("expected three problem responses, got {other:?}"),
    }
    server.stop().await.unwrap();
}

#[tokio::test]
async fn personas_crud_and_isolation() {
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    let bob = server.account("bob").await.unwrap();

    let p = alice
        .create_persona(&CreatePersona {
            handle: "Ink_Fox".into(),
            display_name: Some(" Ink Fox ".into()),
            summary: Some("Writes horror.".into()),
        })
        .await
        .unwrap();
    assert_eq!(p.handle, "ink_fox");
    assert_eq!(p.display_name.as_deref(), Some("Ink Fox"));

    // Handles are unique across the whole server, not per account.
    let clash = bob
        .create_persona(&CreatePersona {
            handle: "ink_fox".into(),
            display_name: None,
            summary: None,
        })
        .await
        .unwrap_err();
    assert_eq!(status(&clash), 409);

    // Another account can neither see, edit nor delete it.
    assert!(bob.personas().await.unwrap().is_empty());
    let edit = bob
        .update_persona(p.id, &UpdatePersona::default())
        .await
        .unwrap_err();
    assert_eq!(status(&edit), 404);
    let delete = bob.delete_persona(p.id).await.unwrap_err();
    assert_eq!(status(&delete), 404);

    // Absent fields are unchanged; an empty string clears.
    let updated = alice
        .update_persona(
            p.id,
            &UpdatePersona {
                display_name: None,
                summary: Some(String::new()),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.display_name.as_deref(), Some("Ink Fox"));
    assert_eq!(updated.summary, None);

    alice.delete_persona(p.id).await.unwrap();
    assert!(alice.personas().await.unwrap().is_empty());
    server.stop().await.unwrap();
}

#[tokio::test]
async fn persona_limit_is_enforced() {
    let Some(server) = TestServer::start_with(|c| c.max_personas_per_account = 2)
        .await
        .unwrap()
    else {
        return;
    };
    let alice = server.account("alice").await.unwrap();
    for handle in ["one", "two"] {
        alice
            .create_persona(&CreatePersona {
                handle: handle.into(),
                display_name: None,
                summary: None,
            })
            .await
            .unwrap();
    }
    let err = alice
        .create_persona(&CreatePersona {
            handle: "three".into(),
            display_name: None,
            summary: None,
        })
        .await
        .unwrap_err();
    assert_eq!(status(&err), 403);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn bad_tokens_are_rejected() {
    let server = server!();
    for token in [
        "",
        "qt_",
        "qt_notarealtoken",
        "Bearer qt_x",
        "not-a-quill-token",
    ] {
        let err = server
            .client()
            .with_token(token)
            .personas()
            .await
            .unwrap_err();
        assert_eq!(status(&err), 401, "token {token:?}");
    }
    server.stop().await.unwrap();
}

#[tokio::test]
async fn oversized_bodies_are_refused_with_413() {
    let server = server!();
    let limit = server
        .client()
        .instance()
        .await
        .unwrap()
        .limits
        .max_envelope_bytes;
    let body = format!(
        r#"{{"username":"big","password":"{}"}}"#,
        "x".repeat(usize::try_from(limit).unwrap())
    );
    let response = reqwest::Client::new()
        .post(format!("{}/api/v1/auth/register", server.url()))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 413);
    assert_eq!(
        response.headers()["content-type"],
        "application/problem+json"
    );
    server.stop().await.unwrap();
}

#[tokio::test]
async fn malformed_json_is_a_problem_response() {
    let server = server!();
    let response = reqwest::Client::new()
        .post(format!("{}/api/v1/auth/login", server.url()))
        .header("content-type", "application/json")
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
    assert_eq!(
        response.headers()["content-type"],
        "application/problem+json"
    );
    server.stop().await.unwrap();
}
