//! The app's frame: the server and persona switchers, the sections, the
//! status line, and the upkeep loop that runs for the server in view.

use std::{future::poll_fn, rc::Rc, task::Poll};

use dioxus::prelude::*;
use uuid::Uuid;

use crate::{
    app::{ProblemLine, open_server, use_ui},
    backend::{Backend, Keys},
    model::{Section, Status},
    personas::PersonasPage,
    server_rules::RulesPage,
    settings::SettingsPage,
};

/// Upkeep's pace while connected, and the longest wait while offline.
const UPKEEP_EVERY_MS: u64 = 5 * 60 * 1000;
const RETRY_FIRST_MS: u64 = 5_000;
const RETRY_MOST_MS: u64 = 2 * 60 * 1000;
/// The persona switcher's value for "make a new persona".
const NEW: &str = "new";

#[component]
pub fn Frame() -> Element {
    let (_, mut model) = use_ui();
    let m = model.read();
    let section = m.section;
    let rules_banner = m.rules_to_accept() && section != Section::Rules;
    let key = format!("{}-{}", m.current.unwrap_or_default(), m.upkeep_round);
    let server = m.current;
    let notice = m.notice.clone();
    let problem = m.problem.clone();
    drop(m);
    rsx! {
        div { class: "frame",
            Nav {}
            main { class: "main", id: "main",
                div { class: "page",
                    if rules_banner {
                        div { class: "nudge", role: "status",
                            p { "The server rules have changed. Read and accept them to keep posting, editing, commenting and reporting." }
                            button { class: "primary", onclick: move |_| model.write().go(Section::Rules), "Read the rules" }
                        }
                    }
                    if let Some(text) = notice {
                        div { class: "notice", role: "status", "{text}" }
                    }
                    if let Some(p) = problem {
                        ProblemLine { problem: p }
                    }
                    match section {
                        Section::Home => rsx! { Home {} },
                        Section::Board => rsx! {
                            Coming {
                                title: "Board",
                                text: "The prompt board comes next: browse prompts, post your own, and ask to write with the people behind them.",
                            }
                        },
                        Section::Requests => rsx! {
                            Coming {
                                title: "Requests",
                                text: "Requests to write together will show here: the ones you've sent, and the ones waiting on your prompts.",
                            }
                        },
                        Section::Sessions => rsx! {
                            Coming {
                                title: "Sessions",
                                text: "Your writing sessions will live here, with a writing space that checks your spelling and previews Markdown as you type.",
                            }
                        },
                        Section::Personas => rsx! { PersonasPage {} },
                        Section::Settings => rsx! { SettingsPage {} },
                        Section::Rules => rsx! { RulesPage {} },
                    }
                }
            }
            StatusLine {}
            if let Some(server) = server {
                // Keyed: a new server, or "Try now", restarts the loop.
                Upkeep { key: "{key}", server }
            }
        }
    }
}

#[component]
fn Nav() -> Element {
    let (backend, mut model) = use_ui();
    let m = model.read();
    let servers = m.servers.clone();
    let current = m.current.map(|c| c.to_string()).unwrap_or_default();
    let personas = m.personas().to_vec();
    let persona = m.persona.map(|p| p.to_string()).unwrap_or_default();
    let section = m.section;
    let signed_in = m.signed_in();
    drop(m);

    let b = backend.clone();
    let on_server = move |e: FormEvent| match e.value().as_str() {
        NEW => model.write().begin_add(),
        id => {
            if let Ok(id) = id.parse::<Uuid>() {
                open_server(b.clone(), model, id);
            }
        }
    };
    let on_persona = move |e: FormEvent| match e.value().as_str() {
        NEW => model.write().go(Section::Personas),
        id => {
            if let Ok(id) = id.parse() {
                model.write().choose_persona(id);
            }
        }
    };
    rsx! {
        nav { class: "nav", aria_label: "Quill",
            div { class: "switchers",
                div {
                    label { r#for: "server-switch", "Server" }
                    select { id: "server-switch", onchange: on_server,
                        for s in servers {
                            option {
                                key: "{s.id}",
                                value: "{s.id}",
                                selected: s.id.to_string() == current,
                                "{s.name}"
                            }
                        }
                        option { value: NEW, "Add a server…" }
                    }
                }
                if signed_in {
                    div {
                        label { r#for: "persona-switch", "Writing as" }
                        select { id: "persona-switch", onchange: on_persona,
                            if persona.is_empty() {
                                option { value: "", selected: true, disabled: true, "No persona yet" }
                            }
                            for p in personas.iter().filter(|p| p.keys == Keys::Ready) {
                                option {
                                    key: "{p.persona.id}",
                                    value: "{p.persona.id}",
                                    selected: p.persona.id.to_string() == persona,
                                    "@{p.persona.handle}"
                                }
                            }
                            option { value: NEW, "Make a persona…" }
                        }
                    }
                }
            }
            for s in Section::ALL {
                button {
                    key: "{s.label()}",
                    class: "section",
                    aria_current: if s == section { "page" } else { "false" },
                    onclick: move |_| model.write().go(s),
                    "{s.label()}"
                }
            }
        }
    }
}

#[component]
fn StatusLine() -> Element {
    let (_, mut model) = use_ui();
    let m = model.read();
    let name = m.server_name();
    let status = m.status;
    drop(m);
    let (class, text) = match status {
        Status::Connecting => ("status", format!("Connecting to {name}…")),
        Status::Connected => ("status", format!("Connected to {name}")),
        Status::Offline => (
            "status offline",
            format!("Can't reach {name}. Trying again in a moment."),
        ),
        Status::SignedOut => ("status offline", format!("Signed out of {name}")),
        Status::Locked => ("status offline", format!("{name} is locked")),
    };
    rsx! {
        footer { class, role: "status",
            span { class: "dot", aria_hidden: "true" }
            span { "{text}" }
            match status {
                Status::Offline => rsx! {
                    button { class: "quiet", onclick: move |_| model.write().try_now(), "Try now" }
                },
                Status::SignedOut => rsx! {
                    button { class: "quiet", onclick: move |_| model.write().sign_in_again(), "Sign in" }
                },
                Status::Locked => rsx! {
                    button {
                        class: "quiet",
                        onclick: move |_| model.write().screen = crate::model::Screen::Unlock,
                        "Unlock"
                    }
                },
                _ => rsx! {},
            }
        }
    }
}

/// Waits `ms`, or less if a live notice comes first (when `listen`).
async fn wait(backend: &dyn Backend, ms: u64, listen: bool) {
    let mut sleep = backend.sleep(ms);
    let mut notice = listen.then(|| backend.next_notice());
    poll_fn(|cx| {
        if sleep.as_mut().poll(cx).is_ready() {
            return Poll::Ready(());
        }
        if let Some(n) = notice.as_mut() {
            match n.as_mut().poll(cx) {
                Poll::Ready(Ok(())) => return Poll::Ready(()),
                // No live notices (offline, or an old server): just wait.
                Poll::Ready(Err(_)) => notice = None,
                Poll::Pending => {}
            }
        }
        Poll::Pending
    })
    .await;
}

/// Background upkeep for `server`: on start, every few minutes, and on
/// each live notice. Offline, it retries sooner, backing off.
#[component]
fn Upkeep(server: Uuid) -> Element {
    let (backend, mut model) = use_ui();
    use_hook(move || {
        let b: Rc<dyn Backend> = backend.clone();
        spawn(async move {
            let mut retry = RETRY_FIRST_MS;
            // An open in flight gets one short wait before the loop loads
            // the server itself, so a lost reply can't leave it connecting.
            let mut opening_waited = false;
            loop {
                let (loaded, signed_in, status) = {
                    let m = model.peek();
                    (m.standing.is_some(), m.signed_in(), m.status)
                };
                if !loaded && (status != Status::Connecting || opening_waited) {
                    // Opened while offline: load where the login stands.
                    let result = b.standing().await;
                    model.write().opened(server, result);
                } else if signed_in {
                    let result = b.upkeep().await;
                    model.write().upkept(server, result);
                }
                // The rules too, to notice a change that needs accepting.
                let reachable = {
                    let m = model.peek();
                    m.standing.is_some() && m.status != Status::Offline
                };
                if reachable {
                    let rules = b.rules().await;
                    model.write().rules_loaded(server, rules);
                }
                let (status, signed_in) = {
                    let m = model.peek();
                    (m.status, m.signed_in())
                };
                match status {
                    Status::Offline => {
                        wait(&*b, retry, false).await;
                        retry = (retry * 2).min(RETRY_MOST_MS);
                    }
                    // Still opening: look again shortly.
                    Status::Connecting => {
                        wait(&*b, RETRY_FIRST_MS, false).await;
                        opening_waited = true;
                    }
                    _ => {
                        retry = RETRY_FIRST_MS;
                        wait(&*b, UPKEEP_EVERY_MS, signed_in).await;
                    }
                }
            }
        });
    });
    rsx! {}
}

#[component]
fn Home() -> Element {
    let (_, mut model) = use_ui();
    let m = model.read();
    let name = m.server_name();
    let in_use = m.persona_in_use().map(|p| {
        p.persona
            .display_name
            .clone()
            .unwrap_or_else(|| format!("@{}", p.persona.handle))
    });
    let no_persona = m.signed_in() && m.personas().is_empty();
    let signed_out = m.status == Status::SignedOut;
    let nudge = !m.backup_nudge_dismissed && in_use.is_some();
    drop(m);
    rsx! {
        h1 {
            match &in_use {
                Some(who) => rsx! { "Hello, {who}" },
                None => rsx! { "Welcome to {name}" },
            }
        }
        if signed_out {
            div { class: "nudge",
                p { "You're signed out of {name}. Sign in to see your personas and keep your keys up to date." }
                button { class: "primary", onclick: move |_| model.write().sign_in_again(), "Sign in" }
            }
        }
        if no_persona {
            div { class: "nudge",
                p { "Writing and posting need a persona: a name you write under. It takes a moment to make one." }
                button { class: "primary", onclick: move |_| model.write().go(Section::Personas), "Make a persona" }
            }
        }
        if nudge {
            div { class: "nudge",
                p { "Your persona's keys live only on this computer for now. Quill will soon be able to back them up, and will remind you here when it can." }
                button {
                    class: "quiet",
                    onclick: move |_| model.write().backup_nudge_dismissed = true,
                    "Dismiss"
                }
            }
        }
        p { class: "coming",
            "The prompt board, requests and writing sessions are on their way. For now you can make personas and set Quill up the way you like in Settings."
        }
    }
}

#[component]
fn Coming(title: String, text: String) -> Element {
    rsx! {
        h1 { "{title}" }
        p { class: "coming", "{text}" }
    }
}
