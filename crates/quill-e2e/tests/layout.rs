//! The board's layout (#51) end to end: every page has one nav and no
//! inline styles, the reading pane opens with `open=`, and the filter
//! builder hands the board the same filters the old form did.

// Shared helpers below are test code too; panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use quill_core::Client;
use quill_e2e::{TEST_PASSWORD, TestServer};
use quill_proto::{
    api::CreatePersona,
    board::{CreatePrompt, Maturity},
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

    async fn sign_in(&mut self, login: &str) {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs([("username", login), ("password", TEST_PASSWORD)])
            .finish();
        let req = self
            .http
            .post(format!("{}/login", self.base))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("sec-fetch-site", "same-origin")
            .body(body);
        assert_eq!(self.send(req).await.status, StatusCode::SEE_OTHER);
    }
}

async fn post(client: &Client, handle: &str, title: &str, maturity: Maturity) -> Uuid {
    let persona = client
        .create_persona(&CreatePersona {
            handle: handle.into(),
            display_name: None,
            summary: None,
        })
        .await
        .unwrap();
    client
        .create_prompt(&CreatePrompt {
            persona: persona.id,
            title: title.into(),
            body: "A *keeper* finds the ledger.".into(),
            maturity,
            tags: vec!["genre:horror".into()],
            state: None,
        })
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn every_page_has_one_nav_and_no_inline_styles() {
    let server = server!();
    server.admin_account("boss").await.unwrap();
    let alice = server.account("alice").await.unwrap();
    let id = post(
        &alice,
        "alice_pen",
        "The keeper's ledger",
        Maturity::General,
    )
    .await;

    let mut admin = Browser::new(&server);
    admin.sign_in("boss").await;
    let open = format!("/?open={id}");
    let prompt = format!("/p/{id}");
    for path in [
        "/",
        open.as_str(),
        prompt.as_str(),
        "/filters",
        "/filters?type=genre",
        "/new",
        "/mine",
        "/more",
        "/notices",
        "/announcements",
        "/rules",
        "/admin/reports",
        "/admin/log",
        "/admin/settings/tags",
        "/admin/settings/tag-types",
        "/admin/settings/rules",
        "/admin/moderation/scripts",
    ] {
        let page = admin.get(path).await;
        assert_eq!(page.status, StatusCode::OK, "{path}");
        // The CSP forbids inline styles: they'd be ignored, so none are sent.
        assert!(!page.body.contains(" style="), "{path} has an inline style");
        assert_eq!(
            page.body.matches(r#"class="site-nav""#).count(),
            1,
            "{path}"
        );
    }
    // Admin pages sit beside the sidebar.
    let tags = admin.get("/admin/settings/tags").await;
    assert!(tags.body.contains(r#"<nav class="admin-side""#));
}

#[tokio::test]
async fn the_reading_pane_opens_from_the_list() {
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    let id = post(
        &alice,
        "alice_pen",
        "The keeper's ledger",
        Maturity::General,
    )
    .await;
    let mut visitor = Browser::new(&server);

    let board = visitor.get("/").await;
    assert!(board.body.contains(&format!(r#"href="/?open={id}""#)));
    assert!(!board.body.contains("has-open"));

    let opened = visitor.get(&format!("/?open={id}")).await;
    assert!(opened.body.contains("board-layout has-open"));
    assert!(
        opened
            .body
            .contains("<p>A <em>keeper</em> finds the ledger.</p>")
    );
    assert!(opened.body.contains(&format!(r#"href="/p/{id}""#)));

    // Unknown or above the visitor's cap: the list alone.
    let unknown = visitor.get(&format!("/?open={}", Uuid::now_v7())).await;
    assert!(!unknown.body.contains("has-open"));
    let mature = post(&alice, "alice_dark", "The cellar", Maturity::Mature).await;
    let held = visitor.get(&format!("/?open={mature}")).await;
    assert!(!held.body.contains("has-open"));
}

#[tokio::test]
async fn the_filter_builder_hands_the_board_its_usual_filters() {
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    post(
        &alice,
        "alice_pen",
        "The keeper's ledger",
        Maturity::General,
    )
    .await;
    let mut visitor = Browser::new(&server);

    // Genre's tags with their counts; horror is on one prompt.
    let genre = visitor.get("/filters?type=genre&include=theme%3Asea").await;
    assert_eq!(genre.status, StatusCode::OK);
    assert!(
        genre
            .body
            .contains(r#"name="tag:genre:horror" value="any" checked=true"#)
    );
    assert!(genre.body.contains("1 prompt<"));

    // Hiding horror from that page keeps theme:sea, set on another.
    let applied = visitor
        .get(
            "/filters?include=theme%3Asea&exclude=&shown=genre%3Ahorror&type=genre\
             &tag%3Agenre%3Ahorror=hide&maturity=teen&go=apply",
        )
        .await;
    assert_eq!(applied.status, StatusCode::SEE_OTHER);
    assert_eq!(
        applied.headers[header::LOCATION],
        "/?include=theme%3Asea&exclude=genre%3Ahorror&maturity=teen"
    );

    // Switching type keeps a choice made on screen.
    let switched = visitor
        .get(
            "/filters?include=&exclude=&shown=genre%3Ahorror&type=genre\
             &tag%3Agenre%3Ahorror=has&to_type=theme",
        )
        .await;
    assert!(
        switched
            .body
            .contains(r#"name="include" value="genre:horror""#)
    );
    assert!(
        switched
            .body
            .contains(r#"value="theme" aria-current="true""#)
    );
}
