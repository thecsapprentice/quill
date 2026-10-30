//! Keys and devices (3d): linking this computer to the login's other
//! devices, approving a new one from here, the device list with renaming
//! and removing, and each persona's keys in advanced mode.
//!
//! Linking checks two things in words (protocol draft, section 6): the
//! six words typed on the device in use prove the new computer's key, and
//! the six words both then show prove they joined the same group. Either
//! failing stops and explains; nothing is taken or linked on a mismatch.

use dioxus::prelude::*;
use qrcode::{Color, QrCode};
use uuid::Uuid;

use crate::{
    app::{ProblemLine, use_ui},
    backend::{
        Approval, DEVICE_NAME_MAX, Device, Keys, LinkStart, PersonaKeys, Problem, RemoveReason,
    },
    field::{Field, Kind},
    first_run::Card,
};

/// How often the new computer looks for its link's answer, between looks
/// (each look itself waits a few seconds on the native backend).
const LOOK_EVERY_MS: u64 = 1_000;

/// A date from seconds since 1970, as the board writes dates.
fn date(secs: u64) -> String {
    let at = time::OffsetDateTime::from_unix_timestamp(i64::try_from(secs).unwrap_or(0))
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    quill_board_ui::short_date(at)
}

/// A device's name, or how to refer to one without.
fn shown_name(d: &Device) -> String {
    d.name.clone().unwrap_or_else(|| {
        if d.this_device {
            "This computer".into()
        } else {
            "An unnamed device".into()
        }
    })
}

/// Six words, numbered, as both screens show them.
#[component]
fn Words(words: Vec<String>, label: String) -> Element {
    rsx! {
        ol { class: "link-words", aria_label: "{label}",
            for (i, w) in words.iter().enumerate() {
                li { key: "{i}", "{w}" }
            }
        }
    }
}

/// The link code as a QR code, drawn as SVG elements (no image, no HTML).
#[component]
fn Qr(code: String) -> Element {
    let Ok(qr) = QrCode::new(code.as_bytes()) else {
        return rsx! {};
    };
    let width = qr.width();
    let dark: Vec<(usize, usize)> = qr
        .to_colors()
        .iter()
        .enumerate()
        .filter(|(_, c)| **c == Color::Dark)
        .map(|(i, _)| (i % width, i / width))
        .collect();
    // A quiet zone of 4 modules all round, as scanners expect.
    let size = width + 8;
    rsx! {
        svg {
            class: "qr",
            view_box: "0 0 {size} {size}",
            role: "img",
            "aria-label": "The link code as a QR code",
            "shape-rendering": "crispEdges",
            rect { width: "{size}", height: "{size}", fill: "#fff" }
            for (x, y) in dark {
                rect { x: "{x + 4}", y: "{y + 4}", width: "1", height: "1", fill: "#000" }
            }
        }
    }
}

/// Where linking this computer is up to.
#[derive(Debug, Clone, PartialEq)]
enum Stage {
    Naming,
    Showing(LinkStart),
    Expired,
    Confirming(Vec<String>),
    Mismatch,
}

/// Linking this computer to the login's other devices, as a screen of its
/// own: name it, show the code, compare the words, take the keys.
#[component]
pub fn LinkingStep() -> Element {
    let (backend, mut model) = use_ui();
    let mut stage = use_signal(|| Stage::Naming);
    let initial = use_hook(|| {
        backend
            .device_name()
            .unwrap_or_else(|| backend.suggested_device_name())
    });
    let mut name = use_signal(|| initial.clone());
    let mut busy = use_signal(|| false);
    let mut problem = use_signal(|| None::<Problem>);
    let mut left = use_signal(|| 0u64);
    let mut copied = use_signal(|| false);
    // Bumped by each new code, so an older code's waiting stops.
    let mut round = use_signal(|| 0u32);

    let b = backend.clone();
    let wait = use_callback(move |mine: u32| {
        let b = b.clone();
        spawn(async move {
            loop {
                if round() != mine {
                    return;
                }
                match b.link_answer().await {
                    _ if round() != mine => return,
                    Ok(Some(words)) => {
                        problem.set(None);
                        stage.set(Stage::Confirming(words));
                        return;
                    }
                    Ok(None) => problem.set(None),
                    Err(Problem::Gone) => {
                        stage.set(Stage::Expired);
                        return;
                    }
                    // Offline or a hiccup: keep looking while the code lasts.
                    Err(p) => problem.set(Some(p)),
                }
                b.sleep(LOOK_EVERY_MS).await;
            }
        });
    });
    let b = backend.clone();
    let tick = use_callback(move |mine: u32| {
        let b = b.clone();
        spawn(async move {
            while round() == mine && left() > 0 {
                b.sleep(1_000).await;
                if round() == mine {
                    left.set(left().saturating_sub(1));
                }
            }
        });
    });
    let b = backend.clone();
    let show_code = use_callback(move |()| {
        if busy() {
            return;
        }
        busy.set(true);
        problem.set(None);
        copied.set(false);
        let b = b.clone();
        spawn(async move {
            let result = b.start_link(&name()).await;
            busy.set(false);
            match result {
                Ok(link) => {
                    let mine = round() + 1;
                    round.set(mine);
                    left.set(link.expires_in_secs);
                    stage.set(Stage::Showing(link));
                    wait.call(mine);
                    tick.call(mine);
                }
                Err(p) => problem.set(Some(p)),
            }
        });
    });
    let b = backend.clone();
    let leave = move |_| {
        round.set(round() + 1);
        // Nothing to undo if no code was shown; a code just goes unused.
        let _ = b.abandon_link();
        model.write().link_left();
    };
    let b = backend.clone();
    let differ = move |_| {
        // Take nothing from a group the words don't vouch for.
        if let Err(p) = b.abandon_link() {
            problem.set(Some(p));
        }
        stage.set(Stage::Mismatch);
    };
    let b = backend.clone();
    let matched = move |_| {
        busy.set(true);
        problem.set(None);
        let b = b.clone();
        spawn(async move {
            let result = b.finish_link().await;
            busy.set(false);
            match result {
                Ok(standing) => model.write().linked(standing),
                Err(p) => problem.set(Some(p)),
            }
        });
    };

    let too_long = name().trim().chars().count() > DEVICE_NAME_MAX;
    let hint = if too_long {
        format!("At most {DEVICE_NAME_MAX} characters.")
    } else {
        "Only your own devices see it, to tell your computers apart.".to_owned()
    };
    let server = model.read().server_name();
    let minutes = left().div_ceil(60);
    let time_left = match minutes {
        0 => "This code has expired.".to_owned(),
        1 => "This code works for 1 more minute.".to_owned(),
        n => format!("This code works for {n} more minutes."),
    };
    let body = match stage() {
        Stage::Naming => rsx! {
            Card {
                title: "Link this computer",
                lede: "Link this computer to a device where you already use Quill on {server}. It then gets your personas' keys, and can write as them here. Have that device to hand.",
                Field {
                    label: "What should your other devices call this computer?",
                    id: "device-name",
                    kind: Kind::Text,
                    initial: initial.clone(),
                    hint,
                    invalid: too_long,
                    autofocus: true,
                    oninput: move |t| name.set(t),
                    onenter: move |()| if !too_long { show_code.call(()) },
                }
                div { class: "actions",
                    button {
                        class: "primary",
                        disabled: busy() || too_long,
                        onclick: move |_| show_code.call(()),
                        if busy() { "One moment…" } else { "Show the link code" }
                    }
                    button { class: "quiet", onclick: leave.clone(), "Not now" }
                }
            }
        },
        Stage::Showing(link) => {
            let code = link.code.clone();
            let copy = move |_| {
                let code = code.clone();
                spawn(async move {
                    let js = format!("await navigator.clipboard.writeText({code:?}); return true;");
                    copied.set(document::eval(&js).await.is_ok());
                });
            };
            rsx! {
                Card {
                    title: "Type these words on your other device",
                    lede: "On a device where you already use Quill, open Settings, then Devices, and choose Link a new computer. Type these six words there.",
                    Words { words: link.words.clone(), label: "The six words" }
                    div { class: "link-code",
                        Qr { code: link.code.clone() }
                        div {
                            p { class: "hint", "A device with a camera can scan this code instead." }
                            button { class: "secondary", onclick: copy, "Copy the code" }
                            if copied() {
                                p { class: "hint", role: "status", "Code copied. Paste it on your other device." }
                            }
                        }
                    }
                    p { class: "hint", role: "status", aria_live: "polite",
                        "Waiting for your other device. {time_left}"
                    }
                    div { class: "actions",
                        button { class: "quiet", onclick: leave.clone(), "Cancel" }
                    }
                }
            }
        }
        Stage::Expired => rsx! {
            Card {
                title: "That code expired",
                lede: "Your other device didn't use it in time: link codes last 10 minutes. Show a new one when that device is to hand.",
                div { class: "actions",
                    button { class: "primary", disabled: busy(), onclick: move |_| show_code.call(()), "Show a new code" }
                    button { class: "quiet", onclick: leave.clone(), "Cancel" }
                }
            }
        },
        Stage::Confirming(words) => rsx! {
            Card {
                title: "Check the words match",
                lede: "Your other device now shows six words too. Link only if they're the same as these, in the same order.",
                Words { words, label: "The words to compare" }
                div { class: "actions",
                    button { class: "primary", disabled: busy(), onclick: matched,
                        if busy() { "Linking…" } else { "They match" }
                    }
                    button { class: "secondary", disabled: busy(), onclick: differ, "They don't match" }
                }
            }
        },
        Stage::Mismatch => rsx! {
            Card {
                title: "Nothing was linked",
                lede: "This computer took no keys. The words differ when something came between your two devices, such as a server that isn't playing fair.",
                p { class: "lede",
                    "On your other device, open Settings, then Devices, and remove any device you don't recognise. Then try again, or ask your server's administrator."
                }
                div { class: "actions",
                    button { class: "primary", disabled: busy(), onclick: move |_| show_code.call(()), "Start again" }
                    button { class: "quiet", onclick: leave.clone(), "Close" }
                }
            }
        },
    };
    rsx! {
        {body}
        if let Some(p) = problem() {
            div { class: "card-problem",
                ProblemLine { problem: p }
            }
        }
    }
}

/// The Devices section: this login's devices, linking a new computer from
/// here, and (advanced) each persona's keys.
#[component]
pub fn DevicesPage() -> Element {
    let (backend, model) = use_ui();
    let mut list = use_signal(|| None::<Vec<Device>>);
    let mut problem = use_signal(|| None::<Problem>);
    let mut said = use_signal(|| None::<String>);
    let b = backend.clone();
    let load = use_callback(move |()| {
        let b = b.clone();
        spawn(async move {
            match b.devices().await {
                Ok(l) => {
                    problem.set(None);
                    list.set(Some(l));
                }
                Err(p) => problem.set(Some(p)),
            }
        });
    });
    use_hook(move || load.call(()));
    let m = model.read();
    let advanced = m.settings.advanced;
    let elsewhere: Vec<String> = m
        .personas()
        .iter()
        .filter(|p| p.keys == Keys::Elsewhere)
        .map(|p| format!("@{}", p.persona.handle))
        .collect();
    drop(m);
    // A login alone on this computer has no device group yet.
    let shown = list().map(|l| {
        if l.is_empty() {
            vec![Device {
                id: Uuid::nil(),
                this_device: true,
                name: backend.device_name(),
                since: None,
                personas: Vec::new(),
            }]
        } else {
            l
        }
    });
    let alone = list().is_some_and(|l| l.len() <= 1);
    rsx! {
        h1 { "Devices" }
        p { class: "lede",
            "The computers that can write as your personas. Each keeps its own copy of their keys, and only they see these names."
        }
        if let Some(text) = said() {
            p { class: "hint", role: "status", "{text}" }
        }
        if let Some(p) = problem() {
            ProblemLine { problem: p }
            div { class: "actions",
                button { class: "secondary", onclick: move |_| load.call(()), "Try again" }
            }
        }
        match shown {
            None if problem().is_none() => rsx! { p { class: "coming", "Loading your devices…" } },
            None => rsx! {},
            Some(devices) => rsx! {
                ul { class: "list devices",
                    for d in devices {
                        DeviceRow {
                            key: "{d.id}",
                            device: d,
                            advanced,
                            on_removed: move |name: String| {
                                said.set(Some(format!("{name} was removed. It can't write as your personas any more.")));
                                load.call(());
                            },
                            on_renamed: move |()| load.call(()),
                        }
                    }
                }
                if alone {
                    p { class: "hint", "Only this computer so far." }
                }
            },
        }
        ApprovePanel { on_linked: move |()| load.call(()) }
        if !elsewhere.is_empty() {
            section { class: "panel",
                h2 { "Link this computer" }
                p { "The keys for {elsewhere.join(\", \")} are on another of your devices. Link this computer to write as them here." }
                div { class: "actions",
                    button {
                        class: "secondary",
                        onclick: move |_| {
                            let mut m = model;
                            m.write().begin_link();
                        },
                        "Link this computer"
                    }
                }
            }
        }
        if advanced {
            KeysTable {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum RowMode {
    Showing,
    Renaming,
    Removing,
}

#[component]
fn DeviceRow(
    device: Device,
    advanced: bool,
    on_removed: EventHandler<String>,
    on_renamed: EventHandler<()>,
) -> Element {
    let (backend, _) = use_ui();
    let mut mode = use_signal(|| RowMode::Showing);
    let mut name = use_signal(|| device.name.clone().unwrap_or_default());
    let mut reason = use_signal(RemoveReason::default);
    let mut busy = use_signal(|| false);
    let mut problem = use_signal(|| None::<Problem>);
    let shown = shown_name(&device);
    let id = device.id;
    let writes = if device.personas.is_empty() {
        "Can't write as any of your personas.".to_owned()
    } else {
        let handles: Vec<String> = device.personas.iter().map(|h| format!("@{h}")).collect();
        format!("Writes as {}.", handles.join(", "))
    };
    let b = backend.clone();
    let save = use_callback(move |()| {
        busy.set(true);
        let b = b.clone();
        spawn(async move {
            let result = b.rename_device(&name()).await;
            busy.set(false);
            match result {
                Ok(()) => {
                    problem.set(None);
                    mode.set(RowMode::Showing);
                    on_renamed.call(());
                }
                Err(p) => problem.set(Some(p)),
            }
        });
    });
    let b = backend.clone();
    let removed_name = shown.clone();
    let remove = move |_| {
        busy.set(true);
        let b = b.clone();
        let removed_name = removed_name.clone();
        spawn(async move {
            let result = b.remove_device(id, reason()).await;
            busy.set(false);
            match result {
                Ok(()) => on_removed.call(removed_name),
                Err(p) => problem.set(Some(p)),
            }
        });
    };
    let too_long = name().trim().chars().count() > DEVICE_NAME_MAX;
    rsx! {
        li { class: "item",
            header {
                span { class: "name", "{shown}" }
                if device.this_device {
                    span { class: "tag", "This computer" }
                }
            }
            p { class: "hint",
                if let Some(since) = device.since { "Linked {date(since)}. " }
                "{writes}"
            }
            if advanced && !device.id.is_nil() {
                p { class: "detail", "id {device.id}" }
            }
            match mode() {
                RowMode::Showing => rsx! {
                    div { class: "actions",
                        if device.this_device {
                            button { class: "secondary", onclick: move |_| mode.set(RowMode::Renaming), "Rename" }
                        } else {
                            button { class: "quiet", onclick: move |_| mode.set(RowMode::Removing), "Remove…" }
                        }
                    }
                },
                RowMode::Renaming => rsx! {
                    Field {
                        label: "Name",
                        id: "device-rename",
                        kind: Kind::Text,
                        initial: name.peek().clone(),
                        hint: if too_long { format!("At most {DEVICE_NAME_MAX} characters.") } else { "Only your own devices see it.".to_owned() },
                        invalid: too_long,
                        autofocus: true,
                        oninput: move |t| name.set(t),
                        onenter: move |()| if !too_long { save.call(()) },
                    }
                    div { class: "actions",
                        button { class: "primary", disabled: busy() || too_long, onclick: move |_| save.call(()), "Save name" }
                        button { class: "quiet", onclick: move |_| mode.set(RowMode::Showing), "Cancel" }
                    }
                },
                RowMode::Removing => rsx! {
                    div { class: "notice", role: "alert",
                        "Remove {shown}? It leaves your devices, and its keys for your personas are revoked, so it can't write as them again. This can't be undone; to use it again, link it again."
                    }
                    fieldset {
                        legend { "Why? (optional)" }
                        for r in RemoveReason::ALL {
                            div { key: "{r:?}", class: "choice",
                                input {
                                    r#type: "radio",
                                    id: "remove-reason-{r:?}",
                                    name: "remove-reason",
                                    checked: reason() == r,
                                    onchange: move |_| reason.set(r),
                                }
                                label { r#for: "remove-reason-{r:?}", "{r.label()}" }
                            }
                        }
                    }
                    if reason() == RemoveReason::Compromised {
                        p { class: "hint", "Change your password too, so it can't sign in again." }
                    }
                    div { class: "actions",
                        button { class: "danger", disabled: busy(), onclick: remove, "Remove {shown}" }
                        button { class: "quiet", onclick: move |_| mode.set(RowMode::Showing), "Keep it" }
                    }
                },
            }
            if let Some(p) = problem() {
                ProblemLine { problem: p }
            }
        }
    }
}

/// What approving a new computer from here is up to.
#[derive(Debug, Clone, PartialEq)]
enum Approving {
    Closed,
    Checking,
    NoneYet,
    Several(usize),
    Ready,
    Linked(Vec<String>),
}

/// "Link a new computer": finds the waiting link (only when asked), takes
/// its six words, and shows the words to compare.
#[component]
fn ApprovePanel(on_linked: EventHandler<()>) -> Element {
    let (backend, model) = use_ui();
    let mut state = use_signal(|| Approving::Closed);
    let mut typed = use_signal(String::new);
    let mut wrong = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let mut problem = use_signal(|| None::<Problem>);
    let b = backend.clone();
    let check = use_callback(move |()| {
        state.set(Approving::Checking);
        wrong.set(None);
        let b = b.clone();
        spawn(async move {
            match b.waiting_links().await {
                Ok(0) => state.set(Approving::NoneYet),
                Ok(1) => state.set(Approving::Ready),
                Ok(n) => state.set(Approving::Several(n)),
                Err(p) => {
                    problem.set(Some(p));
                    state.set(Approving::Closed);
                }
            }
        });
    });
    let b = backend.clone();
    let approve = use_callback(move |()| {
        busy.set(true);
        problem.set(None);
        wrong.set(None);
        let b = b.clone();
        spawn(async move {
            let result = b.approve_link(&typed()).await;
            busy.set(false);
            match result {
                Ok(Approval::Approved { confirm }) => {
                    state.set(Approving::Linked(confirm));
                    on_linked.call(());
                }
                Ok(Approval::WordsDiffer) => wrong.set(Some(
                    "Those words don't match the computer that's waiting. Check each one against its screen. If they're right, don't link it: something may have come between your devices.".into(),
                )),
                Ok(Approval::NoneWaiting) => state.set(Approving::NoneYet),
                Ok(Approval::Several(n)) => state.set(Approving::Several(n)),
                Err(p) => problem.set(Some(p)),
            }
        });
    });
    let server = model.read().server_name();
    let count = typed().split_whitespace().count();
    let ready = count == 6 || typed().trim().starts_with("quill-link:");
    rsx! {
        section { class: "panel", aria_label: "Link a new computer",
            h2 { "Link a new computer" }
            match state() {
                Approving::Closed => rsx! {
                    p { "Add a computer you've signed in on, so it can write as your personas too." }
                    div { class: "actions",
                        button { class: "secondary", onclick: move |_| check.call(()), "Link a new computer" }
                    }
                },
                Approving::Checking => rsx! { p { class: "coming", "Looking for the new computer…" } },
                Approving::NoneYet => rsx! {
                    p {
                        "No new computer is waiting yet. On it, sign in to {server} and choose Link this computer. When it shows six words, check again here."
                    }
                    div { class: "actions",
                        button { class: "secondary", onclick: move |_| check.call(()), "Check again" }
                        button { class: "quiet", onclick: move |_| state.set(Approving::Closed), "Cancel" }
                    }
                },
                Approving::Several(n) => rsx! {
                    div { class: "notice", role: "alert",
                        "{n} new computers are waiting to link to your login. If only one is yours, someone else may be trying to link: change your password, wait 10 minutes for the codes to expire, then start again on your new computer."
                    }
                    div { class: "actions",
                        button { class: "secondary", onclick: move |_| check.call(()), "Check again" }
                        button { class: "quiet", onclick: move |_| state.set(Approving::Closed), "Cancel" }
                    }
                },
                Approving::Ready => rsx! {
                    Field {
                        label: "The six words on the new computer",
                        id: "link-words",
                        kind: Kind::Name,
                        hint: wrong().unwrap_or_else(|| "Or paste its whole link code.".to_owned()),
                        invalid: wrong().is_some(),
                        autofocus: true,
                        oninput: move |t| typed.set(t),
                        onenter: move |()| if ready { approve.call(()) },
                    }
                    div { class: "actions",
                        button { class: "primary", disabled: busy() || !ready, onclick: move |_| approve.call(()),
                            if busy() { "Linking…" } else { "Link it" }
                        }
                        button { class: "quiet", onclick: move |_| state.set(Approving::Closed), "Cancel" }
                    }
                },
                Approving::Linked(words) => rsx! {
                    p { "Linked. The new computer should now show these words:" }
                    Words { words, label: "The words to compare" }
                    p { class: "hint",
                        "If it shows anything else, choose They don't match there, then remove it from the list above."
                    }
                    div { class: "actions",
                        button { class: "secondary", onclick: move |_| { typed.set(String::new()); state.set(Approving::Closed); }, "Done" }
                    }
                },
            }
            if let Some(p) = problem() {
                ProblemLine { problem: p }
            }
        }
    }
}

/// Each persona's keys, for advanced mode.
#[component]
fn KeysTable() -> Element {
    let (backend, _) = use_ui();
    let mut keys = use_signal(|| None::<Vec<PersonaKeys>>);
    let mut problem = use_signal(|| None::<Problem>);
    use_hook(move || {
        spawn(async move {
            match backend.key_details().await {
                Ok(k) => keys.set(Some(k)),
                Err(p) => problem.set(Some(p)),
            }
        });
    });
    rsx! {
        section { class: "panel",
            h2 { "Persona keys" }
            if let Some(p) = problem() {
                ProblemLine { problem: p }
            }
            for k in keys().unwrap_or_default() {
                h3 { key: "{k.handle}", "@{k.handle}" }
                table { class: "details",
                    tbody {
                        for (label, value) in k.rows {
                            tr { key: "{label}", th { scope: "row", "{label}" } td { class: "detail", "{value}" } }
                        }
                    }
                }
            }
        }
    }
}
