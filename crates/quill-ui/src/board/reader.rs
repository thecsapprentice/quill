//! Reading a prompt (the board's reading pane, or the whole page when
//! narrow): its text rendered by the client, its tags, Copy link, a
//! report, and for the author's own prompts, editing, status and deleting.

use dioxus::prelude::*;
use quill_board_ui::{maturity_label, reason_label, short_date, state_label};
use quill_proto::board::{CreateReport, Prompt, PromptState, ReportReason, Tag, UpdatePrompt};

use super::{View, use_board, use_board_epoch};
use crate::{
    app::{ProblemLine, use_ui},
    backend::Problem,
    field::Area,
    markdown::Markdown,
};

/// A tag as a coloured chip, "genre: Horror"; removable when given
/// `on_remove` (called with the tag's key).
#[component]
pub fn Chip(tag: Tag, on_remove: Option<EventHandler<String>>, note: Option<String>) -> Element {
    let style = tag
        .color
        .as_ref()
        .map(|c| format!("background:{};color:{}", c.background, c.text_or_auto()));
    let key = tag.key();
    let label = format!("{}: {}", tag.namespace, tag.display);
    rsx! {
        span { class: "chip", style: style.unwrap_or_default(),
            "{label}"
            if let Some(n) = note {
                em { class: "chip-note", " {n}" }
            }
            if let Some(remove) = on_remove {
                button {
                    class: "chip-remove",
                    aria_label: "Remove {label}",
                    onclick: move |_| remove.call(key.clone()),
                    "×"
                }
            }
        }
    }
}

/// The open prompt, refreshed from the server as it opens.
#[component]
pub fn Reader() -> Element {
    let (backend, model) = use_ui();
    let mut board = use_board();
    let Some(prompt) = board.read().open_prompt().cloned() else {
        return rsx! {};
    };
    let id = prompt.id;
    // Fresh details, and a prompt deleted elsewhere leaves the list.
    let b = backend.clone();
    use_hook(move || {
        spawn(async move {
            match b.prompt(id).await {
                Ok(fresh) => board.write().replace(fresh),
                Err(Problem::Gone) => {
                    let mut s = board.write();
                    s.forget(id);
                    s.said = Some(gone_line());
                }
                Err(_) => {}
            }
        });
    });
    let own = model
        .read()
        .personas()
        .iter()
        .any(|p| p.persona.id == prompt.author.id);
    let author = match &prompt.author.display_name {
        Some(name) => format!("{name} (@{})", prompt.author.handle),
        None => format!("@{}", prompt.author.handle),
    };
    let posted = short_date(prompt.created_at);
    let edited = (prompt.updated_at > prompt.created_at).then(|| short_date(prompt.updated_at));
    rsx! {
        article { class: "reader",
            div { class: "reader-back",
                button {
                    class: "quiet",
                    onclick: move |_| board.write().open = None,
                    "‹ Back to the board"
                }
            }
            h2 { "{prompt.title}" }
            p { class: "reader-meta",
                "{author} · {maturity_label(prompt.maturity)} · {state_label(prompt.state)} · {posted}"
                if let Some(e) = edited { " · edited {e}" }
            }
            if prompt.removed {
                div { class: "notice", "An administrator removed this prompt from the board. Only you can see it." }
            }
            Markdown { source: prompt.body.clone() }
            if !prompt.tags.is_empty() {
                div { class: "chips",
                    for t in prompt.tags.iter() {
                        Chip { key: "{t.key()}", tag: t.clone() }
                    }
                }
            }
            CopyLink { id }
            if own {
                OwnActions { prompt: prompt.clone() }
            } else if model.read().signed_in() {
                ReportForm { key: "{id}", prompt: prompt.clone() }
            }
        }
    }
}

fn gone_line() -> String {
    "That prompt is gone: it was deleted, or isn't shown to you any more.".to_owned()
}

#[component]
fn CopyLink(id: uuid::Uuid) -> Element {
    let (backend, _) = use_ui();
    let mut shown = use_signal(|| None::<String>);
    let link = backend.prompt_link(id);
    let Some(link) = link else {
        return rsx! {};
    };
    let copy = {
        let link = link.clone();
        move |_| {
            let link = link.clone();
            spawn(async move {
                // The webview's clipboard; where it isn't available, the
                // link is shown to copy by hand.
                let js = format!("await navigator.clipboard.writeText({link:?}); return true;");
                let copied = document::eval(&js).await.is_ok();
                shown.set(Some(if copied {
                    "Link copied.".to_owned()
                } else {
                    link
                }));
            });
        }
    };
    rsx! {
        div { class: "actions",
            button { class: "quiet", onclick: copy, "Copy link" }
            if let Some(s) = shown() {
                span { class: "hint", role: "status", "{s}" }
            }
        }
    }
}

/// Edit, status and delete, on the author's own prompt.
#[component]
fn OwnActions(prompt: Prompt) -> Element {
    let (backend, _) = use_ui();
    let mut board = use_board();
    let epoch = use_board_epoch();
    let mut deleting = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut problem = use_signal(|| None::<Problem>);
    let id = prompt.id;

    let b = backend.clone();
    let set_state = move |e: FormEvent| {
        let Ok(state) = e.value().parse::<PromptState>() else {
            return;
        };
        let b = b.clone();
        spawn(async move {
            let body = UpdatePrompt {
                state: Some(state),
                ..UpdatePrompt::default()
            };
            match b.update_prompt(id, body).await {
                Ok(p) => {
                    problem.set(None);
                    board.write().replace(p);
                    epoch.changed(board);
                }
                Err(Problem::Gone) => gone(board, id),
                Err(p) => problem.set(Some(p)),
            }
        });
    };
    let b = backend.clone();
    let delete = move |_| {
        busy.set(true);
        let b = b.clone();
        spawn(async move {
            let result = b.delete_prompt(id).await;
            busy.set(false);
            match result {
                Ok(()) | Err(Problem::Gone) => {
                    {
                        let mut s = board.write();
                        s.forget(id);
                        s.said = Some("Prompt deleted.".to_owned());
                    }
                    epoch.changed(board);
                }
                Err(p) => problem.set(Some(p)),
            }
        });
    };
    rsx! {
        div { class: "own",
            div { class: "field",
                label { r#for: "prompt-state", "Status" }
                select { id: "prompt-state", onchange: set_state,
                    for s in PromptState::ALL.iter().copied() {
                        option { key: "{s}", value: "{s}", selected: s == prompt.state, "{state_label(s)}" }
                    }
                }
            }
            if deleting() {
                div { class: "notice", role: "alert",
                    "Delete “{prompt.title}”? Requests waiting on it go with it. This can't be undone."
                }
                div { class: "actions",
                    button { class: "danger", disabled: busy(), onclick: delete, "Delete this prompt" }
                    button { class: "quiet", onclick: move |_| deleting.set(false), "Keep it" }
                }
            } else {
                div { class: "actions",
                    button {
                        class: "secondary",
                        onclick: move |_| {
                            let mut s = board.write();
                            s.said = None;
                            s.view = View::Post(Some(id));
                        },
                        "Edit"
                    }
                    button { class: "quiet", onclick: move |_| deleting.set(true), "Delete…" }
                }
            }
            if let Some(p) = problem() {
                ProblemLine { problem: p }
            }
        }
    }
}

fn gone(mut board: Signal<super::BoardState>, id: uuid::Uuid) {
    let mut s = board.write();
    s.forget(id);
    s.said = Some(gone_line());
}

/// Reporting a prompt: a reason, optional details, sent once.
#[component]
fn ReportForm(prompt: Prompt) -> Element {
    let (backend, _) = use_ui();
    let board = use_board();
    let mut open = use_signal(|| false);
    let mut reason = use_signal(|| None::<ReportReason>);
    let mut details = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut sent = use_signal(|| false);
    let mut problem = use_signal(|| None::<Problem>);
    let id = prompt.id;

    let send = move |_| {
        let Some(r) = reason() else {
            return;
        };
        busy.set(true);
        let b = backend.clone();
        spawn(async move {
            let body = CreateReport {
                prompt: id,
                reason: r,
                details: Some(details().trim().to_owned()).filter(|d| !d.is_empty()),
            };
            let result = b.report(body).await;
            busy.set(false);
            match result {
                Ok(()) => {
                    sent.set(true);
                    open.set(false);
                }
                Err(Problem::Gone) => gone(board, id),
                Err(p) => problem.set(Some(p)),
            }
        });
    };
    if sent() {
        return rsx! {
            p { class: "hint", role: "status", "Thanks. An administrator will look at your report." }
        };
    }
    let details_check =
        crate::checks::length(&details(), quill_proto::limits::REPORT_DETAILS_MAX_CHARS);
    if !open() {
        return rsx! {
            div { class: "actions",
                button { class: "quiet", onclick: move |_| open.set(true), "Report this prompt" }
            }
        };
    }
    rsx! {
        fieldset { class: "report",
            legend { "Report this prompt" }
            p { class: "hint", "An administrator reads it. The author is never told who reported it." }
            for r in ReportReason::ALL.iter().copied() {
                div { key: "{r}", class: "choice",
                    input {
                        r#type: "radio",
                        id: "reason-{r}",
                        name: "reason",
                        checked: reason() == Some(r),
                        onchange: move |_| reason.set(Some(r)),
                    }
                    label { r#for: "reason-{r}", "{reason_label(r)}" }
                }
            }
            Area {
                label: "Anything to add (optional)",
                id: "report-details",
                hint: details_check.hint.clone(),
                invalid: details_check.invalid,
                oninput: move |t| details.set(t),
            }
            if let Some(p) = problem() {
                ProblemLine { problem: p }
            }
            div { class: "actions",
                button {
                    class: "primary",
                    disabled: busy() || reason().is_none() || details_check.invalid,
                    onclick: send,
                    "Send report"
                }
                button { class: "quiet", onclick: move |_| open.set(false), "Cancel" }
            }
        }
    }
}
