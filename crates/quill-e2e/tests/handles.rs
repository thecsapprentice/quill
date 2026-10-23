//! Phase 1g end-to-end tests: reserved and retired persona handles.

// Shared helpers below are test code too; panicking on failure is intended.
#![allow(clippy::unwrap_used)]

use quill_core::{Client, CoreError};
use quill_e2e::TestServer;
use quill_proto::api::CreatePersona;

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

fn persona(handle: &str) -> CreatePersona {
    CreatePersona {
        handle: handle.into(),
        display_name: None,
        summary: None,
    }
}

async fn refused(client: &Client, handle: &str) -> String {
    let err = client.create_persona(&persona(handle)).await.unwrap_err();
    assert_eq!(status(&err), 409, "{handle}");
    err.to_string()
}

#[tokio::test]
async fn reserved_handles_are_refused_to_writers() {
    let server = server!();
    let alice = server.account("alice").await.unwrap();

    // Case variants are lowercased first; digits are read as letters; staff
    // words count as parts; the test server's own name is "localhost".
    for handle in [
        "admin",
        "ADMIN",
        "Site_Admin",
        "ad_min",
        "adm1n",
        "M0derator",
        "quill",
        "everyone",
        "localhost",
        "localhost_news",
    ] {
        let message = refused(&alice, handle).await;
        assert!(
            message.contains("reserved for this server"),
            "{handle}: {message}"
        );
    }
    // Lookalikes that aren't staff words are fine.
    for handle in ["badminton", "moderate_reader", "helpful_otter"] {
        alice.create_persona(&persona(handle)).await.unwrap();
    }

    // Administrators of the board can't take them through the API either;
    // only the server command can grant one.
    let admin = server.admin_account("boss").await.unwrap();
    refused(&admin, "support").await;
    let state = server.state().await.unwrap();
    let granted = quill_server::routes::grant_persona(&state, "boss", &persona("Support"))
        .await
        .unwrap();
    assert_eq!(granted.handle, "support");
    assert!(
        quill_server::routes::grant_persona(&state, "nobody_here", &persona("staff"))
            .await
            .is_err()
    );
    // Granted or not, a taken handle stays taken.
    let message = refused(&alice, "Support").await;
    assert!(message.contains("reserved"), "{message}");
    server.stop().await.unwrap();
}

#[tokio::test]
async fn deleted_handles_are_retired_for_good() {
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    let bob = server.account("bob").await.unwrap();

    let ink = alice.create_persona(&persona("ink_fox")).await.unwrap();
    // Taken and retired handles get the same reply.
    let taken = refused(&bob, "Ink_Fox").await;
    assert!(taken.contains("isn't available"), "{taken}");

    alice.delete_persona(ink.id).await.unwrap();
    for (client, handle) in [(&bob, "ink_fox"), (&alice, "ink_fox"), (&bob, "INK_FOX")] {
        let retired = refused(client, handle).await;
        assert_eq!(retired, taken, "{handle}");
    }
    // Not even the grant command brings it back.
    let state = server.state().await.unwrap();
    assert!(
        quill_server::routes::grant_persona(&state, "bob", &persona("ink_fox"))
            .await
            .is_err()
    );

    // Personas that disappear with their account are retired too, and only
    // the handle is kept.
    bob.create_persona(&persona("salt_wren")).await.unwrap();
    let pool = server.db().await.unwrap();
    sqlx::query("DELETE FROM accounts WHERE login_name = 'bob'")
        .execute(&pool)
        .await
        .unwrap();
    let retired: Vec<String> =
        sqlx::query_scalar("SELECT handle FROM retired_handles ORDER BY handle")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(retired, vec!["ink_fox", "salt_wren"]);
    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns
         WHERE table_name = 'retired_handles' ORDER BY ordinal_position",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(columns, vec!["handle", "retired_at"]);
    pool.close().await;
    refused(&alice, "salt_wren").await;
    server.stop().await.unwrap();
}

#[tokio::test]
async fn administrators_manage_the_reserved_list() {
    use quill_proto::handles::{AddReserved, ReservedKind, ReservedQuery, ReservedSource};
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    let admin = server.admin_account("boss").await.unwrap();

    // Starts with the defaults; writers can't read or change the list.
    let list = admin
        .admin_reserved(&ReservedQuery::default())
        .await
        .unwrap();
    assert!(list.total > 50);
    assert_eq!(list.server_name.as_deref(), Some("localhost"));
    assert!(!list.file_loaded);
    assert!(
        list.entries
            .iter()
            .all(|e| e.source == ReservedSource::Default)
    );
    assert_eq!(
        status(
            &alice
                .admin_reserved(&ReservedQuery::default())
                .await
                .unwrap_err()
        ),
        403
    );

    // Add a brand as a word and one exact handle.
    let added = admin
        .admin_reserve(&AddReserved {
            kind: ReservedKind::Word,
            patterns: vec!["Acme".into(), "acme".into(), "  ".into()],
        })
        .await
        .unwrap();
    assert_eq!(added.added, 1);
    admin
        .admin_reserve(&AddReserved {
            kind: ReservedKind::Exact,
            patterns: vec!["ink_fox".into()],
        })
        .await
        .unwrap();
    for handle in ["acme", "acme_help", "4cme", "ink_fox"] {
        refused(&alice, handle).await;
    }
    alice.create_persona(&persona("ink_foxes")).await.unwrap();
    let check = admin.admin_check_handle("Acme_Help").await.unwrap();
    assert_eq!(check.handle, "acme_help");
    let by = check.reserved_by.unwrap();
    assert_eq!(
        (by.pattern.as_str(), by.kind, by.source),
        ("acme", ReservedKind::Word, ReservedSource::Admin)
    );
    let bad = admin
        .admin_reserve(&AddReserved {
            kind: ReservedKind::Word,
            patterns: vec!["fine".into(), "not fine".into()],
        })
        .await
        .unwrap_err();
    assert_eq!(status(&bad), 400);
    // Nothing from a refused batch was added.
    assert!(
        admin
            .admin_check_handle("fine")
            .await
            .unwrap()
            .reserved_by
            .is_none()
    );

    // Defaults can be removed, and stay removed after a restart.
    admin
        .admin_unreserve(ReservedKind::Word, "support")
        .await
        .unwrap();
    server.state().await.unwrap();
    alice.create_persona(&persona("support")).await.unwrap();
    assert_eq!(
        status(
            &admin
                .admin_unreserve(ReservedKind::Word, "support")
                .await
                .unwrap_err()
        ),
        404
    );

    let log = admin.admin_log(None, None).await.unwrap();
    let details: Vec<&str> = log.iter().filter_map(|e| e.detail.as_deref()).collect();
    assert_eq!(
        details,
        vec!["word: support", "1 exact handles: ink_fox", "1 words: acme"]
    );
    server.stop().await.unwrap();
}

#[tokio::test]
async fn a_reserved_handles_file_is_synced_at_startup() {
    use quill_proto::handles::{ReservedKind, ReservedQuery, ReservedSource};
    let server = server!();
    let alice = server.account("alice").await.unwrap();
    let admin = server.admin_account("boss").await.unwrap();
    let dir = std::env::temp_dir().join(format!("quill-reserved-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("reserved.txt");

    std::fs::write(
        &file,
        "# brands\nacme\nglobex\n=initech_help\nadmin  # already a default\n",
    )
    .unwrap();
    let with_file = |path: std::path::PathBuf| {
        move |c: &mut quill_server::Config| {
            c.reserved_handles_file = Some(path);
        }
    };
    server.state_with(with_file(file.clone())).await.unwrap();
    let from_file = ReservedQuery {
        source: Some(ReservedSource::File),
        ..ReservedQuery::default()
    };
    let list = admin.admin_reserved(&from_file).await.unwrap();
    let patterns: Vec<&str> = list.entries.iter().map(|e| e.pattern.as_str()).collect();
    assert_eq!(patterns, vec!["acme", "globex", "initech_help"]);
    for handle in ["acme_news", "globex", "initech_help"] {
        refused(&alice, handle).await;
    }
    // A default the file repeats stays a default.
    let admin_entry = admin
        .admin_check_handle("admin")
        .await
        .unwrap()
        .reserved_by
        .unwrap();
    assert_eq!(admin_entry.source, ReservedSource::Default);
    // File entries can only change in the file.
    let err = admin
        .admin_unreserve(ReservedKind::Word, "acme")
        .await
        .unwrap_err();
    assert_eq!(status(&err), 400);
    assert!(err.to_string().contains("reserved-handles file"));

    // Editing the file and restarting syncs it.
    std::fs::write(&file, "acme\nhooli\n").unwrap();
    server.state_with(with_file(file.clone())).await.unwrap();
    let list = admin.admin_reserved(&from_file).await.unwrap();
    let patterns: Vec<&str> = list.entries.iter().map(|e| e.pattern.as_str()).collect();
    assert_eq!(patterns, vec!["acme", "hooli"]);
    alice.create_persona(&persona("globex")).await.unwrap();

    // A bad line stops startup and names the line.
    std::fs::write(&file, "acme\nnot ok\n").unwrap();
    let err = server
        .state_with(with_file(file.clone()))
        .await
        .err()
        .unwrap();
    let message = format!("{err:#}");
    assert!(message.contains("line 2"), "{message}");

    // Without a file, its entries go.
    server.state().await.unwrap();
    assert_eq!(admin.admin_reserved(&from_file).await.unwrap().total, 0);

    let log = admin.admin_log(None, None).await.unwrap();
    let synced: Vec<&str> = log
        .iter()
        .filter(|e| e.subject.as_deref() == Some("file"))
        .filter_map(|e| e.detail.as_deref())
        .collect();
    assert_eq!(
        synced,
        vec![
            "reserved handles file: 0 entries; 0 added, 2 removed",
            "reserved handles file: 2 entries; 1 added, 2 removed",
            "reserved handles file: 4 entries; 3 added, 0 removed",
        ]
    );
    assert!(
        log.iter()
            .filter(|e| e.subject.as_deref() == Some("file"))
            .all(|e| e.admin.is_none())
    );
    std::fs::remove_dir_all(&dir).unwrap();
    server.stop().await.unwrap();
}
