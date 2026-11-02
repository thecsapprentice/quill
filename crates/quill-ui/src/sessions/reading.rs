//! Reading a session: its channels as tabs, each channel's notes rendered
//! as Markdown by the app, and live updates as notes arrive. Showing a
//! channel marks its notes read on this computer. Writing comes in 3f-2.

use std::collections::HashMap;
use std::rc::Rc;

use dioxus::prelude::*;
use quill_board_ui::short_date;

use super::{reload, standing_line};
use crate::{
    app::{ProblemLine, use_ui},
    backend::{Note, Problem, SessionView},
    markdown::{Markdown, plain_text},
};

/// When a note was written, in this computer's time zone: "Nov 2, 2026,
/// 14:05".
pub(crate) fn when(offset: time::UtcOffset, at: time::OffsetDateTime) -> String {
    let at = at.to_offset(offset);
    format!("{}, {:02}:{:02}", short_date(at), at.hour(), at.minute())
}

/// A channel's name as a tab: "Story", "Planning", or as named.
fn tab_label(name: &str) -> String {
    let mut c = name.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

#[component]
pub fn SessionPage(id: String) -> Element {
    let (backend, mut model) = use_ui();
    let mut view = use_signal(|| None::<Result<SessionView, Problem>>);
    let mut channel = use_signal(|| "story".to_owned());
    // Each note's element, to bring an answered note into view.
    let mut notes_el = use_signal(HashMap::<u64, Rc<MountedData>>::new);

    // Read again on each load of the list (a note arrived, live).
    let b = backend.clone();
    let sid = id.clone();
    use_effect(move || {
        let _ = model.read().sessions.epoch;
        let (b, sid) = (b.clone(), sid.clone());
        spawn(async move {
            let result = b.session(&sid).await;
            view.set(Some(result));
        });
    });
    // Showing a channel marks its notes read here, then the counts follow.
    let b = backend.clone();
    let sid = id.clone();
    use_effect(move || {
        let ch = channel();
        let Some(Ok(v)) = &*view.read() else {
            return;
        };
        let last = v
            .notes
            .iter()
            .filter(|n| n.channel == ch)
            .map(|n| n.seq)
            .max();
        let unseen = v.channels.iter().any(|c| c.name == ch && c.unseen > 0);
        if let (Some(last), true) = (last, unseen)
            && b.mark_shown(&sid, &ch, last).is_ok()
        {
            reload(b.clone(), model);
        }
    });

    let back = rsx! {
        div { class: "back",
            button {
                class: "quiet",
                onclick: move |_| model.write().sessions.open = None,
                "‹ All sessions"
            }
        }
    };
    let v = match view() {
        None => {
            return rsx! { {back} h1 { "Session" } p { class: "coming", "Reading the session…" } };
        }
        Some(Err(Problem::Gone)) => {
            return rsx! {
                {back}
                h1 { "Session" }
                p { class: "coming", "That session is gone from this login." }
            };
        }
        Some(Err(p)) => {
            // Not readable here: say why, from the list's standing if known.
            let line = model
                .read()
                .sessions
                .list
                .iter()
                .find(|s| s.id == id)
                .and_then(|s| standing_line(&s.standing));
            return rsx! {
                {back}
                h1 { "Session" }
                if let Some(line) = line {
                    p { class: "notice", "{line}" }
                }
                ProblemLine { problem: p }
            };
        }
        Some(Ok(v)) => v,
    };
    let s = &v.summary;
    let heading = match &s.partner {
        Some(p) => format!("Writing with {}", p.name()),
        None => "A session whose other writer left".into(),
    };
    let current = channel();
    let summary = v
        .channels
        .iter()
        .find(|c| c.name == current)
        .and_then(|c| c.summary.clone());
    let shown: Vec<Note> = v
        .notes
        .iter()
        .filter(|n| n.channel == current)
        .cloned()
        .collect();
    let firsts: HashMap<u64, (String, String)> = v
        .notes
        .iter()
        .map(|n| (n.seq, (n.channel.clone(), first_line(&n.content))))
        .collect();
    let offset = backend.utc_offset();
    rsx! {
        {back}
        h1 { "{heading}" }
        if let Some((_, t)) = &s.prompt {
            p { class: "lede", "On “{t}”." }
        }
        if s.ended {
            div { class: "notice", role: "status",
                "This session has ended. What's written stays readable here."
            }
        }
        if v.held_back {
            div { class: "notice", role: "status",
                "Some notes seem to be held back by the server. They'll appear here if it sends them."
            }
        }
        if v.history_missing {
            p { class: "hint", "Older notes aren't on this computer yet. Another of your devices is sending them." }
        }
        div { class: "channels", role: "tablist", aria_label: "Channels",
            for c in v.channels.iter().cloned() {
                button {
                    key: "{c.name}",
                    role: "tab",
                    aria_selected: if c.name == current { "true" } else { "false" },
                    onclick: {
                        let name = c.name.clone();
                        move |_| channel.set(name.clone())
                    },
                    "{tab_label(&c.name)}"
                    if c.unseen > 0 && c.name != current {
                        span { class: "badge", " {c.unseen}" span { class: "sr-only", " unread" } }
                    }
                }
            }
        }
        section { class: "channel", role: "tabpanel", aria_label: "{tab_label(&current)}",
            if let Some(d) = summary {
                p { class: "hint", "{d}" }
            }
            if shown.is_empty() {
                p { class: "coming", "Nothing in {tab_label(&current)} yet." }
            } else {
                ol { class: "notes",
                    for n in shown {
                        NoteItem {
                            key: "{n.seq}",
                            answering: n.reply_to.and_then(|r| firsts.get(&r).cloned().map(|f| (r, f))),
                            when: when(offset, n.at),
                            on_mounted: move |(seq, el): (u64, Rc<MountedData>)| {
                                notes_el.write().insert(seq, el);
                            },
                            on_answer: move |(seq, ch): (u64, String)| {
                                // The answered note may be in another channel.
                                channel.set(ch);
                                if let Some(el) = notes_el.peek().get(&seq).cloned() {
                                    spawn(async move {
                                        let _ = el.scroll_to(ScrollBehavior::Instant).await;
                                    });
                                }
                            },
                            n,
                        }
                    }
                }
            }
        }
    }
}

/// A note's first line in plain words, shortened, for "In answer to".
fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let line = plain_text(line);
    let line = line.trim();
    let mut out: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        out.push('…');
    }
    out
}

#[component]
fn NoteItem(
    n: Note,
    when: String,
    /// The note it answers: its number, channel and first line.
    answering: Option<(u64, (String, String))>,
    on_mounted: EventHandler<(u64, Rc<MountedData>)>,
    on_answer: EventHandler<(u64, String)>,
) -> Element {
    let seq = n.seq;
    rsx! {
        li {
            class: if n.mine { "note mine note-{seq}" } else { "note note-{seq}" },
            id: "note-{seq}",
            onmounted: move |e| on_mounted.call((seq, e.data())),
            header {
                span { class: "name", if n.mine { "You" } else { "{n.author.name()}" } }
                span { class: "hint", " {when}" }
                if n.edited && !n.deleted {
                    span { class: "tag", "Edited" }
                }
            }
            if let Some((to, (ch, first))) = answering {
                button {
                    class: "link answering",
                    onclick: move |_| on_answer.call((to, ch.clone())),
                    "In answer to: {first}"
                }
            }
            if n.deleted {
                p { class: "hint", "This note was deleted." }
            } else {
                div { class: "pitch", Markdown { source: n.content.clone() } }
            }
            if n.attachments > 0 {
                p { class: "hint",
                    if n.attachments == 1 { "1 attachment." } else { "{n.attachments} attachments." }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn times_show_in_this_computers_zone() {
        let at = time::OffsetDateTime::from_unix_timestamp(1_793_657_100)
            .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
        let la = time::UtcOffset::from_hms(-8, 0, 0).unwrap_or(time::UtcOffset::UTC);
        assert_eq!(when(la, at), "Nov 2, 2026, 14:05");
        assert_eq!(when(time::UtcOffset::UTC, at), "Nov 2, 2026, 22:05");
    }

    #[test]
    fn answers_name_their_note_by_its_first_line() {
        assert_eq!(first_line("\n# Chapter one\nThe sea."), "Chapter one");
        assert_eq!(first_line("I'd write the **ghost** *here*."), "I'd write the ghost here.");
        assert_eq!(first_line("> - `quoted` list"), "quoted list");
        assert_eq!(first_line(&"x".repeat(70)).chars().count(), 61);
    }
}
