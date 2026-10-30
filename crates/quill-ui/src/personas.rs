//! The Personas section: list, make, edit and delete personas.

use dioxus::prelude::*;

use crate::{
    app::{ProblemLine, use_ui},
    backend::{Keys, PersonaState, Problem},
    checks,
    field::{Area, Field, Kind},
    first_run::PersonaForm,
};

#[component]
pub fn PersonasPage() -> Element {
    let (_, mut model) = use_ui();
    let m = model.read();
    let personas = m.personas().to_vec();
    let signed_in = m.signed_in();
    drop(m);
    // Opened from a nudge with no personas yet: the form is open already.
    let mut making = use_signal(|| personas.is_empty());

    if !signed_in {
        return rsx! {
            h1 { "Personas" }
            p { class: "coming", "Sign in to see and make personas." }
        };
    }
    rsx! {
        h1 { "Personas" }
        p { class: "lede",
            "Each persona is a name you write under. Other writers see its handle and display name, not your login or your other personas."
        }
        ul { class: "list",
            for p in personas {
                PersonaItem { key: "{p.persona.id}", state: p }
            }
        }
        if making() {
            section { class: "item", aria_label: "New persona",
                h2 { "New persona" }
                PersonaForm {
                    submit_label: "Make this persona",
                    cancel_label: "Cancel",
                    on_created: move |p| {
                        making.set(false);
                        model.write().persona_created(p);
                    },
                    on_cancel: move |_| making.set(false),
                }
            }
        } else {
            button { class: "primary", onclick: move |_| making.set(true), "Make a persona" }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Showing,
    Editing,
    Deleting,
    /// Asking before a new identity key replaces one on another device.
    StartingOver,
}

#[component]
fn PersonaItem(state: PersonaState) -> Element {
    let (backend, mut model) = use_ui();
    let mut mode = use_signal(|| Mode::Showing);
    let mut display = use_signal(|| state.persona.display_name.clone().unwrap_or_default());
    let mut summary = use_signal(|| state.persona.summary.clone().unwrap_or_default());
    let mut busy = use_signal(|| false);
    let mut problem = use_signal(|| None::<Problem>);
    let (in_use, advanced) = {
        let m = model.read();
        (m.persona == Some(state.persona.id), m.settings.advanced)
    };
    let p = state.persona.clone();
    let id = p.id;
    let handle = p.handle.clone();
    let shown = p.display_name.clone().unwrap_or_else(|| handle.clone());

    let b = backend.clone();
    let save = move |_| {
        if busy() || checks::display_name(&display()).invalid {
            return;
        }
        busy.set(true);
        let b = b.clone();
        spawn(async move {
            let result = b.update_persona(id, &display(), &summary()).await;
            busy.set(false);
            match result {
                Ok(p) => {
                    mode.set(Mode::Showing);
                    problem.set(None);
                    model.write().persona_updated(p);
                }
                Err(e) => problem.set(Some(e)),
            }
        });
    };
    let b = backend.clone();
    let delete = move |_| {
        busy.set(true);
        let b = b.clone();
        spawn(async move {
            let result = b.delete_persona(id).await;
            busy.set(false);
            match result {
                Ok(()) => model.write().persona_deleted(id),
                Err(e) => problem.set(Some(e)),
            }
        });
    };
    let display_check = checks::display_name(&display());
    rsx! {
        li { class: "item",
            header {
                span { class: "name", "{shown}" }
                span { class: "handle", "@{handle}" }
                match state.keys {
                    Keys::Ready if in_use => rsx! { span { class: "tag", "In use" } },
                    Keys::Ready => rsx! {},
                    Keys::ToSetUp => rsx! { span { class: "tag", "Not ready on this computer" } },
                    Keys::Elsewhere => rsx! { span { class: "tag", "Keys on another device" } },
                }
            }
            if let Some(s) = p.summary.as_ref().filter(|_| mode() != Mode::Editing) {
                p { "{s}" }
            }
            if advanced {
                p { class: "detail", "id {id}" }
            }
            match mode() {
                Mode::Showing => rsx! {
                    div { class: "actions",
                        if state.keys == Keys::Ready && !in_use {
                            button {
                                class: "secondary",
                                onclick: move |_| model.write().choose_persona(id),
                                "Write as @{handle}"
                            }
                        }
                        if state.keys == Keys::Elsewhere {
                            button {
                                class: "secondary",
                                onclick: move |_| model.write().begin_link(),
                                "Link this computer"
                            }
                            button { class: "quiet", onclick: move |_| mode.set(Mode::StartingOver), "Start over…" }
                        }
                        if state.keys == Keys::ToSetUp {
                            button {
                                class: "secondary",
                                onclick: move |_| model.write().screen = crate::model::Screen::Preparing { persona: id, replace: false },
                                "Get it ready here"
                            }
                        }
                        button { class: "secondary", onclick: move |_| mode.set(Mode::Editing), "Edit" }
                        button { class: "quiet", onclick: move |_| mode.set(Mode::Deleting), "Delete…" }
                    }
                },
                Mode::Editing => rsx! {
                    Field {
                        label: "Display name",
                        id: "display-{id}",
                        kind: Kind::Text,
                        initial: display.peek().clone(),
                        hint: display_check.hint,
                        invalid: display_check.invalid,
                        oninput: move |t| display.set(t),
                    }
                    Area {
                        label: "About this persona",
                        id: "summary-{id}",
                        initial: summary.peek().clone(),
                        hint: "Optional. Shown on your prompts and requests.",
                        oninput: move |t| summary.set(t),
                    }
                    div { class: "actions",
                        button { class: "primary", disabled: busy(), onclick: save, "Save" }
                        button { class: "quiet", onclick: move |_| mode.set(Mode::Showing), "Cancel" }
                    }
                },
                Mode::StartingOver => rsx! {
                    div { class: "notice", role: "alert",
                        p { "Start @{handle} over with a new identity key? Do this only if you can't reach the device that holds its keys: linking keeps everything." }
                        p { "Your other devices stop being able to write as @{handle}, and writers you've worked with are told its key changed. Sessions it was in will need it added again." }
                    }
                    div { class: "actions",
                        button {
                            class: "danger",
                            onclick: move |_| model.write().begin_start_over(id),
                            "Start over with a new key"
                        }
                        button { class: "quiet", onclick: move |_| mode.set(Mode::Showing), "Keep it" }
                    }
                },
                Mode::Deleting => rsx! {
                    div { class: "notice", role: "alert",
                        "Delete @{handle}? Its prompts and any requests waiting on them go with it, and the handle can't be used again. This can't be undone."
                    }
                    div { class: "actions",
                        button { class: "danger", disabled: busy(), onclick: delete, "Delete @{handle}" }
                        button { class: "quiet", onclick: move |_| mode.set(Mode::Showing), "Keep it" }
                    }
                },
            }
            if let Some(e) = problem() {
                ProblemLine { problem: e }
            }
        }
    }
}
