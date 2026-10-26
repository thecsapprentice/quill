//! The four samples as one WebAssembly module, for comparison only: the
//! spike doesn't run it (see FINDINGS.md, "WebAssembly"). An admin would
//! write this, install a Rust toolchain with the wasm32 target, build it
//! with `cargo build --release --target wasm32-unknown-unknown` and upload
//! the `.wasm` file. The `quill_automod` guest crate (not written) would
//! hold `Post`, the actions and bindings to the server's `text` API.

use quill_automod::{Action, Post, flag, reject, text};

const BANNED: &[&str] = &["spam", "free crypto", "paypal"];
const ALLOWED_SITES: &[&str] = &["quill.example", "archiveofourown.org"];

#[quill_automod::check]
fn check(post: &Post) -> Action {
    let all = format!("{} {}", post.title, post.body);

    if !text::find_words(BANNED, &all).is_empty() {
        return reject("This prompt uses words that aren't allowed here.");
    }

    for link in text::links(&post.body) {
        if !ALLOWED_SITES.contains(&link.host.as_str()) {
            return reject("Links to outside sites aren't allowed in prompts.");
        }
    }
    if text::matches(r"\+?\(?\d[\d\s().-]{6,}\d", &post.body) {
        return reject("Please don't share phone numbers in prompts.");
    }

    if let Some(odd) = text::mixed_script_words(&all).first() {
        return flag(&format!("Mixed alphabets in \"{odd}\""), "spam");
    }

    if post.author.account_age_days < 3.0 {
        if post.history.prompts_last_hour >= 3 {
            return reject("New accounts can post up to 3 prompts an hour. Please try again a little later.");
        }
        if post.history.most_similar_on_board > 0.9 {
            return flag("New account posted a near copy of another prompt", "spam");
        }
    }

    Action::Pass
}
