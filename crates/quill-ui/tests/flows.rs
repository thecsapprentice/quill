//! The app driven as a person would: typing into fields and clicking
//! buttons by their labels, against a simulated server
//! (`quill_ui::testing`). Each test is one thing a writer does, start to
//! finish, including what can go wrong on the way.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::rc::Rc;

use quill_ui::testing::{Acceptance, Driver, Sim};

const SERVER: &str = "writers.example";
const OTHER: &str = "other.example";
const PASSWORD: &str = "correct horse battery";
const PASSPHRASE: &str = "a long store passphrase";
const RULES: &str = "# Be kind\n\nNo **spam**. <script>alert(1)</script>";

fn app(sim: &Rc<Sim>) -> Driver {
    Driver::new(sim.clone())
}

/// From a fresh install to the sign-up screen for `domain`.
async fn to_sign_up(d: &mut Driver, domain: &str) {
    d.type_into("server-address", domain).await;
    d.wait_for_text("Takes new logins").await;
    d.click("Continue").await;
    d.wait_for_text(&format!("Make a login on {domain}")).await;
}

/// Makes a login on the open server's sign-up screen.
async fn make_login(d: &mut Driver, login: &str) {
    d.type_into("login-name", login).await;
    d.type_into("login-password", PASSWORD).await;
    d.click("Make my login").await;
}

/// From a fresh install to the frame with a ready persona.
async fn first_run(d: &mut Driver, domain: &str, login: &str, handle: &str) {
    to_sign_up(d, domain).await;
    make_login(d, login).await;
    d.wait_for_text("Make your first persona").await;
    d.type_into("persona-handle", handle).await;
    d.click("Make this persona").await;
    d.wait_for_text(&format!("Hello, @{handle}")).await;
}

// ---- First run ----------------------------------------------------------------

#[tokio::test]
async fn a_new_writer_gets_from_nothing_to_writing() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = app(&sim);
    assert_eq!(d.heading().as_deref(), Some("Welcome to Quill"));
    assert!(d.button_disabled("Continue"), "nothing typed yet");

    first_run(&mut d, SERVER, "ink", "ink_fox").await;
    assert!(d.has_text("Connected to writers.example"));
    assert!(d.has_text("Writing as"));
    assert_eq!(sim.logins(SERVER), ["ink"]);
}

#[tokio::test]
async fn a_server_that_cannot_be_reached_is_said_gently_while_typing() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = app(&sim);
    d.type_into("server-address", "nowhere.example").await;
    d.wait_for_text("No Quill server answers there yet").await;
    assert!(d.button_disabled("Continue"));
    d.type_into("server-address", SERVER).await;
    d.wait_for_text("Takes new logins").await;
    assert!(!d.button_disabled("Continue"));
}

#[tokio::test]
async fn without_a_keychain_the_first_run_asks_for_a_passphrase() {
    let sim = Sim::new().with_server(SERVER).without_keychain().rc();
    let mut d = app(&sim);
    d.type_into("server-address", SERVER).await;
    d.wait_for_text("Takes new logins").await;
    d.click("Continue").await;
    d.wait_for_text("choose a passphrase").await;
    d.type_into("store-passphrase", "short").await;
    assert!(d.has_text("more characters to go"));
    d.click("Continue").await;
    assert!(d.has_text("choose a passphrase"), "too short: still asking");
    d.type_into("store-passphrase", PASSPHRASE).await;
    d.click("Continue").await;
    d.wait_for_text("Make a login on writers.example").await;
}

#[tokio::test]
async fn names_say_what_is_wrong_as_you_type() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = app(&sim);
    to_sign_up(&mut d, SERVER).await;
    d.type_into("login-name", "Ink Fox").await;
    assert!(d.has_text("No spaces. Use _ to join words."));
    d.type_into("login-password", "eleven char").await;
    assert!(d.has_text("1 more character to go."));
    d.click("Make my login").await;
    assert_eq!(sim.logins(SERVER), Vec::<String>::new(), "nothing sent");
}

#[tokio::test]
async fn a_taken_login_name_offers_others() {
    let sim = Sim::new().with_server(SERVER).rc();
    sim.existing_login(SERVER, "ink", PASSWORD, &[]);
    let mut d = app(&sim);
    to_sign_up(&mut d, SERVER).await;
    make_login(&mut d, "ink").await;
    d.wait_for_text("That login name is taken").await;
    d.click("ink_writes").await;
    d.type_into("login-password", PASSWORD).await;
    d.click("Make my login").await;
    d.wait_for_text("Make your first persona").await;
    assert!(sim.logins(SERVER).contains(&"ink_writes".to_owned()));
}

#[tokio::test]
async fn a_taken_handle_offers_others_and_one_can_be_picked() {
    let sim = Sim::new().with_server(SERVER).rc();
    sim.take_handle(SERVER, "ink_fox");
    let mut d = app(&sim);
    to_sign_up(&mut d, SERVER).await;
    make_login(&mut d, "ink").await;
    d.wait_for_text("Make your first persona").await;
    d.type_into("persona-handle", "ink_fox").await;
    d.click("Make this persona").await;
    d.wait_for_text("@ink_fox is taken").await;
    d.click("@the_ink_fox").await;
    d.click("Make this persona").await;
    d.wait_for_text("Hello, @the_ink_fox").await;
}

#[tokio::test]
async fn skipping_the_first_persona_is_followed_up_gently() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = app(&sim);
    to_sign_up(&mut d, SERVER).await;
    make_login(&mut d, "ink").await;
    d.wait_for_text("Make your first persona").await;
    d.click("Not now").await;
    d.wait_for_text("Writing and posting need a persona").await;
    d.click("Make a persona").await;
    // The Personas page, with the form open.
    d.wait_until("the persona form", |d| d.has_field("persona-handle"))
        .await;
    d.type_into("persona-handle", "ink_owl").await;
    d.click("Make this persona").await;
    // Made from the Personas page: it's listed there, ready and in use.
    d.wait_for_text("@ink_owl In use").await;
    d.click("Home").await;
    assert!(d.has_text("Hello, @ink_owl"));
    assert!(!d.has_text("Writing and posting need a persona"));
}

#[tokio::test]
async fn closed_registration_goes_straight_to_signing_in() {
    let sim = Sim::new().with_server(SERVER).rc();
    sim.existing_login(SERVER, "ink", PASSWORD, &[]);
    sim.server(SERVER, |s| s.registration_open = false);
    let mut d = app(&sim);
    d.type_into("server-address", SERVER).await;
    d.wait_for_text("Not taking new logins").await;
    d.click("Continue").await;
    d.wait_for_text("Sign in to writers.example").await;
    assert!(d.has_text("isn't taking new logins right now"));
    assert!(!d.has_button("I'm new here"));
    d.type_into("login-name", "ink").await;
    d.type_into("login-password", "not the password").await;
    d.click("Sign in").await;
    d.wait_for_text("don't match").await;
    d.type_into("login-password", PASSWORD).await;
    d.click("Sign in").await;
    // No persona on this login yet: it's suggested, skippably.
    d.wait_for_text("Make your first persona").await;
    d.click("Not now").await;
    d.wait_for_text("Welcome to writers.example").await;
}

#[tokio::test]
async fn signing_in_where_the_keys_are_elsewhere_says_so_once() {
    let sim = Sim::new().with_server(SERVER).rc();
    sim.existing_login(SERVER, "ink", PASSWORD, &["ink_fox"]);
    let mut d = app(&sim);
    to_sign_up(&mut d, SERVER).await;
    d.click("I have a login").await;
    d.type_into("login-name", "ink").await;
    d.type_into("login-password", PASSWORD).await;
    d.click("Sign in").await;
    d.wait_for_text("This computer is new to your login").await;
    assert!(d.has_text("@ink_fox"));
    d.click("Continue").await;
    d.wait_for_text("Connected to writers.example").await;
}

// ---- Server rules (#42) ---------------------------------------------------

#[tokio::test]
async fn rules_required_on_joining_need_the_box_ticked() {
    let sim = Sim::new().with_server(SERVER).rc();
    sim.set_rules(SERVER, RULES, Acceptance::OnJoin);
    let mut d = app(&sim);
    to_sign_up(&mut d, SERVER).await;
    d.wait_for_text("I've read and accept the server rules")
        .await;
    assert!(d.has_text("No spam"), "the rules can be read here");
    // Raw HTML in the rules stays text: no script element.
    assert!(d.has_text("<script>alert(1)</script>"));
    assert!(!d.has_element("script"));
    d.type_into("login-name", "ink").await;
    d.type_into("login-password", PASSWORD).await;
    assert!(d.button_disabled("Make my login"), "not ticked yet");
    d.set_checked("accept-rules", true).await;
    assert!(!d.button_disabled("Make my login"));
    d.set_checked("accept-rules", false).await;
    assert!(d.button_disabled("Make my login"), "unticked again");
    d.set_checked("accept-rules", true).await;
    d.click("Make my login").await;
    d.wait_for_text("Make your first persona").await;
    assert_eq!(sim.accepted(SERVER, "ink"), 1);
}

#[tokio::test]
async fn rules_required_after_the_sign_up_screen_opened_still_get_a_box() {
    // The bug: the screen asked once, before the administrator required
    // acceptance, and then had no way to accept.
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = app(&sim);
    to_sign_up(&mut d, SERVER).await;
    assert!(!d.has_text("server rules"), "no rules yet");
    sim.set_rules(SERVER, RULES, Acceptance::OnJoin);

    make_login(&mut d, "ink").await;
    d.wait_for_text("Accept the server rules to create an account.")
        .await;
    d.wait_for_text("I've read and accept the server rules")
        .await;
    assert!(d.button_disabled("Make my login"));
    d.set_checked("accept-rules", true).await;
    d.click("Make my login").await;
    d.wait_for_text("Make your first persona").await;
    assert_eq!(sim.accepted(SERVER, "ink"), 1);
}

#[tokio::test]
async fn rules_that_change_during_sign_up_are_shown_again() {
    let sim = Sim::new().with_server(SERVER).rc();
    sim.set_rules(SERVER, "Version one.", Acceptance::OnJoin);
    let mut d = app(&sim);
    to_sign_up(&mut d, SERVER).await;
    d.wait_for_text("I've read and accept the server rules")
        .await;
    d.type_into("login-name", "ink").await;
    d.type_into("login-password", PASSWORD).await;
    d.set_checked("accept-rules", true).await;
    sim.set_rules(SERVER, "Version two.", Acceptance::OnJoin);
    d.click("Make my login").await;
    d.wait_for_text("changed while you were signing up").await;
    d.wait_for_text("Version two.").await;
    assert!(
        d.button_disabled("Make my login"),
        "the new version needs ticking"
    );
    d.set_checked("accept-rules", true).await;
    d.click("Make my login").await;
    d.wait_for_text("Make your first persona").await;
    assert_eq!(sim.accepted(SERVER, "ink"), 2);
}

#[tokio::test]
async fn rules_never_needing_acceptance_are_there_to_read() {
    let sim = Sim::new().with_server(SERVER).rc();
    sim.set_rules(SERVER, RULES, Acceptance::Never);
    let mut d = app(&sim);
    to_sign_up(&mut d, SERVER).await;
    d.wait_for_text("Read the server rules").await;
    assert!(d.has_text("By making a login you agree"));
    assert!(!d.has_field("accept-rules"));
    make_login(&mut d, "ink").await;
    d.wait_for_text("Make your first persona").await;
}

#[tokio::test]
async fn rules_asked_for_after_changes_are_accepted_when_joining_too() {
    // Otherwise a new login would be blocked from posting at once, told
    // the rules "have changed".
    let sim = Sim::new().with_server(SERVER).rc();
    sim.set_rules(SERVER, RULES, Acceptance::OnChange);
    let mut d = app(&sim);
    to_sign_up(&mut d, SERVER).await;
    d.wait_for_text("I've read and accept the server rules")
        .await;
    d.type_into("login-name", "ink").await;
    d.type_into("login-password", PASSWORD).await;
    assert!(d.button_disabled("Make my login"));
    d.set_checked("accept-rules", true).await;
    d.click("Make my login").await;
    d.wait_for_text("Make your first persona").await;
    d.click("Not now").await;
    d.wait_for_text("Connected to writers.example").await;
    assert!(!d.has_text("The server rules have changed"));
    assert_eq!(sim.accepted(SERVER, "ink"), 1);
}

#[tokio::test]
async fn rules_that_fail_to_load_are_never_a_dead_end() {
    let sim = Sim::new().with_server(SERVER).rc();
    sim.set_rules(SERVER, RULES, Acceptance::OnJoin);
    sim.server(SERVER, |s| s.rules_fail = true);
    let mut d = app(&sim);
    to_sign_up(&mut d, SERVER).await;
    d.wait_for_text("Quill couldn't load them").await;
    d.type_into("login-name", "ink").await;
    d.type_into("login-password", PASSWORD).await;
    assert!(d.button_disabled("Make my login"));
    sim.server(SERVER, |s| s.rules_fail = false);
    d.click("Load the rules").await;
    d.wait_for_text("I've read and accept the server rules")
        .await;
    d.set_checked("accept-rules", true).await;
    d.click("Make my login").await;
    d.wait_for_text("Make your first persona").await;
}

#[tokio::test]
async fn changed_rules_show_a_banner_until_accepted() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = app(&sim);
    first_run(&mut d, SERVER, "ink", "ink_fox").await;
    sim.set_rules(SERVER, "# New rules\n\nBe kind.", Acceptance::OnChange);
    // Upkeep notices on its next round; Try now isn't shown when
    // connected, so a fresh start stands in for the five minutes.
    let mut d = app(&sim);
    d.wait_for_text("The server rules have changed").await;
    d.click("Read the rules").await;
    d.wait_for_text("writers.example's rules").await;
    d.click("I accept the rules").await;
    d.wait_for_text("Accepted. Thank you.").await;
    d.click("Home").await;
    assert!(!d.has_text("The server rules have changed"));
    assert_eq!(sim.accepted(SERVER, "ink"), 1);
}

// ---- Servers ------------------------------------------------------------------

#[tokio::test]
async fn cancelling_add_a_server_goes_back_connected() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = app(&sim);
    first_run(&mut d, SERVER, "ink", "ink_fox").await;
    d.choose("server-switch", "new").await;
    d.wait_for_text("Add a server").await;
    d.click("Cancel").await;
    d.wait_for_text("Connected to writers.example").await;
    assert!(d.has_text("Hello, @ink_fox"));
}

#[tokio::test]
async fn a_second_server_is_added_and_switched_to_and_back() {
    let sim = Sim::new().with_server(SERVER).with_server(OTHER).rc();
    let mut d = app(&sim);
    first_run(&mut d, SERVER, "ink", "ink_fox").await;
    d.choose("server-switch", "new").await;
    d.type_into("server-address", OTHER).await;
    d.wait_for_text("Takes new logins").await;
    d.click("Continue").await;
    d.wait_for_text("Make a login on other.example").await;
    make_login(&mut d, "quill").await;
    d.wait_for_text("Make your first persona").await;
    d.click("Not now").await;
    d.wait_for_text("Connected to other.example").await;
    assert!(d.has_text("Welcome to other.example"));

    let first = sim_entry(&sim, SERVER);
    d.choose("server-switch", &first).await;
    d.wait_for_text("Connected to writers.example").await;
    assert!(d.has_text("Hello, @ink_fox"));
    let other = sim_entry(&sim, OTHER);
    d.choose("server-switch", &other).await;
    d.wait_for_text("Connected to other.example").await;
}

fn sim_entry(sim: &Rc<Sim>, domain: &str) -> String {
    use quill_ui::Backend;
    sim.servers()
        .into_iter()
        .find(|s| s.name == domain)
        .unwrap()
        .id
        .to_string()
}

#[tokio::test]
async fn a_passphrase_store_is_unlocked_at_start_and_a_wrong_try_is_said() {
    let sim = Sim::new().with_server(SERVER).without_keychain().rc();
    let mut d = app(&sim);
    d.type_into("server-address", SERVER).await;
    d.wait_for_text("Takes new logins").await;
    d.click("Continue").await;
    d.type_into("store-passphrase", PASSPHRASE).await;
    d.click("Continue").await;
    make_login(&mut d, "ink").await;
    d.wait_for_text("Make your first persona").await;
    d.click("Not now").await;
    lock_all(&sim);

    let mut d = app(&sim);
    d.wait_for_text("Unlock writers.example").await;
    d.type_into("unlock-passphrase", "not it at all").await;
    d.click("Unlock").await;
    d.wait_for_text("That isn't the passphrase.").await;
    d.type_into("unlock-passphrase", PASSPHRASE).await;
    d.click("Unlock").await;
    d.wait_for_text("Connected to writers.example").await;
}

/// The app closed: passphrase stores are locked again.
fn lock_all(sim: &Rc<Sim>) {
    sim.relock();
}

#[tokio::test]
async fn offline_at_start_shows_the_frame_and_recovers() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = app(&sim);
    first_run(&mut d, SERVER, "ink", "ink_fox").await;
    sim.server(SERVER, |s| s.reachable = false);
    let mut d = app(&sim);
    d.wait_for_text("Can't reach writers.example").await;
    assert!(d.has_text("Home"), "the frame is up");
    sim.server(SERVER, |s| s.reachable = true);
    d.click("Try now").await;
    d.wait_for_text("Connected to writers.example").await;
    d.wait_for_text("Hello, @ink_fox").await;
}

#[tokio::test]
async fn signing_out_and_back_in() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = app(&sim);
    first_run(&mut d, SERVER, "ink", "ink_fox").await;
    d.click("Settings").await;
    d.click("Sign out of writers.example").await;
    d.wait_for_text("Sign in to writers.example").await;
    d.type_into("login-name", "ink").await;
    d.type_into("login-password", PASSWORD).await;
    d.click("Sign in").await;
    d.wait_for_text("Hello, @ink_fox").await;
}

#[tokio::test]
async fn removing_the_open_server_opens_the_other() {
    let sim = Sim::new().with_server(SERVER).with_server(OTHER).rc();
    let mut d = app(&sim);
    first_run(&mut d, SERVER, "ink", "ink_fox").await;
    d.choose("server-switch", "new").await;
    d.type_into("server-address", OTHER).await;
    d.wait_for_text("Takes new logins").await;
    d.click("Continue").await;
    make_login(&mut d, "quill").await;
    d.click("Not now").await;
    d.wait_for_text("Connected to other.example").await;
    d.click("Settings").await;
    // other.example is open: remove it.
    d.click_nth("Remove…", 1).await;
    d.click("Remove other.example").await;
    d.wait_for_text("Connected to writers.example").await;
    assert_eq!(quill_ui::Backend::servers(&*sim).len(), 1);
}

// ---- Personas and settings ------------------------------------------------------

#[tokio::test]
async fn a_persona_is_edited_and_deleted_after_one_plain_question() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = app(&sim);
    first_run(&mut d, SERVER, "ink", "ink_fox").await;
    d.click("Personas").await;
    d.click("Edit").await;
    let display = d.field_id_with_prefix("display-");
    d.type_into(&display, "Ink Fox").await;
    d.click("Save").await;
    d.wait_for_text("Ink Fox @ink_fox").await;
    d.click("Delete…").await;
    assert!(d.has_text("This can't be undone"));
    d.click("Keep it").await;
    d.click("Delete…").await;
    d.click("Delete @ink_fox").await;
    d.wait_until("the persona gone", |d| !d.has_text("@ink_fox"))
        .await;
}

#[tokio::test]
async fn ids_show_only_in_advanced_mode() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = app(&sim);
    first_run(&mut d, SERVER, "ink", "ink_fox").await;
    d.click("Settings").await;
    assert!(!d.has_text("Server id"));
    d.set_checked("advanced", true).await;
    d.wait_for_text("Server id (this app's)").await;
    d.set_checked("advanced", false).await;
    assert!(!d.has_text("Server id"));
}

#[tokio::test]
async fn the_theme_and_text_size_apply_at_once() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = app(&sim);
    first_run(&mut d, SERVER, "ink", "ink_fox").await;
    d.click("Settings").await;
    d.set_checked("theme-Dark", true).await;
    assert_eq!(d.root_attr("data-theme").as_deref(), Some("dark"));
    d.choose("text-size", "130").await;
    assert!(d.page_style().contains("font-size: 130%"));
}
