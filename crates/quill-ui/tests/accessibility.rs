//! Every screen keeps the design language's accessibility rules
//! (docs/design/gui-design-language.md, "Accessibility"): buttons have
//! names, fields have labels, invalid fields say why, ids are unique, one
//! h1, images are labelled. Each test walks a part of the app and checks
//! each screen it reaches.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::rc::Rc;

use quill_proto::{
    board::{Maturity, ReportReason},
    notices::{NoticeFields, NoticeKind},
};
use quill_ui::testing::{Acceptance, Driver, Sim};

const SERVER: &str = "writers.example";
const PASSWORD: &str = "correct horse battery";

/// Through first run to the frame, checking each screen on the way.
async fn first_run(sim: &Rc<Sim>) -> Driver {
    let mut d = Driver::new(sim.clone());
    d.assert_accessible("the server screen");
    d.type_into("server-address", SERVER).await;
    d.wait_for_text("Takes new logins").await;
    d.assert_accessible("the server screen, server found");
    d.click("Continue").await;
    d.wait_for_text("Make a login on").await;
    d.assert_accessible("the sign-up screen");
    d.type_into("login-name", "ink").await;
    d.type_into("login-password", "short").await;
    d.assert_accessible("the sign-up screen, password too short");
    d.type_into("login-password", PASSWORD).await;
    d.click("Make my login").await;
    d.wait_for_text("Make your first persona").await;
    d.assert_accessible("the first persona screen");
    d.type_into("persona-handle", "ink_fox").await;
    d.click("Make this persona").await;
    d.wait_for_text("Hello, @ink_fox").await;
    d.assert_accessible("Home");
    d
}

#[tokio::test]
async fn first_run_and_the_frame() {
    let sim = Sim::new().with_server(SERVER).rc();
    sim.set_rules(SERVER, "# Be kind\n\nNo spam.", Acceptance::Never);
    let mut d = first_run(&sim).await;
    d.click("Personas").await;
    d.assert_accessible("Personas");
    d.click("Edit").await;
    d.assert_accessible("Personas, editing");
    d.click("Settings").await;
    d.assert_accessible("Settings");
    d.set_checked("advanced", true).await;
    d.wait_for_text("Details").await;
    d.assert_accessible("Settings, advanced");
    d.click("Read the rules").await;
    d.wait_for_text("No spam.").await;
    d.assert_accessible("Server rules");
    d.click("Settings").await;
    d.click("Your devices").await;
    d.wait_for_text("Only this computer so far.").await;
    d.assert_accessible("Devices");
    d.click("Settings").await;
    d.click("Design reference").await;
    d.wait_for_text("Every part of the app's design language")
        .await;
    d.assert_accessible("the design reference");
}

#[tokio::test]
async fn the_board() {
    let sim = Sim::new().with_server(SERVER).rc();
    sim.fill_board(SERVER, 5);
    sim.post_as(
        SERVER,
        "marrow",
        "Something odd",
        "Text.",
        &[],
        Maturity::General,
    );
    let mut d = first_run(&sim).await;
    d.click("Board").await;
    d.wait_until("the list", |d| !d.has_text("Loading prompts"))
        .await;
    d.assert_accessible("the Board");
    d.click_containing("Something odd").await;
    d.assert_accessible("the Board, reading");
    d.click("Report this prompt").await;
    d.set_checked(&format!("reason-{}", ReportReason::Spam), true)
        .await;
    d.type_into("report-details", &"x".repeat(2_001)).await;
    d.assert_accessible("the report form, details too long");
    d.click("Filters").await;
    d.assert_accessible("the filter builder");
    d.click("Genre").await;
    d.wait_until("the genre tags", |d| d.has_field("any-genre-horror"))
        .await;
    d.assert_accessible("the filter builder, one type");
    d.click("‹ Back to the board").await;
    d.click("Post a prompt").await;
    d.wait_for_text("Add tags").await;
    d.assert_accessible("the post form");
    d.type_into("post-title", &"t".repeat(201)).await;
    d.assert_accessible("the post form, title too long");
    d.click("Cancel").await;
    d.click("My prompts").await;
    d.wait_until("my prompts", |d| !d.has_text("Loading prompts"))
        .await;
    d.assert_accessible("My prompts");
}

#[tokio::test]
async fn notices() {
    let sim = Sim::new().with_server(SERVER).rc();
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
    let mut d = first_run(&sim).await;
    d.click_containing("Notices").await;
    d.wait_for_text("wasn't approved").await;
    d.assert_accessible("Notices");
}

#[tokio::test]
async fn linking_a_computer() {
    let sim = Sim::new().with_server(SERVER).rc();
    sim.existing_login(SERVER, "ink", PASSWORD, &["ink_fox"]);
    sim.other_device(SERVER, "ink", "Laptop", &["ink_fox"]);
    let mut d = Driver::new(sim.clone());
    d.type_into("server-address", SERVER).await;
    d.wait_for_text("Takes new logins").await;
    d.click("Continue").await;
    d.wait_for_text("Make a login on").await;
    d.click("I have a login").await;
    d.assert_accessible("the sign-in screen");
    d.type_into("login-name", "ink").await;
    d.type_into("login-password", PASSWORD).await;
    d.click("Sign in").await;
    d.wait_for_text("This computer is new to your login").await;
    d.assert_accessible("keys elsewhere");
    d.click("Link this computer").await;
    d.assert_accessible("naming this computer");
    d.click("Show the link code").await;
    d.wait_for_text("Type these words on your other device")
        .await;
    d.assert_accessible("the link code");
    sim.approve_from_another_device(SERVER, true);
    d.wait_for_text("Check the words match").await;
    d.assert_accessible("comparing words");
    d.click("They don't match").await;
    d.assert_accessible("nothing was linked");
}

#[tokio::test]
async fn unlocking() {
    let sim = Sim::new().with_server(SERVER).without_keychain().rc();
    let mut d = Driver::new(sim.clone());
    d.type_into("server-address", SERVER).await;
    d.wait_for_text("Takes new logins").await;
    d.click("Continue").await;
    d.wait_for_text("choose a passphrase").await;
    d.assert_accessible("choosing a passphrase");
    d.type_into("store-passphrase", "a long store passphrase")
        .await;
    d.click("Continue").await;
    d.wait_for_text("Make a login on").await;
    drop(d);
    sim.relock();
    let mut d = Driver::new(sim.clone());
    d.wait_for_text("Unlock").await;
    d.assert_accessible("unlocking");
}

#[tokio::test]
async fn requests() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = first_run(&sim).await;
    let mine = sim.post_as(
        SERVER,
        "ink_fox",
        "My prompt",
        "Text.",
        &[],
        Maturity::General,
    );
    let first = sim.request_from(SERVER, "wren", mine, "A pitch.");
    d.wait_for_text("A request is waiting").await;
    d.assert_accessible("Home, a request waiting");
    d.click("Read them").await;
    d.wait_for_text("A pitch.").await;
    d.assert_accessible("a prompt's requests");
    d.click("Decline…").await;
    d.assert_accessible("a prompt's requests, declining");
    sim.withdraw_elsewhere(SERVER, first);
    sim.change_key(SERVER, "wren");
    sim.request_from(SERVER, "wren", mine, "Again.");
    d.wait_for_text("identity key changed").await;
    d.assert_accessible("a prompt's requests, a changed key");
    d.click("‹ All requests").await;
    d.assert_accessible("Requests");

    sim.post_as(
        SERVER,
        "gull",
        "Their prompt",
        "Text too.",
        &[],
        Maturity::General,
    );
    d.click("Board").await;
    d.wait_for_text("Their prompt").await;
    d.click_containing("Their prompt").await;
    d.click("Ask to write together").await;
    d.assert_accessible("asking to write");
    d.type_into("request-pitch", &"x".repeat(4_001)).await;
    d.assert_accessible("asking to write, pitch too long");
    d.click("Cancel").await;
    d.click("Compare words with @gull").await;
    d.wait_for_text("@gull sees these same 12 words").await;
    d.assert_accessible("comparing words on a prompt");

    d.click("Personas").await;
    d.type_into("words-handle", "gull").await;
    d.click("Show the words").await;
    d.wait_for_text("@gull sees these same 12 words").await;
    d.assert_accessible("Personas, comparing words");
}
