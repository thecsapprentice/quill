//! Tag type limits (#52) end to end: how many tags of a type a prompt needs
//! and may carry, checked on every save, never changing existing prompts,
//! and the administrators' checks and warnings.

// Shared helpers below are test code too; panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use quill_core::{Client, CoreError};
use quill_e2e::{TEST_PASSWORD, TestServer};
use quill_proto::{
    api::CreatePersona,
    board::{
        CreateNamespace, CreatePrompt, CreateTag, Maturity, UpdateNamespace, UpdatePrompt,
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

async fn writer(server: &TestServer, name: &str) -> (Client, Uuid) {
    let client = server.account(name).await.unwrap();
    let persona = client
        .create_persona(&CreatePersona {
            handle: format!("{name}_p"),
            display_name: None,
            summary: None,
        })
        .await
        .unwrap();
    (client, persona.id)
}

fn prompt(persona: Uuid, title: &str, tags: &[&str]) -> CreatePrompt {
    CreatePrompt {
        persona,
        title: title.into(),
        body: format!("A prompt about {title}."),
        maturity: Maturity::General,
        tags: tags.iter().map(|t| (*t).to_owned()).collect(),
        state: None,
    }
}

fn limits(min: u32, max: Option<u32>) -> UpdateNamespace {
    UpdateNamespace {
        min: Some(min),
        max,
        clear_max: max.is_none(),
        ..UpdateNamespace::default()
    }
}

#[tokio::test]
async fn every_save_meets_each_types_limits() {
    let server = server!();
    let admin = server.admin_account("boss").await.unwrap();
    let (alice, ap) = writer(&server, "alice").await;

    // Unlimited by default: any number of themes.
    let many = alice
        .create_prompt(&prompt(
            ap,
            "Many",
            &["theme:sea", "theme:grief", "theme:salt"],
        ))
        .await
        .unwrap();

    // Up to two themes.
    let theme = admin
        .admin_update_namespace("theme", &limits(0, Some(2)))
        .await
        .unwrap();
    assert_eq!((theme.min, theme.max), (0, Some(2)));
    assert_eq!(theme.limit_label(), "up to 2");
    let refused = alice
        .create_prompt(&prompt(
            ap,
            "Too many",
            &["theme:sea", "theme:grief", "theme:salt"],
        ))
        .await
        .unwrap_err();
    assert_eq!(status(&refused), 400);
    assert!(
        refused
            .to_string()
            .contains("Pick at most 2 Theme tags (you have 3)."),
        "{refused}"
    );
    alice
        .create_prompt(&prompt(ap, "Two", &["theme:sea", "theme:grief"]))
        .await
        .unwrap();

    // The prompt saved before keeps its three themes...
    assert_eq!(alice.prompt(many.id).await.unwrap().tags.len(), 3);
    // ...its status can change, but an edit must trim them.
    alice
        .update_prompt(
            many.id,
            &UpdatePrompt {
                state: Some(quill_proto::board::PromptState::Closed),
                ..UpdatePrompt::default()
            },
        )
        .await
        .unwrap();
    let retitle = UpdatePrompt {
        title: Some("Many, edited".into()),
        ..UpdatePrompt::default()
    };
    assert_eq!(
        status(&alice.update_prompt(many.id, &retitle).await.unwrap_err()),
        400
    );
    alice
        .update_prompt(
            many.id,
            &UpdatePrompt {
                tags: Some(vec!["theme:sea".into()]),
                ..retitle
            },
        )
        .await
        .unwrap();

    // Exactly one length.
    admin
        .admin_update_namespace("length", &limits(1, Some(1)))
        .await
        .unwrap();
    let none = alice
        .create_prompt(&prompt(ap, "No length", &[]))
        .await
        .unwrap_err();
    assert!(
        none.to_string().contains("Pick exactly 1 Length tag."),
        "{none}"
    );
    let two = alice
        .create_prompt(&prompt(ap, "Two lengths", &["length:short", "length:long"]))
        .await
        .unwrap_err();
    assert!(
        two.to_string()
            .contains("Pick at most 1 Length tag (you have 2)."),
        "{two}"
    );
    alice
        .create_prompt(&prompt(ap, "One length", &["length:short"]))
        .await
        .unwrap();

    // Removing the maximum lifts it.
    admin
        .admin_update_namespace("theme", &limits(0, None))
        .await
        .unwrap();
    alice
        .create_prompt(&prompt(
            ap,
            "Free",
            &["length:long", "theme:sea", "theme:grief", "theme:salt"],
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn an_alias_can_push_a_prompt_over_a_maximum() {
    let server = server!();
    let admin = server.admin_account("boss").await.unwrap();
    let (alice, ap) = writer(&server, "alice").await;
    let p = alice
        .create_prompt(&prompt(ap, "Coast", &["theme:coast", "genre:horror"]))
        .await
        .unwrap();
    admin
        .admin_create_tag(&CreateTag {
            namespace: "theme".into(),
            display: "Sea".into(),
        })
        .await
        .unwrap();
    admin
        .admin_update_namespace("theme", &limits(0, Some(1)))
        .await
        .unwrap();
    // Merging genre:horror into a theme gives the prompt two themes; the
    // admin's change goes through, and the author trims it later.
    admin
        .admin_update_tag(
            "genre",
            "horror",
            &UpdateTag {
                alias_of: Some("theme:sea".into()),
                ..UpdateTag::default()
            },
        )
        .await
        .unwrap_or_else(|e| panic!("alias refused: {e}"));
    let after = alice.prompt(p.id).await.unwrap();
    assert_eq!(
        after.tags.iter().filter(|t| t.namespace == "theme").count(),
        2
    );
}

#[tokio::test]
async fn administrators_cant_set_limits_nobody_could_meet() {
    let server = server!();
    let admin = server.admin_account("boss").await.unwrap();
    let writer_client = server.account("alice").await.unwrap();

    // Only administrators set limits.
    assert_eq!(
        status(
            &writer_client
                .admin_update_namespace("theme", &limits(1, None))
                .await
                .unwrap_err()
        ),
        403
    );

    // Out of range or upside down.
    for (min, max) in [(3, Some(2)), (0, Some(0)), (16, None), (0, Some(16))] {
        let err = admin
            .admin_update_namespace("theme", &limits(min, max))
            .await
            .unwrap_err();
        assert_eq!(status(&err), 400, "{min} {max:?}");
    }

    // A curated-only type can't need more tags than it has.
    let mood = CreateNamespace {
        name: "mood".into(),
        display: "Mood".into(),
        locked: true,
        min: 1,
        max: None,
        color: None,
    };
    let err = admin.admin_create_namespace(&mood).await.unwrap_err();
    assert!(err.to_string().contains("curated-only"), "{err}");
    admin
        .admin_create_namespace(&CreateNamespace { min: 0, ..mood })
        .await
        .unwrap();
    let err = admin
        .admin_update_namespace("mood", &limits(1, None))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("has 0 visible tags"), "{err}");
    admin
        .admin_create_tag(&CreateTag {
            namespace: "mood".into(),
            display: "Eerie".into(),
        })
        .await
        .unwrap();
    admin
        .admin_update_namespace("mood", &limits(1, None))
        .await
        .unwrap();

    // The minimums together must fit on one prompt (15 tags).
    admin
        .admin_update_namespace("theme", &limits(10, None))
        .await
        .unwrap();
    let err = admin
        .admin_update_namespace("genre", &limits(6, None))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("would need 17 tags"), "{err}");
}

// ---- The board ---------------------------------------------------------------

struct Page {
    status: StatusCode,
    headers: header::HeaderMap,
    body: String,
}

/// A browser without JavaScript that keeps its session cookie.
struct Browser {
    http: reqwest::Client,
    base: String,
    cookie: Option<String>,
}

impl Browser {
    fn new(server: &TestServer) -> Self {
        Self {
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            base: server.url(),
            cookie: None,
        }
    }

    async fn send(&mut self, req: reqwest::RequestBuilder) -> Page {
        let req = match &self.cookie {
            Some(c) => req.header(header::COOKIE, c),
            None => req,
        };
        let res = req.send().await.unwrap();
        let page = Page {
            status: res.status(),
            headers: res.headers().clone(),
            body: res.text().await.unwrap(),
        };
        if let Some(set) = page.headers.get(header::SET_COOKIE) {
            self.cookie = Some(set.to_str().unwrap().split(';').next().unwrap().to_owned());
        }
        page
    }

    async fn get(&mut self, path: &str) -> Page {
        let req = self.http.get(format!("{}{path}", self.base));
        self.send(req).await
    }

    async fn post(&mut self, path: &str, fields: &[(&str, &str)]) -> Page {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields)
            .finish();
        let req = self
            .http
            .post(format!("{}{path}", self.base))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("sec-fetch-site", "same-origin")
            .body(body);
        self.send(req).await
    }

    fn csrf(page: &Page) -> String {
        let marker = r#"name="csrf" value=""#;
        let start = page.body.find(marker).unwrap() + marker.len();
        let end = page.body[start..].find('"').unwrap();
        page.body[start..start + end].to_owned()
    }
}

#[tokio::test]
async fn the_board_warns_before_limits_prompts_dont_meet() {
    let server = server!();
    server.admin_account("boss").await.unwrap();
    let (alice, ap) = writer(&server, "alice").await;
    alice
        .create_prompt(&prompt(ap, "Crowded", &["theme:a", "theme:b", "theme:c"]))
        .await
        .unwrap();

    let mut admin = Browser::new(&server);
    let signed_in = admin
        .post(
            "/login",
            &[("username", "boss"), ("password", TEST_PASSWORD)],
        )
        .await;
    assert_eq!(signed_in.status, StatusCode::SEE_OTHER);
    let page = admin.get("/admin/settings/tag-types").await;
    let csrf = Browser::csrf(&page);

    // Up to two themes would leave "Crowded" outside: asked first.
    let asked = admin
        .post(
            "/admin/settings/tag-types/theme/limits",
            &[
                ("csrf", &csrf),
                ("preset", "numbers"),
                ("min", "0"),
                ("max", "2"),
            ],
        )
        .await;
    assert_eq!(asked.status, StatusCode::OK);
    assert!(
        asked
            .body
            .contains("1 prompt on the board wouldn&#39;t meet these limits.")
    );
    let unchanged = server.client().tag_namespaces().await.unwrap();
    assert!(
        unchanged
            .iter()
            .any(|n| n.name == "theme" && n.max.is_none())
    );

    let saved = admin
        .post(
            "/admin/settings/tag-types/theme/limits",
            &[
                ("csrf", &csrf),
                ("min", "0"),
                ("max", "2"),
                ("confirm", "yes"),
            ],
        )
        .await;
    assert_eq!(saved.status, StatusCode::SEE_OTHER);
    let types = admin.get("/admin/settings/tag-types").await;
    assert!(
        types
            .body
            .contains("1 prompt don&#39;t meet these limits yet.")
    );
    let outside = admin.get("/admin/settings/tag-types/theme/outside").await;
    assert_eq!(outside.status, StatusCode::OK);
    assert!(outside.body.contains("Crowded"));

    // A quick choice sets both numbers; no prompt is outside it, so no
    // question is asked.
    let quick = admin
        .post(
            "/admin/settings/tag-types/length/limits",
            &[("csrf", &csrf), ("preset", "optional"), ("min", "9")],
        )
        .await;
    assert_eq!(quick.status, StatusCode::SEE_OTHER);
}
