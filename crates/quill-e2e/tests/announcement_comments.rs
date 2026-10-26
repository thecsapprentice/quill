//! Comments on announcements (#41) end to end: writers comment as
//! personas; comments close with their announcement; reports and removals
//! go through the same inbox, log and notices as prompts.

// Shared helpers below are test code too; panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use quill_core::{Client, CoreError};
use quill_e2e::{TEST_PASSWORD, TestServer};
use quill_proto::{
    announcements::{
        Announcement, CommentQuery, CreateAnnouncement, CreateComment, RemoveComment,
        ReportComment, UpdateAnnouncement,
    },
    api::CreatePersona,
    board::{AdminAction, AdminLogQuery, ReportKind, ReportReason, Resolution},
    notices::NoticeKind,
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
            handle: format!("{name}_pen"),
            display_name: None,
            summary: None,
        })
        .await
        .unwrap();
    (client, persona.id)
}

async fn announce(admin: &Client, title: &str) -> Announcement {
    admin
        .admin_post_announcement(&CreateAnnouncement {
            title: title.into(),
            body: "News.".into(),
            comments_open: true,
        })
        .await
        .unwrap()
}

async fn comment(client: &Client, persona: Uuid, on: Uuid, text: &str) -> Uuid {
    client
        .post_comment(
            on,
            &CreateComment {
                persona,
                body: text.into(),
            },
        )
        .await
        .unwrap()
        .id
}

fn report(reason: ReportReason) -> ReportComment {
    ReportComment {
        reason,
        details: None,
    }
}

/// The ids of the comments `client` sees on `on`.
async fn visible(client: &Client, on: Uuid) -> Vec<(Uuid, bool)> {
    client
        .comments(on, &CommentQuery::default())
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|c| (c.id, c.removed))
        .collect()
}

#[tokio::test]
async fn writers_comment_until_comments_close() {
    let server = server!();
    let admin = server.admin_account("boss").await.unwrap();
    let (alice, alice_pen) = writer(&server, "alice").await;
    let (bob, bob_pen) = writer(&server, "bob").await;
    let anonymous = server.client();
    let a = announce(&admin, "Welcome").await;

    let first = alice
        .post_comment(
            a.id,
            &CreateComment {
                persona: alice_pen,
                body: "Hello *all*".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(first.author.handle, "alice_pen");
    assert_eq!(first.body_html, "<p>Hello <em>all</em></p>\n");
    let second = comment(&bob, bob_pen, a.id, "Hi").await;

    // Only as one of your own personas, with some text, signed in.
    let theirs = bob
        .post_comment(
            a.id,
            &CreateComment {
                persona: alice_pen,
                body: "Not mine".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(status(&theirs), 400);
    let empty = bob
        .post_comment(
            a.id,
            &CreateComment {
                persona: bob_pen,
                body: " \n ".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(status(&empty), 400);
    let signed_out = anonymous
        .post_comment(
            a.id,
            &CreateComment {
                persona: bob_pen,
                body: "Hi".into(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(signed_out, CoreError::NotLoggedIn));

    // Anyone reads them, oldest first, a page at a time.
    let page = anonymous
        .comments(
            a.id,
            &CommentQuery {
                cursor: None,
                limit: Some(1),
            },
        )
        .await
        .unwrap();
    assert_eq!(page.items[0].id, first.id);
    let rest = anonymous
        .comments(
            a.id,
            &CommentQuery {
                cursor: page.next_cursor,
                limit: Some(1),
            },
        )
        .await
        .unwrap();
    assert_eq!(rest.items[0].id, second);
    assert!(rest.next_cursor.is_none());

    // Closed: comments stay, new ones are refused. Reopened: allowed again.
    let close = |open| UpdateAnnouncement {
        comments_open: Some(open),
        ..UpdateAnnouncement::default()
    };
    admin
        .admin_update_announcement(a.id, &close(false))
        .await
        .unwrap();
    let refused = bob
        .post_comment(
            a.id,
            &CreateComment {
                persona: bob_pen,
                body: "Late".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(status(&refused), 403);
    assert_eq!(visible(&anonymous, a.id).await.len(), 2);
    admin
        .admin_update_announcement(a.id, &close(true))
        .await
        .unwrap();
    comment(&bob, bob_pen, a.id, "Back").await;

    // Writers delete only their own comments.
    assert_eq!(
        status(&bob.delete_comment(first.id).await.unwrap_err()),
        404
    );
    alice.delete_comment(first.id).await.unwrap();
    assert!(
        !visible(&anonymous, a.id)
            .await
            .iter()
            .any(|(id, _)| *id == first.id)
    );

    // A deleted announcement takes its comments with it.
    admin.admin_delete_announcement(a.id).await.unwrap();
    let gone = anonymous
        .comments(a.id, &CommentQuery::default())
        .await
        .unwrap_err();
    assert_eq!(status(&gone), 404);
    let gone = bob
        .post_comment(
            a.id,
            &CreateComment {
                persona: bob_pen,
                body: "Hello?".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(status(&gone), 404);
    let gone = anonymous
        .report_comment(second, &report(ReportReason::Spam))
        .await
        .unwrap_err();
    assert_eq!(status(&gone), 404);
}

#[tokio::test]
async fn comments_are_rate_limited_per_login() {
    let server = server!();
    let admin = server.admin_account("boss").await.unwrap();
    let (alice, alice_pen) = writer(&server, "alice").await;
    let a = announce(&admin, "Welcome").await;
    for i in 0..30 {
        comment(&alice, alice_pen, a.id, &format!("Comment {i}")).await;
    }
    let limited = alice
        .post_comment(
            a.id,
            &CreateComment {
                persona: alice_pen,
                body: "One more".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(status(&limited), 429);
}

#[tokio::test]
async fn reported_comments_are_removed_through_the_inbox() {
    let server = server!();
    let admin = server.admin_account("boss").await.unwrap();
    let (alice, alice_pen) = writer(&server, "alice").await;
    let (bob, _) = writer(&server, "bob").await;
    let anonymous = server.client();
    let a = announce(&admin, "Welcome").await;
    let rude = comment(&alice, alice_pen, a.id, "Something rude").await;

    bob.report_comment(rude, &report(ReportReason::Harassment))
        .await
        .unwrap();
    let inbox = admin.admin_reports(false).await.unwrap();
    assert_eq!(inbox.len(), 1);
    let r = &inbox[0];
    assert_eq!(r.kind, ReportKind::Comment);
    assert!(r.prompt.is_none());
    assert_eq!(r.comment.as_ref().unwrap().id, rude);
    let hash: String = quill_server::reports::comment_hash("Something rude")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(r.prompt_hash, hash);

    let resolved = admin
        .resolve_report(r.id, Resolution::Delete)
        .await
        .unwrap();
    assert_eq!(resolved.resolution, Some(Resolution::Delete));
    assert!(resolved.comment.unwrap().removed);

    // Hidden from everyone but its author and administrators, who see it
    // marked; it can't be reported again.
    assert!(visible(&anonymous, a.id).await.is_empty());
    assert!(visible(&bob, a.id).await.is_empty());
    assert_eq!(visible(&alice, a.id).await, vec![(rude, true)]);
    assert_eq!(visible(&admin, a.id).await, vec![(rude, true)]);
    let again = bob
        .report_comment(rude, &report(ReportReason::Spam))
        .await
        .unwrap_err();
    assert_eq!(status(&again), 404);

    // The author is told which announcement and why, never by whom.
    let notices = alice.notices(None).await.unwrap();
    assert_eq!(notices[0].kind, NoticeKind::CommentRemoved);
    let f = &notices[0].fields;
    assert_eq!(f.reason, Some(ReportReason::Harassment));
    assert_eq!(f.announcement, Some(a.id));
    assert_eq!(f.announcement_title.as_deref(), Some("Welcome"));
    assert_eq!(f.handle.as_deref(), Some("alice_pen"));

    // Both steps are logged against alice, with the comment's hash.
    let log = admin
        .admin_log_filtered(&AdminLogQuery::default())
        .await
        .unwrap();
    let removal = &log[0];
    assert_eq!(removal.action, AdminAction::DeleteComment);
    assert_eq!(removal.comment_id, Some(rude));
    assert_eq!(removal.report_id, Some(r.id));
    assert_eq!(removal.prompt_hash.as_deref(), Some(hash.as_str()));
    assert_eq!(removal.targets, vec!["alice".to_owned()]);
    assert_eq!(log[1].action, AdminAction::ReportReceived);
    assert_eq!(log[1].comment_id, Some(rude));
    assert_eq!(log[1].targets, vec!["alice".to_owned()]);
}

#[tokio::test]
async fn administrators_remove_or_discard_directly() {
    let server = server!();
    let admin = server.admin_account("boss").await.unwrap();
    let (alice, alice_pen) = writer(&server, "alice").await;
    let (bob, bob_pen) = writer(&server, "bob").await;
    let a = announce(&admin, "Welcome").await;
    let spam = comment(&alice, alice_pen, a.id, "Buy now").await;
    let fine = comment(&bob, bob_pen, a.id, "All good").await;
    let gone = comment(&bob, bob_pen, a.id, "Oops").await;

    let remove = RemoveComment {
        reason: ReportReason::Spam,
    };
    assert_eq!(
        status(&bob.admin_remove_comment(spam, &remove).await.unwrap_err()),
        403
    );
    admin.admin_remove_comment(spam, &remove).await.unwrap();
    assert_eq!(
        status(&admin.admin_remove_comment(spam, &remove).await.unwrap_err()),
        409
    );
    let log = admin
        .admin_log_filtered(&AdminLogQuery::default())
        .await
        .unwrap();
    assert_eq!(log[0].action, AdminAction::DeleteComment);
    assert_eq!(log[0].report_id, None);
    assert_eq!(log[0].reason, Some(ReportReason::Spam));
    assert_eq!(
        alice.notices(None).await.unwrap()[0].kind,
        NoticeKind::CommentRemoved
    );

    // Discarding a comment report leaves the comment alone.
    alice
        .report_comment(fine, &report(ReportReason::Other))
        .await
        .unwrap();
    // A report outlives the comment its author deleted; then it can only
    // be discarded.
    alice
        .report_comment(gone, &report(ReportReason::Other))
        .await
        .unwrap();
    bob.delete_comment(gone).await.unwrap();
    let inbox = admin.admin_reports(false).await.unwrap();
    assert_eq!(inbox.len(), 2);
    let (on_fine, on_gone) = (&inbox[0], &inbox[1]);
    assert_eq!(on_fine.comment.as_ref().unwrap().id, fine);
    assert!(on_gone.comment.is_none());
    assert_eq!(on_gone.kind, ReportKind::Comment);

    admin
        .resolve_report(on_fine.id, Resolution::Discard)
        .await
        .unwrap();
    assert!(
        visible(&server.client(), a.id)
            .await
            .contains(&(fine, false))
    );
    let refused = admin
        .resolve_report(on_gone.id, Resolution::Delete)
        .await
        .unwrap_err();
    assert_eq!(status(&refused), 409);
    admin
        .resolve_report(on_gone.id, Resolution::Discard)
        .await
        .unwrap();
    let log = admin
        .admin_log_filtered(&AdminLogQuery::default())
        .await
        .unwrap();
    // The comment is gone, so the entry names only the report; the report's
    // own entry still names the comment.
    assert_eq!(log[0].action, AdminAction::Discard);
    assert_eq!(log[0].report_id, Some(on_gone.id));
    assert_eq!(log[0].comment_id, None);
    assert!(log[0].targets.is_empty());
    assert!(log.iter().any(|e| e.action == AdminAction::ReportReceived
        && e.report_id == Some(on_gone.id)
        && e.comment_id == Some(gone)));
    assert_eq!(log[1].action, AdminAction::Discard);
    assert_eq!(log[1].comment_id, Some(fine));
    assert_eq!(log[1].targets, vec!["bob".to_owned()]);
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

/// A browser without JavaScript: follows no redirects, keeps the session.
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
            self.cookie = Some(set.to_str().unwrap().split(';').next().unwrap().to_owned());
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
async fn comments_on_the_board() {
    let server = server!();
    let admin = server.admin_account("boss").await.unwrap();
    let (_, alice_pen) = writer(&server, "alice").await;
    let a = announce(&admin, "Welcome").await;
    let page_path = format!("/announcements/{}", a.id);

    // Signed out: no form, a sign-in link.
    let mut reader = Browser::new(&server);
    let page = reader.get(&page_path).await;
    assert!(page.body.contains("No comments yet."));
    assert!(!page.body.contains("Post comment"));

    let mut alice = Browser::new(&server);
    alice.sign_in("alice").await;
    let page = alice.get(&page_path).await;
    let csrf = page.csrf();
    let pen = alice_pen.to_string();

    // A blank comment comes back with an error.
    let blank = alice
        .post(
            &format!("{page_path}/comments"),
            &[("csrf", &csrf), ("persona", &pen), ("body", " ")],
        )
        .await;
    assert_eq!(blank.status, StatusCode::BAD_REQUEST);
    assert!(blank.body.contains("body is required"));

    let posted = alice
        .post(
            &format!("{page_path}/comments"),
            &[("csrf", &csrf), ("persona", &pen), ("body", "First!")],
        )
        .await;
    assert_eq!(posted.status, StatusCode::SEE_OTHER);
    let location = posted.location().to_owned();
    assert!(location.starts_with(&format!("{page_path}?notice=comment-posted#c-")));
    let id = location.rsplit("#c-").next().unwrap().to_owned();
    let page = alice.get(&location).await;
    assert!(page.body.contains("Comment posted."));
    assert!(page.body.contains("First!"));
    assert!(page.body.contains(&format!("/comments/{id}/delete")));

    // Anyone can report it, signed in or not.
    let form = reader.get(&format!("/comments/{id}/report")).await;
    assert_eq!(form.status, StatusCode::OK);
    let sent = reader
        .post(&format!("/comments/{id}/report"), &[("reason", "spam")])
        .await;
    assert_eq!(sent.status, StatusCode::SEE_OTHER);
    assert!(sent.location().contains("notice=reported"));

    // Administrators see it in the inbox and can remove it in the thread.
    let mut boss = Browser::new(&server);
    boss.sign_in("boss").await;
    let inbox = boss.get("/admin/reports").await;
    assert!(inbox.body.contains("Comment by @alice_pen"));
    let boss_csrf = inbox.csrf();
    let removed = boss
        .post(
            &format!("/admin/comments/{id}/remove"),
            &[("csrf", &boss_csrf), ("reason", "spam")],
        )
        .await;
    assert_eq!(removed.status, StatusCode::SEE_OTHER);
    assert!(removed.location().contains("notice=comment-removed"));

    // Gone for readers; marked for its author, who can follow its link.
    assert!(!reader.get(&page_path).await.body.contains("First!"));
    assert_eq!(
        reader.get(&format!("/comments/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    let page = alice.get(&page_path).await;
    assert!(page.body.contains("First!"));
    assert!(page.body.contains("Removed by a server administrator"));
    let link = alice.get(&format!("/comments/{id}")).await;
    assert_eq!(link.location(), format!("{page_path}#c-{id}"));
    let notices = alice.get("/notices").await;
    assert!(
        notices
            .body
            .contains("removed your comment on “Welcome” (Spam).")
    );

    // The author can still delete it.
    let deleted = alice
        .post(
            &format!("/comments/{id}/delete"),
            &[("csrf", &csrf), ("confirm", "yes")],
        )
        .await;
    assert_eq!(deleted.status, StatusCode::SEE_OTHER);
    assert!(!alice.get(&page_path).await.body.contains("First!"));
}
