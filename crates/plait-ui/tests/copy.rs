//! The brand's weaving words stay a layer around the interface
//! (docs/rebrand-plait.md, "Vocabulary"): an empty state says plainly what's
//! missing first, and its flavour line only follows; the name is explained
//! once, on the welcome card, closed until asked.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::rc::Rc;

use plait_proto::board::Maturity;
use plait_ui::testing::{Driver, Sim};

const SERVER: &str = "writers.example";

async fn signed_up(sim: &Rc<Sim>) -> Driver {
    let mut d = Driver::new(sim.clone());
    d.type_into("server-address", SERVER).await;
    d.wait_for_text("Takes new logins").await;
    d.click("Continue").await;
    d.wait_for_text("Make a login on").await;
    d.type_into("login-name", "ink").await;
    d.type_into("login-password", "correct horse battery").await;
    d.click("Make my login").await;
    d.wait_for_text("Make your first persona").await;
    d.type_into("persona-handle", "ink_fox").await;
    d.click("Make this persona").await;
    d.wait_for_text("Hello, @ink_fox").await;
    d
}

/// The plain line is there, and the flavour line comes after it.
fn plain_then_flavour(d: &Driver, plain: &str, flavour: &str) {
    let text = d.text();
    let at = |s: &str| text.find(s).unwrap_or_else(|| panic!("no {s:?} in {text}"));
    assert!(at(plain) < at(flavour), "{plain:?} comes first");
}

#[tokio::test]
async fn the_welcome_explains_the_name_only_there() {
    let sim = Sim::new().with_server(SERVER).rc();
    let d = Driver::new(sim.clone());
    assert!(d.has_element_with_class("why-plait"));
    assert!(d.has_text("Why Plait?"));
    assert!(d.has_text("a prompt is the warp"));
    let d = signed_up(&sim).await;
    assert!(!d.has_text("Why Plait?"), "nothing later depends on it");
}

#[tokio::test]
async fn empty_states_say_what_is_missing_before_the_flavour() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = signed_up(&sim).await;

    d.click("Board").await;
    d.wait_for_text("No prompts on the board yet.").await;
    plain_then_flavour(
        &d,
        "No prompts on the board yet.",
        "The warp is strung and waiting.",
    );

    d.click("Requests").await;
    d.wait_for_text("You haven't asked to write on a prompt yet.")
        .await;
    plain_then_flavour(
        &d,
        "No requests are waiting on your prompts.",
        "The shuttle is at rest.",
    );

    d.click("Sessions").await;
    d.wait_for_text("No sessions yet.").await;
    plain_then_flavour(&d, "No sessions yet.", "Nothing on the loom yet.");
}

#[tokio::test]
async fn the_flavour_goes_once_something_is_there() {
    let sim = Sim::new().with_server(SERVER).rc();
    sim.post_as(
        SERVER,
        "marrow",
        "The ledger",
        "Text.",
        &[],
        Maturity::General,
    );
    let mut d = signed_up(&sim).await;
    d.click("Board").await;
    d.wait_for_text("The ledger").await;
    assert!(!d.has_text("The warp is strung"));
}
