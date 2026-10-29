//! The server rules (#42): read and accept them in the frame, and the
//! rules box on the sign-up screen.

use dioxus::prelude::*;

use crate::{
    app::{ProblemLine, use_ui},
    backend::{Problem, ServerRules},
    markdown::Markdown,
};

/// The Rules section: the rules, and Accept when this login must.
#[component]
pub fn RulesPage() -> Element {
    let (backend, mut model) = use_ui();
    let mut busy = use_signal(|| false);
    let mut problem = use_signal(|| None::<Problem>);
    let mut accepted = use_signal(|| false);
    let (name, rules) = {
        let m = model.read();
        (m.server_name(), m.rules.clone())
    };

    let b = backend.clone();
    let accept = use_callback(move |version: u32| {
        busy.set(true);
        let b = b.clone();
        spawn(async move {
            let result = b.accept_rules(version).await;
            busy.set(false);
            let id = model.peek().current;
            match result {
                Ok(()) => {
                    problem.set(None);
                    accepted.set(true);
                    model.write().rules_accepted();
                }
                Err(p) => {
                    problem.set(Some(p));
                    // Changed again: show the new version.
                    if let Some(id) = id {
                        let fresh = b.rules().await;
                        model.write().rules_loaded(id, fresh);
                    }
                }
            }
        });
    });
    rsx! {
        h1 { "{name}'s rules" }
        match rules {
            None => rsx! { p { class: "coming", "Loading…" } },
            Some(None) => rsx! { p { class: "coming", "{name} hasn't set any rules." } },
            Some(Some(r)) => rsx! {
                if r.must_accept {
                    div { class: "notice", role: "status",
                        "The rules have changed since you last accepted them. Read them, then accept to keep posting, editing, commenting and reporting."
                    }
                }
                Markdown { source: r.markdown.clone() }
                if r.must_accept {
                    div { class: "actions",
                        button {
                            class: "primary",
                            disabled: busy(),
                            onclick: move |_| accept.call(r.version),
                            "I accept the rules"
                        }
                    }
                }
            },
        }
        if accepted() {
            p { class: "hint", role: "status", "Accepted. Thank you." }
        }
        if let Some(p) = problem() {
            ProblemLine { problem: p }
        }
    }
}

/// The rules on the sign-up screen: folded away to read, and a checkbox
/// when joining means accepting them.
#[component]
pub fn SignUpRules(
    rules: ServerRules,
    required: bool,
    accepted: bool,
    on_accept: EventHandler<bool>,
) -> Element {
    rsx! {
        details { class: "rules-box",
            summary { "Read the server rules" }
            Markdown { source: rules.markdown.clone() }
        }
        if required {
            div { class: "choice",
                input {
                    r#type: "checkbox",
                    id: "accept-rules",
                    checked: accepted,
                    onchange: move |e: FormEvent| on_accept.call(e.checked()),
                }
                label { r#for: "accept-rules", "I've read and accept the server rules" }
            }
        } else {
            p { class: "hint", "By making a login you agree to follow the server rules." }
        }
    }
}
