//! Asking to write on another writer's prompt, from the reader: a pitch
//! only the author's devices can read, and how long to wait for an answer.

use dioxus::prelude::*;
use quill_proto::board::{Prompt, PromptState};

use super::{KeyCheck, reload};
use crate::{
    app::{ProblemLine, use_ui},
    backend::{Keys, PITCH_MAX, Problem, REQUEST_DAYS_MAX},
    checks,
    field::Area,
};

/// How long a request can wait, offered in the form.
const DAYS: [u32; 5] = [1, 3, 7, 14, REQUEST_DAYS_MAX];

fn days_label(d: u32) -> String {
    if d == 1 {
        "1 day".into()
    } else {
        format!("{d} days")
    }
}

#[component]
pub fn AskToWrite(prompt: Prompt) -> Element {
    let (backend, mut model) = use_ui();
    let mut open = use_signal(|| false);
    let mut pitch = use_signal(String::new);
    let mut days = use_signal(|| REQUEST_DAYS_MAX);
    let mut busy = use_signal(|| false);
    let mut problem = use_signal(|| None::<Problem>);
    let mut sent = use_signal(|| false);
    // A changed key on the author's persona: checked before sending.
    let mut checking = use_signal(|| false);

    let author = prompt.author.handle.clone();
    let id = prompt.id;
    let me = model
        .read()
        .persona_in_use()
        .filter(|p| p.keys == Keys::Ready)
        .map(|p| (p.persona.id, p.persona.handle.clone()));

    if !matches!(prompt.state, PromptState::Open | PromptState::Busy) {
        return rsx! {};
    }
    if !prompt.accepting_sessions {
        return rsx! {
            p { class: "hint", "@{author} can't take requests to write together yet." }
        };
    }
    let Some((persona, handle)) = me else {
        return rsx! {
            p { class: "hint", "To ask to write on it, choose a persona that's ready on this computer." }
        };
    };
    if sent() {
        return rsx! {
            p { class: "hint", role: "status",
                "Request sent. Only @{author}'s devices can read your pitch. Their answer will show on Requests and Home."
            }
            div { class: "actions",
                button { class: "quiet", onclick: move |_| model.write().open_requests(None), "See your requests" }
            }
        };
    }
    if !open() {
        return rsx! {
            div { class: "actions",
                button { class: "secondary", onclick: move |_| open.set(true), "Ask to write together" }
            }
        };
    }

    let b = backend.clone();
    let mut send = move || {
        busy.set(true);
        problem.set(None);
        let b = b.clone();
        spawn(async move {
            let result = b.send_request(persona, id, &pitch(), days()).await;
            busy.set(false);
            match result {
                Ok(_) => {
                    sent.set(true);
                    reload(b, model);
                }
                Err(Problem::KeyChanged(_)) => checking.set(true),
                Err(p) => problem.set(Some(p)),
            }
        });
    };
    if checking() {
        let mut send = send.clone();
        return rsx! {
            KeyCheck {
                handle: author.clone(),
                as_persona: persona,
                on_done: move |trusted: bool| {
                    checking.set(false);
                    if trusted {
                        send();
                    }
                },
            }
        };
    }
    let check = checks::length(&pitch(), PITCH_MAX);
    let empty = pitch().trim().is_empty();
    let hint = format!(
        "Who you'd write, and how. Markdown; only @{author}'s devices can read it. {}",
        check.hint
    );
    rsx! {
        section { class: "ask", aria_labelledby: "ask-title",
            h3 { id: "ask-title", "Ask to write together" }
            p { class: "hint", "Asking as @{handle}." }
            Area {
                label: "Your pitch",
                id: "request-pitch",
                initial: pitch.peek().clone(),
                hint,
                invalid: check.invalid,
                oninput: move |t| pitch.set(t),
            }
            div { class: "field",
                label { r#for: "request-days", "Wait for an answer for" }
                select {
                    id: "request-days",
                    onchange: move |e| if let Ok(d) = e.value().parse() { days.set(d) },
                    for d in DAYS {
                        option { key: "{d}", value: "{d}", selected: d == days(), "{days_label(d)}" }
                    }
                }
                p { class: "hint", "If @{author} doesn't answer by then, it lapses." }
            }
            if let Some(p) = problem() {
                ProblemLine { problem: p }
            }
            div { class: "actions",
                button {
                    class: "primary",
                    disabled: busy() || empty || check.invalid,
                    aria_busy: if busy() { "true" } else { "false" },
                    onclick: move |_| send(),
                    if busy() { "Sending…" } else { "Send request" }
                }
                button { class: "quiet", onclick: move |_| open.set(false), "Cancel" }
                if empty {
                    span { class: "hint", role: "status", "Needs a pitch before sending." }
                }
            }
        }
    }
}
