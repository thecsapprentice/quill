//! First run, one question per screen: which server, new or returning,
//! the first persona, then getting it ready. Also the screens that can
//! come up when a server opens: unlock, and keys on another device.

use dioxus::prelude::*;

use crate::{
    app::{ProblemLine, open_server, use_ui},
    backend::{Keys, Persona, Problem, ServerInfo, ServerRules, Step},
    checks,
    field::{Field, Kind},
    server_rules::SignUpRules,
};

/// How long typing must pause before the address is checked.
const CHECK_AFTER_MS: u64 = 400;

#[component]
pub fn Starting() -> Element {
    let (_, model) = use_ui();
    let name = model.read().server_name();
    rsx! {
        div { class: "welcome",
            p { class: "coming", role: "status", "Opening {name}…" }
        }
    }
}

/// A card in the middle of the window, for one question.
#[component]
fn Card(title: String, lede: String, children: Element) -> Element {
    rsx! {
        main { class: "welcome",
            section { class: "card", aria_labelledby: "card-title",
                h1 { id: "card-title", "{title}" }
                p { class: "lede", "{lede}" }
                {children}
            }
        }
    }
}

#[component]
pub fn ServerStep() -> Element {
    let (backend, mut model) = use_ui();
    let mut address = use_signal(String::new);
    let mut round = use_signal(|| 0u32);
    let mut found = use_signal(|| None::<ServerInfo>);
    let mut problem = use_signal(|| None::<Problem>);
    let mut need_passphrase = use_signal(|| false);
    let mut passphrase = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let adding = model.read().adding.is_some();

    let b = backend.clone();
    let on_address = move |text: String| {
        address.set(text.clone());
        found.set(None);
        problem.set(None);
        let mine = round() + 1;
        round.set(mine);
        if text.trim().is_empty() {
            return;
        }
        let b = b.clone();
        spawn(async move {
            b.sleep(CHECK_AFTER_MS).await;
            if round() != mine {
                return;
            }
            let result = b.check_server(&text).await;
            if round() != mine {
                return;
            }
            match result {
                Ok(info) => found.set(Some(info)),
                Err(p) => problem.set(Some(p)),
            }
        });
    };

    let b = backend.clone();
    let add = use_callback(move |()| {
        if busy() || found().is_none() {
            return;
        }
        let pass = need_passphrase().then(|| passphrase.read().clone());
        if pass
            .as_deref()
            .is_some_and(|p| checks::passphrase(p).invalid || p.is_empty())
        {
            return;
        }
        busy.set(true);
        let b = b.clone();
        spawn(async move {
            let result = b.add_server(&address(), pass.as_deref()).await;
            busy.set(false);
            match result {
                Ok(entry) => model.write().added(&*b, &entry),
                Err(Problem::NeedsPassphrase) => need_passphrase.set(true),
                Err(p) => problem.set(Some(p)),
            }
        });
    });

    let b = backend.clone();
    let cancel = move |_| {
        let back = model.write().cancel_add();
        if let Some(id) = back {
            open_server(b.clone(), model, id);
        }
    };

    // While typing, "can't reach" is likely a half-typed address: say it
    // gently rather than as an error.
    let (hint, invalid) = match problem() {
        Some(Problem::Unreachable(_)) => (
            "No Quill server answers there yet. Check the address, or your connection.".to_owned(),
            false,
        ),
        Some(p) => (p.to_string(), true),
        None if found().is_some() => ("Found it.".to_owned(), false),
        None => ("It looks like writers.example.".to_owned(), false),
    };
    let pass_check = checks::passphrase(&passphrase());
    let (title, lede) = if adding {
        ("Add a server", "Which other server do you use?")
    } else {
        (
            "Welcome to Quill",
            "Quill is a place to find writing prompts and write them together. Which server do you use?",
        )
    };
    rsx! {
        Card { title, lede,
            Field {
                label: "Server address",
                id: "server-address",
                kind: Kind::Name,
                placeholder: "writers.example",
                autofocus: true,
                hint,
                invalid,
                oninput: on_address,
                onenter: add,
            }
            if let Some(info) = found() {
                div { class: "server-found", role: "status",
                    strong { "{info.domain}" }
                    if info.registration_open {
                        "Takes new logins."
                    } else {
                        "Not taking new logins. You can still sign in."
                    }
                    p { class: "hint",
                        "It runs the public prompt board. Your writing sessions are end-to-end encrypted: the server passes them on but can't read them."
                    }
                }
            }
            if need_passphrase() {
                div { class: "notice",
                    "This computer has no keychain for Quill to keep its key in, so choose a passphrase. You'll type it each time Quill opens this server."
                }
                Field {
                    label: "Passphrase for this computer",
                    id: "store-passphrase",
                    kind: Kind::NewPassword,
                    autofocus: true,
                    hint: pass_check.hint,
                    invalid: pass_check.invalid,
                    oninput: move |t| passphrase.set(t),
                    onenter: add,
                }
            }
            div { class: "actions",
                button {
                    class: "primary",
                    disabled: busy() || found().is_none(),
                    onclick: move |_| add.call(()),
                    if busy() { "Setting up…" } else { "Continue" }
                }
                if adding {
                    button { class: "quiet", onclick: cancel, "Cancel" }
                }
            }
        }
    }
}

#[component]
pub fn UnlockStep() -> Element {
    let (backend, mut model) = use_ui();
    let mut passphrase = use_signal(String::new);
    let mut generation = use_signal(|| 0u32);
    let mut busy = use_signal(|| false);
    let name = model.read().server_name();
    let others: Vec<_> = {
        let m = model.read();
        m.servers
            .iter()
            .filter(|s| Some(s.id) != m.current)
            .cloned()
            .collect()
    };
    let problem = model.read().problem.clone();

    let b = backend.clone();
    let unlock = use_callback(move |()| {
        let Some(id) = model.peek().current else {
            return;
        };
        if busy() || passphrase().is_empty() {
            return;
        }
        busy.set(true);
        let b = b.clone();
        spawn(async move {
            let result = b.unlock(&passphrase()).await;
            busy.set(false);
            if result.is_err() {
                // Clear the field for the next try.
                generation += 1;
                passphrase.set(String::new());
            }
            model.write().opened(id, result);
        });
    });
    rsx! {
        Card {
            title: format!("Unlock {name}"),
            lede: "Type the passphrase you chose for this server on this computer.",
            Field {
                label: "Passphrase",
                id: "unlock-passphrase",
                kind: Kind::Password,
                autofocus: true,
                generation: generation(),
                oninput: move |t| passphrase.set(t),
                onenter: unlock,
            }
            if let Some(p) = problem {
                ProblemLine { problem: p }
            }
            div { class: "actions",
                button { class: "primary", disabled: busy(), onclick: move |_| unlock.call(()),
                    if busy() { "Unlocking…" } else { "Unlock" }
                }
                for s in others {
                    button {
                        key: "{s.id}",
                        class: "quiet",
                        onclick: {
                            let b = backend.clone();
                            move |_| open_server(b.clone(), model, s.id)
                        },
                        "Open {s.name} instead"
                    }
                }
            }
        }
    }
}

#[component]
pub fn AccountStep() -> Element {
    let (backend, mut model) = use_ui();
    let mut returning = use_signal(|| false);
    let mut open_for_new = use_signal(|| true);
    let mut login = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut generation = use_signal(|| 0u32);
    let mut taken = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut problem = use_signal(|| model.peek().problem.clone());
    // The server's rules, and whether a new login must accept them.
    let mut rules = use_signal(|| None::<ServerRules>);
    let mut rules_to_accept = use_signal(|| false);
    let mut accepted = use_signal(|| false);
    let (name, adding, known) = {
        let m = model.read();
        (
            m.server_name(),
            m.adding.is_some(),
            m.server().is_some_and(|s| s.login.is_some()),
        )
    };

    // Ask once whether the server takes new logins, and for its rules.
    let b = backend.clone();
    use_hook(move || {
        if known {
            returning.set(true);
        }
        let address = model.peek().server().map(|s| s.address.clone());
        if let Some(address) = address {
            spawn(async move {
                if let Ok(info) = b.check_server(&address).await {
                    if !info.registration_open {
                        open_for_new.set(false);
                        returning.set(true);
                    }
                    rules_to_accept.set(info.rules_to_accept);
                    if info.has_rules
                        && let Ok(r) = b.rules().await
                    {
                        rules.set(r);
                    }
                }
            });
        }
    });
    // Loading them again, if that failed when they must be accepted.
    let b = backend.clone();
    let reload_rules = move |_| {
        let b = b.clone();
        spawn(async move {
            match b.rules().await {
                Ok(r) => rules.set(r),
                Err(p) => problem.set(Some(p)),
            }
        });
    };

    let b = backend.clone();
    let submit = use_callback(move |()| {
        let Some(id) = model.peek().current else {
            return;
        };
        let new = !returning();
        let bad = checks::login_name(&login()).invalid
            || login().trim().is_empty()
            || (new && (checks::password(&password()).invalid || password().is_empty()))
            || (new && rules_to_accept() && !accepted());
        if busy() || bad {
            return;
        }
        busy.set(true);
        problem.set(None);
        taken.set(false);
        let b = b.clone();
        spawn(async move {
            let version = accepted().then(|| rules.peek().as_ref().map(|r| r.version));
            let result = if new {
                b.register(&login(), &password(), version.flatten()).await
            } else {
                b.sign_in(&login(), &password()).await
            };
            busy.set(false);
            match result {
                Ok(standing) => model.write().signed_in_now(&*b, id, standing),
                Err(Problem::Taken) => taken.set(true),
                Err(Problem::RulesNotAccepted(text)) => {
                    // The server wants the rules accepted: set since this
                    // screen asked, or changed while signing up. Show the
                    // current version, with the box to tick.
                    rules_to_accept.set(true);
                    accepted.set(false);
                    if let Ok(r) = b.rules().await {
                        rules.set(r);
                    }
                    problem.set(Some(Problem::RulesNotAccepted(text)));
                }
                Err(p) => problem.set(Some(p)),
            }
        });
    });

    let b = backend.clone();
    let back = move |_| {
        if adding {
            let back = model.write().cancel_add();
            if let Some(id) = back {
                open_server(b.clone(), model, id);
            }
        } else {
            model.write().go(crate::model::Section::Home);
        }
    };

    let login_check = if taken() {
        checks::Check {
            hint: "That login name is taken. Try another.".into(),
            invalid: true,
        }
    } else {
        checks::login_name(&login())
    };
    let new = !returning();
    let pass_check = if new {
        checks::password(&password())
    } else {
        checks::Check {
            hint: String::new(),
            invalid: false,
        }
    };
    let suggestions = if taken() {
        checks::suggestions(&login())
    } else {
        Vec::new()
    };
    let (title, lede) = if new {
        (
            format!("Make a login on {name}"),
            "Your login is private: other writers see only your personas, which you make next.",
        )
    } else {
        (
            format!("Sign in to {name}"),
            "Welcome back. Sign in with the login you made on this server.",
        )
    };
    rsx! {
            Card { title, lede,
                if open_for_new() {
                    div { class: "tabs", role: "group", aria_label: "New or returning",
                        button {
                            aria_pressed: if new { "true" } else { "false" },
                            onclick: move |_| { returning.set(false); problem.set(None); },
                            "I'm new here"
                        }
                        button {
                            aria_pressed: if new { "false" } else { "true" },
                            onclick: move |_| { returning.set(true); problem.set(None); },
                            "I have a login"
                        }
                    }
                } else {
                    div { class: "notice",
                        "{name} isn't taking new logins right now. If you have one, sign in."
                    }
                }
                Field {
                    label: "Login name",
                    id: "login-name",
                    kind: Kind::Name,
                    autofocus: true,
                    initial: login.peek().clone(),
                    generation: generation(),
                    hint: login_check.hint,
                    invalid: login_check.invalid,
                    oninput: move |t| { login.set(t); taken.set(false); },
                    onenter: submit,
                }
                if !suggestions.is_empty() {
                    div { class: "suggestions", role: "group", aria_label: "Names to try",
                        for s in suggestions {
                            button {
                                key: "{s}",
                                onclick: {
                                    let s = s.clone();
                                    move |_| {
                                        login.set(s.clone());
                                        taken.set(false);
                                        generation += 1;
                                    }
                                },
                                "{s}"
                            }
                        }
                    }
                }
                // Keyed by tab: switching remounts it with the right hints.
                for k in [new] {
                Field {
                    key: "{k}",
                    label: "Password",
                    id: "login-password",
                    kind: if new { Kind::NewPassword } else { Kind::Password },
                    hint: pass_check.hint.clone(),
                    invalid: pass_check.invalid,
                    oninput: move |t| password.set(t),
                    onenter: submit,
                }
                }
                if new && let Some(r) = rules() {
                    SignUpRules {
                        rules: r,
                        required: rules_to_accept(),
                        accepted: accepted(),
                        on_accept: move |yes| accepted.set(yes),
                    }
                } else if new && rules_to_accept() {
                    // Never a dead end: the box can't show without the rules.
                    div { class: "notice", role: "status",
                        "{name} asks new logins to accept its rules, and Quill couldn't load them."
                    }
                    div { class: "actions",
                        button { class: "secondary", onclick: reload_rules, "Load the rules" }
                    }
                }
                if let Some(p) = problem() {
                    ProblemLine { problem: p }
                }
                div { class: "actions",
    button {
                        class: "primary",
                        disabled: busy() || (new && rules_to_accept() && !accepted()),
                        onclick: move |_| submit.call(()),
                        match (busy(), new) {
                            (true, _) => "One moment…",
                            (false, true) => "Make my login",
                            (false, false) => "Sign in",
                        }
                    }
                    if adding {
                        button { class: "quiet", onclick: back, "Cancel" }
                    } else if known {
                        button { class: "quiet", onclick: back, "Not now" }
                    }
                }
            }
        }
}

/// Handle and display name for a new persona; used by first run and the
/// Personas page. Says a taken handle plainly, with handles to try.
#[component]
pub fn PersonaForm(
    submit_label: String,
    on_created: EventHandler<Persona>,
    on_cancel: EventHandler<()>,
    cancel_label: String,
) -> Element {
    let (backend, _) = use_ui();
    let mut handle = use_signal(String::new);
    let mut display = use_signal(String::new);
    let mut generation = use_signal(|| 0u32);
    let mut taken = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let mut problem = use_signal(|| None::<Problem>);

    let create = use_callback(move |()| {
        let bad = checks::handle(&handle()).invalid
            || handle().trim().is_empty()
            || checks::display_name(&display()).invalid;
        if busy() || bad {
            return;
        }
        busy.set(true);
        problem.set(None);
        let b = backend.clone();
        spawn(async move {
            let result = b.create_persona(&handle(), &display()).await;
            busy.set(false);
            match result {
                Ok(p) => on_created.call(p),
                Err(Problem::Taken) => taken.set(Some(handle().trim().to_ascii_lowercase())),
                Err(p) => problem.set(Some(p)),
            }
        });
    });

    let handle_check = match taken() {
        Some(t) => checks::Check {
            hint: format!("@{t} is taken. Try one of these, or another."),
            invalid: true,
        },
        None => checks::handle(&handle()),
    };
    let display_check = checks::display_name(&display());
    let suggestions = taken().map(|t| checks::suggestions(&t)).unwrap_or_default();
    rsx! {
        Field {
            label: "Handle",
            id: "persona-handle",
            kind: Kind::Name,
            autofocus: true,
            initial: handle.peek().clone(),
            generation: generation(),
            hint: handle_check.hint,
            invalid: handle_check.invalid,
            oninput: move |t| { handle.set(t); taken.set(None); },
            onenter: create,
        }
        if !suggestions.is_empty() {
            div { class: "suggestions", role: "group", aria_label: "Handles to try",
                for s in suggestions {
                    button {
                        key: "{s}",
                        onclick: {
                            let s = s.clone();
                            move |_| {
                                handle.set(s.clone());
                                taken.set(None);
                                generation += 1;
                            }
                        },
                        "@{s}"
                    }
                }
            }
        }
        Field {
            label: "Display name",
            id: "persona-display-name",
            kind: Kind::Text,
            hint: display_check.hint,
            invalid: display_check.invalid,
            oninput: move |t| display.set(t),
            onenter: create,
        }
        if let Some(p) = problem() {
            ProblemLine { problem: p }
        }
        div { class: "actions",
            button { class: "primary", disabled: busy(), onclick: move |_| create.call(()),
                if busy() { "Making it…" } else { "{submit_label}" }
            }
            button { class: "quiet", onclick: move |_| on_cancel.call(()), "{cancel_label}" }
        }
    }
}

#[component]
pub fn PersonaStep() -> Element {
    let (_, mut model) = use_ui();
    rsx! {
        Card {
            title: "Make your first persona",
            lede: "A persona is the name you write under. Other writers see its handle and display name, never your login. You need one to post prompts and write with others, and you can make more later.",
            PersonaForm {
                submit_label: "Make this persona",
                cancel_label: "Not now",
                on_created: move |p| model.write().persona_created(p),
                on_cancel: move |_| model.write().skip_persona(),
            }
        }
    }
}

#[component]
pub fn PreparingStep(persona: uuid::Uuid) -> Element {
    let (backend, mut model) = use_ui();
    let step = use_signal(|| None::<Step>);
    let mut running = use_signal(|| false);
    let handle = model
        .read()
        .personas()
        .iter()
        .find(|p| p.persona.id == persona)
        .map(|p| p.persona.handle.clone())
        .unwrap_or_default();
    let problem = model.read().problem.clone();

    let run = use_callback(move |()| {
        if running() {
            return;
        }
        running.set(true);
        model.write().problem = None;
        let b = backend.clone();
        spawn(async move {
            let progress = move |s: Step| {
                let mut step = step;
                step.set(Some(s));
            };
            let result = b.prepare(persona, &progress).await;
            running.set(false);
            model.write().prepared(persona, result);
        });
    });
    use_hook(move || run.call(()));

    let now = step().map(|s| Step::ALL.iter().position(|a| *a == s).unwrap_or(0));
    let steps: Vec<(usize, &str, &str, &str, &str)> = Step::ALL
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let (class, mark, said) = match now {
                Some(n) if i < n => ("done", "✓", "done"),
                Some(n) if i == n && problem.is_some() => ("now", "!", "stopped"),
                Some(n) if i == n => ("now", "…", "in progress"),
                _ => ("", "·", "to do"),
            };
            (i, s.label(), class, mark, said)
        })
        .collect();
    let title = format!("Getting @{handle} ready");
    rsx! {
        Card {
            title,
            lede: "Quill is making the keys that keep your writing sessions private. This takes a moment, and only happens once on each computer.",
            ol { class: "steps", aria_live: "polite",
                for (i, label, class, mark, said) in steps {
                    li { key: "{i}", class,
                        aria_current: if class == "now" { "step" } else { "false" },
                        span { class: "mark", aria_hidden: "true", "{mark}" }
                        span { "{label}" }
                        span { class: "sr-only", " ({said})" }
                    }
                }
            }
            if let Some(p) = problem {
                ProblemLine { problem: p }
                div { class: "actions",
                    button { class: "primary", onclick: move |_| run.call(()), "Try again" }
                    button { class: "quiet", onclick: move |_| model.write().prepare_later(), "Not now" }
                }
            }
        }
    }
}

#[component]
pub fn ElsewhereStep() -> Element {
    let (_, mut model) = use_ui();
    let handles: Vec<String> = model
        .read()
        .personas()
        .iter()
        .filter(|p| p.keys == Keys::Elsewhere)
        .map(|p| format!("@{}", p.persona.handle))
        .collect();
    let lede = format!(
        "You're signed in, but the keys for {} are on another of your devices. Writing as them here needs those keys.",
        handles.join(", ")
    );
    rsx! {
        Card {
            title: "This computer is new to your login",
            lede,
            p { class: "lede",
                "Soon you'll be able to link this computer from that device, or restore a backup. Until then you can look around, and make a new persona to use here."
            }
            div { class: "actions",
                button { class: "primary", onclick: move |_| model.write().elsewhere_seen(), "Continue" }
            }
        }
    }
}
