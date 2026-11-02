//! Writing sessions (3f): the list, reading one, and what Home says about
//! them. The list lives in the model, reloaded by upkeep on each live
//! notice (a note arriving), so Home, the side bar and this screen agree.
//! Writing, changing notes and safety numbers come in 3f-2 and 3f-3.

use std::rc::Rc;

use dioxus::prelude::*;
use uuid::Uuid;

use crate::{
    app::{Code, use_ui},
    backend::{Backend, Problem, SessionStanding, SessionSummary},
    model::{Model, Section},
};

mod reading;

/// Loads the open server's sessions into the model.
pub async fn load(b: &dyn Backend, mut model: Signal<Model>, server: Uuid) -> Result<(), Problem> {
    let list = b.sessions().await?;
    model.write().sessions_loaded(server, list);
    Ok(())
}

/// Loads in the background; on failure the screen keeps what it had.
pub fn reload(b: Rc<dyn Backend>, model: Signal<Model>) {
    let Some(server) = model.peek().current else {
        return;
    };
    spawn(async move {
        let _ = load(b.as_ref(), model, server).await;
    });
}

/// "1 new note" or "3 new notes".
fn new_notes(n: usize) -> String {
    if n == 1 {
        "1 new note".into()
    } else {
        format!("{n} new notes")
    }
}

/// "with @wren on “The ledger”", for list rows and Home.
fn with_whom(s: &SessionSummary) -> String {
    let who = match &s.partner {
        Some(p) => format!("with {}", p.name()),
        None => "whose other writer left".into(),
    };
    match &s.prompt {
        Some((_, t)) => format!("{who} on “{t}”"),
        None => who,
    }
}

/// What a standing other than joined means, and what to do.
pub(crate) fn standing_line(s: &SessionStanding) -> Option<String> {
    match s {
        SessionStanding::Joined => None,
        SessionStanding::Waiting => Some(
            "This computer isn't in it yet. Another of your devices for this persona adds it when it next opens Quill."
                .into(),
        ),
        SessionStanding::NoKeys => Some(
            "This persona's keys aren't on this computer. Link this computer to write in it here."
                .into(),
        ),
        SessionStanding::Rejoining => Some(
            "This computer was away too long to follow it. It's waiting to be let back in by another of your devices or the other writer."
                .into(),
        ),
        SessionStanding::Broken { .. } => Some(
            "This computer stopped reading it after a check failed. Your other devices may still read it."
                .into(),
        ),
    }
}

#[component]
pub fn SessionsPage() -> Element {
    let (backend, model) = use_ui();
    let b = backend.clone();
    use_hook(move || reload(b, model));
    let s = model.read().sessions.clone();
    if let Some(id) = s.open {
        return rsx! {
            for k in [id] {
                reading::SessionPage { key: "{k}", id: k.clone() }
            }
        };
    }
    rsx! {
        h1 { "Sessions" }
        p { class: "lede", "Where you write with other writers. Only the two of you can read what's written." }
        if !s.loaded {
            p { class: "coming", "Loading sessions…" }
        } else if s.list.is_empty() {
            p { class: "coming",
                "No sessions yet. When a request to write together is accepted, the session starts here."
            }
            div { class: "actions",
                button {
                    class: "secondary",
                    onclick: move |_| { let mut m = model; m.write().go(Section::Board) },
                    "Find a prompt on the Board"
                }
            }
        } else {
            ul { class: "list sessions",
                for x in s.list.iter().cloned() {
                    SessionRow { key: "{x.id}", s: x }
                }
            }
        }
    }
}

#[component]
fn SessionRow(s: SessionSummary) -> Element {
    let (backend, mut model) = use_ui();
    let advanced = model.read().settings.advanced;
    let id = s.id.clone();
    let title = match &s.partner {
        Some(p) => p.name(),
        None => "The other writer left".into(),
    };
    let on = s.prompt.as_ref().map(|(_, t)| format!("On “{t}”"));
    let latest = s.latest.map(|at| reading::when(backend.utc_offset(), at));
    let line = standing_line(&s.standing);
    let code = match &s.standing {
        SessionStanding::Broken { code, .. } => Some(*code),
        _ => None,
    };
    let label = format!("Open the session {}", with_whom(&s));
    rsx! {
        li { class: "item session",
            header {
                span { class: "name", "{title}" }
                if s.unseen > 0 {
                    span { class: "badge", "{new_notes(s.unseen)}" }
                }
                if s.ended {
                    span { class: "tag", "Ended" }
                }
            }
            if let Some(on) = on {
                p { "{on}" }
            }
            if let Some(at) = latest {
                p { class: "hint", "Last note {at}." }
            }
            if let Some(line) = line {
                p { class: "hint",
                    "{line}"
                    if advanced && let Some(c) = code {
                        " "
                        Code { code: c }
                    }
                }
            }
            div { class: "actions",
                button {
                    class: "secondary",
                    aria_label: "{label}",
                    onclick: move |_| model.write().sessions.open = Some(id.clone()),
                    "Open"
                }
            }
        }
    }
}

/// What Home says about sessions: the ones with notes not yet read here.
#[component]
pub fn SessionsOnHome() -> Element {
    let (_, mut model) = use_ui();
    let s = model.read().sessions.clone();
    let fresh: Vec<SessionSummary> = s.list.into_iter().filter(|x| x.unseen > 0).collect();
    rsx! {
        for x in fresh {
            div { key: "{x.id}", class: "nudge",
                p { "{new_notes(x.unseen)} in your session {with_whom(&x)}." }
                button {
                    class: "primary",
                    aria_label: "Read the session {with_whom(&x)}",
                    onclick: {
                        let id = x.id.clone();
                        move |_| model.write().open_sessions(Some(id.clone()))
                    },
                    "Read it"
                }
            }
        }
    }
}
