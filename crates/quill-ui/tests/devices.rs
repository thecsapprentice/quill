//! Keys and devices (3d) driven as a writer would, against the simulated
//! server: linking this computer, approving a new one from here, naming
//! and removing devices, starting a persona over, and what goes wrong.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::rc::Rc;

use quill_ui::{
    backend::RemoveReason,
    testing::{Driver, Sim},
};

const SERVER: &str = "writers.example";
const PASSWORD: &str = "correct horse battery";

/// A login `ink` whose persona `@ink_fox` lives on another device, its
/// laptop.
fn sim() -> Rc<Sim> {
    let sim = Sim::new().with_server(SERVER).rc();
    sim.existing_login(SERVER, "ink", PASSWORD, &["ink_fox"]);
    sim.other_device(SERVER, "ink", "Laptop", &["ink_fox"]);
    sim
}

/// Signs in as `ink` on this new computer, up to the keys-elsewhere screen.
async fn signed_in_here(sim: &Rc<Sim>) -> Driver {
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
    d
}

/// From the keys-elsewhere screen, names this computer and shows the code.
async fn showing_the_code(d: &mut Driver, name: &str) {
    d.click("Link this computer").await;
    d.wait_for_text("What should your other devices call this computer?")
        .await;
    d.type_into("device-name", name).await;
    d.click("Show the link code").await;
    d.wait_for_text("Type these words on your other device")
        .await;
}

/// A new login with a ready persona, on the Devices page.
async fn on_devices(sim: &Rc<Sim>) -> Driver {
    let mut d = Driver::new(sim.clone());
    d.type_into("server-address", SERVER).await;
    d.wait_for_text("Takes new logins").await;
    d.click("Continue").await;
    d.wait_for_text("Make a login on").await;
    d.type_into("login-name", "wren").await;
    d.type_into("login-password", PASSWORD).await;
    d.click("Make my login").await;
    d.wait_for_text("Make your first persona").await;
    d.type_into("persona-handle", "salt_wren").await;
    d.click("Make this persona").await;
    d.wait_for_text("Hello, @salt_wren").await;
    d.click("Settings").await;
    d.click("Your devices").await;
    d.wait_until("the list", |d| !d.has_text("Loading your devices"))
        .await;
    d
}

// ---- Linking this computer -------------------------------------------------------------

#[tokio::test]
async fn a_new_computer_is_linked_by_its_words_and_writes_as_the_persona() {
    let sim = sim();
    let mut d = signed_in_here(&sim).await;
    assert!(d.has_text("@ink_fox"));
    d.click("Link this computer").await;
    d.wait_for_text("What should your other devices call this computer?")
        .await;
    assert_eq!(
        d.attr("device-name", "value").as_deref(),
        Some("writing-laptop"),
        "the computer's own name is suggested"
    );
    d.type_into("device-name", "Study desktop").await;
    d.click("Show the link code").await;
    d.wait_for_text("Type these words on your other device")
        .await;
    assert!(d.has_text("acorn"), "the six words");
    assert!(d.has_element("svg"), "and the QR code");
    assert!(d.has_button("Copy the code"));
    assert!(d.has_text("This code works for 10 more minutes."));

    let shown = sim.approve_from_another_device(SERVER, false);
    d.wait_for_text("Check the words match").await;
    for w in &shown {
        assert!(d.has_text(w), "{w}");
    }
    d.click("They match").await;
    d.wait_for_text("Hello, @ink_fox").await;
    assert!(!d.has_button("Link this computer"), "ready here now");
    assert!(
        sim.devices_of(SERVER, "ink")
            .contains(&(Some("Study desktop".into()), vec!["ink_fox".into()])),
        "{:?}",
        sim.devices_of(SERVER, "ink")
    );
}

#[tokio::test]
async fn words_that_dont_match_link_nothing_and_say_what_to_do() {
    let sim = sim();
    let mut d = signed_in_here(&sim).await;
    showing_the_code(&mut d, "Study desktop").await;
    // The server hands this computer a group of its own making.
    let _other_device_shows = sim.approve_from_another_device(SERVER, true);
    d.wait_for_text("Check the words match").await;
    d.click("They don't match").await;
    d.wait_for_text("Nothing was linked").await;
    assert!(d.has_text("This computer took no keys."));
    assert!(d.has_text("remove any device you don't recognise"));
    d.click("Close").await;
    d.wait_for_text("are on another of your devices").await;
    assert!(d.has_button("Link this computer"), "still not linked");
    assert_eq!(sim.devices_of(SERVER, "ink").len(), 1, "only the laptop");
}

#[tokio::test]
async fn an_expired_code_offers_a_new_one() {
    let sim = sim();
    let mut d = signed_in_here(&sim).await;
    showing_the_code(&mut d, "Study desktop").await;
    let first = d.text();
    sim.expire_links(SERVER);
    d.wait_for_text("That code expired").await;
    d.click("Show a new code").await;
    d.wait_for_text("Type these words on your other device")
        .await;
    assert!(
        d.has_text("ember") && !d.has_text("acorn"),
        "new words, not {first}"
    );
    sim.approve_from_another_device(SERVER, false);
    d.wait_for_text("Check the words match").await;
}

#[tokio::test]
async fn going_offline_while_waiting_keeps_the_code_up() {
    let sim = sim();
    let mut d = signed_in_here(&sim).await;
    showing_the_code(&mut d, "Study desktop").await;
    sim.server(SERVER, |s| s.reachable = false);
    d.wait_for_text("Can't reach writers.example").await;
    assert!(d.has_text("acorn"), "the words stay");
    sim.server(SERVER, |s| s.reachable = true);
    sim.approve_from_another_device(SERVER, false);
    d.wait_for_text("Check the words match").await;
    assert!(!d.has_text("Can't reach"));
}

#[tokio::test]
async fn linking_can_wait_and_is_offered_again_later() {
    let sim = sim();
    let mut d = signed_in_here(&sim).await;
    d.click("Not now").await;
    d.wait_for_text("are on another of your devices").await;
    d.click("Link this computer").await;
    d.wait_for_text("What should your other devices call this computer?")
        .await;
    // A name too long for the other devices to show.
    d.type_into("device-name", &"x".repeat(41)).await;
    assert!(d.button_disabled("Show the link code"));
    assert!(d.has_text("At most 40 characters."));
    d.type_into("device-name", "Study desktop").await;
    d.click("Show the link code").await;
    d.wait_for_text("Type these words on your other device")
        .await;
    d.click("Cancel").await;
    d.wait_for_text("are on another of your devices").await;
    // The persona offers it too.
    d.click("Personas").await;
    assert!(d.has_text("Keys on another device"));
    assert!(d.has_button("Link this computer"));
}

#[tokio::test]
async fn a_computer_removed_elsewhere_is_told_and_can_link_again() {
    let sim = sim();
    let mut d = signed_in_here(&sim).await;
    showing_the_code(&mut d, "Study desktop").await;
    sim.approve_from_another_device(SERVER, false);
    d.wait_for_text("Check the words match").await;
    d.click("They match").await;
    d.wait_for_text("Hello, @ink_fox").await;

    // The laptop removes this computer; the app learns at its next upkeep.
    sim.remove_this_computer(SERVER);
    drop(d);
    let mut d = Driver::new(sim.clone());
    d.wait_for_text("This computer was removed from your devices")
        .await;
    assert!(d.has_text("Another of your devices removed this computer"));
    d.click("Link this computer").await;
    d.type_into("device-name", "Study desktop").await;
    d.click("Show the link code").await;
    d.wait_for_text("Type these words on your other device")
        .await;
    sim.approve_from_another_device(SERVER, false);
    d.wait_for_text("Check the words match").await;
    d.click("They match").await;
    d.wait_for_text("Hello, @ink_fox").await;
}

// ---- Starting over ------------------------------------------------------------------------

#[tokio::test]
async fn a_persona_can_start_over_after_one_warning() {
    let sim = sim();
    let mut d = signed_in_here(&sim).await;
    d.click("Not now").await;
    d.click("Personas").await;
    d.click("Start over…").await;
    assert!(d.has_text("Do this only if you can't reach the device that holds its keys"));
    d.click("Keep it").await;
    assert!(!d.has_text("Do this only if"));
    d.click("Start over…").await;
    d.click("Start over with a new key").await;
    d.wait_until("ready", |d| d.has_text("In use")).await;
    assert!(!d.has_text("Keys on another device"));
    // The laptop can't write as it any more.
    assert_eq!(
        sim.devices_of(SERVER, "ink"),
        vec![(Some("Laptop".into()), Vec::<String>::new())]
    );
}

// ---- Approving a new computer from here ---------------------------------------------------

#[tokio::test]
async fn a_new_computer_is_approved_here_by_typing_its_words() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = on_devices(&sim).await;
    assert!(d.has_text("Only this computer so far."));
    // Found by hand (3d): alone, this computer said it had no personas.
    assert!(d.has_text("Writes as @salt_wren."), "{}", d.text());
    assert!(!d.has_text("No personas on it yet."));
    // Unnamed, it's called "This computer" once, with a nudge to name it.
    assert_eq!(d.text().matches("This computer").count(), 1, "{}", d.text());
    assert!(d.has_text("Name it, so your other devices can tell it apart."));
    d.click("Link a new computer").await;
    d.wait_for_text("No new computer is waiting yet").await;
    assert!(!d.has_field("link-words"), "nothing to type yet");

    let words = sim.link_waiting_elsewhere(SERVER, "wren");
    d.click("Check again").await;
    d.wait_until("the words field", |d| d.has_field("link-words"))
        .await;
    assert!(d.button_disabled("Link it"), "six words first");
    // One word off: a typo, told plainly; nothing is linked.
    let mut wrong = words.clone();
    wrong[5] = "zebra".into();
    d.type_into("link-words", &wrong.join(" ")).await;
    d.click("Link it").await;
    d.wait_for_text("Those words don't match the computer that's waiting")
        .await;
    assert_eq!(sim.devices_of(SERVER, "wren").len(), 0);

    d.type_into("link-words", &words.join(" ").to_uppercase())
        .await;
    d.click("Link it").await;
    d.wait_for_text("The new computer should now show these words")
        .await;
    d.wait_until("two devices", |d| d.has_text("An unnamed device"))
        .await;
    assert!(d.has_text("No personas on it yet."));
    assert!(!d.has_text("Only this computer so far."));
    assert_eq!(sim.devices_of(SERVER, "wren").len(), 2);
    // The new computer names itself after the words match; Done shows it.
    sim.finish_newest_device(SERVER, "wren", "Kitchen laptop");
    d.click("Done").await;
    d.wait_for_text("Kitchen laptop").await;
    assert!(!d.has_text("No personas on it yet."));
}

#[tokio::test]
async fn two_waiting_computers_stop_approval_with_a_warning() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = on_devices(&sim).await;
    sim.link_waiting_elsewhere(SERVER, "wren");
    sim.link_waiting_elsewhere(SERVER, "wren");
    d.click("Link a new computer").await;
    d.wait_for_text("2 new computers are waiting to link to your login")
        .await;
    assert!(d.has_text("someone else may be trying to link"));
    assert!(!d.has_field("link-words"), "nothing can be approved");
}

// ---- The device list ----------------------------------------------------------------------

#[tokio::test]
async fn this_computer_is_renamed_and_another_removed_after_one_question() {
    let sim = sim();
    let mut d = signed_in_here(&sim).await;
    showing_the_code(&mut d, "Study desktop").await;
    sim.approve_from_another_device(SERVER, false);
    d.wait_for_text("Check the words match").await;
    d.click("They match").await;
    d.wait_for_text("Hello, @ink_fox").await;
    d.click("Settings").await;
    d.click("Your devices").await;
    d.wait_for_text("Laptop").await;
    let text = d.text();
    let at = |t: &str| text.find(t).unwrap_or_else(|| panic!("no {t}"));
    assert!(at("Study desktop") < at("Laptop"), "this computer first");
    assert!(d.has_text("Writes as @ink_fox."));

    d.click("Rename").await;
    d.type_into("device-rename", &"y".repeat(41)).await;
    assert!(d.button_disabled("Save name"));
    d.type_into("device-rename", "Attic desktop").await;
    d.click("Save name").await;
    d.wait_for_text("Attic desktop").await;
    assert!(
        sim.devices_of(SERVER, "ink")
            .iter()
            .any(|(n, _)| n.as_deref() == Some("Attic desktop"))
    );

    d.click("Remove…").await;
    assert!(d.has_text("Remove Laptop?"));
    assert!(d.has_text("This can't be undone"));
    d.click("Keep it").await;
    d.click("Remove…").await;
    d.set_checked("remove-reason-Lost", true).await;
    d.click("Remove Laptop").await;
    d.wait_for_text("Laptop was removed.").await;
    assert_eq!(sim.removal_reasons(SERVER), vec![RemoveReason::Lost]);
    assert_eq!(sim.devices_of(SERVER, "ink").len(), 1);
    assert!(!d.has_button("Remove…"), "only this computer is left");
}

#[tokio::test]
async fn advanced_mode_shows_device_ids_and_persona_keys() {
    let sim = Sim::new().with_server(SERVER).rc();
    let mut d = on_devices(&sim).await;
    assert!(!d.has_text("Persona keys"));
    d.click("Settings").await;
    d.set_checked("advanced", true).await;
    d.click("Your devices").await;
    d.wait_for_text("Persona keys").await;
    d.wait_for_text("@salt_wren").await;
    assert!(d.has_text("Identity key"));
    assert!(d.has_text("KeyPackages waiting"));
}
