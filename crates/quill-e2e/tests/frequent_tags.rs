//! Frequent tags (#55) end to end: the tags a writer uses most, suggested
//! on their post form, per persona unless they choose all, and never to
//! anyone else.

// Shared helpers below are test code too; panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use quill_core::{Client, CoreError};
use quill_e2e::{TEST_PASSWORD, TestServer};
use quill_proto::{
    api::CreatePersona,
    board::{
        CreatePrompt, Maturity, TagPolicy, TagStatus, UpdateAccountSettings, UpdateServerSettings,
        UpdateTag,
    },
};
use reqwest::{StatusCode, header};
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

async fn persona(client: &Client, handle: &str) -> Uuid {
    client
        .create_persona(&CreatePersona {
            handle: handle.into(),
            display_name: None,
            summary: None,
        })
        .await
        .unwrap()
        .id
}

async fn post(client: &Client, persona: Uuid, tags: &[&str]) {
    client
        .create_prompt(&CreatePrompt {
            persona,
            title: format!("With {}", tags.join(" ")),
            body: "Body.".into(),
            maturity: Maturity::General,
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
            state: None,
        })
        .await
        .unwrap();
}

async fn keys(client: &Client, persona: Option<Uuid>) -> Vec<String> {
    client
        .frequent_tags(persona)
        .await
        .unwrap()
        .iter()
        .map(quill_proto::board::Tag::key)
        .collect()
}

#[tokio::test]
async fn most_used_first_capped_and_per_persona() {
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    let pen = persona(&alice, "alice_pen").await;
    let ink = persona(&alice, "alice_ink").await;

    // Nothing posted yet: nothing to suggest.
    assert!(keys(&alice, Some(pen)).await.is_empty());

    post(&alice, pen, &["theme:sea", "theme:grief"]).await;
    post(&alice, pen, &["theme:sea", "theme:salt"]).await;
    post(&alice, pen, &["theme:sea", "theme:grief"]).await;
    // Ten more themes, used once each and more recently.
    for i in 0..5 {
        post(
            &alice,
            pen,
            &[&format!("theme:t{i}a"), &format!("theme:t{i}b")],
        )
        .await;
    }
    post(&alice, ink, &["theme:romance", "theme:romance-again"]).await;

    let pen_tags = keys(&alice, Some(pen)).await;
    assert_eq!(pen_tags.len(), 8);
    assert_eq!(&pen_tags[..2], ["theme:sea", "theme:grief"]);
    // Ties go to the most recently used.
    assert_eq!(&pen_tags[2..4], ["theme:t4a", "theme:t4b"]);
    assert!(!pen_tags.contains(&"theme:romance".to_owned()));
    // The other persona's are its own.
    assert_eq!(
        keys(&alice, Some(ink)).await,
        ["theme:romance", "theme:romance-again"]
    );

    // With the setting on, or no persona given, all personas count.
    assert!(keys(&alice, None).await.contains(&"theme:sea".to_owned()));
    alice
        .update_account(&UpdateAccountSettings {
            suggest_all_personas: Some(true),
            ..UpdateAccountSettings::default()
        })
        .await
        .unwrap();
    assert!(alice.account().await.unwrap().suggest_all_personas);
    let all = keys(&alice, Some(ink)).await;
    assert_eq!(&all[..2], ["theme:sea", "theme:grief"]);
}

#[tokio::test]
async fn aliases_count_as_their_target_and_hidden_or_pending_never_show() {
    let server = server!();
    let admin = server.admin_account("boss").await.unwrap();
    let alice = server.account("alice").await.unwrap();
    let pen = persona(&alice, "alice_pen").await;
    post(&alice, pen, &["theme:spooky", "theme:fog"]).await;
    post(&alice, pen, &["theme:horror", "theme:fog"]).await;
    post(&alice, pen, &["theme:horror", "theme:salt"]).await;

    // Merging spooky into horror: three uses of horror, ahead of fog.
    admin
        .admin_update_tag(
            "theme",
            "spooky",
            &UpdateTag {
                alias_of: Some("theme:horror".into()),
                ..UpdateTag::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        keys(&alice, Some(pen)).await,
        ["theme:horror", "theme:fog", "theme:salt"]
    );

    // A hidden tag stays on its prompts but isn't suggested.
    admin
        .admin_update_tag(
            "theme",
            "fog",
            &UpdateTag {
                status: Some(TagStatus::Hidden),
                ..UpdateTag::default()
            },
        )
        .await
        .unwrap();
    // Nor is a tag still awaiting approval.
    admin
        .admin_update_settings(&UpdateServerSettings {
            new_tags: Some(TagPolicy::Approval),
            ..UpdateServerSettings::default()
        })
        .await
        .unwrap();
    post(&alice, pen, &["theme:brand-new", "theme:brand-new-too"]).await;
    assert_eq!(
        keys(&alice, Some(pen)).await,
        ["theme:horror", "theme:salt"]
    );
}

#[tokio::test]
async fn only_the_writer_sees_their_suggestions() {
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    let pen = persona(&alice, "alice_pen").await;
    post(&alice, pen, &["theme:sea"]).await;
    let bob = server.account("bob").await.unwrap();
    persona(&bob, "bob_pen").await;

    // Nobody signed out, and nobody for someone else's persona.
    let anonymous = server.client().frequent_tags(None).await.unwrap_err();
    assert!(matches!(anonymous, CoreError::NotLoggedIn));
    assert_eq!(
        status(&bob.frequent_tags(Some(pen)).await.unwrap_err()),
        400
    );
    assert!(keys(&bob, None).await.is_empty());

    // The post form shows Alice hers, and Bob nothing.
    let form = |login: &'static str| {
        let url = server.url();
        async move {
            let http = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap();
            let body = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs([("username", login), ("password", TEST_PASSWORD)])
                .finish();
            let signed_in = http
                .post(format!("{url}/login"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("sec-fetch-site", "same-origin")
                .body(body)
                .send()
                .await
                .unwrap();
            assert_eq!(signed_in.status(), StatusCode::SEE_OTHER);
            let cookie = signed_in.headers()[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_owned();
            http.get(format!("{url}/new"))
                .header(header::COOKIE, cookie)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap()
        }
    };
    let hers = form("alice").await;
    assert!(hers.contains("You often use"));
    assert!(hers.contains(" for @alice_pen"));
    assert!(hers.contains(r#"name="pick" value="theme:sea""#));
    let his = form("bob").await;
    assert!(!his.contains("You often use"));
}
