//! Phase 1 end-to-end tests: prompts, tags, search, maturity, reports and the
//! admin inbox.

// Shared helpers below are test code too; panicking on failure is intended.
#![allow(clippy::unwrap_used)]

use quill_core::{Client, CoreError};
use quill_e2e::TestServer;
use quill_proto::{
    api::CreatePersona,
    board::{
        CreateNamespace, CreatePrompt, CreateReport, CreateTag, Maturity, Prompt, PromptQuery,
        PromptState, ReportReason, Resolution, TagOrigin, TagPolicy, TagQuery,
        UpdateAccountSettings, UpdatePrompt, UpdateTag,
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
    ($tweak:expr) => {
        match TestServer::start_with($tweak).await.unwrap() {
            Some(s) => s,
            None => return,
        }
    };
}

fn status(err: &CoreError) -> u16 {
    err.status().unwrap_or_default()
}

/// Registers an account with one persona; returns its client and persona id.
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

fn new_prompt(persona: Uuid, title: &str, maturity: Maturity, tags: &[&str]) -> CreatePrompt {
    CreatePrompt {
        persona,
        title: title.into(),
        body: format!("A prompt about {title}."),
        maturity,
        tags: tags.iter().map(|t| (*t).to_owned()).collect(),
        state: None,
    }
}

async fn post(client: &Client, p: CreatePrompt) -> Prompt {
    client.create_prompt(&p).await.unwrap()
}

fn titles(items: &[Prompt]) -> Vec<String> {
    let mut t: Vec<String> = items.iter().map(|p| p.title.clone()).collect();
    t.sort();
    t
}

async fn search(client: &Client, q: PromptQuery) -> Vec<String> {
    titles(&client.prompts(&q).await.unwrap().items)
}

#[tokio::test]
async fn instance_and_account_defaults() {
    let server = server!();
    let info = server.client().instance().await.unwrap();
    assert_eq!(info.tag_policy, TagPolicy::Open);
    assert_eq!(info.default_maturity_max, Maturity::Teen);

    let client = server.account("alice").await.unwrap();
    let settings = client.account().await.unwrap();
    assert_eq!(settings.maturity_max, Maturity::Teen);
    assert!(!settings.is_admin);
    let raised = client
        .update_account(&UpdateAccountSettings {
            maturity_max: Some(Maturity::Explicit),
            staff_name: None,
        })
        .await
        .unwrap();
    assert_eq!(raised.maturity_max, Maturity::Explicit);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn prompt_lifecycle_and_markdown_safety() {
    let server = server!();
    let (alice, persona) = writer(&server, "alice").await;

    let created = alice
        .create_prompt(&CreatePrompt {
            persona,
            title: "  The lighthouse keeper's ledger ".into(),
            body: "A keeper finds *entries* in her own hand.\r\n\r\n<script>alert(1)</script>\n\n![map](https://img.example/m.png)".into(),
            maturity: Maturity::Teen,
            tags: vec![
                "genre:Horror".into(),
                "theme:Lighthouses".into(),
                "genre:horror".into(),
            ],
            state: None,
        })
        .await
        .unwrap();
    assert_eq!(created.title, "The lighthouse keeper's ledger");
    assert_eq!(created.state, PromptState::Open);
    assert!(!created.accepting_sessions);
    assert_eq!(created.author.handle, "alice_p");
    assert!(!created.body.contains('\r'));
    assert!(created.body_html.contains("<em>entries</em>"));
    assert!(!created.body_html.contains("<script"));
    assert!(!created.body_html.contains("<img"));
    let keys: Vec<String> = created.tags.iter().map(|t| t.key()).collect();
    assert_eq!(keys, vec!["genre:horror", "theme:lighthouses"]);
    assert_eq!(created.tags[0].origin, TagOrigin::Server);
    assert_eq!(created.tags[1].origin, TagOrigin::User);
    assert_eq!(created.tags[1].display, "Lighthouses");

    // Anyone can read it.
    let anon = server.client().prompt(created.id).await.unwrap();
    assert_eq!(anon, created);

    let edited = alice
        .update_prompt(
            created.id,
            &UpdatePrompt {
                title: Some("The ledger".into()),
                state: Some(PromptState::Busy),
                tags: Some(vec!["genre:mystery".into()]),
                ..UpdatePrompt::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(edited.title, "The ledger");
    assert_eq!(edited.state, PromptState::Busy);
    assert_eq!(edited.body, created.body);
    assert_eq!(edited.tags.len(), 1);
    assert!(edited.updated_at > created.updated_at);

    // Only the author may edit or delete.
    let (bob, _) = writer(&server, "bob").await;
    let err = bob
        .update_prompt(created.id, &UpdatePrompt::default())
        .await
        .unwrap_err();
    assert_eq!(status(&err), 404);
    assert_eq!(
        status(&bob.delete_prompt(created.id).await.unwrap_err()),
        404
    );

    alice.delete_prompt(created.id).await.unwrap();
    let gone = server.client().prompt(created.id).await.unwrap_err();
    assert_eq!(status(&gone), 404);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn prompt_validation() {
    let server = server!();
    let (alice, persona) = writer(&server, "alice").await;
    let (_, bobs_persona) = writer(&server, "bob").await;

    let mut p = new_prompt(persona, "x", Maturity::General, &[]);
    p.title = "t".repeat(201);
    assert_eq!(status(&alice.create_prompt(&p).await.unwrap_err()), 400);

    let mut p = new_prompt(persona, "x", Maturity::General, &[]);
    p.body = "   ".into();
    assert_eq!(status(&alice.create_prompt(&p).await.unwrap_err()), 400);

    let many: Vec<String> = (0..16).map(|i| format!("theme:t{i}")).collect();
    let mut p = new_prompt(persona, "x", Maturity::General, &[]);
    p.tags = many;
    assert_eq!(status(&alice.create_prompt(&p).await.unwrap_err()), 400);

    // Non-Latin slugs and lookalike letters are refused.
    for tag in ["theme:怪談", "theme:\u{0430}pple", "nocolon"] {
        let p = new_prompt(persona, "x", Maturity::General, &[tag]);
        assert_eq!(
            status(&alice.create_prompt(&p).await.unwrap_err()),
            400,
            "{tag}"
        );
    }

    // Posting as someone else's persona.
    let p = new_prompt(bobs_persona, "x", Maturity::General, &[]);
    assert_eq!(status(&alice.create_prompt(&p).await.unwrap_err()), 400);

    // Anonymous posting.
    let p = new_prompt(persona, "x", Maturity::General, &[]);
    assert!(matches!(
        server.client().create_prompt(&p).await,
        Err(CoreError::NotLoggedIn)
    ));
    server.stop().await.unwrap();
}

#[tokio::test]
async fn tag_policy_and_locked_namespaces() {
    let server = server!();
    let (alice, persona) = writer(&server, "alice").await;

    // Locked namespace: only curated tags.
    let p = new_prompt(
        persona,
        "x",
        Maturity::General,
        &["content-warning:Spiders"],
    );
    assert_eq!(status(&alice.create_prompt(&p).await.unwrap_err()), 400);
    let p = new_prompt(persona, "x", Maturity::General, &["content-warning:gore"]);
    alice.create_prompt(&p).await.unwrap();

    // Unknown namespace.
    let p = new_prompt(persona, "x", Maturity::General, &["vibes:cozy"]);
    assert_eq!(status(&alice.create_prompt(&p).await.unwrap_err()), 400);

    // Creating a tag ahead of time, then again (idempotent).
    let tag = alice
        .create_tag(&CreateTag {
            namespace: "theme".into(),
            display: "Found Family".into(),
        })
        .await
        .unwrap();
    assert_eq!(tag.key(), "theme:found-family");
    assert_eq!(tag.origin, TagOrigin::User);
    alice
        .create_tag(&CreateTag {
            namespace: "theme".into(),
            display: "found family".into(),
        })
        .await
        .unwrap();

    let namespaces = server.client().tag_namespaces().await.unwrap();
    let cw = namespaces
        .iter()
        .find(|n| n.name == "content-warning")
        .unwrap();
    assert!(cw.locked);

    let found = server
        .client()
        .tags(&TagQuery {
            namespace: Some("theme".into()),
            prefix: Some("Fou".into()),
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    let genres = server
        .client()
        .tags(&TagQuery {
            namespace: Some("genre".into()),
            ..TagQuery::default()
        })
        .await
        .unwrap();
    assert!(genres.iter().all(|t| t.origin == TagOrigin::Server));
    assert!(genres.len() >= 10);
    server.stop().await.unwrap();

    // A curated-only server refuses user tags everywhere.
    let server = server!(|c| c.tag_policy = TagPolicy::Curated);
    let (alice, persona) = writer(&server, "alice").await;
    let p = new_prompt(persona, "x", Maturity::General, &["theme:New Thing"]);
    assert_eq!(status(&alice.create_prompt(&p).await.unwrap_err()), 400);
    let p = new_prompt(persona, "x", Maturity::General, &["genre:fantasy"]);
    alice.create_prompt(&p).await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn search_filters_and_maturity() {
    let server = server!();
    let (alice, ap) = writer(&server, "alice").await;
    let (bob, bp) = writer(&server, "bob").await;

    post(
        &alice,
        new_prompt(
            ap,
            "Haunted lighthouse",
            Maturity::General,
            &["genre:horror", "theme:sea"],
        ),
    )
    .await;
    post(
        &alice,
        new_prompt(
            ap,
            "Gory harbour",
            Maturity::Teen,
            &["genre:horror", "content-warning:gore"],
        ),
    )
    .await;
    post(
        &bob,
        new_prompt(bp, "Dragon market", Maturity::General, &["genre:fantasy"]),
    )
    .await;
    post(
        &bob,
        new_prompt(bp, "Midnight feast", Maturity::Mature, &["genre:fantasy"]),
    )
    .await;
    post(
        &bob,
        new_prompt(bp, "After hours", Maturity::Explicit, &["genre:romance"]),
    )
    .await;
    let mut closed = new_prompt(bp, "Closed door", Maturity::General, &["genre:mystery"]);
    closed.state = Some(PromptState::Closed);
    post(&bob, closed).await;

    let anon = server.client();
    // Anonymous visitors see up to Teen.
    assert_eq!(
        search(&anon, PromptQuery::default()).await,
        vec![
            "Closed door",
            "Dragon market",
            "Gory harbour",
            "Haunted lighthouse"
        ]
    );
    // Include requires every tag; exclude removes any.
    assert_eq!(
        search(
            &anon,
            PromptQuery {
                include: Some("genre:horror".into()),
                ..Default::default()
            }
        )
        .await,
        vec!["Gory harbour", "Haunted lighthouse"]
    );
    assert_eq!(
        search(
            &anon,
            PromptQuery {
                include: Some("genre:horror".into()),
                exclude: Some("content-warning:gore".into()),
                ..Default::default()
            }
        )
        .await,
        vec!["Haunted lighthouse"]
    );
    assert_eq!(
        search(
            &anon,
            PromptQuery {
                include: Some("genre:horror,theme:sea".into()),
                ..Default::default()
            }
        )
        .await,
        vec!["Haunted lighthouse"]
    );
    // An unknown required tag matches nothing; an unknown excluded tag is ignored.
    assert!(
        search(
            &anon,
            PromptQuery {
                include: Some("theme:nothing".into()),
                ..Default::default()
            }
        )
        .await
        .is_empty()
    );
    assert_eq!(
        search(
            &anon,
            PromptQuery {
                exclude: Some("theme:nothing".into()),
                ..Default::default()
            }
        )
        .await
        .len(),
        4
    );
    // Full-text search, author and state filters.
    assert_eq!(
        search(
            &anon,
            PromptQuery {
                q: Some("lighthouse".into()),
                ..Default::default()
            }
        )
        .await,
        vec!["Haunted lighthouse"]
    );
    assert_eq!(
        search(
            &anon,
            PromptQuery {
                author: Some("bob_p".into()),
                ..Default::default()
            }
        )
        .await,
        vec!["Closed door", "Dragon market"]
    );
    assert_eq!(
        search(
            &anon,
            PromptQuery {
                state: Some(PromptState::Closed),
                ..Default::default()
            }
        )
        .await,
        vec!["Closed door"]
    );

    // Raising the account cap reveals more; the query can only lower it.
    bob.update_account(&UpdateAccountSettings {
        maturity_max: Some(Maturity::Explicit),
        staff_name: None,
    })
    .await
    .unwrap();
    assert_eq!(search(&bob, PromptQuery::default()).await.len(), 6);
    assert_eq!(
        search(
            &bob,
            PromptQuery {
                maturity_max: Some(Maturity::General),
                ..Default::default()
            }
        )
        .await,
        vec!["Closed door", "Dragon market", "Haunted lighthouse"]
    );
    // A query can't exceed the account cap.
    assert_eq!(
        search(
            &anon,
            PromptQuery {
                maturity_max: Some(Maturity::Explicit),
                ..Default::default()
            }
        )
        .await
        .len(),
        4
    );

    // "me" shows all of your own prompts regardless of the cap.
    assert_eq!(
        search(
            &alice,
            PromptQuery {
                author: Some("me".into()),
                ..Default::default()
            }
        )
        .await,
        vec!["Gory harbour", "Haunted lighthouse"]
    );
    let err = anon
        .prompts(&PromptQuery {
            author: Some("me".into()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(status(&err), 401);
    // Direct links ignore the cap; clients warn instead.
    let explicit = bob
        .prompts(&PromptQuery {
            q: Some("hours".into()),
            ..Default::default()
        })
        .await
        .unwrap()
        .items
        .remove(0);
    assert_eq!(
        anon.prompt(explicit.id).await.unwrap().maturity,
        Maturity::Explicit
    );
    server.stop().await.unwrap();
}

#[tokio::test]
async fn pagination_is_stable_and_complete() {
    let server = server!();
    let (alice, ap) = writer(&server, "alice").await;
    for i in 0..5 {
        post(
            &alice,
            new_prompt(ap, &format!("Prompt {i}"), Maturity::General, &[]),
        )
        .await;
    }
    let client = server.client();
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = client
            .prompts(&PromptQuery {
                limit: Some(2),
                cursor: cursor.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(page.items.len() <= 2);
        seen.extend(page.items.into_iter().map(|p| p.title));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    // Newest first, no duplicates or gaps.
    assert_eq!(
        seen,
        vec!["Prompt 4", "Prompt 3", "Prompt 2", "Prompt 1", "Prompt 0"]
    );
    let bad = client
        .prompts(&PromptQuery {
            cursor: Some("garbage".into()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(status(&bad), 400);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn reports_and_admin_actions() {
    let server = server!();
    let (alice, ap) = writer(&server, "alice").await;
    let admin = server.admin_account("moderator").await.unwrap();
    let keep = post(
        &alice,
        new_prompt(ap, "Fine prompt", Maturity::General, &[]),
    )
    .await;
    let bad = post(
        &alice,
        new_prompt(ap, "Spammy prompt", Maturity::General, &[]),
    )
    .await;

    // Anonymous and signed-in reports both land in the inbox.
    server
        .client()
        .report(&CreateReport {
            prompt: bad.id,
            reason: ReportReason::Spam,
            details: Some("Posted ten times".into()),
        })
        .await
        .unwrap();
    let (carol, _) = writer(&server, "carol").await;
    carol
        .report(&CreateReport {
            prompt: keep.id,
            reason: ReportReason::WrongMaturity,
            details: None,
        })
        .await
        .unwrap();
    let missing = carol
        .report(&CreateReport {
            prompt: Uuid::now_v7(),
            reason: ReportReason::Other,
            details: None,
        })
        .await
        .unwrap_err();
    assert_eq!(status(&missing), 404);

    // Only administrators see the inbox.
    assert_eq!(status(&carol.admin_reports(false).await.unwrap_err()), 403);
    assert!(carol.account().await.map(|s| !s.is_admin).unwrap());
    assert!(admin.account().await.unwrap().is_admin);

    let inbox = admin.admin_reports(false).await.unwrap();
    assert_eq!(inbox.len(), 2);
    let spam = inbox
        .iter()
        .find(|r| r.reason == ReportReason::Spam)
        .unwrap();
    assert_eq!(spam.details.as_deref(), Some("Posted ten times"));
    assert_eq!(spam.prompt.as_ref().unwrap().id, bad.id);
    assert_eq!(spam.prompt_hash.len(), 64);
    let wrong = inbox
        .iter()
        .find(|r| r.reason == ReportReason::WrongMaturity)
        .unwrap();

    // Discard one, delete the other.
    let discarded = admin
        .resolve_report(wrong.id, Resolution::Discard)
        .await
        .unwrap();
    assert_eq!(discarded.resolution, Some(Resolution::Discard));
    let deleted = admin
        .resolve_report(spam.id, Resolution::Delete)
        .await
        .unwrap();
    assert_eq!(deleted.resolution, Some(Resolution::Delete));
    let again = admin
        .resolve_report(spam.id, Resolution::Discard)
        .await
        .unwrap_err();
    assert_eq!(status(&again), 409);
    assert!(admin.admin_reports(false).await.unwrap().is_empty());
    assert_eq!(admin.admin_reports(true).await.unwrap().len(), 2);

    // The removed prompt is gone for everyone but its author.
    assert_eq!(
        status(&server.client().prompt(bad.id).await.unwrap_err()),
        404
    );
    assert_eq!(status(&carol.prompt(bad.id).await.unwrap_err()), 404);
    assert_eq!(
        search(&server.client(), PromptQuery::default()).await,
        vec!["Fine prompt"]
    );
    let own = alice.prompt(bad.id).await.unwrap();
    assert!(own.removed);
    let mine = alice
        .prompts(&PromptQuery {
            author: Some("me".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(mine.items.iter().filter(|p| p.removed).count(), 1);
    let edit = alice
        .update_prompt(
            bad.id,
            &UpdatePrompt {
                title: Some("Fixed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(status(&edit), 403);
    // A removed prompt can't be reported again.
    let rereport = carol
        .report(&CreateReport {
            prompt: bad.id,
            reason: ReportReason::Spam,
            details: None,
        })
        .await
        .unwrap_err();
    assert_eq!(status(&rereport), 404);

    // The log records every report and action, newest first, without text.
    let log = admin.admin_log(None, None).await.unwrap();
    let actions: Vec<&str> = log.iter().map(|e| e.action.as_str()).collect();
    assert_eq!(
        actions,
        vec![
            "delete_prompt",
            "discard",
            "report_received",
            "report_received"
        ]
    );
    assert_eq!(log[0].prompt_hash.as_ref(), Some(&spam.prompt_hash));
    assert!(log[0].admin.is_some());
    assert!(log[3].admin.is_none());
    let older = admin.admin_log(Some(log[1].id), Some(1)).await.unwrap();
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].id, log[2].id);

    // The log is append-only, even for the database owner.
    let db = server.db().await.unwrap();
    assert!(
        sqlx::query("UPDATE admin_log SET action = 'discard'")
            .execute(&db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM admin_log")
            .execute(&db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("TRUNCATE admin_log")
            .execute(&db)
            .await
            .is_err()
    );
    db.close().await;
    server.stop().await.unwrap();
}

#[tokio::test]
async fn reports_are_rate_limited() {
    let server = server!();
    let (alice, ap) = writer(&server, "alice").await;
    let p = post(&alice, new_prompt(ap, "Target", Maturity::General, &[])).await;
    let report = CreateReport {
        prompt: p.id,
        reason: ReportReason::Other,
        details: None,
    };
    for _ in 0..20 {
        alice.report(&report).await.unwrap();
    }
    assert_eq!(status(&alice.report(&report).await.unwrap_err()), 429);
    // Other reporters are unaffected.
    server.client().report(&report).await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn admin_tag_management_and_aliases() {
    let server = server!();
    let (alice, ap) = writer(&server, "alice").await;
    let admin = server.admin_account("moderator").await.unwrap();

    // Non-admins can't manage tags.
    let ns = CreateNamespace {
        name: "setting".into(),
        display: "Setting".into(),
        locked: true,
        required: false,
        color: None,
    };
    assert_eq!(
        status(&alice.admin_create_namespace(&ns).await.unwrap_err()),
        403
    );
    admin.admin_create_namespace(&ns).await.unwrap();
    assert_eq!(
        status(&admin.admin_create_namespace(&ns).await.unwrap_err()),
        409
    );

    let space = admin
        .admin_create_tag(&CreateTag {
            namespace: "setting".into(),
            display: "Space".into(),
        })
        .await
        .unwrap();
    assert_eq!(space.origin, TagOrigin::Server);
    post(
        &alice,
        new_prompt(ap, "Station", Maturity::General, &["setting:space"]),
    )
    .await;

    // Promote a user tag to curated.
    alice
        .create_tag(&CreateTag {
            namespace: "theme".into(),
            display: "Sea".into(),
        })
        .await
        .unwrap();
    let promoted = admin
        .admin_create_tag(&CreateTag {
            namespace: "theme".into(),
            display: "Sea".into(),
        })
        .await
        .unwrap();
    assert_eq!(promoted.origin, TagOrigin::Server);

    // Merge theme:ocean into theme:sea.
    post(
        &alice,
        new_prompt(
            ap,
            "Tides",
            Maturity::General,
            &["theme:Ocean", "genre:fantasy"],
        ),
    )
    .await;
    post(
        &alice,
        new_prompt(ap, "Both", Maturity::General, &["theme:ocean", "theme:sea"]),
    )
    .await;
    let renamed = admin
        .admin_update_tag(
            "theme",
            "ocean",
            &UpdateTag {
                alias_of: Some("theme:sea".into()),
                display: Some("Ocean (old)".into()),
                ..UpdateTag::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(renamed.display, "Ocean (old)");

    let anon = server.client();
    // Filtering by either name finds the same prompts.
    for tag in ["theme:sea", "theme:ocean"] {
        assert_eq!(
            search(
                &anon,
                PromptQuery {
                    include: Some(tag.into()),
                    ..Default::default()
                }
            )
            .await,
            vec!["Both", "Tides"],
            "{tag}"
        );
    }
    // Prompts now carry the canonical tag once, in their original order.
    let tides = alice
        .prompts(&PromptQuery {
            q: Some("Tides".into()),
            ..Default::default()
        })
        .await
        .unwrap()
        .items
        .remove(0);
    let keys: Vec<String> = tides.tags.iter().map(|t| t.key()).collect();
    assert_eq!(keys, vec!["theme:sea", "genre:fantasy"]);
    let both = alice
        .prompts(&PromptQuery {
            q: Some("Both".into()),
            ..Default::default()
        })
        .await
        .unwrap()
        .items
        .remove(0);
    assert_eq!(both.tags.len(), 1);
    // New prompts using the alias get the canonical tag.
    let fresh = post(
        &alice,
        new_prompt(ap, "Fresh", Maturity::General, &["theme:ocean"]),
    )
    .await;
    assert_eq!(fresh.tags[0].key(), "theme:sea");
    // Aliases are hidden from tag listings.
    let listed = anon
        .tags(&TagQuery {
            namespace: Some("theme".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(listed.iter().all(|t| t.slug != "ocean"));

    let self_alias = admin
        .admin_update_tag(
            "theme",
            "sea",
            &UpdateTag {
                alias_of: Some("theme:ocean".into()),
                display: None,
                ..UpdateTag::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(status(&self_alias), 400);
    server.stop().await.unwrap();
}
