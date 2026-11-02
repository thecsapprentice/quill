//! Requests (3e) driven as a writer would, against the simulated server:
//! asking to write from the reader, reading, accepting and declining what
//! waits on your prompts, held requests, a changed identity key on either
//! side, answers on Home, live arrivals, and what goes wrong on the way.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::rc::Rc;

use quill_proto::board::{Maturity, PromptState};
use quill_ui::testing::{Driver, RequestState, Sim};
use uuid::Uuid;

const SERVER: &str = "writers.example";
const PASSWORD: &str = "correct horse battery";
const PITCH: &str = "I'd write the keeper. She never opens the ledger's last page.";

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

/// Another writer's prompt, open on the Board's reader.
async fn reading(sim: &Rc<Sim>, d: &mut Driver, handle: &str, title: &str) -> Uuid {
    let id = sim.post_as(
        SERVER,
        handle,
        title,
        "A keeper finds a ledger.",
        &[],
        Maturity::General,
    );
    d.click("Board").await;
    d.wait_for_text(title).await;
    d.click_containing(title).await;
    d.wait_for_text("A keeper finds a ledger.").await;
    id
}

/// One of `@ink_fox`'s own prompts.
fn own_prompt(sim: &Rc<Sim>, title: &str) -> Uuid {
    sim.post_as(
        SERVER,
        "ink_fox",
        title,
        "Write the night the light failed.",
        &[],
        Maturity::General,
    )
}

fn states(sim: &Rc<Sim>) -> Vec<(String, RequestState)> {
    sim.requests_on(SERVER)
        .into_iter()
        .map(|(_, h, _, s)| (h, s))
        .collect()
}

// ---- Asking ------------------------------------------------------------------

#[tokio::test]
async fn asking_to_write_sends_a_pitch_that_waits_for_an_answer() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    reading(&sim, &mut d, "wren", "The lighthouse ledger").await;
    d.click("Ask to write together").await;
    assert!(d.has_text("Asking as @ink_fox."));
    assert!(d.has_text("only @wren's devices can read it"));
    // The button waits, and says for what.
    assert!(d.button_disabled("Send request"));
    assert!(d.has_text("Needs a pitch before sending."));
    d.type_into("request-pitch", PITCH).await;
    assert!(d.has_text("of 4,000 characters"));
    d.choose_labelled("request-days", "7 days").await;
    d.click("Send request").await;
    d.wait_for_text("Request sent.").await;
    assert_eq!(states(&sim), [("ink_fox".into(), RequestState::Pending)]);

    d.click("See your requests").await;
    d.wait_for_text("You sent").await;
    assert_eq!(d.heading().as_deref(), Some("Requests"));
    assert!(d.has_text("“The lighthouse ledger”"));
    assert!(d.has_text("as @ink_fox"));
    assert!(d.has_text("Waiting for an answer, until"));
}

#[tokio::test]
async fn the_pitch_limit_is_said_while_typing() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    reading(&sim, &mut d, "wren", "The lighthouse ledger").await;
    d.click("Ask to write together").await;
    d.type_into("request-pitch", &"x".repeat(4_001)).await;
    assert!(d.has_text("4,001 of 4,000 characters: 1 too many."));
    assert!(d.button_disabled("Send request"));
    d.type_into("request-pitch", &"x".repeat(4_000)).await;
    assert!(!d.button_disabled("Send request"));
}

#[tokio::test]
async fn a_second_request_on_the_same_prompt_is_refused_in_plain_words() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    reading(&sim, &mut d, "wren", "The lighthouse ledger").await;
    d.click("Ask to write together").await;
    d.type_into("request-pitch", PITCH).await;
    d.click("Send request").await;
    d.wait_for_text("Request sent.").await;
    // Back to the board and the same prompt again.
    d.click("‹ Back to the board").await;
    d.click_containing("The lighthouse ledger").await;
    d.click("Ask to write together").await;
    d.type_into("request-pitch", "Another go.").await;
    d.click("Send request").await;
    d.wait_for_text("You already have a request waiting on this prompt.")
        .await;
    // What was typed is kept.
    assert_eq!(
        d.attr("request-pitch", "value").as_deref(),
        Some("Another go.")
    );
    assert_eq!(sim.requests_on(SERVER).len(), 1);
}

#[tokio::test]
async fn closed_prompts_and_your_own_offer_no_request() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    let id = reading(&sim, &mut d, "wren", "The lighthouse ledger").await;
    sim.set_prompt_state(SERVER, id, PromptState::Closed);
    d.click("‹ Back to the board").await;
    d.click_containing("The lighthouse ledger").await;
    d.settle().await;
    assert!(!d.has_button("Ask to write together"));

    own_prompt(&sim, "My own prompt");
    d.click("‹ Back to the board").await;
    d.click("Refresh").await;
    d.wait_for_text("My own prompt").await;
    d.click_containing("My own prompt").await;
    d.wait_for_text("Write the night the light failed.").await;
    assert!(!d.has_button("Ask to write together"));
}

#[tokio::test]
async fn sending_while_offline_says_so_and_keeps_the_pitch() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    reading(&sim, &mut d, "wren", "The lighthouse ledger").await;
    d.click("Ask to write together").await;
    d.type_into("request-pitch", PITCH).await;
    sim.server(SERVER, |s| s.reachable = false);
    d.click("Send request").await;
    d.wait_for_text("Can't reach writers.example right now.")
        .await;
    assert_eq!(d.attr("request-pitch", "value").as_deref(), Some(PITCH));
    sim.server(SERVER, |s| s.reachable = true);
    d.click("Send request").await;
    d.wait_for_text("Request sent.").await;
}

// ---- On your prompts -----------------------------------------------------------

#[tokio::test]
async fn requests_on_your_prompts_are_read_accepted_and_declined() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    let id = own_prompt(&sim, "The lighthouse ledger");
    sim.request_from(SERVER, "wren", id, PITCH);
    sim.request_from(SERVER, "gull", id, "**Gulls** know the coast.");
    // Home says so, live, with a count in the side bar.
    d.wait_for_text("2 requests are waiting on “The lighthouse ledger”.")
        .await;
    assert!(d.has_button("Requests 2 new"));

    d.click("Read them").await;
    d.wait_for_text(PITCH).await;
    assert_eq!(
        d.heading().as_deref(),
        Some("Requests on “The lighthouse ledger”")
    );
    // Pitches are Markdown, drawn by the app.
    assert!(d.has_text("Gulls know the coast."));
    d.click_nth("Accept", 0).await;
    d.wait_for_text("Accepted @wren's request: it's a writing session now")
        .await;
    d.click("Decline…").await;
    assert!(d.has_text("Decline @gull's request? They aren't told"));
    d.click("Decline this request").await;
    d.wait_for_text("Declined @gull's request.").await;
    d.wait_for_text("No requests are waiting on this prompt now.")
        .await;
    assert_eq!(
        states(&sim),
        [
            ("wren".into(), RequestState::Accepted),
            ("gull".into(), RequestState::Declined)
        ]
    );
    d.click("‹ All requests").await;
    assert!(d.has_text("No requests are waiting on your prompts."));
}

#[tokio::test]
async fn the_reader_leads_from_your_prompt_to_its_requests() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    let id = own_prompt(&sim, "The lighthouse ledger");
    sim.request_from(SERVER, "wren", id, PITCH);
    d.wait_for_text("A request is waiting on “The lighthouse ledger”.")
        .await;
    d.click("Board").await;
    d.click_containing("The lighthouse ledger").await;
    d.wait_for_text("A request to write together is waiting on it.")
        .await;
    d.click("Read them").await;
    d.wait_for_text(PITCH).await;
}

#[tokio::test]
async fn held_requests_are_counted_and_shown_when_asked() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    let id = own_prompt(&sim, "The lighthouse ledger");
    sim.request_from(SERVER, "wren", id, PITCH);
    sim.set_prompt_state(SERVER, id, PromptState::Busy);
    sim.request_from(SERVER, "gull", id, "Held while busy.");
    // A held request doesn't fire a notice: the next look finds it.
    sim.live();
    d.wait_for_text("2 requests are waiting").await;
    d.click("Requests 2 new").await;
    d.wait_for_text("1 request waiting. 1 request more arrived while it was busy.")
        .await;
    d.click("Read the requests on “The lighthouse ledger”")
        .await;
    d.wait_for_text(PITCH).await;
    assert!(!d.has_text("Held while busy."));
    d.click("Show them").await;
    d.wait_for_text("Held while busy.").await;
    assert_eq!(
        states(&sim),
        [
            ("wren".into(), RequestState::Pending),
            ("gull".into(), RequestState::Pending)
        ]
    );
}

#[tokio::test]
async fn a_request_arriving_while_reading_the_queue_shows_up() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    let id = own_prompt(&sim, "The lighthouse ledger");
    sim.request_from(SERVER, "wren", id, PITCH);
    d.wait_for_text("A request is waiting").await;
    d.click("Read them").await;
    d.wait_for_text(PITCH).await;
    sim.request_from(SERVER, "gull", id, "Late, but keen.");
    d.wait_for_text("Late, but keen.").await;
}

#[tokio::test]
async fn a_request_sent_before_linking_says_where_to_answer_it() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    let id = own_prompt(&sim, "The lighthouse ledger");
    sim.request_before_link(SERVER, "wren", id, PITCH);
    d.wait_for_text("A request is waiting").await;
    d.click("Read them").await;
    d.wait_for_text("only your other devices can read it").await;
    assert!(!d.has_text(PITCH));
    assert!(!d.has_button("Accept"));
    assert!(d.has_button("Decline…"));
}

#[tokio::test]
async fn answering_offline_or_after_a_withdrawal_says_what_happened() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    let id = own_prompt(&sim, "The lighthouse ledger");
    let wren = sim.request_from(SERVER, "wren", id, PITCH);
    d.wait_for_text("A request is waiting").await;
    d.click("Read them").await;
    d.wait_for_text(PITCH).await;

    sim.server(SERVER, |s| s.reachable = false);
    d.click("Accept").await;
    d.wait_for_text("Can't reach writers.example right now.")
        .await;
    assert_eq!(states(&sim), [("wren".into(), RequestState::Pending)]);

    sim.server(SERVER, |s| s.reachable = true);
    sim.withdraw_elsewhere(SERVER, wren);
    d.click("Accept").await;
    d.wait_for_text("That request is gone: it was withdrawn, or it lapsed.")
        .await;
}

// ---- A changed key ---------------------------------------------------------------

#[tokio::test]
async fn a_pitch_from_a_changed_key_waits_until_the_words_are_compared() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    let id = own_prompt(&sim, "The lighthouse ledger");
    let first = sim.request_from(SERVER, "wren", id, PITCH);
    d.wait_for_text("A request is waiting").await;
    d.click("Read them").await;
    d.wait_for_text(PITCH).await;

    // Wren starts over on a new computer and asks again.
    sim.withdraw_elsewhere(SERVER, first);
    sim.change_key(SERVER, "wren");
    sim.request_from(SERVER, "wren", id, "Asking again.");
    d.wait_for_text("@wren's identity key changed").await;
    assert!(!d.has_text("Asking again."));
    for w in sim.key_words(SERVER, "ink_fox", "wren") {
        assert!(d.has_text(&w), "the words to compare: {w}");
    }
    // Not now: it stays unread, and can still be declined.
    d.click("Not now").await;
    assert!(d.has_text("Not read: @wren's identity key changed"));
    assert!(d.has_button("Decline…"));
    assert!(!d.has_button("Accept"));

    d.click("Check it now").await;
    d.wait_for_text("@wren's identity key changed").await;
    d.click("The words match: carry on").await;
    d.wait_for_text("Asking again.").await;
    d.click("Accept").await;
    d.wait_for_text("Accepted @wren's request").await;
}

#[tokio::test]
async fn sending_to_a_changed_key_stops_until_it_is_checked() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    reading(&sim, &mut d, "wren", "The lighthouse ledger").await;
    d.click("Ask to write together").await;
    d.type_into("request-pitch", PITCH).await;
    d.click("Send request").await;
    d.wait_for_text("Request sent.").await;

    sim.change_key(SERVER, "wren");
    sim.post_as(
        SERVER,
        "wren",
        "A second prompt",
        "More ledgers.",
        &[],
        Maturity::General,
    );
    d.click("‹ Back to the board").await;
    d.click("Refresh").await;
    d.wait_for_text("A second prompt").await;
    d.click_containing("A second prompt").await;
    d.wait_for_text("More ledgers.").await;
    d.click("Ask to write together").await;
    d.type_into("request-pitch", "Another one.").await;
    d.click("Send request").await;
    d.wait_for_text("@wren's identity key changed").await;
    assert!(d.has_text("started over on a new computer"));
    assert_eq!(sim.requests_on(SERVER).len(), 1, "nothing sent yet");
    // Not now: back to the form, nothing lost.
    d.click("Not now").await;
    assert_eq!(
        d.attr("request-pitch", "value").as_deref(),
        Some("Another one.")
    );

    d.click("Send request").await;
    d.wait_for_text("@wren's identity key changed").await;
    d.click("The words match: carry on").await;
    d.wait_for_text("Request sent.").await;
    assert_eq!(sim.requests_on(SERVER).len(), 2);
}

// ---- What you sent -----------------------------------------------------------------

async fn sent_one(sim: &Rc<Sim>, d: &mut Driver) -> Uuid {
    reading(sim, d, "wren", "The lighthouse ledger").await;
    d.click("Ask to write together").await;
    d.type_into("request-pitch", PITCH).await;
    d.click("Send request").await;
    d.wait_for_text("Request sent.").await;
    sim.requests_on(SERVER)[0].0
}

#[tokio::test]
async fn an_answer_shows_on_home_until_requests_is_opened() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    let id = sent_one(&sim, &mut d).await;
    d.click("Home").await;
    sim.answer_request(SERVER, id, true);
    d.wait_for_text("Your request on “The lighthouse ledger” was accepted")
        .await;
    assert!(d.has_button("Requests 1 new"));

    d.click("See your requests").await;
    d.wait_for_text("Writing together").await;
    d.click("Home").await;
    assert!(!d.has_text("was accepted"));
    assert!(d.has_button("Requests"));

    // Seen stays seen when the app opens again.
    drop(d);
    let d = Driver::new(sim.clone());
    let mut d = d;
    d.wait_for_text("Hello, @ink_fox").await;
    d.settle().await;
    assert!(!d.has_text("was accepted"));
}

#[tokio::test]
async fn a_declined_request_still_reads_as_waiting_then_lapses() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    let id = sent_one(&sim, &mut d).await;
    sim.answer_request(SERVER, id, false);
    d.click("See your requests").await;
    d.wait_for_text("You sent").await;
    // A requester is never told "declined".
    assert!(d.has_text("Waiting for an answer"));
    assert!(!d.has_text("eclined"));

    sim.expire_requests(SERVER);
    d.click("Home").await;
    sim.live();
    d.wait_for_text("got no answer before it lapsed").await;
    d.click("See your requests").await;
    d.wait_for_text("No answer").await;
    d.click("Dismiss").await;
    d.wait_for_text("You haven't asked to write on a prompt yet.")
        .await;
    assert!(sim.requests_on(SERVER).is_empty());
}

#[tokio::test]
async fn withdrawing_asks_once_then_withdraws() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    sent_one(&sim, &mut d).await;
    d.click("See your requests").await;
    d.wait_for_text("You sent").await;
    d.click("Withdraw…").await;
    assert!(d.has_text("Withdraw your request on “The lighthouse ledger”?"));
    d.click("Keep it").await;
    assert_eq!(sim.requests_on(SERVER).len(), 1);
    d.click("Withdraw…").await;
    d.click("Withdraw this request").await;
    d.wait_for_text("You haven't asked to write on a prompt yet.")
        .await;
    assert!(sim.requests_on(SERVER).is_empty());
}

#[tokio::test]
async fn a_request_on_a_deleted_prompt_says_so() {
    let sim = sim();
    let mut d = signed_in(&sim).await;
    sent_one(&sim, &mut d).await;
    let prompt = sim.requests_on(SERVER)[0].2;
    sim.delete_elsewhere(SERVER, prompt);
    d.click("See your requests").await;
    d.wait_for_text("A prompt that was deleted").await;
}
