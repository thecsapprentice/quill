//! The stop on a changed identity key (3e): before a request goes to a
//! writer, or a pitch from one is read, whose key isn't the one this
//! computer first saw. Why it stops, and why the words, are in
//! docs/design/gui-design-language.md ("A changed key").

use dioxus::prelude::*;
use uuid::Uuid;

use crate::{
    app::{ProblemLine, use_ui},
    backend::{KeyChange, Problem},
};

/// Shows what changed and the words to compare. `on_done(true)` once the
/// new key is trusted (or it turns out unchanged), `on_done(false)` on
/// "Not now".
#[component]
pub fn KeyCheck(handle: String, as_persona: Uuid, on_done: EventHandler<bool>) -> Element {
    let (backend, _) = use_ui();
    let mut change = use_signal(|| None::<Result<Option<KeyChange>, Problem>>);
    let mut busy = use_signal(|| false);
    let mut problem = use_signal(|| None::<Problem>);
    let mut again = use_signal(|| 0u32);

    let b = backend.clone();
    let h = handle.clone();
    use_effect(move || {
        let _ = again();
        let (b, h) = (b.clone(), h.clone());
        spawn(async move {
            let result = b.key_change(&h, as_persona).await;
            if matches!(result, Ok(None)) {
                // Nothing to check (any more): carry on.
                on_done.call(true);
            }
            change.set(Some(result));
        });
    });
    let b = backend.clone();
    let h = handle.clone();
    let mut trust = move |fingerprint: String| {
        busy.set(true);
        let (b, h) = (b.clone(), h.clone());
        spawn(async move {
            let result = b.accept_key_change(&h, &fingerprint).await;
            busy.set(false);
            match result {
                Ok(()) => on_done.call(true),
                Err(Problem::KeyChanged(_)) => {
                    problem.set(Some(Problem::Refused(
                        "It changed again while you were checking. Compare these new words.".into(),
                    )));
                    again += 1;
                }
                Err(p) => problem.set(Some(p)),
            }
        });
    };

    let c = match change() {
        None => return rsx! { p { class: "coming", "Checking @{handle}'s identity key…" } },
        Some(Err(p)) => return rsx! { ProblemLine { problem: p } },
        Some(Ok(None)) => return rsx! {},
        Some(Ok(Some(c))) => c,
    };
    let fingerprint = c.fingerprint.clone();
    rsx! {
        section {
            class: "panel key-check",
            aria_labelledby: "key-check-{c.handle}",
            // It may appear below the window's edge (sending from a long
            // prompt): bring it into view, or nothing seems to happen.
            onmounted: move |e: MountedEvent| async move {
                let _ = e.data().scroll_to(ScrollBehavior::Instant).await;
            },
            h3 { id: "key-check-{c.handle}", "@{c.handle}'s identity key changed" }
            p {
                "Quill keeps the identity key it first saw for each writer. @{c.handle}'s is different now. "
                "Usually that's because they started over on a new computer without their old keys. "
                "It could also mean someone is using the server to pass as them."
            }
            p {
                "Ask @{c.handle}, some other way than Quill's requests, to read you their 12 words for the two of you. "
                "Carry on only if they match these:"
            }
            ol { class: "words", aria_label: "The words to compare",
                for (i, w) in c.words.iter().enumerate() {
                    li { key: "{i}", "{w}" }
                }
            }
            if let Some(p) = problem() {
                ProblemLine { problem: p }
            }
            div { class: "actions",
                button {
                    class: "primary",
                    disabled: busy(),
                    aria_busy: if busy() { "true" } else { "false" },
                    onclick: move |_| trust(fingerprint.clone()),
                    if busy() { "Trusting the new key…" } else { "The words match: carry on" }
                }
                button { class: "quiet", onclick: move |_| on_done.call(false), "Not now" }
            }
        }
    }
}
