//! Demo automated moderation: common data, scripts and a chain for every
//! hook, then a few posts that show each outcome. It runs after the demo
//! content is posted, so the seeder's own burst of prompts isn't checked.

use anyhow::{Context, Result, bail};
use quill_proto::{
    automod::{Hook, OnError, SaveModerationData, SetScriptMode, UpdateChain, UploadScript},
    board::{CreatePrompt, CreateReport, Maturity, PromptState, ReportReason},
};
use quill_server::{
    AppState,
    error::ApiError,
    moderation::{chains, data, scripts, templates},
    prompts, reports,
};
use uuid::Uuid;

/// The first version of the shared word list. A second version is saved
/// on top, so the entry has a history to look at and undo.
const BLOCKED_WORDS_V1: &str = "\
# Words and phrases to refuse, one per line.
# Read by \"Blocked words and phrases\" (prompts, edits, comments) and by
# \"Tag names\" (new tags), so both follow any change made here.
free crypto
buy followers
";
const BLOCKED_WORDS_V2: &str = "\
# Words and phrases to refuse, one per line.
# Read by \"Blocked words and phrases\" (prompts, edits, comments) and by
# \"Tag names\" (new tags), so both follow any change made here.
free crypto
buy followers
click here to win
";
const ALLOWED_SITES: &str = "\
# Sites posts may link to, one per line. Subdomains are allowed too.
archiveofourown.org
wikipedia.org
";

/// A custom script that reuses the template's word list for new tags.
const TAG_NAMES: &str = r#"// Tag names
//
// Refuses a new tag whose name uses a word from "blocked_words", the same
// list the "Blocked words and phrases" script reads for posts.

let found = text::find_words(common::lines("blocked_words"), post.tag.display);
if found.len() > 0 {
    return reject("That tag name isn't allowed here. Please pick another.");
}
"#;

/// A custom script for the report chain.
const EMPTY_REPORTS: &str = r#"// Reports with nothing to go on
//
// Closes "other" reports that say nothing about what's wrong, so they
// don't wait in the inbox. Each one is still in the admin log.

if post.report.reason == "other" && post.report.details == "" {
    return dismiss("An \"other\" report with no details");
}
"#;

/// What the demo set up and what happened to the demo posts, for the
/// seeder's summary.
pub struct Summary {
    pub scripts: usize,
    pub refused: Vec<String>,
}

/// Sets everything up as `admin`, then posts the demo submissions as
/// `writer` with `persona`, and a report as `reporter`.
pub async fn seed(
    state: &AppState,
    admin: Uuid,
    writer: Uuid,
    persona: Uuid,
    reporter: Uuid,
    user_tags: bool,
) -> Result<Summary> {
    save_data(
        state,
        admin,
        "blocked_words",
        "Words and phrases to refuse.",
        BLOCKED_WORDS_V1,
    )
    .await?;
    save_data(
        state,
        admin,
        "blocked_words",
        "Words and phrases to refuse.",
        BLOCKED_WORDS_V2,
    )
    .await?;
    save_data(
        state,
        admin,
        "allowed_sites",
        "Sites posts may link to.",
        ALLOWED_SITES,
    )
    .await?;

    let template = |id: &'static str| async move {
        templates::create_from(state, admin, id, None)
            .await
            .with_context(|| format!("creating a script from the {id} template"))
            .map(|s| s.id)
    };
    let blocked = template("blocked_words").await?;
    let contact = template("contact_details").await?;
    let links = template("outside_links").await?;
    let new_accounts = template("new_accounts").await?;
    let duplicates = template("duplicates").await?;
    let shouting = template("shouting").await?;
    // Log-only: it runs and is logged, but never acts. A way to watch a
    // script before trusting it.
    let alphabets = template("mixed_alphabets").await?;
    scripts::set_mode(
        state,
        admin,
        alphabets,
        &SetScriptMode {
            enabled: None,
            log_only: Some(true),
        },
    )
    .await
    .context("making Mixed alphabets log-only")?;
    let tag_names = upload(state, admin, "Tag names", TAG_NAMES).await?;
    let empty_reports =
        upload(state, admin, "Reports with nothing to go on", EMPTY_REPORTS).await?;

    let chain = |hook, on_error, scripts: Vec<Uuid>| async move {
        chains::update(state, admin, hook, &UpdateChain { on_error, scripts })
            .await
            .with_context(|| format!("setting up the {hook} chain"))
    };
    let new_prompt = vec![
        blocked,
        contact,
        links,
        new_accounts,
        duplicates,
        alphabets,
        shouting,
    ];
    chain(Hook::PromptCreated, OnError::Flag, new_prompt).await?;
    chain(
        Hook::PromptEdited,
        OnError::Continue,
        vec![blocked, contact, links, shouting],
    )
    .await?;
    chain(
        Hook::CommentCreated,
        OnError::Continue,
        vec![blocked, contact, links],
    )
    .await?;
    chain(Hook::TagRequested, OnError::Reject, vec![tag_names]).await?;
    chain(Hook::PromptReported, OnError::Continue, vec![empty_reports]).await?;

    let refused = demo_posts(state, writer, persona, reporter, user_tags).await?;
    Ok(Summary {
        scripts: scripts::list(&state.pool).await?.len(),
        refused,
    })
}

async fn save_data(
    state: &AppState,
    admin: Uuid,
    name: &str,
    description: &str,
    content: &str,
) -> Result<()> {
    let body = SaveModerationData {
        description: description.to_owned(),
        content: content.to_owned(),
        base_version: None,
    };
    data::save(state, admin, name, &body)
        .await
        .with_context(|| format!("saving common data {name}"))?;
    Ok(())
}

async fn upload(state: &AppState, admin: Uuid, name: &str, source: &str) -> Result<Uuid> {
    let body = UploadScript {
        name: name.to_owned(),
        source: source.to_owned(),
        log_only: false,
    };
    let script = scripts::upload(state, admin, &body)
        .await
        .with_context(|| format!("uploading script {name:?}"))?;
    Ok(script.id)
}

/// Posts that show each outcome: some refused (a new tag too, if the server
/// allows new tags), one flagged to the inbox, one seen by the log-only
/// script, and a report closed by the report chain. Returns the messages
/// the refused posts got.
async fn demo_posts(
    state: &AppState,
    writer: Uuid,
    persona: Uuid,
    reporter: Uuid,
    user_tags: bool,
) -> Result<Vec<String>> {
    let prompt = |title: &str, body: &str| CreatePrompt {
        persona,
        title: title.to_owned(),
        body: body.to_owned(),
        maturity: Maturity::General,
        tags: vec![
            "genre:comedy".into(),
            "format:prose".into(),
            "length:short".into(),
        ],
        state: Some(PromptState::Open),
    };
    let mut refusals = vec![
        // "Blocked words" sees through the spacing.
        prompt(
            "A quick favour",
            "F R E E crypto for anyone who co-writes with me!",
        ),
        // "Phone numbers and email addresses".
        prompt(
            "Looking for a partner",
            "Text me on +1 (555) 010-7788 and we'll plan it.",
        ),
    ];
    if user_tags {
        // "Tag names" refuses the new tag from the same blocked_words list.
        let mut tagged = prompt(
            "A heist in Low Market",
            "Three thieves, one ledger, no plan.",
        );
        tagged.tags.push("theme:Buy followers".into());
        refusals.push(tagged);
    }
    let mut refused = Vec::new();
    for body in refusals {
        match prompts::post_prompt(state, writer, &body).await {
            Err(ApiError::ContentRejected(message)) => refused.push(message),
            Ok(_) => bail!("the demo prompt {:?} should have been refused", body.title),
            Err(e) => return Err(e).context("posting a demo prompt"),
        }
    }
    // "Shouting" flags it: it's posted, and a report waits in the inbox.
    prompts::post_prompt(
        state,
        writer,
        &prompt(
            "Anyone around tonight?",
            "PLEASE WRITE WITH ME RIGHT NOW, I HAVE THE WHOLE PLOT READY AND NOBODY ELSE WANTS IT",
        ),
    )
    .await
    .context("posting the demo prompt for Shouting")?;
    // "Mixed alphabets" is log-only: the Cyrillic о in "lоg" is logged, nothing more.
    let logged = prompts::post_prompt(
        state,
        writer,
        &prompt(
            "The keeper's logbook",
            "Every entry in the lighthouse lоg ends mid-sentence. Write the night it starts making sense.",
        ),
    )
    .await
    .context("posting the demo prompt for Mixed alphabets")?;
    // "Reports with nothing to go on" closes it as it's filed.
    reports::file_report(
        state,
        Some(reporter),
        &CreateReport {
            prompt: logged.id,
            reason: ReportReason::Other,
            details: None,
        },
    )
    .await
    .context("filing the demo report")?;
    Ok(refused)
}
