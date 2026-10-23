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
