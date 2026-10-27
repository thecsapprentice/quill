//! Automated moderation (#43): scripts, chains and what they decide.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use quill_e2e::TestServer;
use quill_proto::automod::{
    ReplaceScript, SaveModerationData, SetScriptMode, UpdateChain, UploadScript,
};
use quill_server::AppState;
use quill_server::error::ApiError;
use quill_server::moderation::{self, Hook, OnError, Verdict, chains, context, data, scripts};
use uuid::Uuid;

async fn account_id(server: &TestServer, login: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM accounts WHERE login_name = $1")
        .bind(login)
        .fetch_one(&server.db().await.unwrap())
        .await
        .unwrap()
}

async fn admin(server: &TestServer) -> Uuid {
    server.admin_account("boss").await.unwrap();
    account_id(server, "boss").await
}

/// A writer's persona and account.
async fn writer(server: &TestServer, name: &str) -> (Uuid, Uuid) {
    let client = server.account(name).await.unwrap();
    let persona = client
        .create_persona(&quill_proto::api::CreatePersona {
            handle: format!("{name}_pen"),
            display_name: None,
            summary: None,
        })
        .await
        .unwrap();
    (persona.id, account_id(server, name).await)
}

fn subject((persona, account): (Uuid, Uuid), title: &str, body: &str) -> context::Subject {
    context::Subject {
        hook: Hook::PromptCreated,
        title: title.into(),
        body: body.into(),
        tags: vec![],
        maturity: "general".into(),
        previous: None,
        tag: None,
        report: None,
        actor: Some(persona),
        account,
        submitter: Some(account),
        prompt: None,
        about: None,
    }
}

fn upload(name: &str, source: &str) -> UploadScript {
    UploadScript {
        name: name.into(),
        source: source.into(),
        log_only: false,
    }
}

async fn script(state: &AppState, admin: Uuid, name: &str, source: &str) -> Uuid {
    scripts::upload(state, admin, &upload(name, source))
        .await
        .unwrap()
        .id
}

async fn chain(state: &AppState, admin: Uuid, on_error: OnError, scripts: &[Uuid]) {
    chains::update(
        state,
        admin,
        Hook::PromptCreated,
        &UpdateChain {
            on_error,
            scripts: scripts.to_vec(),
        },
    )
    .await
    .unwrap();
}

async fn save(state: &AppState, admin: Uuid, name: &str, content: &str) {
    let body = SaveModerationData {
        description: String::new(),
        content: content.into(),
        base_version: None,
    };
    data::save(state, admin, name, &body).await.unwrap();
}

async fn count(state: &AppState, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM admin_log WHERE action = $1")
        .bind(action)
        .fetch_one(&state.pool)
        .await
        .unwrap()
}

fn refused(r: Result<impl std::fmt::Debug, ApiError>) -> String {
    match r {
        Err(ApiError::BadRequest(m)) => m,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn uploads_are_checked() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let state = server.state().await.unwrap();
    let admin = admin(&server).await;

    let syntax =
        refused(scripts::upload(&state, admin, &upload("Broken", "let x = [1,\nlet y = 2;")).await);
    assert!(syntax.starts_with("line 2:"), "{syntax}");
    let missing = refused(
        scripts::upload(
            &state,
            admin,
            &upload("Words", r#"common::lines("blocked_words")"#),
        )
        .await,
    );
    assert!(
        missing.contains("no common data named blocked_words"),
        "{missing}"
    );
    let field = refused(
        scripts::upload(
            &state,
            admin,
            &upload("Typo", r#"if post.titel == "" { reject() }"#),
        )
        .await,
    );
    assert!(
        field.contains("sample prompt post") && field.contains("titel"),
        "{field}"
    );

    save(&state, admin, "blocked_words", "spam").await;
    let source = r#"let blocked = common::lines("blocked_words");"#;
    let words = scripts::upload(&state, admin, &upload("Words", source))
        .await
        .unwrap();
    assert_eq!(words.data, ["blocked_words"]);
    let taken = scripts::upload(&state, admin, &upload("words", "")).await;
    assert!(matches!(taken, Err(ApiError::Conflict(_))), "{taken:?}");

    let actions: Vec<String> = sqlx::query_scalar(
        "SELECT action || ':' || script_name FROM admin_log
         WHERE script_name IS NOT NULL ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await
    .unwrap();
    assert_eq!(actions, ["upload_script:Words"]);
}

#[tokio::test]
async fn a_chain_stops_at_the_first_script_that_handles() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let state = server.state().await.unwrap();
    let admin = admin(&server).await;
    let wren = writer(&server, "wren").await;
    save(&state, admin, "blocked_words", "spam\nfree crypto").await;

    let words = script(
        &state,
        admin,
        "Words",
        r#"let all = post.title + " " + post.body;
           if text::find_words(common::lines("blocked_words"), all).len() > 0 {
               return reject("This prompt uses words that aren't allowed here.");
           }"#,
    )
    .await;
    let links = script(
        &state,
        admin,
        "Links",
        r#"for link in text::links(post.body) {
               return flag("Outside link to " + link.host, "spam");
           }"#,
    )
    .await;
    chain(&state, admin, OnError::Continue, &[words, links]).await;

    let check = |title: &str, body: &str| moderation::check(&state, subject(wren, title, body));
    assert_eq!(
        check("A quiet story", "Nothing odd here.").await.unwrap(),
        Verdict::Pass
    );

    let Verdict::Flag(flag) = check("A quiet story", "See evil.biz/win").await.unwrap() else {
        panic!("not flagged");
    };
    assert_eq!(flag.script_name, "Links");
    assert_eq!(flag.reason, "Outside link to evil.biz");

    // Words runs first, so a post with both is refused, not flagged.
    let Err(ApiError::ContentRejected(message)) = check("F R E E crypto", "See evil.biz/win").await
    else {
        panic!("not refused");
    };
    assert_eq!(message, "This prompt uses words that aren't allowed here.");

    // The refusal is logged and noted on the writer's account, already
    // read, with the title and message but none of the refused text.
    let (kind, fields, read): (String, serde_json::Value, bool) = sqlx::query_as(
        "SELECT kind, fields, read_at IS NOT NULL FROM account_notices WHERE account_id = $1",
    )
    .bind(wren.1)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(kind, "content_rejected");
    assert!(read);
    assert_eq!(fields["title"], "F R E E crypto");
    assert_eq!(
        fields["message"],
        "This prompt uses words that aren't allowed here."
    );
    assert!(!fields.to_string().contains("evil.biz"));
    let logged: (String, String) =
        sqlx::query_as("SELECT actor, script_name FROM admin_log WHERE action = 'automod_reject'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(logged, ("automod".to_owned(), "Words".to_owned()));
}

#[tokio::test]
async fn a_failing_script_follows_the_chain_setting() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let state = server
        .state_with(|c| c.automod.max_operations = 10_000)
        .await
        .unwrap();
    let admin = admin(&server).await;
    let wren = writer(&server, "wren").await;
    // Fine on the sample post; runs away on a post that mentions a loop.
    let runaway = script(
        &state,
        admin,
        "Runaway",
        r#"if post.body.contains("loop") { loop {} }"#,
    )
    .await;
    let after = script(&state, admin, "After", r#"flag("reached", "other")"#).await;
    let check = || moderation::check(&state, subject(wren, "t", "a loop"));

    chain(&state, admin, OnError::Continue, &[runaway, after]).await;
    let Verdict::Flag(flag) = check().await.unwrap() else {
        panic!("continue")
    };
    assert_eq!(flag.script_name, "After");

    chain(&state, admin, OnError::Reject, &[runaway, after]).await;
    let refusal = check().await;
    assert!(
        matches!(&refusal, Err(ApiError::ContentRejected(m)) if m.contains("couldn't be checked")),
        "{refusal:?}"
    );

    chain(&state, admin, OnError::Flag, &[runaway, after]).await;
    let Verdict::Flag(flag) = check().await.unwrap() else {
        panic!("flag")
    };
    assert_eq!(flag.script_name, "Runaway");
    assert!(flag.reason.contains("operation limit"), "{}", flag.reason);

    // Every failure is logged, whatever the setting.
    assert_eq!(count(&state, "automod_error").await, 3);
}

#[tokio::test]
async fn log_only_scripts_are_logged_and_do_nothing() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let state = server.state().await.unwrap();
    let admin = admin(&server).await;
    let wren = writer(&server, "wren").await;
    let trial = UploadScript {
        log_only: true,
        ..upload("Trial", r#"reject("Not yet")"#)
    };
    let trial = scripts::upload(&state, admin, &trial).await.unwrap().id;
    chain(&state, admin, OnError::Continue, &[trial]).await;

    assert_eq!(
        moderation::check(&state, subject(wren, "t", "b"))
            .await
            .unwrap(),
        Verdict::Pass
    );
    let detail: String =
        sqlx::query_scalar("SELECT detail FROM admin_log WHERE action = 'automod_reject'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert!(detail.starts_with("log only"), "{detail}");

    // Switched off, it doesn't run at all.
    let off = SetScriptMode {
        enabled: Some(false),
        log_only: None,
    };
    scripts::set_mode(&state, admin, trial, &off).await.unwrap();
    moderation::check(&state, subject(wren, "t", "b"))
        .await
        .unwrap();
    assert_eq!(count(&state, "automod_reject").await, 1);
}

#[tokio::test]
async fn replacing_a_script_takes_effect_and_removing_it_leaves_its_chains() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let state = server.state().await.unwrap();
    let admin = admin(&server).await;
    let wren = writer(&server, "wren").await;
    let rule = script(&state, admin, "Rule", "").await;
    chain(&state, admin, OnError::Continue, &[rule]).await;
    let check = || moderation::check(&state, subject(wren, "t", "b"));
    assert_eq!(check().await.unwrap(), Verdict::Pass);

    let new_source = ReplaceScript {
        name: None,
        source: Some(r#"flag("now")"#.into()),
    };
    scripts::replace(&state, admin, rule, &new_source)
        .await
        .unwrap();
    assert!(matches!(check().await.unwrap(), Verdict::Flag(_)));

    scripts::remove(&state, admin, rule).await.unwrap();
    assert_eq!(check().await.unwrap(), Verdict::Pass);
    let chain = chains::get(&state.pool, Hook::PromptCreated).await.unwrap();
    assert!(chain.steps.is_empty());
}

#[tokio::test]
async fn a_chain_can_only_run_scripts_that_work_at_its_hook() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let state = server.state().await.unwrap();
    let admin = admin(&server).await;
    // Works on prompts; the sample comment has no tags, so this fails there.
    let s = script(
        &state,
        admin,
        "Tags",
        r#"if post.tags[0] == "x" { reject() }"#,
    )
    .await;
    let refusal = chains::update(
        &state,
        admin,
        Hook::CommentCreated,
        &UpdateChain {
            on_error: OnError::Continue,
            scripts: vec![s],
        },
    )
    .await;
    let message = refused(refusal);
    assert!(
        message.starts_with("Tags: on a sample comment post"),
        "{message}"
    );
}
