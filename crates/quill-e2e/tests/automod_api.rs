//! Automated moderation (#43) over the API administrators use.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use quill_core::CoreError;
use quill_e2e::TestServer;
use quill_proto::automod::{
    DataOverride, DryRun, DryRunDecision, Hook, OnError, SaveModerationData, SetScriptMode,
    UpdateChain, UploadScript, UseTemplate,
};

#[tokio::test]
async fn an_administrator_sets_up_a_word_list_from_a_template() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let admin = server.admin_account("boss").await.unwrap();

    let templates = admin.automod_templates().await.unwrap();
    assert_eq!(templates.len(), 7);
    let words = templates.iter().find(|t| t.id == "blocked_words").unwrap();
    assert_eq!(words.data, ["blocked_words"]);

    // Using it creates the list it reads, with notes for the admin.
    let script = admin
        .automod_use_template(
            "blocked_words",
            &UseTemplate {
                name: Some("Words".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(script.data, ["blocked_words"]);
    let list = admin.automod_data("blocked_words").await.unwrap();
    assert!(list.content.starts_with('#'));
    assert_eq!(list.used_by, ["Words"]);

    // Fill the list, and put the script in the prompt chain.
    let saved = admin
        .automod_save_data(
            "blocked_words",
            &SaveModerationData {
                description: list.description.clone(),
                content: format!("{}spam\n", list.content),
                base_version: Some(list.version),
            },
        )
        .await
        .unwrap();
    assert_eq!(saved.version, 2);
    let chain = admin
        .automod_update_chain(
            Hook::PromptCreated,
            &UpdateChain {
                on_error: OnError::Continue,
                scripts: vec![script.id],
            },
        )
        .await
        .unwrap();
    assert_eq!(chain.steps[0].name, "Words");

    // A dry run on typed text, and again with an edit to the list that
    // isn't saved yet.
    let run = |body: &str, data: Vec<DataOverride>| DryRun {
        hook: Hook::PromptCreated,
        script: None,
        prompt: None,
        title: "A title".into(),
        body: body.into(),
        data,
    };
    let result = admin
        .automod_dry_run(&run("s p a m for sale", vec![]))
        .await
        .unwrap();
    assert_eq!(result.steps[0].decision, DryRunDecision::Reject);
    assert!(result.steps[0].decides);
    let unsaved = vec![DataOverride {
        name: "blocked_words".into(),
        content: "scam".into(),
    }];
    let result = admin
        .automod_dry_run(&run("s p a m for sale", unsaved))
        .await
        .unwrap();
    assert_eq!(result.steps[0].decision, DryRunDecision::Pass);
    // Nothing a dry run decides is logged.
    let log = admin.admin_log(None, Some(50)).await.unwrap();
    assert!(log.iter().all(|e| e.action.as_str() != "automod_reject"));

    // History and undo.
    let history = admin.automod_data_history("blocked_words").await.unwrap();
    assert_eq!(
        history.iter().map(|v| v.version).collect::<Vec<_>>(),
        [2, 1]
    );
    let first = admin
        .automod_data_version("blocked_words", 1)
        .await
        .unwrap();
    assert!(!first.content.contains("spam"));
    let undone = admin
        .automod_restore_data("blocked_words", 1)
        .await
        .unwrap();
    assert_eq!((undone.version, undone.content), (3, first.content));

    // The list can't be deleted while the script reads it.
    match admin.automod_delete_data("blocked_words").await {
        Err(CoreError::Api(p)) => assert_eq!(p.status, 409),
        other => panic!("{other:?}"),
    }
    admin.automod_remove(script.id).await.unwrap();
    admin.automod_delete_data("blocked_words").await.unwrap();
}

#[tokio::test]
async fn script_mistakes_come_back_with_their_line() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let admin = server.admin_account("boss").await.unwrap();
    let broken = UploadScript {
        name: "Broken".into(),
        source: "let a = 1;\nif a > { reject() }".into(),
        log_only: false,
    };
    match admin.automod_upload(&broken).await {
        Err(CoreError::Api(p)) => {
            assert_eq!(p.status, 400);
            assert!(p.detail.unwrap_or_default().starts_with("line 2:"));
        }
        other => panic!("{other:?}"),
    }

    let fine = UploadScript {
        name: "Fine".into(),
        source: "if post.body == \"x\" { reject() }".into(),
        log_only: true,
    };
    let script = admin.automod_upload(&fine).await.unwrap();
    assert!(script.log_only);
    let off = SetScriptMode {
        enabled: Some(false),
        log_only: None,
    };
    assert!(
        !admin
            .automod_set_mode(script.id, &off)
            .await
            .unwrap()
            .enabled
    );
    assert_eq!(admin.automod_scripts().await.unwrap().len(), 1);
    assert_eq!(admin.automod_chains().await.unwrap().len(), Hook::ALL.len());
}

#[tokio::test]
async fn writers_cannot_see_or_change_moderation() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let writer = server.account("wren").await.unwrap();
    for result in [
        writer.automod_scripts().await.map(drop),
        writer.automod_data_list().await.map(drop),
        writer.automod_templates().await.map(drop),
    ] {
        match result {
            Err(CoreError::Api(p)) => assert_eq!(p.status, 403),
            other => panic!("{other:?}"),
        }
    }
}
