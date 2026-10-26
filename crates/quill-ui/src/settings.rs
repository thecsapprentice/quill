//! The Settings section: the board, appearance, servers, this computer's
//! store, signing out, and advanced mode with its details.

use dioxus::prelude::*;
use quill_proto::board::Maturity;
use uuid::Uuid;

use crate::{
    app::{ProblemLine, open_server, use_ui},
    backend::{AppSettings, BoardSettings, Details, Problem, Protection, ServerEntry, Theme},
    field::{Field, Kind},
    model::Model,
    rules,
};

const TEXT_SIZES: [u16; 5] = [90, 100, 115, 130, 150];

fn maturity_label(m: Maturity) -> &'static str {
    match m {
        Maturity::General => "General: suitable for everyone",
        Maturity::Teen => "Teen: some mature themes",
        Maturity::Mature => "Mature: adult themes",
        Maturity::Explicit => "Explicit: everything",
    }
}

#[component]
pub fn SettingsPage() -> Element {
    let (_, model) = use_ui();
    let m = model.read();
    let signed_in = m.signed_in();
    let advanced = m.settings.advanced;
    let has_store = m.standing.is_some();
    drop(m);
    rsx! {
        h1 { "Settings" }
        if signed_in {
            BoardFieldset {}
        }
        AppearanceFieldset {}
        ServersFieldset {}
        if has_store {
            StoreFieldset {}
        }
        if signed_in {
            SignOutFieldset {}
        }
        AdvancedFieldset {}
        if advanced {
            DetailsTable {}
        }
    }
}

/// Saves the app's settings, and the model's copy of them.
fn save_app(backend: &dyn crate::Backend, mut model: Signal<Model>, new: AppSettings) {
    match backend.set_app_settings(new.clone()) {
        Ok(()) => model.write().settings_changed(new),
        Err(p) => model.write().problem = Some(p),
    }
}

#[component]
fn BoardFieldset() -> Element {
    let (backend, _) = use_ui();
    let mut board = use_signal(|| None::<BoardSettings>);
    let mut problem = use_signal(|| None::<Problem>);
    let b = backend.clone();
    use_hook(move || {
        spawn(async move {
            match b.board_settings().await {
                Ok(s) => board.set(Some(s)),
                Err(p) => problem.set(Some(p)),
            }
        });
    });
    let b = backend.clone();
    let on_change = move |e: FormEvent| {
        let Ok(m) = e.value().parse::<Maturity>() else {
            return;
        };
        let b = b.clone();
        spawn(async move {
            match b
                .set_board_settings(BoardSettings { maturity_max: m })
                .await
            {
                Ok(s) => {
                    board.set(Some(s));
                    problem.set(None);
                }
                Err(p) => problem.set(Some(p)),
            }
        });
    };
    let current = board().map(|b| b.maturity_max);
    rsx! {
        fieldset {
            legend { "The board" }
            div { class: "field",
                label { r#for: "maturity", "Show prompts up to" }
                select { id: "maturity", disabled: current.is_none(), onchange: on_change,
                    for m in Maturity::ALL.iter().copied() {
                        option {
                            key: "{m}",
                            value: "{m}",
                            selected: Some(m) == current,
                            "{maturity_label(m)}"
                        }
                    }
                }
                p { class: "hint", "Kept with your login on this server, so it follows you to other devices." }
            }
            if let Some(p) = problem() {
                ProblemLine { problem: p }
            }
        }
    }
}

#[component]
fn AppearanceFieldset() -> Element {
    let (backend, model) = use_ui();
    let settings = model.read().settings.clone();
    let themes = [
        (Theme::System, "Match this computer"),
        (Theme::Light, "Light"),
        (Theme::Dark, "Dark"),
    ];
    let b = backend.clone();
    let s = settings.clone();
    let on_size = move |e: FormEvent| {
        if let Ok(size) = e.value().parse() {
            save_app(
                &*b,
                model,
                AppSettings {
                    text_size: size,
                    ..s.clone()
                },
            );
        }
    };
    rsx! {
        fieldset {
            legend { "Appearance" }
            for (theme, label) in themes {
                div { key: "{label}", class: "choice",
                    input {
                        r#type: "radio",
                        id: "theme-{label}",
                        name: "theme",
                        checked: settings.theme == theme,
                        onchange: {
                            let b = backend.clone();
                            let s = settings.clone();
                            move |_| save_app(&*b, model, AppSettings { theme, ..s.clone() })
                        },
                    }
                    label { r#for: "theme-{label}", "{label}" }
                }
            }
            div { class: "field",
                label { r#for: "text-size", "Text size" }
                select { id: "text-size", onchange: on_size,
                    for size in TEXT_SIZES {
                        option {
                            key: "{size}",
                            value: "{size}",
                            selected: settings.text_size == size,
                            "{size}%"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ServersFieldset() -> Element {
    let (_, mut model) = use_ui();
    let servers = model.read().servers.clone();
    rsx! {
        fieldset {
            legend { "Servers" }
            ul { class: "list",
                for s in servers {
                    ServerItem { key: "{s.id}", server: s }
                }
            }
            button { class: "secondary", onclick: move |_| model.write().begin_add(), "Add a server" }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Showing,
    Renaming,
    Removing,
}

#[component]
fn ServerItem(server: ServerEntry) -> Element {
    let (backend, mut model) = use_ui();
    let mut mode = use_signal(|| Mode::Showing);
    let mut name = use_signal(|| server.name.clone());
    let mut busy = use_signal(|| false);
    let mut problem = use_signal(|| None::<Problem>);
    let id = server.id;
    let in_view = model.read().current == Some(id);

    let b = backend.clone();
    let rename = use_callback(move |()| match b.rename_server(id, &name()) {
        Ok(()) => {
            model.write().refresh_servers(&*b);
            mode.set(Mode::Showing);
            problem.set(None);
        }
        Err(p) => problem.set(Some(p)),
    });
    let b = backend.clone();
    let remove = move |_| {
        busy.set(true);
        let b = b.clone();
        spawn(async move {
            let result = b.remove_server(id).await;
            busy.set(false);
            match result {
                Ok(()) => {
                    let next: Option<Uuid> = model.write().removed(&*b, id);
                    if let Some(next) = next {
                        open_server(b.clone(), model, next);
                    }
                }
                Err(p) => problem.set(Some(p)),
            }
        });
    };
    let b = backend.clone();
    rsx! {
        li { class: "item",
            header {
                span { class: "name", "{server.name}" }
                span { class: "handle", "{server.address}" }
                if in_view {
                    span { class: "tag", "Open" }
                }
            }
            if let Some(login) = &server.login {
                p { "Signed in as {login}" }
            }
            match mode() {
                Mode::Showing => rsx! {
                    div { class: "actions",
                        if !in_view {
                            button {
                                class: "secondary",
                                onclick: move |_| open_server(b.clone(), model, id),
                                "Open"
                            }
                        }
                        button { class: "secondary", onclick: move |_| mode.set(Mode::Renaming), "Rename" }
                        button { class: "quiet", onclick: move |_| mode.set(Mode::Removing), "Remove…" }
                    }
                },
                Mode::Renaming => rsx! {
                    Field {
                        label: "Name in Quill",
                        id: "rename-{id}",
                        kind: Kind::Text,
                        initial: name.peek().clone(),
                        autofocus: true,
                        hint: "Only shown on this computer.",
                        oninput: move |t| name.set(t),
                        onenter: rename,
                    }
                    div { class: "actions",
                        button { class: "primary", onclick: move |_| rename.call(()), "Save" }
                        button { class: "quiet", onclick: move |_| mode.set(Mode::Showing), "Cancel" }
                    }
                },
                Mode::Removing => rsx! {
                    div { class: "notice", role: "alert",
                        "Remove {server.name} from Quill? This deletes this computer's copy: your sign-in, and the keys made here. Your login and personas stay on the server, but keys that are only on this computer are lost."
                    }
                    div { class: "actions",
                        button { class: "danger", disabled: busy(), onclick: remove, "Remove {server.name}" }
                        button { class: "quiet", onclick: move |_| mode.set(Mode::Showing), "Keep it" }
                    }
                },
            }
            if let Some(p) = problem() {
                ProblemLine { problem: p }
            }
        }
    }
}

#[component]
fn StoreFieldset() -> Element {
    let (backend, _) = use_ui();
    let mut protection = use_signal(|| backend.protection());
    let mut changing = use_signal(|| false);
    let mut passphrase = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut problem = use_signal(|| None::<Problem>);
    let keychain = backend.keychain_available();

    let b = backend.clone();
    let apply = use_callback(move |to_keychain: bool| {
        let pass = (!to_keychain).then(|| passphrase.read().clone());
        if busy()
            || pass
                .as_deref()
                .is_some_and(|p| rules::passphrase(p).invalid || p.is_empty())
        {
            return;
        }
        busy.set(true);
        let b = b.clone();
        spawn(async move {
            let result = b.set_protection(pass.as_deref()).await;
            busy.set(false);
            match result {
                Ok(()) => {
                    changing.set(false);
                    problem.set(None);
                    protection.set(b.protection());
                }
                Err(p) => problem.set(Some(p)),
            }
        });
    });
    let check = rules::passphrase(&passphrase());
    let said = match protection() {
        Some(Protection::Keychain) => {
            "Locked with this computer's keychain, so there's nothing to type."
        }
        Some(Protection::Passphrase) => {
            "Locked with a passphrase you type when Quill opens this server."
        }
        None => "Locked. Unlock it to change how.",
    };
    rsx! {
        fieldset {
            legend { "This computer's copy" }
            p { class: "hint",
                "Quill keeps your sign-in and your keys for this server in an encrypted file on this computer. {said}"
            }
            if changing() {
                Field {
                    label: "New passphrase",
                    id: "new-store-passphrase",
                    kind: Kind::NewPassword,
                    autofocus: true,
                    hint: check.hint,
                    invalid: check.invalid,
                    oninput: move |t| passphrase.set(t),
                    onenter: move |_| apply.call(false),
                }
                div { class: "actions",
                    button { class: "primary", disabled: busy(), onclick: move |_| apply.call(false), "Use this passphrase" }
                    button { class: "quiet", onclick: move |_| changing.set(false), "Cancel" }
                }
            } else if protection().is_some() {
                div { class: "actions",
                    button {
                        class: "secondary",
                        onclick: move |_| changing.set(true),
                        if protection() == Some(Protection::Passphrase) { "Change the passphrase" } else { "Use a passphrase instead" }
                    }
                    if keychain && protection() == Some(Protection::Passphrase) {
                        button {
                            class: "secondary",
                            disabled: busy(),
                            onclick: move |_| apply.call(true),
                            "Use the keychain instead"
                        }
                    }
                }
            }
            if !keychain {
                p { class: "hint", "This computer has no keychain Quill can use, so a passphrase is the only choice." }
            }
            if let Some(p) = problem() {
                ProblemLine { problem: p }
            }
        }
    }
}

#[component]
fn SignOutFieldset() -> Element {
    let (backend, mut model) = use_ui();
    let name = model.read().server_name();
    let mut busy = use_signal(|| false);
    let sign_out = move |_| {
        busy.set(true);
        let b = backend.clone();
        spawn(async move {
            let result = b.sign_out().await;
            busy.set(false);
            match result {
                Ok(()) => model.write().signed_out(),
                Err(p) => model.write().problem = Some(p),
            }
        });
    };
    rsx! {
        fieldset {
            legend { "Signing out" }
            p { class: "hint",
                "Signing out keeps this computer's keys, so signing in again picks up where you left off."
            }
            button { class: "secondary", disabled: busy(), onclick: sign_out, "Sign out of {name}" }
        }
    }
}

#[component]
fn AdvancedFieldset() -> Element {
    let (backend, model) = use_ui();
    let settings = model.read().settings.clone();
    let mut folder = use_signal(|| settings.stores_folder.clone());
    let mut busy = use_signal(|| false);
    let mut problem = use_signal(|| None::<Problem>);
    let mut moved = use_signal(|| false);

    let b = backend.clone();
    let s = settings.clone();
    let toggle = move |e: FormEvent| {
        save_app(
            &*b,
            model,
            AppSettings {
                advanced: e.checked(),
                ..s.clone()
            },
        );
    };
    let b = backend.clone();
    let move_stores = use_callback(move |()| {
        if busy() {
            return;
        }
        busy.set(true);
        moved.set(false);
        let b = b.clone();
        let mut model = model;
        spawn(async move {
            let result = b.move_stores(&folder()).await;
            busy.set(false);
            match result {
                Ok(()) => {
                    moved.set(true);
                    problem.set(None);
                    model.write().settings_changed(b.app_settings());
                }
                Err(p) => problem.set(Some(p)),
            }
        });
    });
    rsx! {
        fieldset {
            legend { "Advanced" }
            div { class: "choice",
                input {
                    r#type: "checkbox",
                    id: "advanced",
                    checked: settings.advanced,
                    onchange: toggle,
                }
                label { r#for: "advanced", "Advanced mode: show ids, keys and what Quill does in the background" }
            }
            if settings.advanced {
                Field {
                    label: "Folder for this computer's copies",
                    id: "stores-folder",
                    kind: Kind::Name,
                    initial: settings.stores_folder.clone(),
                    hint: if moved() { "Moved." } else { "Changing it moves the files there." },
                    oninput: move |t| folder.set(t),
                    onenter: move_stores,
                }
                div { class: "actions",
                    button { class: "secondary", disabled: busy(), onclick: move |_| move_stores.call(()), "Move them" }
                }
                if let Some(p) = problem() {
                    ProblemLine { problem: p }
                }
            }
        }
    }
}

#[component]
fn DetailsTable() -> Element {
    let (backend, model) = use_ui();
    let mut details = use_signal(|| None::<Result<Details, Problem>>);
    // Reload when the server or where its login stands changes.
    let key = use_memo(move || {
        let m = model.read();
        format!("{:?}-{:?}", m.current, m.status)
    });
    use_effect(move || {
        key.read();
        let b = backend.clone();
        spawn(async move {
            let d = b.details().await;
            details.set(Some(d));
        });
    });
    rsx! {
        section { aria_label: "Details",
            h2 { "Details" }
            match details() {
                None => rsx! { p { class: "coming", "Loading…" } },
                Some(Err(p)) => rsx! { ProblemLine { problem: p } },
                Some(Ok(d)) => rsx! {
                    table { class: "details",
                        tbody {
                            for (i, (k, v)) in d.rows.into_iter().enumerate() {
                                tr { key: "{i}",
                                    th { scope: "row", "{k}" }
                                    td { "{v}" }
                                }
                            }
                        }
                    }
                },
            }
        }
    }
}
