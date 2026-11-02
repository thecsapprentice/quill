//! One prompt's queue, read on this computer: each pitch, with accepting
//! and declining. Reading it joins each request's group (core's `inbox`),
//! so a pitch that fails a check, or came before this computer was
//! linked, says so instead of showing.

use dioxus::prelude::*;
use quill_board_ui::short_date;
use uuid::Uuid;

use super::{CompareWords, KeyCheck, reload, requests_n};
use crate::{
    app::{ProblemLine, use_ui},
    backend::{Inbox, IncomingRequest, Problem, Unread},
    markdown::Markdown,
};

/// The prompt's title and its author persona, once fetched.
#[derive(Debug, Clone, PartialEq)]
struct About {
    title: String,
    author: Uuid,
}

#[component]
pub fn PromptQueue(prompt: Uuid) -> Element {
    let (backend, mut model) = use_ui();
    let mut about = use_signal(|| None::<Result<About, Problem>>);
    let mut inbox = use_signal(|| None::<Result<Inbox, Problem>>);
    let mut held = use_signal(|| false);
    let mut busy_held = use_signal(|| false);
    let mut said = use_signal(|| None::<String>);

    let b = backend.clone();
    use_hook(move || {
        spawn(async move {
            let result = b.prompt(prompt).await.map(|p| About {
                title: p.title,
                author: p.author.id,
            });
            about.set(Some(result));
        });
    });
    // Read again on each load of the counts (a live arrival), keeping
    // held ones shown once released.
    let b = backend.clone();
    use_effect(move || {
        let _ = model.read().requests.epoch;
        let release = *held.peek();
        let b = b.clone();
        spawn(async move {
            let result = b.inbox(prompt, release).await;
            busy_held.set(false);
            inbox.set(Some(result));
        });
    });
    let b = backend.clone();
    let show_held = move |_| {
        held.set(true);
        busy_held.set(true);
        let b = b.clone();
        spawn(async move {
            let result = b.inbox(prompt, true).await;
            busy_held.set(false);
            inbox.set(Some(result));
            reload(b, model);
        });
    };
    let b = backend.clone();
    let done = use_callback(move |line: String| {
        said.set(Some(line));
        reload(b.clone(), model);
    });

    let title = match about() {
        Some(Ok(a)) => Some(a.title),
        _ => None,
    };
    let author = match about() {
        Some(Ok(a)) => Some(a.author),
        _ => None,
    };
    rsx! {
        div { class: "back",
            button {
                class: "quiet",
                onclick: move |_| model.write().requests.open = None,
                "‹ All requests"
            }
        }
        h1 {
            match &title {
                Some(t) => rsx! { "Requests on “{t}”" },
                None => rsx! { "Requests on your prompt" },
            }
        }
        if let Some(line) = said() {
            p { class: "hint", role: "status", "{line}" }
        }
        match inbox() {
            None => rsx! { p { class: "coming", "Reading the requests…" } },
            Some(Err(Problem::Gone)) => rsx! {
                p { class: "coming", "That prompt is gone: it was deleted, or isn't yours any more." }
            },
            Some(Err(p)) => rsx! { ProblemLine { problem: p } },
            Some(Ok(i)) => rsx! {
                if i.requests.is_empty() {
                    p { class: "coming", "No requests are waiting on this prompt now." }
                } else {
                    ul { class: "list",
                        for r in i.requests.iter().cloned() {
                            Incoming { key: "{r.id}", r, author, done }
                        }
                    }
                }
                if i.held > 0 {
                    div { class: "nudge",
                        p { "{requests_n(i.held)} more arrived while this prompt was busy." }
                        button {
                            class: "secondary",
                            disabled: busy_held(),
                            aria_busy: if busy_held() { "true" } else { "false" },
                            onclick: show_held,
                            if busy_held() { "Reading them…" } else { "Show them" }
                        }
                    }
                }
            },
        }
    }
}

/// One request: who, when, the pitch, and the answer.
#[component]
fn Incoming(r: IncomingRequest, author: Option<Uuid>, done: EventHandler<String>) -> Element {
    let (backend, model) = use_ui();
    let mut declining = use_signal(|| false);
    // "Not now" on a changed key: the pitch stays unread.
    let mut skipped = use_signal(|| false);
    let mut busy = use_signal(|| None::<&'static str>);
    let mut problem = use_signal(|| None::<Problem>);
    let id = r.id;
    let who = match &r.display_name {
        Some(n) => n.clone(),
        None => format!("@{}", r.handle),
    };
    let handle = r.handle.clone();
    let arrived = short_date(r.received_at);
    let lapses = short_date(r.expires_at);

    let b = backend.clone();
    let mut answer = move |accept: bool| {
        busy.set(Some(if accept { "accept" } else { "decline" }));
        let b = b.clone();
        let who = who.clone();
        spawn(async move {
            let result = if accept {
                b.accept_request(id).await
            } else {
                b.decline_request(id).await
            };
            busy.set(None);
            match result {
                Ok(()) if accept => done.call(format!(
                    "Accepted {who}'s request: it's a writing session now, under Sessions."
                )),
                Ok(()) => done.call(format!("Declined {who}'s request.")),
                Err(Problem::Gone) => {
                    done.call("That request is gone: it was withdrawn, or it lapsed.".into())
                }
                Err(p) => problem.set(Some(p)),
            }
        });
    };
    let accept = {
        let mut answer = answer.clone();
        move |_| answer(true)
    };
    let decline = move |_| answer(false);
    let b = backend.clone();
    let checked = move |trusted: bool| {
        if trusted {
            // Read again: the pitch shows now.
            reload(b.clone(), model);
        } else {
            skipped.set(true);
        }
    };
    let who = match &r.display_name {
        Some(n) => format!("{n} (@{})", r.handle),
        None => format!("@{}", r.handle),
    };
    // Accepting needs the pitch read here; declining doesn't, and a pitch
    // that failed its checks can still be declined.
    let readable = r.pitch.is_ok();
    let answerable = readable
        || matches!(
            r.pitch,
            Err(Unread::Refused { .. } | Unread::NotForThisComputer | Unread::SentFromHere)
        )
        || (matches!(r.pitch, Err(Unread::KeyChanged)) && skipped());
    rsx! {
        li { class: "item request",
            header {
                span { class: "name", "{who}" }
            }
            p { class: "hint", "Arrived {arrived}. It lapses on {lapses} if it isn't answered." }
            match r.pitch.clone() {
                Ok(text) => rsx! {
                    div { class: "pitch", Markdown { source: text } }
                    if let Some(me) = author {
                        CompareWords { handle: handle.clone(), as_persona: me }
                    }
                },
                Err(Unread::KeyChanged) => rsx! {
                    if skipped() {
                        p { class: "hint",
                            "Not read: @{handle}'s identity key changed, and you haven't checked it. "
                            button { class: "link", onclick: move |_| skipped.set(false), "Check it now" }
                        }
                    } else if let Some(me) = author {
                        KeyCheck { handle: handle.clone(), as_persona: me, on_done: checked }
                    }
                },
                Err(Unread::SentFromHere) => rsx! {
                    p { class: "hint",
                        "This computer sent it, from another login, so it can't read it as the author here. Read it on a computer where you use this login."
                    }
                },
                Err(Unread::NotForThisComputer) => rsx! {
                    p { class: "hint",
                        "It was sent before this computer was linked, so only your other devices can read it. Answer it there."
                    }
                },
                Err(Unread::Refused { detail }) => rsx! {
                    ProblemLine {
                        problem: Problem::Refused(
                            "It failed Quill's checks, so it isn't shown: it may have been changed on the way. Decline it, or let it lapse.".into(),
                        ),
                    }
                    if model.read().settings.advanced {
                        p { class: "detail", "{detail}" }
                    }
                },
            }
            if let Some(p) = problem() {
                ProblemLine { problem: p }
            }
            if declining() {
                div { class: "notice", role: "alert",
                    "Decline {who}'s request? They aren't told: to them it looks unanswered until it lapses."
                }
                div { class: "actions",
                    button {
                        class: "danger",
                        disabled: busy().is_some(),
                        aria_busy: if busy() == Some("decline") { "true" } else { "false" },
                        onclick: decline,
                        if busy() == Some("decline") { "Declining…" } else { "Decline this request" }
                    }
                    button { class: "quiet", onclick: move |_| declining.set(false), "Keep it" }
                }
            } else if answerable {
                div { class: "actions",
                    if readable {
                        button {
                            class: "primary",
                            disabled: busy().is_some(),
                            aria_busy: if busy() == Some("accept") { "true" } else { "false" },
                            onclick: accept,
                            if busy() == Some("accept") { "Accepting…" } else { "Accept" }
                        }
                    }
                    button { class: "quiet", onclick: move |_| declining.set(true), "Decline…" }
                }
            }
        }
    }
}
