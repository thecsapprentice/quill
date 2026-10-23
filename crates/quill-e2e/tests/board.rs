//! End-to-end tests for the server-rendered board, driven like a browser
//! with JavaScript off: plain GETs and form POSTs, cookies handled by hand.

// Shared helpers below are test code too; panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use quill_e2e::{TEST_PASSWORD, TestServer};
use reqwest::{StatusCode, header};

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

impl Page {
    fn location(&self) -> &str {
        self.headers[header::LOCATION].to_str().unwrap()
    }

    /// The CSRF token embedded in the page's forms.
    fn csrf(&self) -> String {
        let marker = r#"name="csrf" value=""#;
        let start = self.body.find(marker).expect("page has no CSRF field") + marker.len();
        let end = self.body[start..].find('"').unwrap();
        self.body[start..start + end].to_owned()
    }
}

/// A browser without JavaScript: follows no redirects, keeps one cookie.
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
        if let Some(set) = res.headers().get(header::SET_COOKIE) {
            let set = set.to_str().unwrap();
            assert!(set.starts_with("__Host-quill_session="));
            for attr in ["Secure", "HttpOnly", "SameSite=Strict", "Path=/"] {
                assert!(set.contains(attr), "cookie lacks {attr}: {set}");
            }
            let pair = set.split(';').next().unwrap();
            self.cookie = if pair.ends_with('=') {
                None
            } else {
                Some(pair.to_owned())
            };
        }
        Page {
            status: res.status(),
            headers: res.headers().clone(),
            body: res.text().await.unwrap(),
        }
    }

    async fn get(&mut self, path: &str) -> Page {
        let req = self.http.get(format!("{}{path}", self.base));
        self.send(req).await
    }

    /// Posts a form the way a browser does from one of our own pages.
    async fn post(&mut self, path: &str, fields: &[(&str, &str)]) -> Page {
        self.post_with(path, fields, &[("sec-fetch-site", "same-origin")])
            .await
    }

    async fn post_with(
        &mut self,
        path: &str,
        fields: &[(&str, &str)],
        extra: &[(&'static str, &str)],
    ) -> Page {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields)
            .finish();
        let mut req = self
            .http
            .post(format!("{}{path}", self.base))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body);
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        self.send(req).await
    }

    /// Registers, then creates a persona; returns the CSRF token.
    async fn register(&mut self, login: &str, handle: &str) -> String {
        let page = self
            .post(
                "/register",
                &[
                    ("username", login),
                    ("password", TEST_PASSWORD),
                    ("confirm", TEST_PASSWORD),
                ],
            )
            .await;
        assert_eq!(page.status, StatusCode::SEE_OTHER);
        assert_eq!(page.location(), "/personas?notice=registered");
        let personas = self.get("/personas?notice=registered").await;
        assert!(personas.body.contains("Account created."));
        let csrf = personas.csrf();
        let created = self
            .post("/personas", &[("csrf", &csrf), ("handle", handle)])
            .await;
        assert_eq!(created.status, StatusCode::SEE_OTHER);
        csrf
    }

    /// Posts a prompt as the first persona; returns its page path.
    async fn post_prompt(&mut self, csrf: &str, title: &str, maturity: &str) -> String {
        let form = self.get("/new").await;
        let start = form.body.find(r#"name="persona""#).unwrap();
        let value_at = form.body[start..].find(r#"<option value=""#).unwrap() + start + 15;
        let persona = form.body[value_at..value_at + 36].to_owned();
        let page = self
            .post(
                "/new",
                &[
                    ("csrf", csrf),
                    ("persona", &persona),
                    ("title", title),
                    (
                        "body",
                        "A *keeper* finds the ledger.\r\n\r\n<script>x()</script>",
                    ),
                    ("maturity", maturity),
                    ("state", "open"),
                    ("tags", "genre:horror, theme:Lighthouses"),
                ],
            )
            .await;
        assert_eq!(page.status, StatusCode::SEE_OTHER, "{}", page.body);
        let location = page.location().to_owned();
        assert!(location.ends_with("?notice=posted"), "{location}");
        location.trim_end_matches("?notice=posted").to_owned()
    }
}

#[tokio::test]
async fn public_pages_and_security_headers() {
    let server = server!();
    let mut b = Browser::new(&server);

    let home = b.get("/").await;
    assert_eq!(home.status, StatusCode::OK);
    assert!(home.body.starts_with("<!DOCTYPE html>"));
    assert!(home.body.contains("No prompts yet."));
    assert!(!home.body.contains("<script"));
    let csp = home.headers[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap();
    assert!(csp.contains("default-src 'none'"));
    assert!(!csp.contains("script-src"));
    assert_eq!(home.headers[header::X_FRAME_OPTIONS], "DENY");
    assert_eq!(home.headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(home.headers[header::REFERRER_POLICY], "same-origin");
    assert!(
        home.body
            .contains("https://github.com/thecsapprentice/quill")
    );

    let css = b.get("/static/board.css").await;
    assert_eq!(css.status, StatusCode::OK);
    assert!(
        css.headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/css")
    );
    assert_eq!(b.get("/favicon.ico").await.status, StatusCode::NO_CONTENT);

    let missing = b.get("/no/such/page").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert!(missing.body.contains("Page not found"));
    // Unknown API paths stay JSON problems, not HTML.
    let api = b.get("/api/v1/nope").await;
    assert_eq!(api.status, StatusCode::NOT_FOUND);
    assert_eq!(
        api.headers[header::CONTENT_TYPE],
        "application/problem+json"
    );

    // Signed-out visitors are sent to sign in.
    for path in ["/new", "/mine", "/personas", "/settings", "/admin/reports"] {
        let page = b.get(path).await;
        assert_eq!(page.status, StatusCode::SEE_OTHER, "{path}");
        assert_eq!(page.location(), "/login");
    }
    server.stop().await.unwrap();
}

#[tokio::test]
async fn post_edit_and_delete_through_the_board() {
    let server = server!();
    let mut b = Browser::new(&server);
    let csrf = b.register("alice", "ink_fox").await;

    let path = b.post_prompt(&csrf, "The <ledger>", "teen").await;
    let page = b.get(&format!("{path}?notice=posted")).await;
    assert!(page.body.contains("Prompt posted."));
    assert!(page.body.contains("The &#60;ledger&#62;"));
    assert!(page.body.contains("<em>keeper</em>"));
    assert!(!page.body.contains("<script>"));
    assert!(page.body.contains(r#"href="/?include=genre%3Ahorror""#));

    // It shows on the board, and the board filters work.
    let board = b.get("/?include=genre%3Ahorror").await;
    assert!(board.body.contains("The &#60;ledger&#62;"));
    let filtered = b.get("/?exclude=genre%3Ahorror").await;
    assert!(filtered.body.contains("No prompts match these filters."));
    let bad = b.get("/?include=nocolon").await;
    assert!(bad.body.contains("must be written as namespace:name"));

    // Edit, change status, then delete.
    let form = b.get(&format!("{path}/edit")).await;
    assert!(form.body.contains("genre:Horror, theme:Lighthouses"));
    let saved = b
        .post(
            &format!("{path}/edit"),
            &[
                ("csrf", &csrf),
                ("title", "The ledger"),
                ("body", "Edited."),
                ("maturity", "general"),
                ("state", "busy"),
                ("tags", "genre:mystery"),
            ],
        )
        .await;
    assert_eq!(saved.location(), format!("{path}?notice=saved"));
    let page = b.get(&path).await;
    assert!(page.body.contains("The ledger"));
    assert!(page.body.contains("Busy"));
    assert!(page.body.contains("genre: </span>Mystery"));

    let status = b
        .post(
            &format!("{path}/state"),
            &[("csrf", &csrf), ("state", "closed")],
        )
        .await;
    assert_eq!(status.location(), format!("{path}?notice=status"));
    assert!(b.get(&path).await.body.contains("Closed"));

    // Deleting needs the confirmation box.
    let unconfirmed = b.post(&format!("{path}/delete"), &[("csrf", &csrf)]).await;
    assert_eq!(unconfirmed.status, StatusCode::BAD_REQUEST);
    let deleted = b
        .post(
            &format!("{path}/delete"),
            &[("csrf", &csrf), ("confirm", "yes")],
        )
        .await;
    assert_eq!(deleted.location(), "/mine?notice=deleted");
    assert_eq!(b.get(&path).await.status, StatusCode::NOT_FOUND);

    // Form errors re-render with the visitor's input kept.
    let form = b.get("/new").await;
    assert_eq!(form.status, StatusCode::OK);
    let start = form.body.find(r#"<option value=""#).unwrap() + 15;
    let persona = form.body[start..start + 36].to_owned();
    let err = b
        .post(
            "/new",
            &[
                ("csrf", &csrf),
                ("persona", &persona),
                ("title", "Kept title"),
                ("body", "Kept body"),
                ("maturity", "general"),
                ("tags", "content-warning:Spiders"),
            ],
        )
        .await;
    assert_eq!(err.status, StatusCode::BAD_REQUEST);
    assert!(err.body.contains("Kept title"));
    assert!(err.body.contains("Kept body"));
    assert!(err.body.contains("is not a tag on this server"));
    server.stop().await.unwrap();
}

#[tokio::test]
async fn forms_reject_csrf_and_cross_site_posts() {
    let server = server!();
    let mut b = Browser::new(&server);
    let csrf = b.register("alice", "ink_fox").await;

    let forged = b
        .post(
            "/settings",
            &[("csrf", "wrong"), ("maturity_max", "explicit")],
        )
        .await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
    assert!(forged.body.contains("This form has expired"));
    let missing = b.post("/settings", &[("maturity_max", "explicit")]).await;
    assert_eq!(missing.status, StatusCode::FORBIDDEN);

    // A browser reporting another site as the source is refused outright,
    // even with a valid token.
    let cross = b
        .post_with(
            "/settings",
            &[("csrf", &csrf), ("maturity_max", "explicit")],
            &[("sec-fetch-site", "cross-site")],
        )
        .await;
    assert_eq!(cross.status, StatusCode::FORBIDDEN);
    let host = server.url().trim_start_matches("http://").to_owned();
    let foreign_origin = b
        .post_with(
            "/login",
            &[("username", "alice"), ("password", TEST_PASSWORD)],
            &[("origin", "https://evil.example"), ("host", &host)],
        )
        .await;
    assert_eq!(foreign_origin.status, StatusCode::FORBIDDEN);

    // The genuine form still works.
    let ok = b
        .post("/settings", &[("csrf", &csrf), ("maturity_max", "mature")])
        .await;
    assert_eq!(ok.location(), "/settings?notice=settings");
    let settings = b.get("/settings").await;
    assert!(settings.body.contains(r#"<option value="mature" selected"#));
    server.stop().await.unwrap();
}

#[tokio::test]
async fn sign_in_failures_and_sign_out() {
    let server = server!();
    let mut b = Browser::new(&server);
    let csrf = b.register("alice", "ink_fox").await;

    let out = b.post("/logout", &[("csrf", &csrf)]).await;
    assert_eq!(out.location(), "/?notice=signed-out");
    assert!(b.cookie.is_none());
    assert_eq!(b.get("/settings").await.status, StatusCode::SEE_OTHER);

    for (user, pass) in [("alice", "wrong password!!"), ("nobody", TEST_PASSWORD)] {
        let page = b
            .post("/login", &[("username", user), ("password", pass)])
            .await;
        assert_eq!(page.status, StatusCode::UNAUTHORIZED);
        assert!(page.body.contains("don&#39;t match an account"));
        assert!(page.body.contains(&format!(r#"value="{user}""#)));
    }
    let ok = b
        .post(
            "/login",
            &[("username", "alice"), ("password", TEST_PASSWORD)],
        )
        .await;
    assert_eq!(ok.location(), "/?notice=signed-in");
    assert!(
        b.get("/?notice=signed-in")
            .await
            .body
            .contains("Signed in.")
    );

    let mut other = Browser::new(&server);
    let taken = other
        .post(
            "/register",
            &[
                ("username", "Alice"),
                ("password", TEST_PASSWORD),
                ("confirm", TEST_PASSWORD),
            ],
        )
        .await;
    assert_eq!(taken.status, StatusCode::BAD_REQUEST);
    assert!(taken.body.contains("That login name is taken."));
    let mismatch = other
        .post(
            "/register",
            &[
                ("username", "bob"),
                ("password", TEST_PASSWORD),
                ("confirm", "different one!"),
            ],
        )
        .await;
    assert!(mismatch.body.contains("The two passwords don&#39;t match."));
    server.stop().await.unwrap();
}

#[tokio::test]
async fn maturity_warning_reports_and_admin_inbox() {
    let server = server!();
    let mut author = Browser::new(&server);
    let csrf = author.register("alice", "ink_fox").await;
    let mature = author.post_prompt(&csrf, "After dark", "mature").await;
    let teen = author.post_prompt(&csrf, "Daylight", "teen").await;

    // Anonymous visitors see Teen and below on the board...
    let mut anon = Browser::new(&server);
    let board = anon.get("/").await;
    assert!(board.body.contains("Daylight"));
    assert!(!board.body.contains("After dark"));
    // ...and a warning before a Mature prompt reached by link.
    let held = anon.get(&mature).await;
    assert!(
        held.body
            .contains("is rated Mature, above your setting (Teen)")
    );
    assert!(!held.body.contains("<em>keeper</em>"));
    let shown = anon.get(&format!("{mature}?show=1")).await;
    assert!(shown.body.contains("<em>keeper</em>"));

    // Anonymous report through the form.
    let form = anon.get(&format!("{teen}/report")).await;
    assert!(form.body.contains("Wrong maturity rating"));
    let no_reason = anon
        .post(&format!("{teen}/report"), &[("details", "x")])
        .await;
    assert_eq!(no_reason.status, StatusCode::BAD_REQUEST);
    let sent = anon
        .post(
            &format!("{teen}/report"),
            &[("reason", "spam"), ("details", "Posted ten times")],
        )
        .await;
    assert_eq!(sent.location(), format!("{teen}?notice=reported"));

    // Non-admins can't see the inbox.
    let refused = author.get("/admin/reports").await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);

    // An administrator removes the prompt from the board.
    let mut admin = Browser::new(&server);
    let admin_csrf = admin.register("moderator", "mod_desk").await;
    let pool = server.db().await.unwrap();
    sqlx::query("UPDATE accounts SET is_admin = true WHERE login_name = 'moderator'")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let inbox = admin.get("/admin/reports").await;
    assert_eq!(inbox.status, StatusCode::OK);
    assert!(inbox.body.contains("Posted ten times"));
    assert!(inbox.body.contains("Daylight"));
    let marker = "/admin/reports/";
    let start = inbox.body.find(marker).unwrap() + marker.len();
    let report_id = inbox.body[start..start + 36].to_owned();
    let resolved = admin
        .post(
            &format!("/admin/reports/{report_id}/resolve"),
            &[("csrf", &admin_csrf), ("action", "delete")],
        )
        .await;
    assert_eq!(resolved.location(), "/admin/reports?notice=resolved");
    assert!(
        admin
            .get("/admin/reports")
            .await
            .body
            .contains("No open reports.")
    );
    let log = admin.get("/admin/log").await;
    assert!(log.body.contains("Prompt removed"));
    assert!(log.body.contains("Report received"));

    assert!(!anon.get("/").await.body.contains("Daylight"));
    assert_eq!(anon.get(&teen).await.status, StatusCode::NOT_FOUND);
    let own = author.get(&teen).await;
    assert!(own.body.contains("removed this prompt from the board"));
    assert!(!own.body.contains(&format!("{teen}/edit")));
    server.stop().await.unwrap();
}

#[tokio::test]
async fn server_settings_manage_tag_types_and_tags() {
    let server = server!();
    let mut writer = Browser::new(&server);
    let writer_csrf = writer.register("alice", "ink_fox").await;
    writer.post_prompt(&writer_csrf, "Lamps", "general").await;

    // Signed out: sign in first. Not an admin: refused, with no header link.
    let mut anon = Browser::new(&server);
    assert_eq!(anon.get("/admin/settings/tags").await.location(), "/login");
    let refused = writer.get("/admin/settings/tag-types").await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert!(!writer.get("/").await.body.contains("Server settings"));
    let forged = writer
        .post(
            "/admin/settings/tag-types/genre",
            &[
                ("csrf", &writer_csrf),
                ("display", "Genre"),
                ("required", "yes"),
            ],
        )
        .await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);

    let mut admin = Browser::new(&server);
    let csrf = admin.register("moderator", "mod_desk").await;
    let pool = server.db().await.unwrap();
    sqlx::query("UPDATE accounts SET is_admin = true WHERE login_name = 'moderator'")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert!(
        admin
            .get("/")
            .await
            .body
            .contains(r#"href="/admin/settings""#)
    );
    assert_eq!(
        admin.get("/admin/settings").await.location(),
        "/admin/settings/tag-types"
    );

    // Make Genre required. Unticked boxes mean "off".
    let types = admin.get("/admin/settings/tag-types").await;
    assert!(
        types
            .body
            .contains(r#"action="/admin/settings/tag-types/content-warning""#)
    );
    let saved = admin
        .post(
            "/admin/settings/tag-types/genre",
            &[("csrf", &csrf), ("display", "Genre"), ("required", "yes")],
        )
        .await;
    assert_eq!(
        saved.location(),
        "/admin/settings/tag-types?notice=tag-type-saved"
    );
    let types = admin
        .get("/admin/settings/tag-types?notice=tag-type-saved")
        .await;
    assert!(types.body.contains("Tag type saved."));

    // The prompt form says so, and enforces it.
    let form = writer.get("/new").await;
    assert!(
        form.body
            .contains("Required: at least one Genre (genre:…) tag.")
    );
    let start = form.body.find(r#"<option value=""#).unwrap() + 15;
    let persona = form.body[start..start + 36].to_owned();
    let missing = writer
        .post(
            "/new",
            &[
                ("csrf", &writer_csrf),
                ("persona", &persona),
                ("title", "No genre"),
                ("body", "Body"),
                ("maturity", "general"),
                ("tags", "format:prose"),
            ],
        )
        .await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST);
    assert!(missing.body.contains("Add: Genre."));
    assert!(missing.body.contains("No genre"));

    // Adding a tag type that exists re-shows the form with the input kept.
    let dup = admin
        .post(
            "/admin/settings/tag-types",
            &[
                ("csrf", &csrf),
                ("name", "genre"),
                ("display", "Genre again"),
            ],
        )
        .await;
    assert_eq!(dup.status, StatusCode::BAD_REQUEST);
    assert!(dup.body.contains("already exists"));
    assert!(dup.body.contains(r#"value="Genre again""#));
    let added = admin
        .post(
            "/admin/settings/tag-types",
            &[
                ("csrf", &csrf),
                ("name", "setting"),
                ("display", "Setting"),
                ("locked", "yes"),
            ],
        )
        .await;
    assert_eq!(
        added.location(),
        "/admin/settings/tag-types?notice=tag-type-created"
    );

    // Tags: add, rename, merge, delete.
    let created = admin
        .post(
            "/admin/settings/tags",
            &[
                ("csrf", &csrf),
                ("namespace", "setting"),
                ("name", "Deep space"),
            ],
        )
        .await;
    assert_eq!(
        created.location(),
        "/admin/settings/tags/setting/deep-space?notice=tag-created"
    );
    let renamed = admin
        .post(
            "/admin/settings/tags/setting/deep-space",
            &[("csrf", &csrf), ("display", "Outer space")],
        )
        .await;
    assert_eq!(
        renamed.location(),
        "/admin/settings/tags/setting/deep-space?notice=tag-saved"
    );
    let list = admin.get("/admin/settings/tags?namespace=theme").await;
    assert!(list.body.contains("Lighthouses"));
    assert!(list.body.contains("1 prompt"));
    assert!(!list.body.contains("Outer space"));
    let bad_merge = admin
        .post(
            "/admin/settings/tags/theme/lighthouses/merge",
            &[("csrf", &csrf), ("target", "theme:nowhere")],
        )
        .await;
    assert_eq!(bad_merge.status, StatusCode::BAD_REQUEST);
    assert!(bad_merge.body.contains("unknown tag"));

    let confirm = admin.get("/admin/settings/tags/genre/horror/delete").await;
    assert!(confirm.body.contains("It&#39;s on 1 prompt."));
    assert!(confirm.body.contains("Genre is a required type."));
    let unconfirmed = admin
        .post(
            "/admin/settings/tags/genre/horror/delete",
            &[("csrf", &csrf)],
        )
        .await;
    assert_eq!(unconfirmed.status, StatusCode::BAD_REQUEST);
    let deleted = admin
        .post(
            "/admin/settings/tags/genre/horror/delete",
            &[("csrf", &csrf), ("confirm", "yes")],
        )
        .await;
    assert_eq!(
        deleted.location(),
        "/admin/settings/tags?namespace=genre&notice=tag-deleted"
    );
    assert_eq!(
        admin.get("/admin/settings/tags/genre/horror").await.status,
        StatusCode::NOT_FOUND
    );
    assert!(!anon.get("/").await.body.contains("genre: </span>Horror"));

    let log = admin.get("/admin/log").await;
    for event in [
        "Tag deleted",
        "Tag renamed",
        "Tag added",
        "Tag type added",
        "Tag type changed",
    ] {
        assert!(log.body.contains(event), "{event}");
    }
    assert!(log.body.contains("<code>genre:horror</code>"));
    server.stop().await.unwrap();
}

#[tokio::test]
async fn tag_requests_through_the_board() {
    let server = server!();
    let mut admin = Browser::new(&server);
    let csrf = admin.register("moderator", "mod_desk").await;
    let pool = server.db().await.unwrap();
    sqlx::query("UPDATE accounts SET is_admin = true WHERE login_name = 'moderator'")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    // Turn approval on with the switch.
    let types = admin.get("/admin/settings/tag-types").await;
    assert!(types.body.contains(r#"value="open" checked=true"#));
    let saved = admin
        .post(
            "/admin/settings/new-tags",
            &[("csrf", &csrf), ("new_tags", "approval")],
        )
        .await;
    assert_eq!(
        saved.location(),
        "/admin/settings/tag-types?notice=new-tags-saved"
    );

    // A writer's new tag (post_prompt uses theme:Lighthouses) waits.
    let mut writer = Browser::new(&server);
    let writer_csrf = writer.register("alice", "ink_fox").await;
    let form = writer.get("/new").await;
    assert!(
        form.body
            .contains("Others see a new tag once an administrator approves it.")
    );
    let path = writer.post_prompt(&writer_csrf, "Lamps", "general").await;
    let own = writer.get(&path).await;
    assert!(
        own.body
            .contains("Lighthouses <em>(awaiting approval)</em>")
    );
    let mut anon = Browser::new(&server);
    assert!(!anon.get(&path).await.body.contains("Lighthouses"));

    // The admin sees the count and the request.
    let home = admin.get("/").await;
    assert!(
        home.body
            .contains(r#"title="Tag requests waiting">1</span>"#)
    );
    let requests = admin.get("/admin/settings/tag-requests").await;
    assert!(requests.body.contains("theme:lighthouses"));
    let approved = admin
        .post(
            "/admin/settings/tags/theme/lighthouses/status",
            &[("csrf", &csrf), ("status", "visible"), ("back", "requests")],
        )
        .await;
    assert_eq!(
        approved.location(),
        "/admin/settings/tag-requests?notice=tag-approved"
    );
    let after = admin
        .get("/admin/settings/tag-requests?notice=tag-approved")
        .await;
    assert!(after.body.contains("Tag approved."));
    assert!(after.body.contains("No tag requests waiting."));
    assert!(!after.body.contains(r#"class="count""#));
    assert!(anon.get(&path).await.body.contains("Lighthouses"));

    // Reject a second request through the confirmation page.
    let second = writer.post_prompt(&writer_csrf, "Again", "general").await;
    let pool = server.db().await.unwrap();
    sqlx::query(
        "INSERT INTO tags (namespace, slug, display, origin, status)
         VALUES ('theme', 'junk', 'Junk', 'user', 'pending')",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    let confirm = admin.get("/admin/settings/tags/theme/junk/delete").await;
    assert!(confirm.body.contains("Reject Junk?"));
    let rejected = admin
        .post(
            "/admin/settings/tags/theme/junk/delete",
            &[("csrf", &csrf), ("confirm", "yes")],
        )
        .await;
    assert_eq!(
        rejected.location(),
        "/admin/settings/tag-requests?notice=tag-rejected"
    );

    // Hide a tag from its page: gone for the author and visitors.
    let hidden = admin
        .post(
            "/admin/settings/tags/genre/horror/status",
            &[("csrf", &csrf), ("status", "hidden"), ("back", "tag")],
        )
        .await;
    assert_eq!(
        hidden.location(),
        "/admin/settings/tags/genre/horror?notice=tag-hidden"
    );
    assert!(!writer.get(&second).await.body.contains("Horror"));
    assert!(
        !anon
            .get("/?include=genre%3Ahorror")
            .await
            .body
            .contains("Again")
    );
    assert!(
        admin
            .get(&second)
            .await
            .body
            .contains("Horror <em>(hidden)</em>")
    );

    let log = admin.get("/admin/log").await;
    for event in [
        "Setting changed",
        "Tag approved",
        "Tag rejected",
        "Tag hidden",
    ] {
        assert!(log.body.contains(event), "{event}");
    }
    server.stop().await.unwrap();
}
