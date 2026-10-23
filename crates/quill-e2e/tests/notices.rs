//! Phase 1h end-to-end tests: account notices of administrators' actions.

// Shared helpers below are test code too; panicking on failure is intended.
#![allow(clippy::unwrap_used)]

use quill_core::Client;
use quill_e2e::TestServer;
use quill_proto::{
    api::CreatePersona,
    board::{
        ChipColor, CreatePrompt, CreateReport, Maturity, ReportReason, Resolution, TagPolicy,
        TagStatus, UpdateServerSettings, UpdateTag,
    },
    notices::{AccountNotice, NoticeKind},
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

async fn post(client: &Client, persona: Uuid, title: &str, tags: &[&str]) -> Uuid {
    client
        .create_prompt(&CreatePrompt {
            persona,
            title: title.into(),
            body: "Body.".into(),
            maturity: Maturity::General,
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
            state: None,
        })
        .await
        .unwrap()
        .id
}

/// `(kind, titles of the affected prompts)`, newest first.
async fn summary(client: &Client) -> Vec<(NoticeKind, Vec<String>)> {
    client
        .notices(None)
        .await
        .unwrap()
        .into_iter()
        .map(|n| (n.kind, n.prompts.into_iter().map(|p| p.title).collect()))
        .collect()
}

#[tokio::test]
async fn a_removed_prompt_tells_its_author_and_nobody_else() {
    let server = server!();
    let (alice, ap) = writer(&server, "alice").await;
    let bob = server.account("bob").await.unwrap();
    let admin = server.admin_account("boss").await.unwrap();
    let id = post(&alice, ap, "Lamps", &[]).await;

    let report = |reason| CreateReport {
        prompt: id,
        reason,
        details: Some("Posted three times by bob".into()),
    };
    bob.report(&report(ReportReason::Other)).await.unwrap();
    bob.report(&report(ReportReason::Spam)).await.unwrap();
    let reports = admin.admin_reports(false).await.unwrap();
    // Discarding a report tells nobody anything.
    admin
        .resolve_report(reports[0].id, Resolution::Discard)
        .await
        .unwrap();
    assert!(alice.notices(None).await.unwrap().is_empty());
    admin
        .resolve_report(reports[1].id, Resolution::Delete)
        .await
        .unwrap();

    let notices = alice.notices(None).await.unwrap();
    assert_eq!(notices.len(), 1);
    let n: &AccountNotice = &notices[0];
    assert_eq!(n.kind, NoticeKind::PromptRemoved);
    assert_eq!(n.fields.reason, Some(ReportReason::Spam));
    assert_eq!(n.prompts[0].title, "Lamps");
    assert_eq!(n.prompts[0].handle, "alice_pen");
    assert!(!n.read);
    // Nothing identifies the reporter, the report's text or the administrator.
    let json = serde_json::to_string(&notices).unwrap();
    for secret in ["bob", "boss", "three times"] {
        assert!(!json.contains(secret), "{secret} in {json}");
    }
    assert!(bob.notices(None).await.unwrap().is_empty());
    assert!(admin.notices(None).await.unwrap().is_empty());

    // Read and unread.
    assert_eq!(alice.account().await.unwrap().unread_notices, 1);
    alice.mark_notices_read().await.unwrap();
    assert_eq!(alice.account().await.unwrap().unread_notices, 0);
    assert!(alice.notices(None).await.unwrap()[0].read);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn tag_actions_tell_each_affected_account_about_its_own_prompts() {
    let server = server!();
    let (alice, ap) = writer(&server, "alice").await;
    let (bob, bp) = writer(&server, "bob").await;
    let (carol, cp) = writer(&server, "carol").await;
    let admin = server.admin_account("boss").await.unwrap();
    admin
        .admin_update_settings(&UpdateServerSettings {
            new_tags: Some(TagPolicy::Approval),
        })
        .await
        .unwrap();
    post(&alice, ap, "A1", &["theme:Selkies", "genre:horror"]).await;
    post(&bob, bp, "B1", &["theme:selkies"]).await;
    post(&bob, bp, "B2", &["genre:horror"]).await;
    post(&carol, cp, "C1", &["genre:mystery"]).await;

    // Approve, rename, recolour (no notice), hide, show.
    admin
        .admin_update_tag(
            "theme",
            "selkies",
            &UpdateTag {
                status: Some(TagStatus::Visible),
                ..UpdateTag::default()
            },
        )
        .await
        .unwrap();
    admin
        .admin_update_tag(
            "genre",
            "horror",
            &UpdateTag {
                display: Some("Horrors".into()),
                ..UpdateTag::default()
            },
        )
        .await
        .unwrap();
    admin
        .admin_update_tag(
            "genre",
            "horror",
            &UpdateTag {
                color: Some(ChipColor {
                    background: "#000000".into(),
                    text: None,
                }),
                ..UpdateTag::default()
            },
        )
        .await
        .unwrap();
    for status in [TagStatus::Hidden, TagStatus::Visible] {
        admin
            .admin_update_tag(
                "genre",
                "horror",
                &UpdateTag {
                    status: Some(status),
                    ..UpdateTag::default()
                },
            )
            .await
            .unwrap();
    }

    // Merge one request, reject another, delete a tag.
    post(&alice, ap, "A2", &["theme:Selchies"]).await;
    admin
        .admin_update_tag(
            "theme",
            "selchies",
            &UpdateTag {
                alias_of: Some("theme:selkies".into()),
                ..UpdateTag::default()
            },
        )
        .await
        .unwrap();
    post(&bob, bp, "B3", &["theme:Junk"]).await;
    admin.admin_delete_tag("theme", "junk").await.unwrap();
    admin.admin_delete_tag("genre", "horror").await.unwrap();

    let s = |titles: &[&str]| titles.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        summary(&alice).await,
        vec![
            (NoticeKind::TagDeleted, s(&["A1"])),
            (NoticeKind::TagMerged, s(&["A2"])),
            (NoticeKind::TagShown, s(&["A1"])),
            (NoticeKind::TagHidden, s(&["A1"])),
            (NoticeKind::TagRenamed, s(&["A1"])),
            (NoticeKind::TagApproved, s(&["A1"])),
        ]
    );
    assert_eq!(
        summary(&bob).await,
        vec![
            (NoticeKind::TagDeleted, s(&["B2"])),
            (NoticeKind::TagRejected, s(&["B3"])),
            (NoticeKind::TagShown, s(&["B2"])),
            (NoticeKind::TagHidden, s(&["B2"])),
            (NoticeKind::TagRenamed, s(&["B2"])),
            (NoticeKind::TagApproved, s(&["B1"])),
        ]
    );
    assert!(carol.notices(None).await.unwrap().is_empty());

    let alice_notices = alice.notices(None).await.unwrap();
    let merged = &alice_notices[1].fields;
    assert_eq!(merged.tag.as_deref(), Some("theme:selchies"));
    assert_eq!(merged.into.as_deref(), Some("theme:selkies"));
    let renamed = &alice_notices[4].fields;
    assert_eq!(
        (renamed.from.as_deref(), renamed.to.as_deref()),
        (Some("Horror"), Some("Horrors"))
    );
    assert_eq!(alice.account().await.unwrap().unread_notices, 6);

    // Every notice points at the admin-log entry for its action.
    let pool = server.db().await.unwrap();
    let mismatched: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM account_notices n JOIN admin_log l ON l.id = n.log_id
         WHERE NOT (
             (n.kind = 'tag_approved' AND l.action = 'approve_tag') OR
             (n.kind = 'tag_renamed'  AND l.action = 'rename_tag')  OR
             (n.kind = 'tag_hidden'   AND l.action = 'hide_tag')    OR
             (n.kind = 'tag_shown'    AND l.action = 'show_tag')    OR
             (n.kind = 'tag_merged'   AND l.action = 'alias_tag')   OR
             (n.kind = 'tag_rejected' AND l.action = 'reject_tag')  OR
             (n.kind = 'tag_deleted'  AND l.action = 'delete_tag'))",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(mismatched, 0);
    pool.close().await;
    server.stop().await.unwrap();
}
