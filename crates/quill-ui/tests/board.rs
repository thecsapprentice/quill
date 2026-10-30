//! The board (3c) driven as a writer would, against the simulated server:
//! browsing, searching, filtering, reading, reporting, posting with tag
//! limits, editing and deleting, and what goes wrong in each.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::rc::Rc;

use quill_proto::board::{Maturity, PromptState, ReportReason, TagPolicy};
use quill_ui::testing::{Acceptance, Driver, Sim};

const SERVER: &str = "writers.example";
const PASSWORD: &str = "correct horse battery";

/// A fresh install signed in as `ink` with persona `@ink_fox`, on the Board.
async fn on_the_board(sim: &Rc<Sim>) -> Driver {
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
    d.click("Board").await;
    d.wait_until("the board loaded", |d| !d.has_text("Loading prompts"))
        .await;
    d
}

fn sim() -> Rc<Sim> {
    Sim::new().with_server(SERVER).rc()
}

/// Fills the post form's title and text.
async fn write_prompt(d: &mut Driver, title: &str, body: &str) {
    d.click("Post a prompt").await;
    d.wait_for_text("Add tags").await;
    d.type_into("post-title", title).await;
    d.type_into("post-body", body).await;
}

// ---- Browsing and reading --------------------------------------------------

#[tokio::test]
async fn an_empty_board_says_so_and_offers_posting() {
    let sim = sim();
    let d = on_the_board(&sim).await;
    assert!(d.has_text("No prompts on the board yet."));
    assert!(d.has_button("Post a prompt"));
    assert!(d.has_text("Pick a prompt to read it here."));
}

#[tokio::test]
async fn prompts_are_listed_newest_first_and_read_beside_the_list() {
    let sim = sim();
    sim.fill_board(SERVER, 5);
    let mut d = on_the_board(&sim).await;
    let text = d.text();
    let at = |t: &str| text.find(t).unwrap_or_else(|| panic!("no {t}"));
    assert!(
        at("Prompt number 4") < at("Prompt number 0"),
        "newest first"
    );
    // Each row carries its tags, before any prompt is opened.
    assert!(d.has_text("genre: Mystery"));
    d.click_containing("Prompt number 4").await;
    d.wait_for_text("The body of prompt number 4.").await;
    assert!(d.has_text("@writer0"));
    assert!(d.has_text("genre: Horror"));
    assert!(d.has_button("Report this prompt"));
    assert!(!d.has_button("Edit"), "not ours");
}

#[tokio::test]
async fn prompt_text_is_markdown_drawn_by_the_app() {
    let sim = sim();
    sim.post_as(
        SERVER,
        "marrow",
        "The lighthouse keeper's last letter",
        "A letter arrives **forty years** late. <script>alert(1)</script>",
        &["genre:Horror"],
        Maturity::Teen,
    );
    let mut d = on_the_board(&sim).await;
    d.click_containing("The lighthouse keeper").await;
    d.wait_for_text("forty years").await;
    assert!(!d.has_element("script"), "raw HTML stays text");
}

#[tokio::test]
async fn searching_narrows_the_list_and_says_when_nothing_matches() {
    let sim = sim();
    sim.fill_board(SERVER, 6);
    let mut d = on_the_board(&sim).await;
    d.type_into("board-search", "number 3").await;
    d.wait_until("the search", |d| !d.has_text("Prompt number 5"))
        .await;
    assert!(d.has_text("Prompt number 3"));
    d.type_into("board-search", "zebra").await;
    d.wait_for_text("No prompts match").await;
    d.type_into("board-search", "").await;
    d.wait_for_text("Prompt number 5").await;
}

/// Found by hand (3c): in a narrow window the page scrolls, not the list,
/// so loading on the list's scroll never happened and a button stood in.
/// Reaching the end now loads more whichever box scrolls, with no button.
#[tokio::test]
async fn more_prompts_come_as_the_end_of_the_list_comes_into_view() {
    let sim = sim();
    sim.fill_board(SERVER, 45);
    let mut d = on_the_board(&sim).await;
    assert!(d.has_text("Prompt number 25"));
    assert!(!d.has_text("Prompt number 24"), "one page of 20");
    assert!(!d.has_button("Show more prompts"), "no button to press");
    d.bring_into_view("load-more").await;
    d.wait_for_text("Prompt number 5 ").await;
    d.bring_into_view("load-more").await;
    d.wait_for_text("That's every prompt.").await;
    assert!(d.has_text("Prompt number 0"));
    assert!(
        !d.has_element_with_class("load-more"),
        "nothing more to load"
    );
}

/// Found by hand (3c): a prompt posted from My prompts didn't show on the
/// Board, which kept the list it had, until a search or filter reloaded it.
#[tokio::test]
async fn a_prompt_posted_from_my_prompts_shows_on_the_board() {
    let sim = sim();
    sim.fill_board(SERVER, 3);
    let mut d = on_the_board(&sim).await;
    d.click("My prompts").await;
    d.wait_until("my prompts", |d| !d.has_text("Loading prompts"))
        .await;
    write_prompt(&mut d, "Posted from mine", "Text.").await;
    d.click("Post prompt").await;
    d.wait_for_text("Prompt posted.").await;
    d.click("Board").await;
    d.wait_until("the board's list", |d| {
        d.has_button_containing("Posted from mine")
    })
    .await;

    // Deleting it from My prompts takes it off the Board too.
    d.click("My prompts").await;
    d.wait_for_text("Posted from mine").await;
    d.click_containing("Posted from mine").await;
    d.click("Delete…").await;
    d.click("Delete this prompt").await;
    d.wait_for_text("Prompt deleted.").await;
    d.click("Board").await;
    d.wait_until("the board's list", |d| {
        !d.has_button_containing("Posted from mine") && d.has_text("Prompt number 2")
    })
    .await;
}

#[tokio::test]
async fn new_prompts_from_others_are_found_by_refresh_and_on_return() {
    let sim = sim();
    sim.fill_board(SERVER, 3);
    let mut d = on_the_board(&sim).await;
    sim.post_as(
        SERVER,
        "marrow",
        "Fresh from marrow",
        "Text.",
        &[],
        Maturity::General,
    );
    assert!(!d.has_text("Fresh from marrow"));
    d.click("Refresh").await;
    d.wait_for_text("Fresh from marrow").await;

    // Away and back: the list stays put, and says what's new.
    d.click_containing("Prompt number 1").await;
    d.click("Home").await;
    sim.post_as(
        SERVER,
        "marrow",
        "Another one",
        "Text.",
        &[],
        Maturity::General,
    );
    d.click("Board").await;
    d.wait_for_text("1 new prompt.").await;
    assert!(
        !d.has_button_containing("Another one"),
        "not moved in under the reader"
    );
    assert!(d.has_text("The body of prompt number 1."), "still reading");
    d.click("Show them").await;
    d.wait_until("shown", |d| d.has_button_containing("Another one"))
        .await;
    assert!(!d.has_text("1 new prompt."));
}

#[tokio::test]
async fn the_board_looks_for_new_prompts_while_open() {
    let sim = sim();
    sim.fill_board(SERVER, 2);
    let mut d = on_the_board(&sim).await;
    sim.post_as(
        SERVER,
        "marrow",
        "Arrived meanwhile",
        "Text.",
        &[],
        Maturity::General,
    );
    sim.post_as(
        SERVER,
        "marrow",
        "And another",
        "Text.",
        &[],
        Maturity::General,
    );
    assert!(!d.has_text("new prompts"), "not until the next look");
    sim.time_passes();
    d.wait_for_text("2 new prompts.").await;
    d.click("Show them").await;
    d.wait_until("shown", |d| d.has_button_containing("Arrived meanwhile"))
        .await;
}

#[tokio::test]
async fn a_prompt_deleted_elsewhere_leaves_the_list_when_opened() {
    let sim = sim();
    let id = sim.post_as(
        SERVER,
        "marrow",
        "Soon gone",
        "Text.",
        &[],
        Maturity::General,
    );
    let mut d = on_the_board(&sim).await;
    sim.delete_elsewhere(SERVER, id);
    d.click_containing("Soon gone").await;
    d.wait_for_text("That prompt is gone").await;
    assert!(!d.has_button_containing("Soon gone"));
}

#[tokio::test]
async fn the_board_says_when_the_server_cannot_be_reached_and_retries() {
    let sim = sim();
    sim.fill_board(SERVER, 2);
    let mut d = on_the_board(&sim).await;
    d.click("Home").await;
    sim.server(SERVER, |s| s.reachable = false);
    // A fresh look at the board, while the server is away.
    d.click("Board").await;
    d.type_into("board-search", "prompt").await;
    d.wait_for_text("Can't reach").await;
    sim.server(SERVER, |s| s.reachable = true);
    d.click("Try again").await;
    d.wait_for_text("Prompt number 1").await;
}

// ---- Filters ----------------------------------------------------------------

#[tokio::test]
async fn a_filter_built_by_type_shows_only_matching_prompts() {
    let sim = sim();
    sim.fill_board(SERVER, 8);
    let mut d = on_the_board(&sim).await;
    d.click("Filters").await;
    d.wait_for_text("Filter prompts").await;
    d.click("Genre").await;
    d.wait_until("the genre tags", |d| d.has_field("has-genre-horror"))
        .await;
    d.click_id("has-genre-horror").await;
    assert!(d.pressed("has-genre-horror"));
    assert!(d.has_text("Has all of genre: Horror"));
    d.click("Show prompts").await;
    d.wait_until("the filtered list", |d| !d.has_text("Prompt number 7"))
        .await;
    assert!(d.has_text("Prompt number 4") && d.has_text("Prompt number 0"));
    assert!(!d.has_text("Prompt number 1"));
    assert!(d.has_button("Filters (1)"));

    // Removing the chip from the summary drops it.
    d.click("Remove genre: Horror").await;
    d.wait_for_text("Prompt number 7").await;
    assert!(d.has_button("Filters"));
}

#[tokio::test]
async fn hiding_a_tag_and_choosing_basics() {
    let sim = sim();
    sim.fill_board(SERVER, 8);
    sim.post_as(SERVER, "marrow", "A teen one", "Text.", &[], Maturity::Teen);
    let mut d = on_the_board(&sim).await;
    d.click("Filters").await;
    d.set_checked("rating-Some(General)", true).await;
    d.click("Genre").await;
    d.wait_until("the genre tags", |d| d.has_field("hide-genre-mystery"))
        .await;
    d.click_id("hide-genre-mystery").await;
    d.click("Show prompts").await;
    d.wait_until("the filtered list", |d| !d.has_text("A teen one"))
        .await;
    assert!(d.has_text("General or under"));
    assert!(!d.has_text("Prompt number 1"), "mystery is hidden");
    assert!(d.has_text("Prompt number 2"));
}

#[tokio::test]
async fn leaving_the_builder_without_showing_keeps_the_old_filter() {
    let sim = sim();
    sim.fill_board(SERVER, 4);
    let mut d = on_the_board(&sim).await;
    d.click("Filters").await;
    d.click("Genre").await;
    d.wait_until("the genre tags", |d| d.has_field("has-genre-horror"))
        .await;
    d.click_id("has-genre-horror").await;
    d.click("‹ Back to the board").await;
    assert!(d.has_text("Prompt number 3"));
    assert!(d.has_button("Filters"));
    // Clear all empties the draft.
    d.click("Filters").await;
    d.click("Genre").await;
    d.wait_until("the genre tags", |d| d.has_field("has-genre-horror"))
        .await;
    d.click_id("has-genre-horror").await;
    d.click("Clear all").await;
    assert!(d.has_text("Nothing set"));
}

#[tokio::test]
async fn a_type_with_many_tags_is_searched_and_paged() {
    let sim = sim();
    // 40 themes over four prompts (a prompt takes at most 15 tags).
    for chunk in 0..4 {
        let many: Vec<String> = (chunk * 10..chunk * 10 + 10)
            .map(|i| format!("theme:Place {i:02}"))
            .collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        sim.post_as(
            SERVER,
            "marrow",
            &format!("Themes {chunk}"),
            "Text.",
            &refs,
            Maturity::General,
        );
    }
    let mut d = on_the_board(&sim).await;
    d.click("Filters").await;
    d.click("Theme").await;
    d.wait_for_text("Showing 1–30 of 40").await;
    d.click("Next 30").await;
    d.wait_for_text("Showing 31–40 of 40").await;
    d.type_into("tag-search", "place 07").await;
    d.wait_until("the search", |d| d.has_text("of 1")).await;
    assert!(d.has_field("has-theme-place-07"));
}

// ---- Reporting -----------------------------------------------------------------

#[tokio::test]
async fn a_report_needs_a_reason_and_is_sent_once() {
    let sim = sim();
    sim.post_as(
        SERVER,
        "marrow",
        "Something odd",
        "Text.",
        &[],
        Maturity::General,
    );
    let mut d = on_the_board(&sim).await;
    d.click_containing("Something odd").await;
    d.click("Report this prompt").await;
    assert!(d.has_text("The author is never told who reported it."));
    assert!(d.button_disabled("Send report"));
    d.set_checked("reason-spam", true).await;
    d.type_into("report-details", "Advertising.").await;
    d.click("Send report").await;
    d.wait_for_text("Thanks. An administrator will look at your report.")
        .await;
    let reports = sim.reports(SERVER);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].reason, ReportReason::Spam);
    assert_eq!(reports[0].details.as_deref(), Some("Advertising."));
}

// ---- Posting -------------------------------------------------------------------

#[tokio::test]
async fn posting_waits_for_the_tag_limits_and_says_what_is_missing() {
    let sim = sim();
    sim.set_limits(SERVER, "length", 1, Some(1));
    let mut d = on_the_board(&sim).await;
    d.click("Post a prompt").await;
    d.wait_for_text("Add tags").await;
    assert!(d.button_disabled("Post prompt"));
    assert!(d.has_text("Needs a title, the prompt's text, 1 length tag before posting."));
    assert!(d.has_text("Length: pick 1"), "a dashed slot");
    d.type_into("post-title", "The night porter").await;
    d.type_into(
        "post-body",
        "A hotel with one more *floor* than the lift admits.",
    )
    .await;
    assert!(d.has_text("floor"), "the preview draws it");
    d.wait_until("the length tags", |d| d.has_button("length: Short"))
        .await;
    d.click("length: Short").await;
    assert!(d.has_text("1 of exactly 1 ✓"));
    d.click("Post prompt").await;
    d.wait_for_text("Prompt posted.").await;
    assert!(d.has_text("The night porter"));
    assert_eq!(sim.titles(SERVER), ["The night porter"]);
}

#[tokio::test]
async fn a_full_type_takes_no_more_until_one_is_removed() {
    let sim = sim();
    sim.set_limits(SERVER, "genre", 0, Some(1));
    let mut d = on_the_board(&sim).await;
    write_prompt(&mut d, "Title", "Text.").await;
    d.wait_until("the genre tags", |d| d.has_button("genre: Horror"))
        .await;
    d.click("genre: Horror").await;
    d.click("genre: Mystery").await;
    d.wait_for_text("Genre is full (up to 1)").await;
    d.click("Remove genre: Horror").await;
    d.click("genre: Mystery").await;
    assert!(d.has_button("Remove genre: Mystery"));
}

#[tokio::test]
async fn new_tags_are_typed_with_their_type_and_may_need_approval() {
    let sim = sim();
    sim.set_tag_policy(SERVER, TagPolicy::Approval);
    let mut d = on_the_board(&sim).await;
    write_prompt(&mut d, "Lighthouse", "Text.").await;
    d.type_into("tag-find", "lighthouse keeper").await;
    d.click("Add").await;
    d.wait_for_text("type its type first").await;
    d.type_into("tag-find", "theme: lighthouse keeper").await;
    d.click("Add").await;
    assert!(d.has_text("theme: lighthouse keeper new, needs approval"));
    d.click("Post prompt").await;
    d.wait_for_text("Prompt posted.").await;
}

#[tokio::test]
async fn a_curated_server_takes_only_its_own_tags() {
    let sim = sim();
    sim.set_tag_policy(SERVER, TagPolicy::Curated);
    let mut d = on_the_board(&sim).await;
    write_prompt(&mut d, "Title", "Text.").await;
    d.type_into("tag-find", "theme: something new").await;
    d.click("Add").await;
    d.wait_for_text("only takes its own tags").await;
    assert!(!d.has_button("Remove theme: something new"));
}

#[tokio::test]
async fn tags_found_by_name_are_added_from_the_matches() {
    let sim = sim();
    let mut d = on_the_board(&sim).await;
    write_prompt(&mut d, "Title", "Text.").await;
    d.type_into("tag-find", "myst").await;
    d.wait_until("the matches", |d| {
        d.has_text("Matching tags") || d.has_button("genre: Mystery")
    })
    .await;
    d.click_nth("genre: Mystery", 0).await;
    assert!(d.has_button("Remove genre: Mystery"));
}

#[tokio::test]
async fn the_tags_you_often_use_are_offered() {
    let sim = sim();
    let mut d = on_the_board(&sim).await;
    write_prompt(&mut d, "First", "Text.").await;
    d.wait_until("the genre tags", |d| d.has_button("genre: Horror"))
        .await;
    d.click("genre: Horror").await;
    d.click("Post prompt").await;
    d.wait_for_text("Prompt posted.").await;
    write_prompt(&mut d, "Second", "Text.").await;
    d.wait_for_text("You often use").await;
}

#[tokio::test]
async fn a_moderation_refusal_keeps_what_was_typed() {
    let sim = sim();
    sim.refuse_word(SERVER, "casino", "Please don't advertise here.");
    let mut d = on_the_board(&sim).await;
    write_prompt(&mut d, "Win big", "Visit my casino tonight.").await;
    d.click("Post prompt").await;
    d.wait_for_text("Please don't advertise here.").await;
    assert_eq!(d.attr("post-title", "value").as_deref(), Some("Win big"));
    d.type_into("post-body", "A heist at a seaside arcade.")
        .await;
    d.click("Post prompt").await;
    d.wait_for_text("Prompt posted.").await;
}

#[tokio::test]
async fn rules_to_accept_before_posting_lead_to_the_rules() {
    let sim = sim();
    let mut d = on_the_board(&sim).await;
    sim.set_rules(SERVER, "# Be kind", Acceptance::OnChange);
    write_prompt(&mut d, "Title", "Text.").await;
    d.click("Post prompt").await;
    d.wait_for_text("Read and accept them to continue").await;
    d.click("Read the rules").await;
    d.wait_for_text("I accept the rules").await;
}

#[tokio::test]
async fn without_a_ready_persona_posting_offers_to_make_one() {
    let sim = sim();
    let mut d = on_the_board(&sim).await;
    d.click("Personas").await;
    d.click("Delete…").await;
    d.click("Delete @ink_fox").await;
    d.wait_until("the persona gone", |d| !d.has_text("@ink_fox"))
        .await;
    d.click("Board").await;
    d.click("Post a prompt").await;
    d.wait_for_text("none of yours is ready on this computer")
        .await;
    d.click("Make a persona").await;
    d.wait_until("the persona form", |d| d.has_field("persona-handle"))
        .await;
}

// ---- Your own prompts ---------------------------------------------------------

#[tokio::test]
async fn your_prompt_is_edited_its_status_set_and_deleted_after_one_question() {
    let sim = sim();
    let mut d = on_the_board(&sim).await;
    write_prompt(&mut d, "First draft", "Text.").await;
    d.click("Post prompt").await;
    d.wait_for_text("Prompt posted.").await;
    assert!(!d.has_button("Report this prompt"), "your own");

    d.click("Edit").await;
    d.wait_for_text("Edit your prompt").await;
    d.type_into("post-title", "Second draft").await;
    d.click("Save changes").await;
    d.wait_for_text("Changes saved.").await;
    assert!(d.has_text("Second draft"));

    d.choose("prompt-state", "closed").await;
    d.wait_until("the status", |d| d.has_text("· Closed ·"))
        .await;
    assert_eq!(
        sim.prompt_by_title(SERVER, "Second draft").unwrap().1,
        PromptState::Closed
    );

    d.click("Delete…").await;
    assert!(d.has_text("This can't be undone."));
    d.click("Keep it").await;
    d.click("Delete…").await;
    d.click("Delete this prompt").await;
    d.wait_for_text("Prompt deleted.").await;
    assert!(sim.titles(SERVER).is_empty());
}

#[tokio::test]
async fn ids_never_show_on_the_board() {
    let sim = sim();
    sim.fill_board(SERVER, 3);
    let mut d = on_the_board(&sim).await;
    d.click_containing("Prompt number 2").await;
    d.wait_for_text("The body of prompt number 2.").await;
    let text = d.text();
    let looks_like_uuid = text
        .split_whitespace()
        .any(|w| w.len() == 36 && w.matches('-').count() == 4);
    assert!(!looks_like_uuid, "{text}");
}

/// Found by hand (3c): the title and text limits were only told after
/// pressing Post. They're said as the writer types, and Post waits.
#[tokio::test]
async fn length_limits_are_said_while_typing() {
    let sim = sim();
    let mut d = on_the_board(&sim).await;
    d.click("Post a prompt").await;
    d.wait_for_text("Add tags").await;
    assert!(d.has_text("Up to 200 characters."));
    assert!(d.has_text("Up to 20,000 characters."));
    d.type_into("post-title", &"t".repeat(201)).await;
    assert!(d.has_text("201 of 200 characters: 1 too many."));
    assert_eq!(
        d.attr("post-title", "aria-invalid").as_deref(),
        Some("true")
    );
    d.type_into("post-body", &"b".repeat(20_005)).await;
    assert!(d.has_text("20,005 of 20,000 characters: 5 too many."));
    assert!(d.has_text("Needs a shorter title, shorter text before posting."));
    assert!(d.button_disabled("Post prompt"));
    d.type_into("post-title", "A fair title").await;
    d.type_into("post-body", "A fair body.").await;
    assert!(d.has_text("12 of 200 characters."));
    assert!(!d.button_disabled("Post prompt"));
}

#[tokio::test]
async fn report_details_say_their_limit_while_typing() {
    let sim = sim();
    sim.post_as(
        SERVER,
        "marrow",
        "Something odd",
        "Text.",
        &[],
        Maturity::General,
    );
    let mut d = on_the_board(&sim).await;
    d.click_containing("Something odd").await;
    d.click("Report this prompt").await;
    d.set_checked("reason-spam", true).await;
    assert!(d.has_text("Up to 2,000 characters."));
    d.type_into("report-details", &"x".repeat(2_001)).await;
    assert!(d.has_text("2,001 of 2,000 characters: 1 too many."));
    assert!(d.button_disabled("Send report"));
}
