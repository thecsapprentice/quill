//! Phase 1d end-to-end tests: tag types, required tag types, tag deletion and
//! the admin log entries they produce, through the JSON API.

// Shared helpers below are test code too; panicking on failure is intended.
#![allow(clippy::unwrap_used)]

use quill_core::{Client, CoreError};
use quill_e2e::TestServer;
use quill_proto::{
    api::CreatePersona,
    board::{
        AdminAction, AdminTagQuery, CreatePrompt, CreateTag, Maturity, PromptQuery, PromptState,
        UpdateNamespace, UpdatePrompt, UpdateTag,
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

fn message(err: &CoreError) -> String {
    err.to_string()
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

fn new_prompt(persona: Uuid, title: &str, tags: &[&str]) -> CreatePrompt {
    CreatePrompt {
        persona,
        title: title.into(),
        body: format!("A prompt about {title}."),
        maturity: Maturity::General,
        tags: tags.iter().map(|t| (*t).to_owned()).collect(),
        state: None,
    }
}

fn require(on: bool) -> UpdateNamespace {
    UpdateNamespace {
        min: Some(u32::from(on)),
        ..UpdateNamespace::default()
    }
}

#[tokio::test]
async fn required_tag_types_apply_to_every_save() {
    let server = server!();
    let (alice, ap) = writer(&server, "alice").await;
    let admin = server.admin_account("moderator").await.unwrap();

    // Posted before Genre becomes required, with no genre tag.
    let old = alice
        .create_prompt(&new_prompt(ap, "Old", &["format:prose"]))
        .await
        .unwrap();

    assert_eq!(
        status(
            &alice
                .admin_update_namespace("genre", &require(true))
                .await
                .unwrap_err()
        ),
        403
    );
    let genre = admin
        .admin_update_namespace("genre", &require(true))
        .await
        .unwrap();
    assert!(genre.required());
    let listed = alice.tag_namespaces().await.unwrap();
    assert!(listed.iter().any(|n| n.name == "genre" && n.required()));
    assert!(listed.iter().any(|n| n.name == "format" && !n.required()));
    assert_eq!(
        status(
            &admin
                .admin_update_namespace("no-such-type", &require(true))
                .await
                .unwrap_err()
        ),
        404
    );

    // New prompts need a genre tag.
    let refused = alice
        .create_prompt(&new_prompt(ap, "New", &["format:prose"]))
        .await
        .unwrap_err();
    assert_eq!(status(&refused), 400);
    assert!(
        message(&refused).contains("Pick at least 1 Genre tag."),
        "{refused}"
    );
    alice
        .create_prompt(&new_prompt(ap, "New", &["genre:mystery"]))
        .await
        .unwrap();

    // The old prompt is untouched, and its status can still change...
    let fetched = alice.prompt(old.id).await.unwrap();
    assert_eq!(fetched.tags.len(), 1);
    alice
        .update_prompt(
            old.id,
            &UpdatePrompt {
                state: Some(PromptState::Busy),
                ..UpdatePrompt::default()
            },
        )
        .await
        .unwrap();
    // ...but any other edit must add a genre.
    let edit = UpdatePrompt {
        title: Some("Old, edited".into()),
        ..UpdatePrompt::default()
    };
    assert_eq!(
        status(&alice.update_prompt(old.id, &edit).await.unwrap_err()),
        400
    );
    let edited = alice
        .update_prompt(
            old.id,
            &UpdatePrompt {
                tags: Some(vec!["format:prose".into(), "genre:horror".into()]),
                ..edit
            },
        )
        .await
        .unwrap();
    assert_eq!(edited.title, "Old, edited");

    // Requiring two types names both when both are missing.
    admin
        .admin_update_namespace("length", &require(true))
        .await
        .unwrap();
    let both = alice
        .create_prompt(&new_prompt(ap, "Bare", &[]))
        .await
        .unwrap_err();
    assert!(
        message(&both).contains("Pick at least 1 Genre tag. Pick at least 1 Length tag."),
        "{both}"
    );

    // Turning the rule off lifts it.
    admin
        .admin_update_namespace("genre", &require(false))
        .await
        .unwrap();
    admin
        .admin_update_namespace("length", &require(false))
        .await
        .unwrap();
    alice
        .create_prompt(&new_prompt(ap, "Bare", &[]))
        .await
        .unwrap();

    // Each change is logged; saving with no change logs nothing.
    let renamed = admin
        .admin_update_namespace(
            "genre",
            &UpdateNamespace {
                display: Some("Genres".into()),
                locked: Some(true),
                ..UpdateNamespace::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(renamed.display, "Genres");
    admin
        .admin_update_namespace("genre", &UpdateNamespace::default())
        .await
        .unwrap();
    let log = admin.admin_log(None, None).await.unwrap();
    assert!(log.iter().all(|e| e.action == AdminAction::UpdateTagType));
    assert_eq!(log.len(), 5);
    assert_eq!(log[0].subject.as_deref(), Some("genre"));
    assert_eq!(
        log[0].detail.as_deref(),
        Some("renamed from Genre to Genres; curated tags only: on")
    );
    assert_eq!(
        log[4].detail.as_deref(),
        Some("tags per prompt: any number → at least 1")
    );
    assert!(log[0].prompt_hash.is_none() && log[0].reason.is_none());
    assert!(log[0].admin.is_some());
    server.stop().await.unwrap();
}

#[tokio::test]
async fn deleting_a_tag_removes_it_everywhere() {
    let server = server!();
    let (alice, ap) = writer(&server, "alice").await;
    let admin = server.admin_account("moderator").await.unwrap();

    for title in ["One", "Two"] {
        alice
            .create_prompt(&new_prompt(
                ap,
                title,
                &["theme:Lighthouses", "genre:horror"],
            ))
            .await
            .unwrap();
    }
    alice
        .create_prompt(&new_prompt(ap, "Three", &["theme:Beacons"]))
        .await
        .unwrap();
    admin
        .admin_update_tag(
            "theme",
            "beacons",
            &UpdateTag {
                alias_of: Some("theme:lighthouses".into()),
                display: None,
                ..UpdateTag::default()
            },
        )
        .await
        .unwrap();

    // The admin listing shows usage and aliases.
    let themes = admin
        .admin_tags(&AdminTagQuery {
            namespace: Some("theme".into()),
            ..AdminTagQuery::default()
        })
        .await
        .unwrap();
    let find = |slug: &str| themes.iter().find(|t| t.tag.slug == slug).unwrap();
    assert_eq!(find("lighthouses").uses, 3);
    assert_eq!(
        find("beacons").alias_of.as_deref(),
        Some("theme:lighthouses")
    );
    let searched = admin
        .admin_tags(&AdminTagQuery {
            search: Some("HOUSE".into()),
            ..AdminTagQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(searched.len(), 1);
    assert_eq!(
        status(
            &alice
                .admin_tags(&AdminTagQuery::default())
                .await
                .unwrap_err()
        ),
        403
    );

    assert_eq!(
        status(
            &alice
                .admin_delete_tag("theme", "lighthouses")
                .await
                .unwrap_err()
        ),
        403
    );
    let deleted = admin
        .admin_delete_tag("theme", "lighthouses")
        .await
        .unwrap();
    assert_eq!(deleted.prompts_updated, 3);
    assert_eq!(
        status(
            &admin
                .admin_delete_tag("theme", "lighthouses")
                .await
                .unwrap_err()
        ),
        404
    );

    // Gone from every prompt, the listing and the alias.
    let page = alice.prompts(&PromptQuery::default()).await.unwrap();
    assert_eq!(page.items.len(), 3);
    for p in &page.items {
        assert!(p.tags.iter().all(|t| t.namespace != "theme"), "{}", p.title);
    }
    let one = page.items.iter().find(|p| p.title == "One").unwrap();
    assert_eq!(one.tags[0].key(), "genre:horror");
    let themes = admin
        .admin_tags(&AdminTagQuery {
            namespace: Some("theme".into()),
            ..AdminTagQuery::default()
        })
        .await
        .unwrap();
    assert!(themes.is_empty());

    let log = admin.admin_log(None, None).await.unwrap();
    assert_eq!(log[0].action, AdminAction::DeleteTag);
    assert_eq!(log[0].subject.as_deref(), Some("theme:lighthouses"));
    assert_eq!(
        log[0].detail.as_deref(),
        Some("Lighthouses; removed from 3 prompt(s); 1 alias(es) deleted too")
    );
    assert_eq!(log[1].action, AdminAction::AliasTag);
    assert_eq!(
        log[1].detail.as_deref(),
        Some("now an alias of theme:lighthouses; 1 prompt(s) moved")
    );
    server.stop().await.unwrap();
}

#[tokio::test]
async fn creating_tags_and_types_is_logged() {
    let server = server!();
    let admin = server.admin_account("moderator").await.unwrap();
    admin
        .admin_create_namespace(&quill_proto::board::CreateNamespace {
            name: "setting".into(),
            display: "Setting".into(),
            locked: false,
            min: 1,
            max: None,
            color: None,
        })
        .await
        .unwrap();
    let tag = CreateTag {
        namespace: "setting".into(),
        display: "Deep space".into(),
    };
    admin.admin_create_tag(&tag).await.unwrap();
    // Already curated: refused rather than silently repeated.
    assert_eq!(
        status(&admin.admin_create_tag(&tag).await.unwrap_err()),
        409
    );
    admin
        .admin_update_tag(
            "setting",
            "deep-space",
            &UpdateTag {
                display: Some("Outer space".into()),
                alias_of: None,
                ..UpdateTag::default()
            },
        )
        .await
        .unwrap();

    let log = admin.admin_log(None, None).await.unwrap();
    let actions: Vec<AdminAction> = log.iter().map(|e| e.action).collect();
    assert_eq!(
        actions,
        vec![
            AdminAction::RenameTag,
            AdminAction::CreateTag,
            AdminAction::CreateTagType
        ]
    );
    assert_eq!(
        log[0].detail.as_deref(),
        Some("renamed from Deep space to Outer space")
    );
    assert_eq!(log[1].subject.as_deref(), Some("setting:deep-space"));
    assert_eq!(
        log[2].detail.as_deref(),
        Some("Setting; curated tags only: off; tags per prompt: at least 1")
    );
    server.stop().await.unwrap();
}

#[tokio::test]
async fn chip_colours_through_the_api() {
    use quill_proto::board::{ChipColor, TagQuery};
    let server = server!();
    let admin = server.admin_account("moderator").await.unwrap();
    let anon = server.client();
    let color = |bg: &str, fg: Option<&str>| ChipColor {
        background: bg.into(),
        text: fg.map(str::to_owned),
    };
    let genre_tag = |slug: &'static str| {
        let anon = anon.clone();
        async move {
            anon.tags(&TagQuery {
                namespace: Some("genre".into()),
                ..TagQuery::default()
            })
            .await
            .unwrap()
            .into_iter()
            .find(|t| t.slug == slug)
            .unwrap()
        }
    };

    // Types start with colours; tags inherit them, text resolved.
    assert_eq!(
        genre_tag("horror").await.color,
        Some(color("#dbeafe", Some("#1e3a8a")))
    );

    // A tag's own colour wins; "#ABC" is normalized; text is automatic.
    admin
        .admin_update_tag(
            "genre",
            "horror",
            &UpdateTag {
                color: Some(color("#ABC", None)),
                ..UpdateTag::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        genre_tag("horror").await.color,
        Some(color("#aabbcc", Some("#000000")))
    );
    let listed = admin
        .admin_tags(&AdminTagQuery {
            search: Some("horror".into()),
            ..AdminTagQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(listed[0].own_color, Some(color("#aabbcc", None)));

    // Bad colours are refused.
    let bad = admin
        .admin_update_tag(
            "genre",
            "horror",
            &UpdateTag {
                color: Some(color("blue", None)),
                ..UpdateTag::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(status(&bad), 400);

    // Clearing the tag's colour falls back to its type; clearing the type's
    // leaves no colour at all.
    admin
        .admin_update_tag(
            "genre",
            "horror",
            &UpdateTag {
                clear_color: true,
                ..UpdateTag::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        genre_tag("horror").await.color,
        Some(color("#dbeafe", Some("#1e3a8a")))
    );
    let ns = admin
        .admin_update_namespace(
            "genre",
            &UpdateNamespace {
                clear_color: true,
                ..UpdateNamespace::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(ns.color, None);
    assert_eq!(genre_tag("horror").await.color, None);

    let log = admin.admin_log(None, None).await.unwrap();
    let details: Vec<&str> = log.iter().filter_map(|e| e.detail.as_deref()).collect();
    assert_eq!(
        details,
        vec![
            "colour: none",
            "colour: same as its type",
            "colour: automatic text on #aabbcc"
        ]
    );
    assert_eq!(log[1].action, AdminAction::RecolorTag);
    server.stop().await.unwrap();
}
