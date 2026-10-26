//! Announcements (#41) end to end: administrators post, edit, open or
//! close comments and delete them; anyone reads them; the board shows the
//! newest as a banner that a reader can dismiss with a cookie.

// Shared helpers below are test code too; panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use quill_core::{Client, CoreError};
use quill_e2e::{TEST_PASSWORD, TestServer};
use quill_proto::{
    announcements::{AnnouncementQuery, CreateAnnouncement, UpdateAnnouncement},
    board::{AdminAction, AdminLogQuery},
};
use reqwest::{StatusCode, header};

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

fn create(title: &str) -> CreateAnnouncement {
    CreateAnnouncement {
        title: title.into(),
        body: "Read *this*.".into(),
        comments_open: true,
    }
}

/// `(action, subject, detail)` of the newest `n` log entries, oldest first.
async fn log_tail(admin: &Client, n: usize) -> Vec<(AdminAction, String, Option<String>)> {
    let log = admin
        .admin_log_filtered(&AdminLogQuery::default())
        .await
        .unwrap();
    let mut tail: Vec<_> = log
        .into_iter()
        .take(n)
        .map(|e| (e.action, e.subject.unwrap_or_default(), e.detail))
        .collect();
    tail.reverse();
    tail
}

#[tokio::test]
async fn only_administrators_post_and_everyone_reads() {
    let server = server!();
    let admin = server.admin_account("boss").await.unwrap();
    let writer = server.account("alice").await.unwrap();
    let anonymous = server.client();

    let refused = writer
        .admin_post_announcement(&create("Mine"))
        .await
        .unwrap_err();
    assert_eq!(status(&refused), 403);
    let refused = anonymous
        .admin_post_announcement(&create("Mine"))
        .await
        .unwrap_err();
    assert!(matches!(refused, CoreError::NotLoggedIn));

    let blank = admin
        .admin_post_announcement(&create("  "))
        .await
        .unwrap_err();
    assert_eq!(status(&blank), 400);

    let first = admin
        .admin_post_announcement(&create("Welcome"))
        .await
        .unwrap();
    assert!(first.comments_open);
    assert!(!first.edited());
    // Rendered with the prompts' sanitizer.
    assert_eq!(first.body_html, "<p>Read <em>this</em>.</p>\n");
    let second = admin
        .admin_post_announcement(&CreateAnnouncement {
            comments_open: false,
            ..create("<b>Downtime</b> on Friday")
        })
        .await
        .unwrap();
    assert!(!second.comments_open);

    // Anyone reads them, newest first, a page at a time.
    let page = anonymous
        .announcements(&AnnouncementQuery {
            cursor: None,
            limit: Some(1),
        })
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, second.id);
    let rest = anonymous
        .announcements(&AnnouncementQuery {
            cursor: page.next_cursor,
            limit: Some(1),
        })
        .await
        .unwrap();
    assert_eq!(rest.items[0].id, first.id);
    assert!(rest.next_cursor.is_none());
    assert_eq!(
        anonymous.announcement(second.id).await.unwrap().title,
        "<b>Downtime</b> on Friday"
    );

    assert_eq!(
        log_tail(&admin, 2).await,
        vec![
            (AdminAction::PostAnnouncement, "Welcome".into(), None),
            (
                AdminAction::PostAnnouncement,
                "<b>Downtime</b> on Friday".into(),
                Some("comments closed".into())
            ),
        ]
    );
}

#[tokio::test]
async fn edits_comment_switches_and_deletes_are_logged() {
    let server = server!();
    let admin = server.admin_account("boss").await.unwrap();
    let writer = server.account("alice").await.unwrap();
    let a = admin
        .admin_post_announcement(&create("Welcome"))
        .await
        .unwrap();

    let refused = writer
        .admin_update_announcement(
            a.id,
            &UpdateAnnouncement {
                comments_open: Some(false),
                ..UpdateAnnouncement::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(status(&refused), 403);

    let edited = admin
        .admin_update_announcement(
            a.id,
            &UpdateAnnouncement {
                title: Some("Welcome, writers".into()),
                body: Some("New text.".into()),
                comments_open: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(edited.title, "Welcome, writers");
    assert_eq!(edited.body_html, "<p>New text.</p>\n");
    assert!(edited.edited());

    // Closing comments doesn't count as an edit of the text.
    let closed = admin
        .admin_update_announcement(
            a.id,
            &UpdateAnnouncement {
                comments_open: Some(false),
                ..UpdateAnnouncement::default()
            },
        )
        .await
        .unwrap();
    assert!(!closed.comments_open);
    assert_eq!(closed.updated_at, edited.updated_at);

    // Saving what's already there changes nothing and logs nothing.
    admin
        .admin_update_announcement(
            a.id,
            &UpdateAnnouncement {
                title: Some("Welcome, writers".into()),
                body: None,
                comments_open: Some(false),
            },
        )
        .await
        .unwrap();

    assert_eq!(
        status(&writer.admin_delete_announcement(a.id).await.unwrap_err()),
        403
    );
    admin.admin_delete_announcement(a.id).await.unwrap();

    // Gone everywhere: reading, listing, changing and deleting again.
    let anonymous = server.client();
    assert_eq!(
        status(&anonymous.announcement(a.id).await.unwrap_err()),
        404
    );
    let list = anonymous
        .announcements(&AnnouncementQuery::default())
        .await
        .unwrap();
    assert!(list.items.is_empty());
    let again = admin
        .admin_update_announcement(
            a.id,
            &UpdateAnnouncement {
                comments_open: Some(true),
                ..UpdateAnnouncement::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(status(&again), 404);
    assert_eq!(
        status(&admin.admin_delete_announcement(a.id).await.unwrap_err()),
        404
    );

    assert_eq!(
        log_tail(&admin, 4).await,
        vec![
            (AdminAction::PostAnnouncement, "Welcome".into(), None),
            (
                AdminAction::EditAnnouncement,
                "Welcome, writers".into(),
                Some("title and text changed; was “Welcome”".into())
            ),
            (
                AdminAction::SetAnnouncementComments,
                "Welcome, writers".into(),
                Some("comments: open → closed".into())
            ),
            (
                AdminAction::DeleteAnnouncement,
                "Welcome, writers".into(),
                None
            ),
        ]
    );
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

    fn set_cookie(&self) -> Option<&str> {
        self.headers
            .get(header::SET_COOKIE)
            .map(|v| v.to_str().unwrap())
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

    /// Keeps the cookie a response set, replacing one of the same name.
    fn keep(&mut self, page: &Page) {
        if let Some(set) = page.set_cookie() {
            let pair = set.split(';').next().unwrap().to_owned();
            let name = pair.split('=').next().unwrap().to_owned();
            self.cookies.retain(|c| !c.starts_with(&format!("{name}=")));
            self.cookies.push(pair);
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
        self.keep(&page);
        page
    }

    async fn get(&mut self, path: &str) -> Page {
        let req = self.http.get(format!("{}{path}", self.base));
        self.send(req).await
    }

    async fn post_from(&mut self, site: &str, path: &str, fields: &[(&str, &str)]) -> Page {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields)
            .finish();
        let req = self
            .http
            .post(format!("{}{path}", self.base))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("sec-fetch-site", site)
            .body(body);
        self.send(req).await
    }

    async fn post(&mut self, path: &str, fields: &[(&str, &str)]) -> Page {
        self.post_from("same-origin", path, fields).await
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

fn banner_shows(page: &Page, title: &str) -> bool {
    page.body.contains("announcement-banner") && page.body.contains(title)
}

#[tokio::test]
async fn the_banner_shows_the_newest_until_dismissed() {
    let server = server!();
    let admin = server.admin_account("boss").await.unwrap();
    let mut reader = Browser::new(&server);

    let board = reader.get("/").await;
    assert!(!board.body.contains("announcement-banner"));

    let old = admin
        .admin_post_announcement(&create("Old news"))
        .await
        .unwrap();
    let new = admin
        .admin_post_announcement(&create("Big news"))
        .await
        .unwrap();
    let board = reader.get("/").await;
    assert!(banner_shows(&board, "Big news"));
    assert!(!board.body.contains("Old news"));

    // Another site can't dismiss it for the reader.
    let id = new.id.to_string();
    let forged = reader
        .post_from("cross-site", "/announcements/dismiss", &[("id", &id)])
        .await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);

    let dismissed = reader.post("/announcements/dismiss", &[("id", &id)]).await;
    assert_eq!(dismissed.status, StatusCode::SEE_OTHER);
    assert_eq!(dismissed.location(), "/");
    let cookie = dismissed.set_cookie().unwrap();
    assert!(cookie.starts_with(&format!("__Host-quill_dismissed={id};")));
    for attr in ["Secure", "HttpOnly", "Path=/", "Max-Age="] {
        assert!(cookie.contains(attr), "cookie lacks {attr}: {cookie}");
    }
    assert!(!reader.get("/").await.body.contains("announcement-banner"));

    // Deleting the newest doesn't bring back an older one.
    admin.admin_delete_announcement(new.id).await.unwrap();
    assert!(!reader.get("/").await.body.contains("announcement-banner"));
    // A newer one shows again.
    admin
        .admin_post_announcement(&create("Newer news"))
        .await
        .unwrap();
    assert!(banner_shows(&reader.get("/").await, "Newer news"));

    // Every live announcement is listed and has its own page.
    let list = reader.get("/announcements").await;
    assert_eq!(list.status, StatusCode::OK);
    assert!(list.body.contains("Newer news") && list.body.contains("Old news"));
    assert!(!list.body.contains("Big news"));
    let page = reader.get(&format!("/announcements/{}", old.id)).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("From the administrators"));
    assert!(!page.body.contains("/admin/settings/announcements/"));
    let gone = reader.get(&format!("/announcements/{}", new.id)).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn administrators_manage_announcements_on_the_board() {
    let server = server!();
    server.admin_account("boss").await.unwrap();
    server.account("alice").await.unwrap();

    let mut writer = Browser::new(&server);
    writer.sign_in("alice").await;
    let refused = writer.get("/admin/settings/announcements").await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);

    let mut b = Browser::new(&server);
    b.sign_in("boss").await;
    let tab = b.get("/admin/settings/announcements").await;
    assert_eq!(tab.status, StatusCode::OK);
    assert!(tab.body.contains("No announcements yet."));
    let csrf = tab.csrf();

    // A missing title comes back with the text kept.
    let bad = b
        .post(
            "/admin/settings/announcements",
            &[("csrf", &csrf), ("title", ""), ("body", "Kept text")],
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert!(bad.body.contains("Kept text"));

    // Posted without ticking "Allow comments": they're closed.
    let posted = b
        .post(
            "/admin/settings/announcements",
            &[("csrf", &csrf), ("title", "Hello"), ("body", "Hi *all*")],
        )
        .await;
    assert_eq!(posted.status, StatusCode::SEE_OTHER);
    let page = b.get(posted.location()).await;
    assert!(page.body.contains("Announcement posted."));
    assert!(page.body.contains("<p>Hi <em>all</em></p>"));
    let edit_path = {
        let marker = r#"href="/admin/settings/announcements/"#;
        let start = page.body.find(marker).unwrap() + r#"href=""#.len();
        let end = page.body[start..].find('"').unwrap();
        page.body[start..start + end].to_owned()
    };
    let list = b.get("/admin/settings/announcements").await;
    assert!(list.body.contains("Open comments"));

    let opened = b
        .post(
            &format!("{edit_path}/comments"),
            &[("csrf", &csrf), ("comments", "on")],
        )
        .await;
    assert_eq!(
        opened.location(),
        "/admin/settings/announcements?notice=announcement-comments"
    );
    assert!(
        b.get("/admin/settings/announcements")
            .await
            .body
            .contains("Close comments")
    );

    let saved = b
        .post(
            &edit_path,
            &[("csrf", &csrf), ("title", "Hello again"), ("body", "Hi")],
        )
        .await;
    assert_eq!(saved.status, StatusCode::SEE_OTHER);
    assert!(b.get("/").await.body.contains("Hello again"));

    // Deleting needs the box ticked.
    let unconfirmed = b
        .post(&format!("{edit_path}/delete"), &[("csrf", &csrf)])
        .await;
    assert_eq!(unconfirmed.status, StatusCode::BAD_REQUEST);
    let deleted = b
        .post(
            &format!("{edit_path}/delete"),
            &[("csrf", &csrf), ("confirm", "yes")],
        )
        .await;
    assert_eq!(
        deleted.location(),
        "/admin/settings/announcements?notice=announcement-deleted"
    );
    assert!(!b.get("/").await.body.contains("announcement-banner"));

    // Forms without the session's token are refused.
    let forged = b
        .post(
            "/admin/settings/announcements",
            &[("csrf", "wrong"), ("title", "Nope"), ("body", "Nope")],
        )
        .await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
}
