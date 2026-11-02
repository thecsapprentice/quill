//! Collaboration requests (3e): the Requests screen (what waits on the
//! login's prompts, and what it sent), a prompt's queue, asking to write
//! from the reader, and the stop on a changed identity key. Counts live in
//! the model, reloaded by upkeep on each live notice, so Home, the side
//! bar and this screen agree.

use std::rc::Rc;

use dioxus::prelude::*;
use quill_board_ui::short_date;
use uuid::Uuid;

use crate::{
    app::{ProblemLine, use_ui},
    backend::{Backend, Problem, RequestStatus, SentRequest, WaitingPrompt},
    model::{Model, Section},
};

mod ask;
mod key_check;
mod queue;
mod words;

pub use ask::AskToWrite;
pub use key_check::KeyCheck;
pub use words::{CompareWords, WordsLookup};

/// Loads the open server's requests into the model.
pub async fn load(b: &dyn Backend, mut model: Signal<Model>, server: Uuid) -> Result<(), Problem> {
    let waiting = b.waiting_requests().await?;
    let sent = b.sent_requests().await?;
    let seen = b.answers_seen();
    model.write().requests_loaded(server, waiting, sent, seen);
    Ok(())
}

/// Loads in the background, saying nothing if it fails: the screen keeps
/// what it had, and upkeep tries again.
pub fn reload(b: Rc<dyn Backend>, model: Signal<Model>) {
    let Some(server) = model.peek().current else {
        return;
    };
    spawn(async move {
        let _ = load(b.as_ref(), model, server).await;
    });
}

/// Marks one answer to a sent request seen, so Home stops pointing at it.
fn mark_answer_seen(b: &dyn Backend, mut model: Signal<Model>, id: Uuid) {
    let mut seen = model.peek().requests.seen.clone();
    if seen.contains(&id) {
        return;
    }
    seen.push(id);
    let _ = b.set_answers_seen(&seen);
    model.write().requests.seen = seen;
}

/// "1 request" or "3 requests".
fn requests_n(n: u32) -> String {
    if n == 1 {
        "1 request".into()
    } else {
        format!("{n} requests")
    }
}

#[component]
pub fn RequestsPage() -> Element {
    let (backend, mut model) = use_ui();
    let b = backend.clone();
    use_hook(move || reload(b, model));
    // Answers shown here are seen: Home stops pointing at them.
    let b = backend.clone();
    use_effect(move || {
        let r = model.read().requests.clone();
        let answered: Vec<Uuid> = r
            .sent
            .iter()
            .filter(|s| s.status != RequestStatus::Waiting)
            .map(|s| s.id)
            .collect();
        if !r.loaded || answered.iter().all(|id| r.seen.contains(id)) {
            return;
        }
        let mut seen = r.seen.clone();
        seen.extend(answered.into_iter().filter(|id| !r.seen.contains(id)));
        let _ = b.set_answers_seen(&seen);
        model.write().requests.seen = seen;
    });

    let r = model.read().requests.clone();
    if let Some(prompt) = r.open {
        return rsx! {
            for k in [prompt] {
                queue::PromptQueue { key: "{k}", prompt: k }
            }
        };
    }
    rsx! {
        h1 { "Requests" }
        p { class: "lede",
            "Requests to write together: the ones waiting on your prompts, and the ones you've sent."
        }
        if !r.loaded {
            p { class: "coming", "Loading requests…" }
        } else {
            section { class: "requests-part", aria_labelledby: "on-your-prompts",
                h2 { id: "on-your-prompts", "On your prompts" }
                if r.waiting.is_empty() {
                    p { class: "coming", "No requests are waiting on your prompts." }
                } else {
                    ul { class: "list",
                        for w in r.waiting.iter().cloned() {
                            WaitingItem { key: "{w.prompt}", w }
                        }
                    }
                }
            }
            section { class: "requests-part", aria_labelledby: "you-sent",
                h2 { id: "you-sent", "You sent" }
                if r.sent.is_empty() {
                    p { class: "coming",
                        "You haven't asked to write on a prompt yet. Find one on the Board, and ask from it."
                    }
                    div { class: "actions",
                        button { class: "secondary", onclick: move |_| model.write().go(Section::Board), "Go to the Board" }
                    }
                } else {
                    ul { class: "list",
                        for s in r.sent.iter().cloned() {
                            SentItem { key: "{s.id}", s }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn WaitingItem(w: WaitingPrompt) -> Element {
    let (_, mut model) = use_ui();
    let held =
        (w.held > 0).then(|| format!("{} more arrived while it was busy.", requests_n(w.held)));
    let prompt = w.prompt;
    rsx! {
        li { class: "item",
            header {
                span { class: "name", "{w.title}" }
            }
            p {
                if w.pending > 0 {
                    "{requests_n(w.pending)} waiting. "
                }
                if let Some(h) = held { "{h}" }
            }
            div { class: "actions",
                button {
                    class: "secondary",
                    aria_label: "Read the requests on “{w.title}”",
                    onclick: move |_| model.write().requests.open = Some(prompt),
                    "Read them"
                }
            }
        }
    }
}

#[component]
fn SentItem(s: SentRequest) -> Element {
    let (backend, model) = use_ui();
    let mut asking = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut problem = use_signal(|| None::<Problem>);
    let id = s.id;
    let title = match &s.prompt {
        Some((_, t)) => format!("“{t}”"),
        None => "A prompt that was deleted".into(),
    };
    let until = short_date(s.expires_at);
    let withdraw = move |_| {
        busy.set(true);
        let b = backend.clone();
        spawn(async move {
            let result = b.withdraw_request(id).await;
            busy.set(false);
            match result {
                Ok(()) | Err(Problem::Gone) => reload(b, model),
                Err(p) => problem.set(Some(p)),
            }
        });
    };
    rsx! {
        li { class: "item",
            header {
                span { class: "name", "{title}" }
                span { class: "handle", "as @{s.persona}" }
                match s.status {
                    RequestStatus::Session => rsx! { span { class: "tag", "Writing together" } },
                    RequestStatus::NoResponse => rsx! { span { class: "tag", "No answer" } },
                    _ => rsx! {},
                }
            }
            match s.status {
                RequestStatus::Session => rsx! {
                    p { "Accepted: it's a writing session now." }
                    div { class: "actions",
                        crate::sessions::OpenSession {
                            prompt: s.prompt.as_ref().map(|p| p.0),
                            me: Some(s.persona.clone()),
                        }
                    }
                },
                RequestStatus::NoResponse => rsx! {
                    p { "It lapsed on {until} without an answer." }
                    div { class: "actions",
                        button { class: "quiet", disabled: busy(), onclick: withdraw, "Dismiss" }
                    }
                },
                _ => rsx! {
                    p { "Waiting for an answer, until {until}." }
                    if asking() {
                        div { class: "notice", role: "alert",
                            "Withdraw your request on {title}? Its author won't see it any more."
                        }
                        div { class: "actions",
                            button {
                                class: "danger",
                                disabled: busy(),
                                aria_busy: if busy() { "true" } else { "false" },
                                onclick: withdraw,
                                if busy() { "Withdrawing…" } else { "Withdraw this request" }
                            }
                            button { class: "quiet", onclick: move |_| asking.set(false), "Keep it" }
                        }
                    } else {
                        div { class: "actions",
                            button { class: "quiet", onclick: move |_| asking.set(true), "Withdraw…" }
                        }
                    }
                },
            }
            if let Some(p) = problem() {
                ProblemLine { problem: p }
            }
        }
    }
}

/// What Home says about requests: new ones on the login's prompts, and
/// answers to the ones it sent, each leading to them.
#[component]
pub fn RequestsOnHome() -> Element {
    let (backend, mut model) = use_ui();
    let r = model.read().requests.clone();
    if !r.loaded {
        return rsx! {};
    }
    let count = r.waiting_count();
    let one = (r.waiting.len() == 1).then(|| r.waiting[0].clone());
    let waiting_line = match &one {
        Some(w) if count == 1 => format!("A request is waiting on “{}”.", w.title),
        Some(w) => format!("{} are waiting on “{}”.", requests_n(count), w.title),
        None => format!("{} are waiting on your prompts.", requests_n(count)),
    };
    let open = one.map(|w| w.prompt);
    let answers: Vec<SentRequest> = r.new_answers().into_iter().cloned().collect();
    rsx! {
        if count > 0 {
            div { class: "nudge",
                p { "{waiting_line}" }
                button {
                    class: "primary",
                    onclick: move |_| model.write().open_requests(open),
                    "Read them"
                }
            }
        }
        for a in answers {
            div { key: "{a.id}", class: "nudge",
                p { "{answer_line(&a)}" }
                if a.status == RequestStatus::Session {
                    // Opening the session is reading the answer.
                    crate::sessions::OpenSession {
                        prompt: a.prompt.as_ref().map(|p| p.0),
                        me: Some(a.persona.clone()),
                        on_open: {
                            let b = backend.clone();
                            let id = a.id;
                            move |()| mark_answer_seen(b.as_ref(), model, id)
                        },
                    }
                } else {
                    button {
                        class: "secondary",
                        onclick: move |_| model.write().open_requests(None),
                        "See your requests"
                    }
                }
            }
        }
    }
}

fn answer_line(a: &SentRequest) -> String {
    let on = match &a.prompt {
        Some((_, t)) => format!("on “{t}”"),
        None => "on a prompt since deleted".into(),
    };
    match a.status {
        RequestStatus::Session => {
            format!("Your request {on} was accepted: you're writing together.")
        }
        _ => format!("Your request {on} got no answer before it lapsed."),
    }
}
