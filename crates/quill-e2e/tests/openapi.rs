//! The OpenAPI file (2m) against a running server: a story that touches
//! most of the API is run through a server that keeps every exchange, and
//! each answer must be one the file documents for that operation: its
//! status, its media type, and for JSON a body that passes the schema.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{collections::BTreeSet, time::Duration};

use quill_core::{
    Client, Store, backup, devices, keys, requests,
    sessions::{self, Draft},
};
use quill_e2e::{Exchange, TEST_PASSWORD, TestServer};
use quill_proto::{
    announcements::{
        AnnouncementQuery, CommentQuery, CreateAnnouncement, CreateComment, RemoveComment,
        ReportComment, UpdateAnnouncement,
    },
    api::{CreatePersona, Credentials, Persona, UpdatePersona},
    board::{
        AdminLogTargetQuery, AdminTagQuery, CreateNamespace, CreatePrompt, CreateReport, CreateTag,
        Maturity, PromptQuery, PromptState, ReportReason, Resolution, TagQuery,
        UpdateAccountSettings, UpdatePrompt,
    },
    handles::{AddReserved, ReservedKind, ReservedQuery},
};
use serde_json::Value;

const BASE: &str = "https://quill.test/openapi.json";

async fn writer(client: &Client, handle: &str) -> (Persona, Store) {
    let p = client
        .create_persona(&CreatePersona {
            handle: handle.into(),
            display_name: None,
            summary: None,
        })
        .await
        .unwrap();
    let store = Store::in_memory();
    keys::init(&store, client, &p, false).await.unwrap();
    keys::publish(&store, client, &p, 5).await.unwrap();
    (p, store)
}

async fn signed_in(server: &TestServer, name: &str) -> Client {
    let c = server.client();
    let token = c
        .login(&Credentials {
            username: name.into(),
            password: TEST_PASSWORD.into(),
        })
        .await
        .unwrap();
    c.with_token(&token.token)
}

/// Runs the story; what each call returns is checked by the file, not here.
async fn story(server: &TestServer) {
    let anon = server.client();
    anon.instance().await.unwrap();
    let admin = server.admin_account("root").await.unwrap();
    let alice_c = server.account("alice").await.unwrap();
    let (fox, alice) = writer(&alice_c, "ink_fox").await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    // A wrong password, and a duplicate login: documented errors.
    assert!(
        anon.login(&Credentials {
            username: "alice".into(),
            password: "not the password".into(),
        })
        .await
        .is_err()
    );
    assert!(server.account("alice").await.is_err());

    // Personas and the board.
    alice_c
        .update_persona(
            fox.id,
            &UpdatePersona {
                display_name: Some("Ink Fox".into()),
                summary: None,
            },
        )
        .await
        .unwrap();
    let spare = alice_c
        .create_persona(&CreatePersona {
            handle: "spare_fox".into(),
            display_name: None,
            summary: None,
        })
        .await
        .unwrap();
    alice_c.personas().await.unwrap();
    alice_c.delete_persona(spare.id).await.unwrap();
    alice_c.account().await.unwrap();
    alice_c
        .update_account(&UpdateAccountSettings {
            maturity_max: Some(Maturity::General),
            staff_name: None,
        })
        .await
        .unwrap();
    let ns = anon.tag_namespaces().await.unwrap();
    anon.tags(&TagQuery::default()).await.unwrap();
    if let Some(open) = ns.iter().find(|n| !n.locked) {
        let _ = alice_c
            .create_tag(&CreateTag {
                namespace: open.name.clone(),
                display: "Lighthouses".into(),
            })
            .await;
    }
    let prompt = alice_c
        .create_prompt(&CreatePrompt {
            persona: fox.id,
            title: "The lighthouse ledger".into(),
            body: "A keeper finds a ledger.".into(),
            maturity: Maturity::General,
            tags: Vec::new(),
            state: Some(PromptState::Open),
        })
        .await
        .unwrap();
    alice_c
        .update_prompt(
            prompt.id,
            &UpdatePrompt {
                body: Some("A keeper finds a ledger, still wet.".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    anon.prompts(&PromptQuery::default()).await.unwrap();
    anon.prompt(prompt.id).await.unwrap();
    assert!(anon.prompt(uuid::Uuid::now_v7()).await.is_err());

    // Keys.
    anon.actor_keys("ink_fox").await.unwrap();
    let keys = alice_c.persona_keys(fox.id).await.unwrap();
    if let Some(d) = keys.devices.first() {
        alice_c.key_package_status(fox.id, d.id).await.unwrap();
    }

    // A request, a session, and everything written in it.
    let sent = requests::send(&bob, &bob_c, &wren, prompt.id, "A pitch.", None)
        .await
        .unwrap();
    bob_c.own_requests().await.unwrap();
    bob_c.request_status(sent.request.id).await.unwrap();
    requests::inbox(&alice, &alice_c, prompt.id, false)
        .await
        .unwrap();
    let id = requests::accept(&alice, &alice_c, sent.request.id)
        .await
        .unwrap();
    for (s, c) in [(&alice, &alice_c), (&bob, &bob_c)] {
        sessions::sync(s, c).await.unwrap();
    }
    let first = sessions::write(&alice, &alice_c, &id, "The ledger.", None)
        .await
        .unwrap();
    sessions::name_channel(&alice, &alice_c, &id, "world", Some("Places"))
        .await
        .unwrap();
    let filed = sessions::post(
        &bob,
        &bob_c,
        &id,
        Draft {
            text: "A map.",
            channel: Some("world"),
            reply_to: Some(first),
            files: vec![("map.txt".into(), b"a map".repeat(100))],
        },
    )
    .await
    .unwrap();
    sessions::sync(&alice, &alice_c).await.unwrap();
    sessions::edit(&bob, &bob_c, &id, filed, "A better map.")
        .await
        .unwrap();
    sessions::delete(&alice, &alice_c, &id, first)
        .await
        .unwrap();
    for (s, c) in [(&alice, &alice_c), (&bob, &bob_c)] {
        sessions::sync(s, c).await.unwrap();
    }
    alice_c.sessions().await.unwrap();
    alice_c.storage().await.unwrap();

    // A second device, its history by bundle.
    let phone = Store::in_memory();
    let phone_c = signed_in(server, "bob").await;
    let pending = devices::start_link(&phone, &phone_c).await.unwrap();
    devices::approve(&bob, &bob_c, &pending.code.parse().unwrap())
        .await
        .unwrap();
    devices::wait_for_link(
        &phone,
        &phone_c,
        &pending.link_id,
        Duration::from_millis(50),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    devices::finish_link(&phone, &phone_c).await.unwrap();
    sessions::sync(&bob, &bob_c).await.unwrap();
    devices::maintain(&phone, &phone_c).await.unwrap();
    devices::maintain(&bob, &bob_c).await.unwrap();
    devices::maintain(&phone, &phone_c).await.unwrap();

    // A backup, restored on a new device that rejoins.
    let code = backup::create(&alice, &alice_c, false)
        .await
        .unwrap()
        .code
        .unwrap()
        .join(" ");
    alice_c.backup_info().await.unwrap();
    let restored = Store::in_memory();
    let restored_c = signed_in(server, "alice").await;
    backup::restore(&restored, &restored_c, &code)
        .await
        .unwrap();
    sessions::sync(&bob, &bob_c).await.unwrap();
    sessions::sync(&restored, &restored_c).await.unwrap();

    // Reports and administration.
    anon.report(&CreateReport {
        prompt: prompt.id,
        reason: ReportReason::Spam,
        details: Some("Testing.".into()),
    })
    .await
    .unwrap();
    let reports = admin.admin_reports(false).await.unwrap();
    admin
        .resolve_report(reports[0].id, Resolution::Discard)
        .await
        .unwrap();
    let log = admin.admin_log(None, None).await.unwrap();
    if let Some(entry) = log.first() {
        admin
            .admin_log_entry(entry.id, &AdminLogTargetQuery::default())
            .await
            .unwrap();
    }
    admin
        .admin_create_namespace(&CreateNamespace {
            name: "setting".into(),
            display: "Setting".into(),
            locked: false,
            required: false,
            color: None,
        })
        .await
        .unwrap();
    admin
        .admin_create_tag(&CreateTag {
            namespace: "setting".into(),
            display: "Sea".into(),
        })
        .await
        .unwrap();
    admin.admin_tags(&AdminTagQuery::default()).await.unwrap();
    admin.admin_delete_tag("setting", "sea").await.unwrap();
    admin.admin_settings().await.unwrap();
    admin
        .admin_reserve(&AddReserved {
            kind: ReservedKind::Exact,
            patterns: vec!["staff".into()],
        })
        .await
        .unwrap();
    admin
        .admin_reserved(&ReservedQuery::default())
        .await
        .unwrap();
    admin.admin_check_handle("staff").await.unwrap();
    admin.admin_storage().await.unwrap();
    let news = admin
        .admin_post_announcement(&CreateAnnouncement {
            title: "News".into(),
            body: "Read *this*.".into(),
            comments_open: true,
            sticky: false,
        })
        .await
        .unwrap();
    admin
        .admin_update_announcement(
            news.id,
            &UpdateAnnouncement {
                comments_open: Some(false),
                ..UpdateAnnouncement::default()
            },
        )
        .await
        .unwrap();
    anon.announcements(&AnnouncementQuery::default())
        .await
        .unwrap();
    anon.announcement(news.id).await.unwrap();
    admin
        .admin_update_announcement(
            news.id,
            &UpdateAnnouncement {
                comments_open: Some(true),
                ..UpdateAnnouncement::default()
            },
        )
        .await
        .unwrap();
    let said = alice_c
        .post_comment(news.id, &CreateComment::as_persona(fox.id, "Thanks".into()))
        .await
        .unwrap();
    let kept = alice_c
        .post_comment(news.id, &CreateComment::as_persona(fox.id, "More".into()))
        .await
        .unwrap();
    anon.comments(news.id, &CommentQuery::default())
        .await
        .unwrap();
    anon.report_comment(
        said.id,
        &ReportComment {
            reason: ReportReason::Spam,
            details: None,
        },
    )
    .await
    .unwrap();
    admin.admin_reports(false).await.unwrap();
    admin
        .admin_remove_comment(
            said.id,
            &RemoveComment {
                reason: ReportReason::Spam,
            },
        )
        .await
        .unwrap();
    alice_c.delete_comment(kept.id).await.unwrap();
    admin.admin_delete_announcement(news.id).await.unwrap();
    anon.announcement(news.id).await.unwrap_err();
    alice_c.notices(None).await.unwrap();
    alice_c.mark_notices_read().await.unwrap();

    // Leaving, and signing out.
    sessions::leave(&bob, &bob_c, &id).await.unwrap();
    alice_c.delete_prompt(prompt.id).await.unwrap();
    anon.logout().await.unwrap_err();
    restored_c.logout().await.unwrap();
}

/// `/prompts/{id}` → segments, `None` for a parameter.
fn template(path: &str) -> Vec<Option<&str>> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(|s| (!s.starts_with('{')).then_some(s))
        .collect()
}

/// The documented path a request path matches, preferring literal
/// segments over parameters.
fn operation<'a>(paths: &'a [String], path: &str) -> Option<&'a String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    paths
        .iter()
        .filter(|p| {
            let t = template(p);
            t.len() == segments.len()
                && t.iter()
                    .zip(&segments)
                    .all(|(t, s)| t.is_none_or(|t| t == *s))
        })
        .min_by_key(|p| template(p).iter().filter(|t| t.is_none()).count())
}

fn pointer_escape(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_server_answers_as_the_openapi_file_says() {
    let Some((server, recording)) = TestServer::start_recording().await.unwrap() else {
        return;
    };
    // The file the server serves is the committed one.
    let served: Value = reqwest::get(format!("{}/api/v1/openapi.json", server.url()))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let committed: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/openapi.json"
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(served, committed);

    story(&server).await;

    let mut compiler = boon::Compiler::new();
    compiler.add_resource(BASE, committed.clone()).unwrap();
    let mut schemas = boon::Schemas::new();
    let paths: Vec<String> = committed["paths"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let mut seen = BTreeSet::new();
    let mut checked = 0;
    let mut wrong = Vec::new();
    for Exchange {
        method,
        path,
        status,
        content_type,
        body,
    } in recording.exchanges()
    {
        let Some(api_path) = path.strip_prefix("/api/v1") else {
            continue;
        };
        if api_path == "/openapi.json" {
            continue;
        }
        let what = format!("{method} {api_path} → {status}");
        let method = method.to_lowercase();
        let Some(template) = operation(&paths, api_path) else {
            wrong.push(format!("{what}: no such path in the file"));
            continue;
        };
        let op = &committed["paths"][template][&method];
        if op.is_null() {
            wrong.push(format!("{what}: no such operation in the file"));
            continue;
        }
        seen.insert(format!("{method} {template}"));
        let status = status.to_string();
        let response = &op["responses"][&status];
        if response.is_null() {
            wrong.push(format!("{what}: the file doesn't list {status}"));
            continue;
        }
        let content = &response["content"];
        let Some(media) = content_type.as_deref() else {
            if content.is_object() {
                wrong.push(format!("{what}: no body, but the file gives one"));
            }
            continue;
        };
        let media = media.split(';').next().unwrap_or_default().trim();
        if content[media].is_null() {
            wrong.push(format!("{what}: {media} isn't a documented media type"));
            continue;
        }
        if !media.ends_with("json") {
            continue;
        }
        let pointer = format!(
            "{BASE}#/paths/{}/{method}/responses/{status}/content/{}/schema",
            pointer_escape(template),
            pointer_escape(media)
        );
        let index = compiler
            .compile(&pointer, &mut schemas)
            .unwrap_or_else(|e| panic!("{what}: the schema doesn't compile: {e}"));
        let value: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => {
                wrong.push(format!("{what}: not JSON: {e}"));
                continue;
            }
        };
        if let Err(e) = schemas.validate(&value, index) {
            wrong.push(format!("{what}: {e:#}"));
        }
        checked += 1;
    }
    assert!(
        wrong.is_empty(),
        "answers the file doesn't describe:\n{}",
        wrong.join("\n")
    );
    let operations = paths
        .iter()
        .flat_map(|p| {
            committed["paths"][p]
                .as_object()
                .unwrap()
                .keys()
                .map(move |m| format!("{m} {p}"))
        })
        .count();
    eprintln!(
        "{} of {operations} operations exercised, {checked} JSON bodies checked",
        seen.len()
    );
    assert!(seen.len() >= 70, "only {} operations exercised", seen.len());
    server.stop().await.unwrap();
}
