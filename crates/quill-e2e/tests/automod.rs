//! Automated moderation (#43): common data, scripts and chains.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use quill_e2e::TestServer;
use quill_proto::automod::SaveModerationData;
use quill_server::error::ApiError;
use quill_server::moderation::data;
use uuid::Uuid;

async fn admin_id(server: &TestServer, login: &str) -> Uuid {
    server.admin_account(login).await.unwrap();
    sqlx::query_scalar("SELECT id FROM accounts WHERE login_name = $1")
        .bind(login)
        .fetch_one(&server.db().await.unwrap())
        .await
        .unwrap()
}

fn body(content: &str, base: Option<i32>) -> SaveModerationData {
    SaveModerationData {
        description: "Words to refuse".into(),
        content: content.into(),
        base_version: base,
    }
}

#[tokio::test]
async fn common_data_keeps_every_version() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let state = server.state().await.unwrap();
    let admin = admin_id(&server, "boss").await;

    assert_eq!(
        data::save(&state, admin, "blocked_words", &body("spam", None))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        data::save(&state, admin, "blocked_words", &body("spam\nscam", Some(1)))
            .await
            .unwrap(),
        2
    );

    let entry = data::get(&state.pool, "blocked_words").await.unwrap();
    assert_eq!((entry.version, entry.content.as_str()), (2, "spam\nscam"));
    assert_eq!(entry.updated_by.as_deref(), Some("boss"));
    assert_eq!(entry.description, "Words to refuse");

    // An edit started from version 1 would overwrite version 2: refused.
    let stale = data::save(&state, admin, "blocked_words", &body("other", Some(1))).await;
    assert!(matches!(stale, Err(ApiError::Conflict(_))), "{stale:?}");

    // Undo saves the old content again as a new version.
    assert_eq!(
        data::restore(&state, admin, "blocked_words", 1)
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        data::get(&state.pool, "blocked_words")
            .await
            .unwrap()
            .content,
        "spam"
    );
    let versions: Vec<i32> = data::history(&state.pool, "blocked_words")
        .await
        .unwrap()
        .iter()
        .map(|v| v.version)
        .collect();
    assert_eq!(versions, [3, 2, 1]);
    assert_eq!(
        data::version(&state.pool, "blocked_words", 2)
            .await
            .unwrap(),
        "spam\nscam"
    );

    // Every save is in the admin log.
    let logged: Vec<String> = sqlx::query_scalar(
        "SELECT detail FROM admin_log WHERE action = 'save_moderation_data' ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await
    .unwrap();
    assert_eq!(
        logged,
        ["version 1", "version 2", "version 3, restoring version 1"]
    );
}

#[tokio::test]
async fn common_data_names_and_sizes_are_checked() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let state = server
        .state_with(|c| {
            c.automod.data_entry_bytes = 10;
            c.automod.data_total_bytes = 15;
        })
        .await
        .unwrap();
    let admin = admin_id(&server, "boss").await;

    for bad in ["Blocked", "two words", ""] {
        let r = data::save(&state, admin, bad, &body("x", None)).await;
        assert!(matches!(r, Err(ApiError::BadRequest(_))), "{bad}: {r:?}");
    }
    let too_big = data::save(&state, admin, "big", &body("12345678901", None)).await;
    assert!(
        matches!(too_big, Err(ApiError::BadRequest(_))),
        "{too_big:?}"
    );

    data::save(&state, admin, "first", &body("1234567890", None))
        .await
        .unwrap();
    let over_total = data::save(&state, admin, "second", &body("123456", None)).await;
    assert!(
        matches!(over_total, Err(ApiError::BadRequest(_))),
        "{over_total:?}"
    );
    // Replacing an entry counts its new size, not old plus new.
    data::save(&state, admin, "first", &body("12345", None))
        .await
        .unwrap();
    data::save(&state, admin, "second", &body("123456", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn common_data_in_use_cannot_be_deleted() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let state = server.state().await.unwrap();
    let admin = admin_id(&server, "boss").await;
    data::save(&state, admin, "blocked_words", &body("spam", None))
        .await
        .unwrap();
    data::save(&state, admin, "unused", &body("x", None))
        .await
        .unwrap();

    let script = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO moderation_scripts (id, name, source, source_hash) VALUES ($1, 'Word list', '', '')",
    )
    .bind(script)
    .execute(&state.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO moderation_script_data VALUES ($1, 'blocked_words')")
        .bind(script)
        .execute(&state.pool)
        .await
        .unwrap();

    let refused = data::delete(&state, admin, "blocked_words").await;
    let Err(ApiError::Conflict(message)) = refused else {
        panic!("{refused:?}");
    };
    assert!(message.contains("Word list"), "{message}");
    assert_eq!(
        data::list(&state.pool).await.unwrap()[0].used_by,
        ["Word list"]
    );

    data::delete(&state, admin, "unused").await.unwrap();
    assert!(matches!(
        data::get(&state.pool, "unused").await,
        Err(ApiError::NotFound)
    ));
    assert!(matches!(
        data::history(&state.pool, "unused").await,
        Err(ApiError::NotFound)
    ));
}

#[tokio::test]
async fn a_snapshot_keeps_its_version() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let state = server.state().await.unwrap();
    let admin = admin_id(&server, "boss").await;
    data::save(&state, admin, "blocked_words", &body("spam", None))
        .await
        .unwrap();

    let names = vec!["blocked_words".to_owned(), "missing".to_owned()];
    let before = data::snapshot_for_tests(&state, &names).await.unwrap();
    data::save(&state, admin, "blocked_words", &body("scam", None))
        .await
        .unwrap();
    let after = data::snapshot_for_tests(&state, &names).await.unwrap();

    // A chain that started before the edit still sees the old list.
    assert_eq!(&*before.get("blocked_words").unwrap().lines(), ["spam"]);
    assert_eq!(&*after.get("blocked_words").unwrap().lines(), ["scam"]);
    assert!(after.get("missing").is_none());

    // The same version is shared, not parsed again.
    let again = data::snapshot_for_tests(&state, &names).await.unwrap();
    assert!(std::sync::Arc::ptr_eq(
        after.get("blocked_words").unwrap(),
        again.get("blocked_words").unwrap()
    ));
}
