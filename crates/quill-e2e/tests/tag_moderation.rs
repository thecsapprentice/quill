//! Phase 1e end-to-end tests: the new-tags switch, tag requests and hidden
//! tags, checked from every viewer's side: anonymous visitors, the author,
//! another writer and an administrator, on prompts and through filters.

// Shared helpers below are test code too; panicking on failure is intended.
#![allow(clippy::unwrap_used)]

use quill_core::{Client, CoreError};
use quill_e2e::TestServer;
use quill_proto::{
    api::CreatePersona,
    board::{
        AdminAction, AdminTagQuery, CreatePrompt, Maturity, PromptQuery, TagPolicy, TagQuery,
        TagStatus, UpdatePrompt, UpdateServerSettings, UpdateTag,
    },
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

async fn post(client: &Client, persona: Uuid, title: &str, tags: &[&str]) -> Uuid {
    client
        .create_prompt(&CreatePrompt {
            persona,
            title: title.into(),
            body: format!("A prompt about {title}."),
            maturity: Maturity::General,
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
            state: None,
        })
        .await
        .unwrap()
        .id
}

/// The tag keys `viewer` sees on a prompt, with any non-visible status.
async fn seen(viewer: &Client, id: Uuid) -> Vec<String> {
    viewer
        .prompt(id)
        .await
        .unwrap()
        .tags
        .iter()
        .map(|t| match t.status {
            TagStatus::Visible => t.key(),
            other => format!("{} ({other})", t.key()),
        })
        .collect()
}

/// Titles of prompts matching an include and/or exclude filter, sorted.
async fn filtered(viewer: &Client, include: Option<&str>, exclude: Option<&str>) -> Vec<String> {
    let mut titles: Vec<String> = viewer
        .prompts(&PromptQuery {
            include: include.map(str::to_owned),
            exclude: exclude.map(str::to_owned),
            ..PromptQuery::default()
        })
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|p| p.title)
        .collect();
    titles.sort();
    titles
}

async fn set_new_tags(admin: &Client, policy: TagPolicy) {
    let s = admin
        .admin_update_settings(&UpdateServerSettings {
            new_tags: Some(policy),
        })
        .await
        .unwrap();
    assert_eq!(s.new_tags, policy);
}

fn set_status(status: TagStatus) -> UpdateTag {
    UpdateTag {
        status: Some(status),
        ..UpdateTag::default()
    }
}

#[tokio::test]
async fn the_new_tags_switch() {
    let server = server!();
    let (alice, ap) = writer(&server, "alice").await;
    let admin = server.admin_account("moderator").await.unwrap();

    assert_eq!(
        admin.admin_settings().await.unwrap().new_tags,
        TagPolicy::Open
    );
    assert_eq!(status(&alice.admin_settings().await.unwrap_err()), 403);
    assert_eq!(
        status(
            &alice
                .admin_update_settings(&UpdateServerSettings {
                    new_tags: Some(TagPolicy::Curated)
                })
                .await
                .unwrap_err()
        ),
        403
    );

    // Not allowed: existing tags only.
    set_new_tags(&admin, TagPolicy::Curated).await;
    assert_eq!(
        server.client().instance().await.unwrap().tag_policy,
        TagPolicy::Curated
    );
    let refused = alice
        .create_prompt(&CreatePrompt {
            persona: ap,
            title: "Seals".into(),
            body: "Body".into(),
            maturity: Maturity::General,
            tags: vec!["theme:Selkies".into()],
            state: None,
        })
        .await
        .unwrap_err();
    assert_eq!(status(&refused), 400);
    assert!(
        refused
            .to_string()
            .contains("this server doesn't accept new tags")
    );
    post(&alice, ap, "Old tags", &["genre:horror"]).await;

    // Allowed again: created, visible at once.
    set_new_tags(&admin, TagPolicy::Open).await;
    let id = post(&alice, ap, "Seals", &["theme:Selkies"]).await;
    assert_eq!(seen(&server.client(), id).await, vec!["theme:selkies"]);
    // Setting the same value again logs nothing.
    set_new_tags(&admin, TagPolicy::Open).await;

    let log = admin.admin_log(None, None).await.unwrap();
    assert_eq!(log.len(), 2);
    assert!(log.iter().all(|e| e.action == AdminAction::ChangeSetting));
    assert_eq!(
        log[0].detail.as_deref(),
        Some("new tags from writers: not allowed → allowed")
    );
    assert_eq!(
        log[1].detail.as_deref(),
        Some("new tags from writers: allowed → not allowed")
    );
    server.stop().await.unwrap();
}

#[tokio::test]
async fn tag_requests_are_seen_only_by_authors_until_approved() {
    let server = server!();
    let (alice, ap) = writer(&server, "alice").await;
    let (bob, bp) = writer(&server, "bob").await;
    let admin = server.admin_account("moderator").await.unwrap();
    let anon = server.client();
    set_new_tags(&admin, TagPolicy::Approval).await;

    let a1 = post(&alice, ap, "A1", &["theme:Selkies", "genre:horror"]).await;
    // Bob uses the same new tag: it joins the same request.
    let b1 = post(&bob, bp, "B1", &["theme:selkies"]).await;

    // Who sees the request on Alice's prompt.
    assert_eq!(seen(&anon, a1).await, vec!["genre:horror"]);
    assert_eq!(seen(&bob, a1).await, vec!["genre:horror"]);
    assert_eq!(
        seen(&alice, a1).await,
        vec!["theme:selkies (pending)", "genre:horror"]
    );
    assert_eq!(
        seen(&admin, a1).await,
        vec!["theme:selkies (pending)", "genre:horror"]
    );
    assert_eq!(seen(&bob, b1).await, vec!["theme:selkies (pending)"]);

    // Filtering by it acts as if it didn't exist, except for administrators.
    for viewer in [&anon, &alice, &bob] {
        assert!(
            filtered(viewer, Some("theme:selkies"), None)
                .await
                .is_empty()
        );
        assert_eq!(
            filtered(viewer, None, Some("theme:selkies")).await,
            vec!["A1", "B1"]
        );
    }
    assert_eq!(
        filtered(&admin, Some("theme:selkies"), None).await,
        vec!["A1", "B1"]
    );
    let themes = anon
        .tags(&TagQuery {
            namespace: Some("theme".into()),
            ..TagQuery::default()
        })
        .await
        .unwrap();
    assert!(themes.is_empty());

    // The request list.
    let pending = |admin: &Client| {
        let admin = admin.clone();
        async move {
            admin
                .admin_tags(&AdminTagQuery {
                    status: Some(TagStatus::Pending),
                    ..AdminTagQuery::default()
                })
                .await
                .unwrap()
        }
    };
    let requests = pending(&admin).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].tag.key(), "theme:selkies");
    assert_eq!(requests[0].uses, 2);

    // Approve: everyone sees it and can filter by it.
    assert_eq!(
        status(
            &admin
                .admin_update_tag("theme", "selkies", &set_status(TagStatus::Pending))
                .await
                .unwrap_err()
        ),
        400
    );
    admin
        .admin_update_tag("theme", "selkies", &set_status(TagStatus::Visible))
        .await
        .unwrap();
    assert_eq!(seen(&anon, a1).await, vec!["theme:selkies", "genre:horror"]);
    assert_eq!(
        filtered(&anon, Some("theme:selkies"), None).await,
        vec!["A1", "B1"]
    );

    // Merge: the request's prompts move to the existing tag.
    let a2 = post(&alice, ap, "A2", &["theme:Selchies"]).await;
    assert_eq!(seen(&anon, a2).await, Vec::<String>::new());
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
    assert_eq!(seen(&anon, a2).await, vec!["theme:selkies"]);
    assert!(pending(&admin).await.is_empty());
    // Using the merged name now just gets the existing tag.
    let a3 = post(&alice, ap, "A3", &["theme:selchies"]).await;
    assert_eq!(seen(&anon, a3).await, vec!["theme:selkies"]);

    // Reject: deleted, and gone from its prompts.
    let a4 = post(&alice, ap, "A4", &["theme:Junk", "genre:horror"]).await;
    assert_eq!(
        seen(&alice, a4).await,
        vec!["theme:junk (pending)", "genre:horror"]
    );
    let rejected = admin.admin_delete_tag("theme", "junk").await.unwrap();
    assert_eq!(rejected.prompts_updated, 1);
    assert_eq!(seen(&alice, a4).await, vec!["genre:horror"]);

    let log = admin.admin_log(None, None).await.unwrap();
    let actions: Vec<AdminAction> = log.iter().map(|e| e.action).collect();
    assert_eq!(
        actions,
        vec![
            AdminAction::RejectTag,
            AdminAction::AliasTag,
            AdminAction::ApproveTag,
            AdminAction::ChangeSetting
        ]
    );
    assert!(
        log[1]
            .detail
            .as_deref()
            .unwrap()
            .starts_with("request merged; now an alias of theme:selkies")
    );
    server.stop().await.unwrap();
}

#[tokio::test]
async fn hidden_tags_are_for_administrators_only() {
    let server = server!();
    let (alice, ap) = writer(&server, "alice").await;
    let (bob, bp) = writer(&server, "bob").await;
    let admin = server.admin_account("moderator").await.unwrap();
    let anon = server.client();

    let id = post(&alice, ap, "Lamps", &["genre:horror", "format:prose"]).await;
    post(&bob, bp, "Other", &["format:prose"]).await;
    assert_eq!(
        status(
            &alice
                .admin_update_tag("genre", "horror", &set_status(TagStatus::Hidden))
                .await
                .unwrap_err()
        ),
        403
    );
    admin
        .admin_update_tag("genre", "horror", &set_status(TagStatus::Hidden))
        .await
        .unwrap();

    // Nobody but administrators sees it, not even the author.
    for viewer in [&anon, &alice, &bob] {
        assert_eq!(seen(viewer, id).await, vec!["format:prose"]);
        assert!(
            filtered(viewer, Some("genre:horror"), None)
                .await
                .is_empty()
        );
        assert_eq!(
            filtered(viewer, None, Some("genre:horror")).await,
            vec!["Lamps", "Other"]
        );
    }
    assert_eq!(
        seen(&admin, id).await,
        vec!["genre:horror (hidden)", "format:prose"]
    );
    assert_eq!(
        filtered(&admin, Some("genre:horror"), None).await,
        vec!["Lamps"]
    );
    let genres = anon
        .tags(&TagQuery {
            namespace: Some("genre".into()),
            ..TagQuery::default()
        })
        .await
        .unwrap();
    assert!(genres.iter().all(|t| t.slug != "horror"));

    // The author edits the tags they can see; the hidden one stays.
    alice
        .update_prompt(
            id,
            &UpdatePrompt {
                tags: Some(vec!["format:poetry".into()]),
                ..UpdatePrompt::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(seen(&alice, id).await, vec!["format:poetry"]);
    assert_eq!(
        seen(&admin, id).await,
        vec!["format:poetry", "genre:horror (hidden)"]
    );
    // An administrator editing can remove it, since they can see it.
    let (_, admin_persona) = (
        (),
        admin
            .create_persona(&CreatePersona {
                handle: "night_desk".into(),
                display_name: None,
                summary: None,
            })
            .await
            .unwrap()
            .id,
    );
    let own = post(&admin, admin_persona, "Admin's", &["genre:horror"]).await;
    admin
        .update_prompt(
            own,
            &UpdatePrompt {
                tags: Some(vec!["format:prose".into()]),
                ..UpdatePrompt::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(seen(&admin, own).await, vec!["format:prose"]);

    // Shown again: back for everyone.
    admin
        .admin_update_tag("genre", "horror", &set_status(TagStatus::Visible))
        .await
        .unwrap();
    assert_eq!(seen(&anon, id).await, vec!["format:poetry", "genre:horror"]);

    let log = admin.admin_log(None, None).await.unwrap();
    assert_eq!(log[0].action, AdminAction::ShowTag);
    assert_eq!(log[1].action, AdminAction::HideTag);
    assert_eq!(log[1].subject.as_deref(), Some("genre:horror"));
    server.stop().await.unwrap();
}
