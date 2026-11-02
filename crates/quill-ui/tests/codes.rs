//! Error codes (#76) driven as a writer would: every problem has one, and
//! the app shows it beside the words only in advanced mode. Covers a
//! server refusal, a refusal decided on this computer, being offline, and
//! an inline reason a pitch can't be shown.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::rc::Rc;

use quill_proto::board::Maturity;
use quill_ui::{
    backend::codes,
    testing::{Driver, Sim},
};

const SERVER: &str = "writers.example";
const PASSWORD: &str = "correct horse battery";

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

async fn advanced(d: &mut Driver, on: bool) {
    d.click("Settings").await;
    d.set_checked("advanced", on).await;
}

#[tokio::test]
async fn a_server_refusal_shows_its_code_only_in_advanced_mode() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = signed_in(&sim).await;
    sim.post_as(
        SERVER,
        "wren",
        "The ledger",
        "A keeper's ledger.",
        &[],
        Maturity::General,
    );
    d.click("Board").await;
    d.wait_for_text("The ledger").await;
    d.click_containing("The ledger").await;
    d.click("Ask to write together").await;
    d.type_into("request-pitch", "The keeper.").await;
    d.click("Send request").await;
    d.wait_for_text("Request sent.").await;
    let code = codes::REQUEST_ALREADY_WAITING.to_string();

    // A second request: refused in words, with no code by default.
    d.click("‹ Back to the board").await;
    d.click_containing("The ledger").await;
    d.click("Ask to write together").await;
    d.type_into("request-pitch", "Again.").await;
    d.click("Send request").await;
    d.wait_for_text("You already have a request waiting").await;
    assert!(!d.has_text(&code), "no code outside advanced mode");

    advanced(&mut d, true).await;
    d.click("Board").await;
    d.click_containing("The ledger").await;
    d.click("Ask to write together").await;
    d.type_into("request-pitch", "Again.").await;
    d.click("Send request").await;
    d.wait_for_text("You already have a request waiting").await;
    assert!(d.has_text(&format!(
        "waiting on this prompt. Withdraw it to send another. {code}"
    )));
}

#[tokio::test]
async fn a_refusal_decided_here_shows_its_code_in_advanced_mode() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = signed_in(&sim).await;
    advanced(&mut d, true).await;
    d.click("Rename").await;
    let id = d.field_id_with_prefix("rename-");
    d.type_into(&id, "").await;
    d.click("Save").await;
    d.wait_for_text("Use 1 to 60 characters.").await;
    assert!(d.has_text(&codes::SERVER_NAME_BAD.to_string()));
}

#[tokio::test]
async fn offline_shows_its_code_in_the_status_line_in_advanced_mode() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = signed_in(&sim).await;
    let code = codes::UNREACHABLE.to_string();
    sim.server(SERVER, |s| s.reachable = false);
    sim.time_passes();
    d.wait_for_text("Can't reach writers.example. Trying again")
        .await;
    assert!(!d.has_text(&code));
    advanced(&mut d, true).await;
    assert!(d.has_text(&format!("Trying again in a moment. {code}")));
}

#[tokio::test]
async fn an_inline_reason_shows_its_code_in_advanced_mode() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = signed_in(&sim).await;
    let id = sim.post_as(SERVER, "ink_fox", "Mine", "Text.", &[], Maturity::General);
    sim.request_before_link(SERVER, "wren", id, "A pitch.");
    let code = codes::REQUEST_NOT_SENT_HERE.to_string();
    d.wait_for_text("A request is waiting").await;
    d.click("Read them").await;
    d.wait_for_text("only your other devices can read it").await;
    assert!(!d.has_text(&code));

    advanced(&mut d, true).await;
    // Back to the queue, still open where it was.
    d.click("Requests 1 new").await;
    d.wait_for_text("only your other devices can read it").await;
    assert!(d.has_text(&format!("Answer it there. {code}")));
}
