//! Phase 1h end-to-end tests: every admin-log entry says who acted and
//! which accounts it was against; administrators can filter by either.

// Shared helpers below are test code too; panicking on failure is intended.
#![allow(clippy::unwrap_used)]

use quill_core::{Client, CoreError};
use quill_e2e::TestServer;
use quill_proto::{
    api::CreatePersona,
    board::{
        AdminAction, AdminLogEntry, AdminLogQuery, CreatePrompt, CreateReport, LogActor, Maturity,
        ReportReason, Resolution, TagPolicy, UpdateServerSettings,
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

/// `(action, who, targets)` for each entry, newest first.
fn summary(log: &[AdminLogEntry]) -> Vec<(AdminAction, String, Vec<String>)> {
    log.iter()
        .map(|e| {
            let who = match (e.actor, &e.admin_login) {
                (LogActor::Admin, Some(l)) => l.clone(),
                (actor, _) => format!("({actor})"),
            };
            (e.action, who, e.targets.clone())
        })
        .collect()
}

fn q(admin: Option<&str>, account: Option<&str>) -> AdminLogQuery {
    AdminLogQuery {
        admin: admin.map(str::to_owned),
        account: account.map(str::to_owned),
        ..AdminLogQuery::default()
    }
}

#[tokio::test]
async fn every_entry_names_who_acted_and_against_whom() {
    let server = server!();
    let (alice, ap) = writer(&server, "alice").await;
    let (bob, bp) = writer(&server, "bob").await;
    let boss = server.admin_account("boss").await.unwrap();
    let ward = server.admin_account("ward").await.unwrap();
    let lamps = post(&alice, ap, "Lamps", &["genre:horror"]).await;
    post(&bob, bp, "Tides", &["genre:horror"]).await;

    bob.report(&CreateReport {
        prompt: lamps,
        reason: ReportReason::Spam,
        details: None,
    })
    .await
    .unwrap();
    let report = ward.admin_reports(false).await.unwrap()[0].id;
    ward.resolve_report(report, Resolution::Discard)
        .await
        .unwrap();
    boss.admin_delete_tag("genre", "horror").await.unwrap();
    boss.admin_update_settings(&UpdateServerSettings {
        new_tags: Some(TagPolicy::Curated),
    })
    .await
    .unwrap();
    // A server-made entry: a restart that syncs a reserved-handles file.
    let dir = std::env::temp_dir().join(format!("quill-log-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("r.txt"), "acme\n").unwrap();
    let file = dir.join("r.txt");
    server
        .state_with(move |c| c.reserved_handles_file = Some(file))
        .await
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    let log = boss.admin_log(None, None).await.unwrap();
    assert_eq!(
        summary(&log),
        vec![
            (AdminAction::ReserveHandles, "(server)".into(), s(&[])),
            (AdminAction::ChangeSetting, "boss".into(), s(&[])),
            (AdminAction::DeleteTag, "boss".into(), s(&["alice", "bob"])),
            (AdminAction::Discard, "ward".into(), s(&["alice"])),
            // The reporter (bob) is never named, even to administrators.
            (
                AdminAction::ReportReceived,
                "(reporter)".into(),
                s(&["alice"])
            ),
        ]
    );
    assert!(log[4].admin.is_none() && log[4].admin_login.is_none());

    // Watch one administrator, or one account; login names in any case.
    let by_ward = boss
        .admin_log_filtered(&q(Some("WARD"), None))
        .await
        .unwrap();
    assert_eq!(
        summary(&by_ward),
        vec![(AdminAction::Discard, "ward".into(), s(&["alice"]))]
    );
    let against_bob = boss
        .admin_log_filtered(&q(None, Some("bob")))
        .await
        .unwrap();
    assert_eq!(against_bob.len(), 1);
    assert_eq!(against_bob[0].action, AdminAction::DeleteTag);
    let both = boss
        .admin_log_filtered(&q(Some("boss"), Some("alice")))
        .await
        .unwrap();
    assert_eq!(both.len(), 1);
    assert_eq!(status(&alice.admin_log(None, None).await.unwrap_err()), 403);

    // Deleting accounts doesn't erase who did what to whom.
    let pool = server.db().await.unwrap();
    sqlx::query("DELETE FROM accounts WHERE login_name IN ('ward', 'alice')")
        .execute(&pool)
        .await
        .unwrap();
    let by_ward = boss
        .admin_log_filtered(&q(Some("ward"), None))
        .await
        .unwrap();
    assert_eq!(by_ward[0].admin_login.as_deref(), Some("ward"));
    assert_eq!(by_ward[0].admin, None);
    assert_eq!(by_ward[0].targets, s(&["alice"]));

    // The targets are append-only too.
    for sql in [
        "UPDATE admin_log_targets SET login = 'someone'",
        "DELETE FROM admin_log_targets",
        "TRUNCATE admin_log_targets",
        "UPDATE admin_log SET admin_login = 'someone'",
    ] {
        assert!(sqlx::query(sql).execute(&pool).await.is_err(), "{sql}");
    }
    pool.close().await;
    server.stop().await.unwrap();
}
