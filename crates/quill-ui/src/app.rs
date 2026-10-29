//! The root view: holds the [`Model`] and shows the screen it names.
//!
//! The platform provides a [`Ui`] as root context (the backend); every view
//! reaches it and the model through [`use_ui`]. Backend calls run with
//! `spawn` and hand their results to the model, as `model.rs` describes.

use std::rc::Rc;

use dioxus::prelude::*;
use uuid::Uuid;

use crate::{
    backend::{Backend, Problem, Theme},
    first_run, frame,
    model::{Model, Screen},
};

/// The backend, as the root context the platform provides.
#[derive(Clone)]
pub struct Ui(pub Rc<dyn Backend>);

/// A model to start from instead of the backend's settings (tests).
#[derive(Clone)]
pub struct StartWith(pub Model);

/// The backend and the model.
pub fn use_ui() -> (Rc<dyn Backend>, Signal<Model>) {
    (use_context::<Ui>().0, use_context::<Signal<Model>>())
}

/// Opens server `id`: the frame stays up and fills in when it's open.
pub fn open_server(backend: Rc<dyn Backend>, mut model: Signal<Model>, id: Uuid) {
    model.write().opening(id);
    spawn(async move {
        let result = backend.open_server(id).await;
        model.write().opened(id, result);
    });
}

#[component]
pub fn App() -> Element {
    let backend = use_context::<Ui>().0;
    let model = use_context_provider(|| {
        let start = try_consume_context::<StartWith>()
            .map(|s| s.0)
            .unwrap_or_else(|| Model::load(&*backend));
        Signal::new(start)
    });
    use_hook(|| {
        let m = model.peek();
        if let (Screen::Starting, Some(id)) = (m.screen, m.current) {
            drop(m);
            open_server(backend.clone(), model, id);
        }
    });

    let m = model.read();
    let theme = match m.settings.theme {
        Theme::System => "system",
        Theme::Light => "light",
        Theme::Dark => "dark",
    };
    let size = m.settings.text_size;
    rsx! {
        // Text size scales everything sized in rem.
        style { "html {{ font-size: {size}%; }}" }
        div { class: "quill", "data-theme": theme,
            match m.screen {
                Screen::Starting => rsx! { first_run::Starting {} },
                Screen::Server => rsx! { first_run::ServerStep {} },
                Screen::Unlock => rsx! { first_run::UnlockStep {} },
                Screen::Account => rsx! { first_run::AccountStep {} },
                Screen::FirstPersona => rsx! { first_run::PersonaStep {} },
                Screen::Preparing { persona } => rsx! { first_run::PreparingStep { key: "{persona}", persona } },
                Screen::Elsewhere => rsx! { first_run::ElsewhereStep {} },
                Screen::Frame => rsx! { frame::Frame {} },
            }
        }
    }
}

/// A problem, said plainly; the raw error too in advanced mode.
#[component]
pub fn ProblemLine(problem: Problem) -> Element {
    let (_, mut model) = use_ui();
    let (advanced, in_frame) = {
        let m = model.read();
        (m.settings.advanced, m.screen == Screen::Frame)
    };
    // A refusal for unaccepted rules becomes the way to accept them.
    let to_rules = in_frame && matches!(problem, Problem::RulesNotAccepted(_));
    rsx! {
        p { class: "problem", role: "alert",
            "{problem}"
            if advanced && let Some(detail) = problem.detail() {
                br {}
                span { class: "detail", "{detail}" }
            }
        }
        if to_rules {
            div { class: "actions",
                button {
                    class: "secondary",
                    onclick: move |_| model.write().go(crate::model::Section::Rules),
                    "Read the rules"
                }
            }
        }
    }
}
