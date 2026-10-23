//! Phase 1h end-to-end tests: every admin-log entry says who acted and
//! which accounts it was against; administrators can filter by either.

// Shared helpers below are test code too; panicking on failure is intended.
#![allow(clippy::unwrap_used)]

use quill_core::{Client, CoreError};
use quill_e2e::TestServer;
use quill_proto::{
    api::CreatePersona,
    board::{
        AdminAction, AdminLogEntry, AdminLogQuery, AdminLogTargetQuery, CreatePrompt, CreateReport,
        LogActor, Maturity, ReportReason, Resolution, TagPolicy, UpdateServerSettings,
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
        ..UpdateServerSettings::default()
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

#[tokio::test]
async fn a_decision_against_many_accounts_is_listed_in_full_a_page_at_a_time() {
    let server = server!();
    let boss = server.admin_account("boss").await.unwrap();
    // Eight writers use one tag; the last posts it twice.
    let names = ["ana", "ben", "cy", "dee", "eli", "fay", "gus", "hal"];
    let mut writers = Vec::new();
    for name in names {
        let (client, persona) = writer(&server, name).await;
        post(&client, persona, &format!("{name} 1"), &["genre:horror"]).await;
        writers.push((client, persona));
    }
    let (hal, hp) = &writers[7];
    let hal_2 = post(hal, *hp, "hal 2", &["genre:horror"]).await;
    boss.admin_delete_tag("genre", "horror").await.unwrap();
    // One prompt goes after the decision; one writer leaves.
    hal.delete_prompt(hal_2).await.unwrap();
    let pool = server.db().await.unwrap();
    sqlx::query("DELETE FROM accounts WHERE login_name = 'cy'")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    // The listing names five and counts all eight.
    let log = boss.admin_log(None, None).await.unwrap();
    let entry = &log[0];
    assert_eq!(entry.action, AdminAction::DeleteTag);
    assert_eq!(entry.targets, ["ana", "ben", "cy", "dee", "eli"]);
    assert_eq!(entry.target_count, 8);
    // Filtering for an account puts it in the preview.
    let for_hal = boss
        .admin_log_filtered(&q(None, Some("hal")))
        .await
        .unwrap();
    assert_eq!(for_hal[0].targets[0], "hal");
    assert_eq!(for_hal[0].target_count, 8);

    // The entry's page lists everyone, three at a time.
    let mut seen = Vec::new();
    let mut after = None;
    loop {
        let page = boss
            .admin_log_entry(
                entry.id,
                &AdminLogTargetQuery {
                    after: after.clone(),
                    limit: Some(3),
                },
            )
            .await
            .unwrap();
        assert_eq!(page.entry, *entry);
        seen.extend(page.targets);
        match page.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    let logins: Vec<_> = seen.iter().map(|t| t.login.as_str()).collect();
    assert_eq!(logins, names);
    let cy = &seen[2];
    assert!(cy.account_deleted && cy.prompts.is_empty());
    let ana = &seen[0];
    assert!(!ana.account_deleted);
    assert_eq!(ana.prompts.len(), 1);
    assert_eq!(
        (ana.prompts[0].title.as_str(), ana.prompts[0].exists),
        ("ana 1", true)
    );
    let hal_prompts: Vec<_> = seen[7]
        .prompts
        .iter()
        .map(|p| (p.title.as_str(), p.exists))
        .collect();
    assert_eq!(hal_prompts, [("hal 1", true), ("hal 2", false)]);

    // Unknown entries, and writers, get nothing.
    let none = boss
        .admin_log_entry(999_999, &AdminLogTargetQuery::default())
        .await
        .unwrap_err();
    assert_eq!(status(&none), 404);
    let (ana, _) = &writers[0];
    let denied = ana
        .admin_log_entry(entry.id, &AdminLogTargetQuery::default())
        .await
        .unwrap_err();
    assert_eq!(status(&denied), 403);
    server.stop().await.unwrap();
}
