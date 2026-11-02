//! Sessions (3f-1) driven as a writer would, against the simulated server:
//! a request becoming a session, the list, reading channels, notes
//! arriving live with unread counts on Home and in the side bar, answers,
//! edits and deletions, a standing other than joined, and an ended session.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::rc::Rc;

use quill_ui::{
    backend::{SessionStanding, codes},
    testing::{Asked, Driver, Sim},
};

const SERVER: &str = "writers.example";
const PASSWORD: &str = "correct horse battery";

fn sim() -> Rc<Sim> {
    Sim::new().with_server(SERVER).rc()
}

/// A fresh install signed in as `ink`, writing as `@ink_fox`, on Home.
async fn signed_in(sim: &Rc<Sim>) -> Driver {
    let mut d = Driver::new(sim.clone());
    d.type_into("server-address", SERVER).await;
    d.wait_for_text("Takes new logins").await;
    d.click("Continue").await;
    d.wait_for_text("Make a login on").await;
    d.type_into("login-name", "ink").await;
    d.type_into("login-password", PASSWORD).await;
    d.click("Make my login").await;
    d.wait_for_text("Make your first persona").await;
    d.type_into("persona-handle", "ink_fox").await;
    d.click("Make this persona").await;
    d.wait_for_text("Hello, @ink_fox").await;
    d
}

/// Signed in, with a session with `@wren` on "The ledger"; on Sessions.
async fn with_a_session(sim: &Rc<Sim>) -> (Driver, String) {
    let mut d = signed_in(sim).await;
    sim.take_handle(SERVER, "wren");
    let id = sim.session_with(SERVER, "ink_fox", "wren", "The ledger");
    d.click("Sessions").await;
    d.wait_for_text("On “The ledger”").await;
    (d, id)
}

#[tokio::test]
async fn no_sessions_yet_says_how_one_starts() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    d.click("Sessions").await;
    d.wait_for_text("No sessions yet.").await;
    d.click("Find a prompt on the Board").await;
    assert_eq!(d.heading().as_deref(), Some("Board"));
}

#[tokio::test]
async fn an_accepted_request_becomes_a_session_that_opens_with_the_pitch() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    sim.post_as(
        SERVER,
        "wren",
        "The ledger",
        "A keeper's ledger.",
        &[],
        quill_proto::board::Maturity::General,
    );
    d.click("Board").await;
    d.wait_for_text("The ledger").await;
    d.click_containing("The ledger").await;
    d.click("Ask to write together").await;
    d.type_into("request-pitch", "I'd write the **keeper**.")
        .await;
    d.click("Send request").await;
    d.wait_for_text("Request sent.").await;
    let request = sim.requests_on(SERVER)[0].0;
    sim.answer_request(SERVER, request, true);

    d.click("Sessions").await;
    d.wait_for_text("On “The ledger”").await;
    d.click("Open the session with @wren on “The ledger”").await;
    d.wait_for_text("Writing with @wren").await;
    assert_eq!(d.heading().as_deref(), Some("Writing with @wren"));
    // Story is empty; the pitch opened Planning.
    assert!(d.has_text("Nothing in Story yet."));
    d.click("Planning").await;
    d.wait_for_text("I'd write the").await;
    assert!(d.has_text("You"));
}

#[tokio::test]
async fn notes_arrive_live_and_their_counts_clear_once_read() {
    let sim = sim();
    let (mut d, id) = with_a_session(&sim).await;
    d.click("Home").await;
    sim.write_as(SERVER, &id, "wren", "story", "The lamp went out at *nine*.");
    d.wait_for_text("1 new note in your session with @wren on “The ledger”.")
        .await;
    assert!(d.has_button("Sessions 1 unread"));

    d.click("Read the session with @wren on “The ledger”").await;
    d.wait_for_text("The lamp went out at").await;
    // Read here, so the counts go.
    d.wait_until("the count clears", |d| d.has_button("Sessions"))
        .await;

    // Another arrives while reading: it shows without a reload.
    sim.write_as(SERVER, &id, "wren", "story", "Then the bell.");
    d.wait_for_text("Then the bell.").await;
    // A note in another channel marks that tab.
    sim.write_as(SERVER, &id, "wren", "planning", "Shall we keep it short?");
    d.wait_until("the planning tab's count", |d| {
        d.has_button("Planning 1 unread")
    })
    .await;
    d.click("Planning 1 unread").await;
    d.wait_for_text("Shall we keep it short?").await;
    d.wait_until("the tab's count clears", |d| d.has_button("Planning"))
        .await;
}

#[tokio::test]
async fn named_channels_answers_edits_and_deletions_show() {
    let sim = sim();
    let (mut d, id) = with_a_session(&sim).await;
    let first = sim.write_as(SERVER, &id, "wren", "story", "Chapter one begins at sea.");
    sim.name_channel(SERVER, &id, "research", "Ships and tides.");
    sim.reply_as(
        SERVER,
        &id,
        "wren",
        "research",
        "Tide tables for 1890.",
        first,
    );
    let edited = sim.write_as(SERVER, &id, "wren", "story", "A first try.");
    sim.edit_note(SERVER, &id, edited, "A second try.");
    let gone = sim.write_as(SERVER, &id, "wren", "story", "Never mind this.");
    sim.delete_note(SERVER, &id, gone);

    d.click("Open the session with @wren on “The ledger”").await;
    d.wait_for_text("Chapter one begins at sea.").await;
    assert!(d.has_text("A second try."));
    assert!(d.has_text("Edited"));
    assert!(!d.has_text("Never mind this."));
    assert!(d.has_text("This note was deleted."));

    d.click("Research 1 unread").await;
    d.wait_for_text("Ships and tides.").await;
    assert!(d.has_text("Tide tables for 1890."));
    // The answer leads back to its note, in Story.
    let _ = d.asked_of(&format!("note-{first}"));
    d.click("In answer to: Chapter one begins at sea.").await;
    d.wait_for_text("A second try.").await;
    assert_eq!(d.asked_of(&format!("note-{first}")), [Asked::IntoView]);
}

#[tokio::test]
async fn a_session_this_computer_cant_read_says_why() {
    let sim = sim();
    let (mut d, id) = with_a_session(&sim).await;
    sim.set_session_standing(&id, SessionStanding::Waiting);
    d.click("Board").await;
    d.click("Sessions").await;
    d.wait_for_text("This computer isn't in it yet.").await;
    d.click("Open the session with @wren on “The ledger”").await;
    d.wait_for_text("isn't in the session yet").await;

    // A failed check shows its code in advanced mode.
    sim.set_session_standing(
        &id,
        SessionStanding::Broken {
            code: codes::SESSION_CHECK_FAILED,
            detail: "a forged commit".into(),
        },
    );
    d.click("‹ All sessions").await;
    d.click("Settings").await;
    d.set_checked("advanced", true).await;
    d.click("Sessions").await;
    d.wait_for_text("stopped reading it after a check failed")
        .await;
    assert!(d.has_text(&codes::SESSION_CHECK_FAILED.to_string()));
}

#[tokio::test]
async fn an_ended_session_stays_readable() {
    let sim = sim();
    let (mut d, id) = with_a_session(&sim).await;
    sim.write_as(SERVER, &id, "wren", "story", "Goodbye, keeper.");
    sim.leave_as(SERVER, &id, "wren");
    d.click("Board").await;
    d.click("Sessions 1 unread").await;
    d.wait_for_text("Ended").await;
    assert!(d.has_text("The other writer left"));
    d.click_containing("Open").await;
    d.wait_for_text("This session has ended.").await;
    assert!(d.has_text("Goodbye, keeper."));
}

#[tokio::test]
async fn accepting_a_request_on_your_prompt_starts_a_session() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    let id = sim.post_as(
        SERVER,
        "ink_fox",
        "Mine",
        "Text.",
        &[],
        quill_proto::board::Maturity::General,
    );
    sim.request_from(SERVER, "wren", id, "Let me write the *sea*.");
    d.wait_for_text("A request is waiting").await;
    d.click("Read them").await;
    d.wait_for_text("Let me write the").await;
    d.click("Accept").await;
    d.wait_for_text("Accepted @wren's request").await;
    d.click("Sessions").await;
    d.wait_for_text("On “Mine”").await;
    assert_eq!(sim.sessions_of(SERVER, "ink").len(), 1);
}
