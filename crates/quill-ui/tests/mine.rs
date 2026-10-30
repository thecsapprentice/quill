//! My prompts and Notices (3c) driven as a writer would, against the
//! simulated server: the login's own prompts by persona, removed ones
//! marked, and notices counted, read and followed to their prompts.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::rc::Rc;

use quill_proto::{
    board::{Maturity, ReportReason},
    notices::{NoticeFields, NoticeKind},
};
use quill_ui::testing::{Driver, Sim};

const SERVER: &str = "writers.example";
const PASSWORD: &str = "correct horse battery";

/// Signs in as `ink`, whose personas are `@ink_fox` and `@ink_tales`.
async fn signed_in(sim: &Rc<Sim>) -> Driver {
    let mut d = Driver::new(sim.clone());
    d.type_into("server-address", SERVER).await;
    d.wait_for_text("Takes new logins").await;
    d.click("Continue").await;
    d.wait_for_text("Make a login on").await;
    d.click("I have a login").await;
    d.type_into("login-name", "ink").await;
    d.type_into("login-password", PASSWORD).await;
    d.click("Sign in").await;
    d.wait_for_text("This computer is new to your login").await;
    d.click("Not now").await;
    d.wait_for_text("Connected to writers.example").await;
    d
}

fn sim() -> Rc<Sim> {
    let sim = Sim::new().with_server(SERVER).rc();
    sim.existing_login(SERVER, "ink", PASSWORD, &["ink_fox", "ink_tales"]);
    sim
}

#[tokio::test]
async fn my_prompts_lists_only_mine_and_filters_by_persona() {
    let sim = sim();
    sim.post_as(
        SERVER,
        "ink_fox",
        "Foxes at dusk",
        "Text.",
        &[],
        Maturity::General,
    );
    sim.post_as(
        SERVER,
        "ink_tales",
        "Tales from the pier",
        "Text.",
        &[],
        Maturity::General,
    );
    sim.post_as(
        SERVER,
        "marrow",
        "Someone else's",
        "Text.",
        &[],
        Maturity::General,
    );
    let mut d = signed_in(&sim).await;
    d.click("My prompts").await;
    d.wait_for_text("Foxes at dusk").await;
    assert!(d.has_text("Tales from the pier"));
    assert!(!d.has_text("Someone else's"), "only the login's own");
    assert!(d.has_text("All my personas (2)"));

    d.choose_labelled("mine-persona", "@ink_tales").await;
    assert!(d.has_text("Tales from the pier"));
    assert!(!d.has_text("Foxes at dusk"));
    d.choose_labelled("mine-persona", "All my personas (2)")
        .await;
    assert!(d.has_text("Foxes at dusk"));

    // The Board's search and filters aren't here.
    assert!(!d.has_field("board-search"));
    assert!(!d.has_button("Filters"));
}

#[tokio::test]
async fn a_removed_prompt_shows_only_in_my_prompts_marked_as_removed() {
    let sim = sim();
    let id = sim.post_as(
        SERVER,
        "ink_fox",
        "Taken down",
        "Text.",
        &[],
        Maturity::General,
    );
    sim.post_as(
        SERVER,
        "ink_fox",
        "Still up",
        "Text.",
        &[],
        Maturity::General,
    );
    sim.remove_prompt(SERVER, id);
    let mut d = signed_in(&sim).await;
    d.click("Board").await;
    d.wait_for_text("Still up").await;
    assert!(!d.has_text("Taken down"), "off the board");

    d.click("My prompts").await;
    d.wait_for_text("Taken down").await;
    assert!(d.has_text("· removed"));
    d.click_containing("Taken down").await;
    d.wait_for_text("‹ Back to the board").await;
    assert!(d.has_text("removed"), "the reader says so too");
}

#[tokio::test]
async fn my_prompts_shows_changes_made_on_the_board() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    d.click("My prompts").await;
    d.wait_for_text("You haven't posted a prompt yet.").await;
    // Posted from elsewhere (another device): a new visit picks it up.
    sim.post_as(
        SERVER,
        "ink_fox",
        "From the laptop",
        "Text.",
        &[],
        Maturity::General,
    );
    d.click("Board").await;
    d.click("My prompts").await;
    d.wait_for_text("From the laptop").await;
}

#[tokio::test]
async fn my_prompts_and_notices_need_a_signed_in_login() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    assert!(d.has_button("My prompts"));
    d.click("Settings").await;
    d.click("Sign out of writers.example").await;
    d.wait_for_text("Welcome back").await;
    d.click("Not now").await;
    d.wait_for_text("Signed out of writers.example").await;
    assert!(d.has_button("Board"), "the board stays open to read");
    assert!(!d.has_button("My prompts"));
    assert!(!d.has_button_containing("Notices"));
}

#[tokio::test]
async fn unread_notices_are_counted_then_read_on_opening() {
    let sim = sim();
    sim.notify(
        SERVER,
        "ink",
        NoticeKind::PromptRemoved,
        NoticeFields {
            reason: Some(ReportReason::Spam),
            ..NoticeFields::default()
        },
        &[("Buy my book", "ink_fox")],
    );
    sim.notify(
        SERVER,
        "ink",
        NoticeKind::TagRejected,
        NoticeFields {
            tag: Some("theme:Moonlite".into()),
            ..NoticeFields::default()
        },
        &[],
    );
    let mut d = signed_in(&sim).await;
    d.wait_until("the count", |d| d.has_button("Notices 2 unread"))
        .await;
    d.click("Notices 2 unread").await;
    d.wait_for_text("An administrator removed your prompt")
        .await;
    assert!(d.has_text("Spam"), "the reason's category");
    assert!(d.has_text("wasn't approved"));
    assert!(d.has_text("New"), "marked new on this visit");
    // Read now: the count goes, here and on the server.
    d.wait_until("the count cleared", |d| d.has_button("Notices"))
        .await;
    d.click("Board").await;
    d.click("Notices").await;
    d.wait_for_text("An administrator removed your prompt")
        .await;
    assert!(!d.has_text("New"), "not new a second time");
}

#[tokio::test]
async fn a_notice_opens_its_prompt_in_my_prompts() {
    let sim = sim();
    let id = sim.post_as(
        SERVER,
        "ink_fox",
        "Buy my book",
        "Read it.",
        &[],
        Maturity::General,
    );
    sim.remove_prompt(SERVER, id);
    sim.notify_about(
        SERVER,
        "ink",
        NoticeKind::PromptRemoved,
        NoticeFields::default(),
        id,
    );
    let mut d = signed_in(&sim).await;
    d.click_containing("Notices").await;
    d.wait_for_text("Only you can still see it").await;
    d.click("Buy my book").await;
    d.wait_for_text("Read it.").await;
    assert_eq!(d.heading().as_deref(), Some("My prompts"));
}

#[tokio::test]
async fn no_notices_says_so() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    d.click("Notices").await;
    d.wait_for_text("Nothing yet.").await;
}

#[tokio::test]
async fn older_notices_come_a_page_at_a_time() {
    let sim = sim();
    for i in 0..55 {
        sim.notify(
            SERVER,
            "ink",
            NoticeKind::TagDeleted,
            NoticeFields {
                tag: Some(format!("theme:Gone {i:02}")),
                ..NoticeFields::default()
            },
            &[],
        );
    }
    let mut d = signed_in(&sim).await;
    d.click_containing("Notices").await;
    d.wait_for_text("Gone 54").await;
    assert!(!d.has_text("Gone 04"), "one page first");
    d.click("Older notices").await;
    d.wait_for_text("Gone 04").await;
    assert!(d.has_text("Gone 00"));
    assert!(!d.has_button("Older notices"), "that was the last");
}

#[tokio::test]
async fn notices_say_when_the_server_cannot_be_reached() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    sim.server(SERVER, |s| s.reachable = false);
    d.click("Notices").await;
    d.wait_for_text("Try again").await;
    sim.server(SERVER, |s| s.reachable = true);
    d.click("Try again").await;
    d.wait_for_text("Nothing yet.").await;
}
