//! Server rules (#42) end to end: administrators write them, anyone reads
//! them, and accounts accept them on joining, after a change, or both, as
//! the administrators choose.

// Shared helpers below are test code too; panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use quill_core::{Client, CoreError};
use quill_e2e::{TEST_PASSWORD, TestServer};
use quill_proto::{
    api::{CreatePersona, Registration},
    board::{
        AdminAction, AdminLogQuery, CreatePrompt, CreateReport, Maturity, ReportReason,
        UpdatePrompt, UpdateServerSettings,
    },
    rules::{RulesAcceptance, UpdateRules},
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

/// Whether the error is the `rules-not-accepted` problem.
fn rules_not_accepted(err: &CoreError) -> bool {
    matches!(err, CoreError::Api(p) if p.kind.ends_with("/rules-not-accepted") && p.status == 403)
}

fn rules(body: &str, minor: bool) -> UpdateRules {
    UpdateRules {
        body: body.into(),
        minor,
    }
}

async fn set_acceptance(admin: &Client, a: RulesAcceptance) {
    admin
        .admin_update_settings(&UpdateServerSettings {
            rules_acceptance: Some(a),
            ..UpdateServerSettings::default()
        })
        .await
        .unwrap();
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

fn prompt(persona: Uuid, title: &str) -> CreatePrompt {
    CreatePrompt {
        persona,
        title: title.into(),
        body: "Body.".into(),
        maturity: Maturity::General,
        tags: Vec::new(),
        state: None,
    }
}

#[tokio::test]
async fn administrators_write_the_rules_and_anyone_reads_them() {
    let server = server!();
    let admin = server.admin_account("boss").await.unwrap();
    let writer = server.account("alice").await.unwrap();
    let anonymous = server.client();

    // Nothing to read before the first save.
    assert_eq!(status(&anonymous.rules().await.unwrap_err()), 404);
    assert_eq!(anonymous.instance().await.unwrap().rules_updated_at, None);

    let refused = writer
        .admin_update_rules(&rules("Mine", false))
        .await
        .unwrap_err();
    assert_eq!(status(&refused), 403);
    let blank = admin
        .admin_update_rules(&rules("  ", false))
        .await
        .unwrap_err();
    assert_eq!(status(&blank), 400);

    let first = admin
        .admin_update_rules(&rules("# Be kind\n\nNo <b>spam</b>.", false))
        .await
        .unwrap();
    assert_eq!(first.version, 1);
    // The prompts' sanitizer: headings start at h3, raw HTML is text.
    assert_eq!(
        first.body_html,
        "<h3>Be kind</h3>\n<p>No &lt;b&gt;spam&lt;/b&gt;.</p>\n"
    );
    let read = anonymous.rules().await.unwrap();
    assert_eq!(read.version, 1);
    assert!(!read.must_accept);
    assert_eq!(
        anonymous.instance().await.unwrap().rules_updated_at,
        Some(read.updated_at)
    );

    let second = admin
        .admin_update_rules(&rules("Be kind.", true))
        .await
        .unwrap();
    assert_eq!(second.version, 2);

    // Each save is logged with its version and a hash, not the text.
    let log = admin
        .admin_log_filtered(&AdminLogQuery::default())
        .await
        .unwrap();
    let saves: Vec<_> = log
        .iter()
        .filter(|e| e.action == AdminAction::UpdateRules)
        .collect();
    assert_eq!(saves.len(), 2);
    let detail = saves[0].detail.as_deref().unwrap();
    assert!(
        detail.starts_with("version 2 (minor edit); SHA-256 "),
        "{detail}"
    );
    assert!(!detail.contains("Be kind"));
    assert_eq!(detail.len(), "version 2 (minor edit); SHA-256 ".len() + 64);
    assert!(
        saves[1]
            .detail
            .as_deref()
            .unwrap()
            .starts_with("version 1 (accounts accept it again")
    );
}

#[tokio::test]
async fn accounts_accept_changes_before_writing_again() {
    let server = server!();
    let admin = server.admin_account("boss").await.unwrap();
    let alice = server.account("alice").await.unwrap();
    let pen = persona(&alice, "alice_pen").await;
    let own = alice.create_prompt(&prompt(pen, "Before")).await.unwrap();

    set_acceptance(&admin, RulesAcceptance::OnChange).await;
    admin
        .admin_update_rules(&rules("Version one.", false))
        .await
        .unwrap();

    // Alice joined before these rules: she reads freely but can't write.
    let read = alice.rules().await.unwrap();
    assert!(read.must_accept);
    assert!(!admin.rules().await.unwrap().must_accept);
    let refused = alice.create_prompt(&prompt(pen, "Nope")).await.unwrap_err();
    assert!(rules_not_accepted(&refused), "{refused:?}");
    let edit = UpdatePrompt {
        title: Some("Edited".into()),
        ..UpdatePrompt::default()
    };
    let refused = alice.update_prompt(own.id, &edit).await.unwrap_err();
    assert!(rules_not_accepted(&refused), "{refused:?}");
    let report = CreateReport {
        prompt: own.id,
        reason: ReportReason::Spam,
        details: None,
    };
    let refused = alice.report(&report).await.unwrap_err();
    assert!(rules_not_accepted(&refused), "{refused:?}");
    // Closing her prompt isn't writing anything new.
    alice
        .update_prompt(
            own.id,
            &UpdatePrompt {
                state: Some(quill_proto::board::PromptState::Closed),
                ..UpdatePrompt::default()
            },
        )
        .await
        .unwrap();

    // Accepting names the version read.
    assert_eq!(status(&alice.accept_rules(7).await.unwrap_err()), 409);
    alice.accept_rules(1).await.unwrap();
    assert!(!alice.rules().await.unwrap().must_accept);
    alice.create_prompt(&prompt(pen, "Now")).await.unwrap();

    // A minor edit doesn't ask again; a change does.
    admin
        .admin_update_rules(&rules("Version one, fixed.", true))
        .await
        .unwrap();
    assert!(!alice.rules().await.unwrap().must_accept);
    admin
        .admin_update_rules(&rules("Version three.", false))
        .await
        .unwrap();
    assert!(alice.rules().await.unwrap().must_accept);

    // Bob joined under version three, so nothing stops him.
    let bob = server.account("bob").await.unwrap();
    let bob_pen = persona(&bob, "bob_pen").await;
    assert!(!bob.rules().await.unwrap().must_accept);
    bob.create_prompt(&prompt(bob_pen, "Hello")).await.unwrap();

    // With acceptance off, nobody is stopped.
    set_acceptance(&admin, RulesAcceptance::None).await;
    assert!(!alice.rules().await.unwrap().must_accept);
    alice.create_prompt(&prompt(pen, "Free")).await.unwrap();
}

#[tokio::test]
async fn joining_means_accepting_when_the_server_asks() {
    let server = server!();
    let admin = server.admin_account("boss").await.unwrap();
    set_acceptance(&admin, RulesAcceptance::OnJoin).await;
    let anonymous = server.client();
    let join = |name: &str, accept_rules| Registration {
        username: name.into(),
        password: TEST_PASSWORD.into(),
        accept_rules,
    };

    // No rules yet: nothing to accept.
    anonymous.register_with(&join("early", None)).await.unwrap();

    admin
        .admin_update_rules(&rules("Version one.", false))
        .await
        .unwrap();
    admin
        .admin_update_rules(&rules("Version two.", false))
        .await
        .unwrap();
    let info = anonymous.instance().await.unwrap();
    assert_eq!(info.rules_acceptance, RulesAcceptance::OnJoin);

    let refused = anonymous
        .register_with(&join("carol", None))
        .await
        .unwrap_err();
    assert!(rules_not_accepted(&refused), "{refused:?}");
    let stale = anonymous
        .register_with(&join("carol", Some(1)))
        .await
        .unwrap_err();
    assert!(rules_not_accepted(&stale), "{stale:?}");
    anonymous
        .register_with(&join("carol", Some(2)))
        .await
        .unwrap();

    // Carol's acceptance was recorded: with "both", only "early" (who
    // joined before these rules) must accept them.
    set_acceptance(&admin, RulesAcceptance::Both).await;
    let pool = server.db().await.unwrap();
    let accepted: Vec<(String, i32)> = sqlx::query_as(
        "SELECT a.login_name, r.version FROM rules_acceptances r
         JOIN accounts a ON a.id = r.account_id ORDER BY a.login_name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(accepted, vec![("boss".into(), 2), ("carol".into(), 2)]);
}

// ---- The board ---------------------------------------------------------------

struct Page {
    status: StatusCode,
    headers: header::HeaderMap,
    body: String,
}

impl Page {
    fn location(&self) -> &str {
        self.headers[header::LOCATION].to_str().unwrap()
    }

    fn csrf(&self) -> String {
        let marker = r#"name="csrf" value=""#;
        let start = self.body.find(marker).expect("page has no CSRF field") + marker.len();
        let end = self.body[start..].find('"').unwrap();
        self.body[start..start + end].to_owned()
    }
}

/// A browser without JavaScript that sends the cookies it's given.
struct Browser {
    http: reqwest::Client,
    base: String,
    cookies: Vec<String>,
}

impl Browser {
    fn new(server: &TestServer) -> Self {
        Self {
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            base: server.url(),
            cookies: Vec::new(),
        }
    }

    async fn send(&mut self, req: reqwest::RequestBuilder) -> Page {
        let req = if self.cookies.is_empty() {
            req
        } else {
            req.header(header::COOKIE, self.cookies.join("; "))
        };
        let res = req.send().await.unwrap();
        let page = Page {
            status: res.status(),
            headers: res.headers().clone(),
            body: res.text().await.unwrap(),
        };
        if let Some(set) = page.headers.get(header::SET_COOKIE) {
            let pair = set.to_str().unwrap().split(';').next().unwrap().to_owned();
            let name = pair.split('=').next().unwrap().to_owned();
            self.cookies.retain(|c| !c.starts_with(&format!("{name}=")));
            self.cookies.push(pair);
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

    async fn sign_in(&mut self, login: &str) {
        let page = self
            .post(
                "/login",
                &[("username", login), ("password", TEST_PASSWORD)],
            )
            .await;
        assert_eq!(page.status, StatusCode::SEE_OTHER);
    }
}

#[tokio::test]
async fn the_board_shows_asks_for_and_edits_the_rules() {
    let server = server!();
    server.admin_account("boss").await.unwrap();
    let alice = server.account("alice").await.unwrap();
    persona(&alice, "alice_pen").await;

    let mut visitor = Browser::new(&server);
    let empty = visitor.get("/rules").await;
    assert_eq!(empty.status, StatusCode::OK);
    assert!(empty.body.contains("written any rules yet"));
    assert!(!visitor.get("/").await.body.contains(r#"href="/rules""#));

    // Writers can't reach the editor.
    let mut writer = Browser::new(&server);
    writer.sign_in("alice").await;
    assert_eq!(
        writer.get("/admin/settings/rules").await.status,
        StatusCode::FORBIDDEN
    );

    let mut admin = Browser::new(&server);
    admin.sign_in("boss").await;
    let editor = admin.get("/admin/settings/rules").await;
    assert_eq!(editor.status, StatusCode::OK);
    let csrf = editor.csrf();
    let preview = admin
        .post(
            "/admin/settings/rules",
            &[
                ("csrf", &csrf),
                ("body", "# Be *kind*"),
                ("action", "preview"),
            ],
        )
        .await;
    assert_eq!(preview.status, StatusCode::OK);
    assert!(preview.body.contains("<h3>Be <em>kind</em></h3>"));
    // A preview saves nothing.
    assert!(server.client().rules().await.is_err());

    let saved = admin
        .post(
            "/admin/settings/rules",
            &[("csrf", &csrf), ("body", "# Be *kind*"), ("action", "save")],
        )
        .await;
    assert_eq!(saved.status, StatusCode::SEE_OTHER);
    assert_eq!(saved.location(), "/admin/settings/rules?notice=rules-saved");
    let page = visitor.get("/rules").await;
    assert!(page.body.contains("<h3>Be <em>kind</em></h3>"));
    assert!(page.body.contains("Version 1"));
    assert!(visitor.get("/").await.body.contains(r#"href="/rules""#));

    // Ask for acceptance on joining and after changes.
    let set = admin
        .post(
            "/admin/settings/rules-acceptance",
            &[("csrf", &csrf), ("acceptance", "both")],
        )
        .await;
    assert_eq!(set.status, StatusCode::SEE_OTHER);

    // Alice joined before: every page shows the banner until she accepts.
    let board = writer.get("/").await;
    assert!(board.body.contains("rules-banner"));
    let rules_page = writer.get("/rules").await;
    assert!(!rules_page.body.contains("rules-banner"));
    let accepted = writer
        .post(
            "/rules/accept",
            &[("csrf", &rules_page.csrf()), ("version", "1")],
        )
        .await;
    assert_eq!(accepted.status, StatusCode::SEE_OTHER);
    assert_eq!(accepted.location(), "/?notice=rules-accepted");
    assert!(!writer.get("/").await.body.contains("rules-banner"));

    // The sign-up form asks, and won't go on without the tick.
    let mut newcomer = Browser::new(&server);
    let form = newcomer.get("/register").await;
    assert!(form.body.contains(r#"name="accept_rules""#));
    let fields = [
        ("username", "dana"),
        ("password", TEST_PASSWORD),
        ("confirm", TEST_PASSWORD),
        ("rules_version", "1"),
    ];
    let refused = newcomer.post("/register", &fields).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(refused.body.contains("Accept the server rules"));
    let mut ticked = fields.to_vec();
    ticked.push(("accept_rules", "on"));
    let joined = newcomer.post("/register", &ticked).await;
    assert_eq!(joined.status, StatusCode::SEE_OTHER);
    assert!(!newcomer.get("/").await.body.contains("rules-banner"));
}
