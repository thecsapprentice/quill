//! Automated moderation (#43) at each hook, through the API writers use.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use quill_core::{Client, CoreError};
use quill_e2e::TestServer;
use quill_proto::announcements::{CreateAnnouncement, CreateComment};
use quill_proto::api::CreatePersona;
use quill_proto::automod::{OnError, UpdateChain, UploadScript};
use quill_proto::board::{
    CreatePrompt, CreateReport, Maturity, ReportReason, TagPolicy, UpdatePrompt,
};
use quill_server::AppState;
use quill_server::moderation::{Hook, chains, scripts};
use uuid::Uuid;

struct Setup {
    server: TestServer,
    state: AppState,
    admin: Client,
    admin_id: Uuid,
}

async fn setup(tweak: impl FnOnce(&mut quill_server::Config)) -> Option<Setup> {
    let server = TestServer::start_with(tweak).await.unwrap()?;
    let state = server.state().await.unwrap();
    let admin = server.admin_account("boss").await.unwrap();
    let admin_id = sqlx::query_scalar("SELECT id FROM accounts WHERE login_name = 'boss'")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    Some(Setup {
        server,
        state,
        admin,
        admin_id,
    })
}

impl Setup {
    /// Puts one script, alone, in `hook`'s chain.
    async fn rule(&self, hook: Hook, name: &str, source: &str) {
        let upload = UploadScript {
            name: name.into(),
            source: source.into(),
            log_only: false,
        };
        let id = scripts::upload(&self.state, self.admin_id, &upload)
            .await
            .unwrap()
            .id;
        let chain = UpdateChain {
            on_error: OnError::Continue,
            scripts: vec![id],
        };
        chains::update(&self.state, self.admin_id, hook, &chain)
            .await
            .unwrap();
    }

    async fn writer(&self, name: &str) -> (Client, Uuid) {
        let client = self.server.account(name).await.unwrap();
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
}

fn prompt(persona: Uuid, title: &str, body: &str, tags: &[&str]) -> CreatePrompt {
    CreatePrompt {
        persona,
        title: title.into(),
        body: body.into(),
        maturity: Maturity::General,
        tags: tags.iter().map(|t| (*t).to_owned()).collect(),
        state: None,
    }
}

/// The problem a refused request comes back with: its type and message.
fn refusal<T: std::fmt::Debug>(r: Result<T, CoreError>) -> String {
    match r {
        Err(CoreError::Api(p)) => {
            assert!(p.kind.ends_with("/content-rejected"), "{p:?}");
            assert_eq!(p.status, 422);
            p.detail.unwrap_or_default()
        }
        other => panic!("{other:?}"),
    }
}

const NO_LINKS: &str = r#"
    for link in text::links(post.body) {
        return reject("Links to outside sites aren't allowed in prompts.");
    }"#;

#[tokio::test]
async fn a_refused_prompt_is_not_saved() {
    let Some(s) = setup(|_| {}).await else { return };
    s.rule(Hook::PromptCreated, "No links", NO_LINKS).await;
    let (wren, persona) = s.writer("wren").await;

    let message = refusal(
        wren.create_prompt(&prompt(
            persona,
            "Write with me",
            "Join me at evil.biz/x",
            &[],
        ))
        .await,
    );
    assert_eq!(message, "Links to outside sites aren't allowed in prompts.");
    let saved: i64 = sqlx::query_scalar("SELECT count(*) FROM prompts")
        .fetch_one(&s.state.pool)
        .await
        .unwrap();
    assert_eq!(saved, 0);

    // Fixing it goes through.
    wren.create_prompt(&prompt(persona, "Write with me", "Message me here.", &[]))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_refused_edit_leaves_the_old_version_live() {
    let Some(s) = setup(|_| {}).await else { return };
    s.rule(Hook::PromptEdited, "No links", NO_LINKS).await;
    let (wren, persona) = s.writer("wren").await;
    let p = wren
        .create_prompt(&prompt(persona, "A story", "The original text.", &[]))
        .await
        .unwrap();

    let edit = UpdatePrompt {
        body: Some("Now with evil.biz/x".into()),
        ..UpdatePrompt::default()
    };
    refusal(wren.update_prompt(p.id, &edit).await);
    assert_eq!(wren.prompt(p.id).await.unwrap().body, "The original text.");

    // Changing only open/busy/closed isn't an edit, so isn't checked.
    let state_only = UpdatePrompt {
        state: Some(quill_proto::board::PromptState::Busy),
        ..UpdatePrompt::default()
    };
    wren.update_prompt(p.id, &state_only).await.unwrap();
}

#[tokio::test]
async fn an_edit_sees_the_previous_version() {
    let Some(s) = setup(|_| {}).await else { return };
    s.rule(
        Hook::PromptEdited,
        "No retitling",
        r#"if post.previous.title != post.title { reject("Titles can't change.") }"#,
    )
    .await;
    let (wren, persona) = s.writer("wren").await;
    let p = wren
        .create_prompt(&prompt(persona, "A story", "Text.", &[]))
        .await
        .unwrap();
    let body_only = UpdatePrompt {
        body: Some("New text.".into()),
        ..UpdatePrompt::default()
    };
    wren.update_prompt(p.id, &body_only).await.unwrap();
    let retitle = UpdatePrompt {
        title: Some("Another".into()),
        ..UpdatePrompt::default()
    };
    assert_eq!(
        refusal(wren.update_prompt(p.id, &retitle).await),
        "Titles can't change."
    );
}

#[tokio::test]
async fn a_flag_lets_the_prompt_through_and_files_a_report() {
    let Some(s) = setup(|_| {}).await else { return };
    s.rule(
        Hook::PromptCreated,
        "Shouting",
        r#"if text::caps_ratio(post.body) > 0.8 { flag("Mostly capitals", "spam") }"#,
    )
    .await;
    let (wren, persona) = s.writer("wren").await;
    let p = wren
        .create_prompt(&prompt(persona, "Loud", "WRITE WITH ME NOW", &[]))
        .await
        .unwrap();

    let reports = s.admin.admin_reports(false).await.unwrap();
    assert_eq!(reports.len(), 1);
    let r = &reports[0];
    assert_eq!(r.prompt.as_ref().map(|p| p.id), Some(p.id));
    assert_eq!(r.script.as_deref(), Some("Shouting"));
    assert_eq!(r.details.as_deref(), Some("Mostly capitals"));
    assert_eq!(r.reason, ReportReason::Spam);

    // Once per version: posting the same edit loop again adds nothing.
    let same = UpdatePrompt {
        title: Some("Loud".into()),
        ..UpdatePrompt::default()
    };
    wren.update_prompt(p.id, &same).await.unwrap();
    assert_eq!(s.admin.admin_reports(false).await.unwrap().len(), 1);
}

#[tokio::test]
async fn comments_are_checked() {
    let Some(s) = setup(|_| {}).await else { return };
    s.rule(
        Hook::CommentCreated,
        "Words",
        r#"if text::find_words(["scam"], post.body).len() > 0 { reject("Not here, please.") }"#,
    )
    .await;
    let a = s
        .admin
        .admin_post_announcement(&CreateAnnouncement {
            title: "News".into(),
            body: "Hello.".into(),
            comments_open: true,
            sticky: false,
        })
        .await
        .unwrap();
    let (wren, persona) = s.writer("wren").await;
    let comment = |body: &str| CreateComment {
        voice: Default::default(),
        persona: Some(persona),
        body: body.into(),
    };
    assert_eq!(
        refusal(wren.post_comment(a.id, &comment("total s c a m")).await),
        "Not here, please."
    );
    wren.post_comment(a.id, &comment("Lovely news."))
        .await
        .unwrap();

    // The notice names the announcement, not the refused comment.
    let fields: serde_json::Value =
        sqlx::query_scalar("SELECT fields FROM account_notices WHERE kind = 'content_rejected'")
            .fetch_one(&s.state.pool)
            .await
            .unwrap();
    assert_eq!(fields["refused"], "comment");
    assert_eq!(fields["title"], "News");
}

#[tokio::test]
async fn a_new_tag_can_be_flagged_in_the_queue() {
    let Some(s) = setup(|c| c.tag_policy = TagPolicy::Approval).await else {
        return;
    };
    s.rule(
        Hook::TagRequested,
        "Tag names",
        r#"if post.tag.display.len() > 20 { flag("A very long tag name") }"#,
    )
    .await;
    let (wren, persona) = s.writer("wren").await;
    wren.create_prompt(&prompt(
        persona,
        "A story",
        "Text.",
        &["theme:an extremely long tag name"],
    ))
    .await
    .unwrap();
    let (flag_script, flag_reason): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT flag_script, flag_reason FROM tags WHERE status = 'pending'")
            .fetch_one(&s.state.pool)
            .await
            .unwrap();
    assert_eq!(flag_script.as_deref(), Some("Tag names"));
    assert_eq!(flag_reason.as_deref(), Some("A very long tag name"));
}

#[tokio::test]
async fn a_report_can_be_dismissed() {
    let Some(s) = setup(|_| {}).await else { return };
    s.rule(
        Hook::PromptReported,
        "Old accounts",
        r#"if post.report.reason == "spam" && post.author.removals == 0 && post.history.prompts_last_day < 2 {
               dismiss("A single prompt from a writer with a clean record")
           }"#,
    )
    .await;
    let (wren, persona) = s.writer("wren").await;
    let p = wren
        .create_prompt(&prompt(persona, "A story", "Text.", &[]))
        .await
        .unwrap();
    let (reporter, _) = s.writer("rook").await;
    reporter
        .report(&CreateReport {
            prompt: p.id,
            reason: ReportReason::Spam,
            details: None,
        })
        .await
        .unwrap();

    // Filed, but closed: nothing open in the inbox, and it's in the log.
    assert!(s.admin.admin_reports(false).await.unwrap().is_empty());
    let all = s.admin.admin_reports(true).await.unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[0].resolved_at.is_some());
    let logged: String =
        sqlx::query_scalar("SELECT script_name FROM admin_log WHERE action = 'automod_dismiss'")
            .fetch_one(&s.state.pool)
            .await
            .unwrap();
    assert_eq!(logged, "Old accounts");
}

#[tokio::test]
async fn administrators_are_not_checked() {
    let Some(s) = setup(|_| {}).await else { return };
    s.rule(Hook::PromptCreated, "Everything", "reject()").await;
    let persona = s
        .admin
        .create_persona(&CreatePersona {
            handle: "boss_pen".into(),
            display_name: None,
            summary: None,
        })
        .await
        .unwrap();
    s.admin
        .create_prompt(&prompt(persona.id, "Welcome", "Hello.", &[]))
        .await
        .unwrap();

    let (wren, persona) = s.writer("wren").await;
    let message = refusal(
        wren.create_prompt(&prompt(persona, "Hi", "Hello.", &[]))
            .await,
    );
    assert_eq!(message, quill_proto::automod::GENERIC_REFUSAL);
}
