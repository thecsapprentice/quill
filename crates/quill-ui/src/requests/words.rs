//! The 12 words two writers compare (#72): shown where they meet before a
//! session (a prompt, a request), and found for any writer from Personas.
//! Both apps derive the same words, so either writer can read theirs out
//! when the other sees a changed key (design language, "A changed key").

use dioxus::prelude::*;
use uuid::Uuid;

use crate::{
    app::{ProblemLine, use_ui},
    backend::{Keys, Problem},
    checks,
    field::{Field, Kind},
};

/// The words, or why there are none.
#[component]
fn Words(handle: String, words: Option<Vec<String>>) -> Element {
    match words {
        None => rsx! {
            p { class: "hint", role: "status", "@{handle} has no identity key yet, so there are no words to compare." }
        },
        Some(w) => rsx! {
            p { class: "hint",
                "@{handle} sees these same 12 words for the two of you. If either of you ever sees a changed key, read them to each other some other way than Quill."
            }
            ol { class: "words", aria_label: "Your words with @{handle}",
                for (i, word) in w.iter().enumerate() {
                    li { key: "{i}", "{word}" }
                }
            }
        },
    }
}

/// "Compare words with @name", opened in place.
#[component]
pub fn CompareWords(handle: String, as_persona: Uuid) -> Element {
    let (backend, _) = use_ui();
    let mut shown = use_signal(|| None::<Result<Option<Vec<String>>, Problem>>);
    let mut open = use_signal(|| false);
    let h = handle.clone();
    let show = move |_| {
        open.set(true);
        let (b, h) = (backend.clone(), h.clone());
        spawn(async move {
            shown.set(Some(b.identity_words(&h, as_persona).await));
        });
    };
    if !open() {
        return rsx! {
            div { class: "actions",
                button { class: "quiet", onclick: show, "Compare words with @{handle}" }
            }
        };
    }
    rsx! {
        div { class: "compare-words",
            match shown() {
                None => rsx! { p { class: "coming", "Working out the words…" } },
                Some(Err(p)) => rsx! { ProblemLine { problem: p } },
                Some(Ok(words)) => rsx! { Words { handle: handle.clone(), words } },
            }
            div { class: "actions",
                button { class: "quiet", onclick: move |_| open.set(false), "Hide the words" }
            }
        }
    }
}

/// On Personas: the words with any writer, by handle, for the persona in
/// use. Works before any prompt or request brings the two together.
#[component]
pub fn WordsLookup() -> Element {
    let (backend, model) = use_ui();
    let mut typed = use_signal(String::new);
    let mut shown = use_signal(|| None::<(String, Result<Option<Vec<String>>, Problem>)>);
    let mut busy = use_signal(|| false);
    let me = model
        .read()
        .persona_in_use()
        .filter(|p| p.keys == Keys::Ready)
        .map(|p| (p.persona.id, p.persona.handle.clone()));
    let Some((persona, mine)) = me else {
        return rsx! {};
    };
    let handle = typed().trim().trim_start_matches('@').to_owned();
    let check = checks::handle(&handle);
    let ready = !handle.is_empty() && !check.invalid;
    let mut look = move || {
        let h = typed().trim().trim_start_matches('@').to_owned();
        busy.set(true);
        let b = backend.clone();
        spawn(async move {
            let result = b.identity_words(&h, persona).await;
            busy.set(false);
            shown.set(Some((h, result)));
        });
    };
    rsx! {
        section { class: "panel", aria_labelledby: "words-title",
            h2 { id: "words-title", "Compare words with another writer" }
            p {
                "If a writer's identity key changes, Quill asks you both to compare 12 words. Find yours with them here, as @{mine}."
            }
            Field {
                label: "Their handle",
                id: "words-handle",
                kind: Kind::Name,
                hint: "The writer's @handle.",
                invalid: !handle.is_empty() && check.invalid,
                oninput: move |t| typed.set(t),
                onenter: {
                    let mut look = look.clone();
                    move |_| if ready { look() }
                },
            }
            div { class: "actions",
                button {
                    class: "secondary",
                    disabled: !ready || busy(),
                    aria_busy: if busy() { "true" } else { "false" },
                    onclick: move |_| look(),
                    "Show the words"
                }
            }
            match shown() {
                None => rsx! {},
                Some((h, Err(Problem::Gone))) => rsx! {
                    p { class: "hint", role: "status", "There's no writer @{h} on this server." }
                },
                Some((_, Err(p))) => rsx! { ProblemLine { problem: p } },
                Some((h, Ok(words))) => rsx! { Words { handle: h, words } },
            }
        }
    }
}
