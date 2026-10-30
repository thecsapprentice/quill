//! The filter builder (mockup D2; N2 and N3 when narrow). It edits a copy
//! of the board's filter; "Show prompts" puts it to use, and leaving
//! another way keeps the old one.

use dioxus::prelude::*;
use quill_proto::board::{Maturity, PromptState, TagOrder, TagPage, TagPageQuery};

use super::{Choice, Filter, View, reader::Chip, reload, state_word, use_board};
use crate::{
    app::{ProblemLine, use_ui},
    backend::Problem,
};

/// Tags per page of a type.
const PER_PAGE: u32 = 30;
/// The pane for rating and status, rather than a tag type.
const BASICS: &str = "";

#[component]
pub fn FilterBuilder() -> Element {
    let (backend, _) = use_ui();
    let mut board = use_board();
    let draft = use_signal(|| board.peek().filter.clone());
    let mut pane = use_signal(|| BASICS.to_owned());
    let types = board.read().types.clone().unwrap_or_default();
    let d = draft.read().clone();
    let in_type = |ns: &str| {
        d.has
            .iter()
            .chain(d.hides.iter())
            .filter(|t| t.namespace == ns)
            .count()
    };
    let basics_set = usize::from(d.maturity.is_some()) + usize::from(d.state.is_some());

    let show = move |_| {
        {
            let mut b = board.write();
            b.filter = draft.peek().clone();
            b.view = View::List;
        }
        reload(backend.clone(), board);
    };
    rsx! {
        div { class: "filters",
            header { class: "filters-head",
                button { class: "quiet", onclick: move |_| board.write().view = View::List, "‹ Back to the board" }
                h1 { "Filter prompts" }
                p { class: "hint", "Pick a tag type. Each tag can be ignored, required or hidden." }
            }
            nav { class: "filter-types", aria_label: "Tag types",
                button {
                    class: "section",
                    aria_current: if pane() == BASICS { "page" } else { "false" },
                    onclick: move |_| pane.set(BASICS.to_owned()),
                    "Rating and status"
                    if basics_set > 0 { span { class: "count", " {basics_set}" } }
                }
                for t in types.iter() {
                    {
                        let name = t.name.clone();
                        let set = in_type(&t.name);
                        let current = pane() == t.name;
                        rsx! {
                            button {
                                key: "{t.name}",
                                class: "section",
                                aria_current: if current { "page" } else { "false" },
                                onclick: move |_| pane.set(name.clone()),
                                "{t.display}"
                                if set > 0 { span { class: "count", " {set}" } }
                            }
                        }
                    }
                }
            }
            section { class: "filter-pane",
                if pane() == BASICS {
                    Basics { draft }
                } else {
                    TypePane { key: "{pane}", namespace: pane(), draft }
                }
            }
            aside { class: "filter-summary", aria_label: "Your filter",
                h2 { "Your filter" }
                DraftSummary { draft }
                div { class: "actions",
                    button { class: "primary", onclick: show, "Show prompts" }
                    button {
                        class: "quiet",
                        onclick: move |_| {
                            let mut d = draft;
                            d.write().clear();
                        },
                        "Clear all"
                    }
                }
            }
        }
    }
}

#[component]
fn DraftSummary(draft: Signal<Filter>) -> Element {
    let d = draft.read().clone();
    let mut draft = draft;
    let remove = move |key: String| {
        let mut f = draft.write();
        f.has.retain(|t| t.key() != key);
        f.hides.retain(|t| t.key() != key);
    };
    if d.count() == 0 {
        return rsx! { p { class: "hint", "Nothing set: every prompt shows." } };
    }
    rsx! {
        if let Some(basics) = d.basics() {
            p { "{basics}" }
        }
        if !d.has.is_empty() {
            h3 { "Has all of" }
            div { class: "chips",
                for t in d.has.iter() {
                    Chip { key: "{t.key()}", tag: t.clone(), on_remove: remove }
                }
            }
        }
        if !d.hides.is_empty() {
            h3 { "Hides" }
            div { class: "chips",
                for t in d.hides.iter() {
                    Chip { key: "{t.key()}", tag: t.clone(), on_remove: remove }
                }
            }
        }
    }
}

#[component]
fn Basics(draft: Signal<Filter>) -> Element {
    let mut draft = draft;
    let d = draft.read().clone();
    let ratings: Vec<(Option<Maturity>, String)> =
        std::iter::once((None, "Up to my setting".to_owned()))
            .chain(Maturity::ALL.iter().map(|m| {
                (
                    Some(*m),
                    format!("{} or under", quill_board_ui::maturity_label(*m)),
                )
            }))
            .collect();
    let states: Vec<(Option<PromptState>, String)> =
        std::iter::once((None, "Any status".to_owned()))
            .chain(
                PromptState::ALL
                    .iter()
                    .map(|s| (Some(*s), format!("{} only", state_word(*s)))),
            )
            .collect();
    rsx! {
        h2 { "Rating and status" }
        fieldset {
            legend { "Rating" }
            for (m, label) in ratings {
                div { key: "{label}", class: "choice",
                    input {
                        r#type: "radio",
                        id: "rating-{m:?}",
                        name: "rating",
                        checked: d.maturity == m,
                        onchange: move |_| draft.write().maturity = m,
                    }
                    label { r#for: "rating-{m:?}", "{label}" }
                }
            }
        }
        fieldset {
            legend { "Status" }
            for (s, label) in states {
                div { key: "{label}", class: "choice",
                    input {
                        r#type: "radio",
                        id: "status-{s:?}",
                        name: "status",
                        checked: d.state == s,
                        onchange: move |_| draft.write().state = s,
                    }
                    label { r#for: "status-{s:?}", "{label}" }
                }
            }
        }
    }
}

/// One tag type's tags: search, order, letters, pages, and a setting each.
#[component]
fn TypePane(namespace: String, draft: Signal<Filter>) -> Element {
    let (backend, _) = use_ui();
    let board = use_board();
    let mut search = use_signal(String::new);
    let mut order = use_signal(|| TagOrder::Used);
    let mut letter = use_signal(|| None::<char>);
    let mut offset = use_signal(|| 0u32);
    let mut page = use_signal(|| None::<TagPage>);
    let mut problem = use_signal(|| None::<Problem>);
    let mut round = use_signal(|| 0u32);
    let display = board
        .read()
        .type_of(&namespace)
        .map(|t| t.display.clone())
        .unwrap_or_else(|| namespace.clone());

    let ns = namespace.clone();
    let load = use_callback(move |()| {
        let mine = round() + 1;
        round.set(mine);
        let query = TagPageQuery {
            search: Some(search()).filter(|s| !s.trim().is_empty()),
            letter: letter(),
            order: Some(order()),
            offset: Some(offset()),
            limit: Some(PER_PAGE),
        };
        let b = backend.clone();
        let ns = ns.clone();
        spawn(async move {
            let result = b.tag_page(&ns, query).await;
            if round() != mine {
                return;
            }
            match result {
                Ok(p) => {
                    problem.set(None);
                    page.set(Some(p));
                }
                Err(p) => problem.set(Some(p)),
            }
        });
    });
    use_hook(move || load.call(()));

    let p = page();
    let total = p.as_ref().map_or(0, |p| p.total);
    let shown_to = (offset() + PER_PAGE).min(u32::try_from(total).unwrap_or(u32::MAX));
    let letters: Vec<char> = ('A'..='Z').collect();
    let d = draft.read().clone();
    rsx! {
        div { class: "type-head",
            h2 { "{display}" span { class: "hint", " {total} tags" } }
            input {
                id: "tag-search",
                r#type: "search",
                aria_label: "Search {display}",
                placeholder: "Search {display}",
                autocomplete: "off",
                "writingsuggestions": "false",
                spellcheck: "false",
                oninput: move |e| {
                    search.set(e.value());
                    offset.set(0);
                    load.call(());
                },
            }
            select {
                id: "tag-order",
                aria_label: "Order",
                onchange: move |e| {
                    order.set(if e.value() == "name" { TagOrder::Name } else { TagOrder::Used });
                    offset.set(0);
                    load.call(());
                },
                option { value: "used", selected: order() == TagOrder::Used, "Most used" }
                option { value: "name", selected: order() == TagOrder::Name, "A to Z" }
            }
        }
        div { class: "letters", role: "group", aria_label: "First letter",
            button {
                aria_pressed: if letter().is_none() { "true" } else { "false" },
                onclick: move |_| { letter.set(None); offset.set(0); load.call(()); },
                "All"
            }
            for c in letters {
                button {
                    key: "{c}",
                    aria_pressed: if letter() == Some(c) { "true" } else { "false" },
                    onclick: move |_| { letter.set(Some(c)); offset.set(0); load.call(()); },
                    "{c}"
                }
            }
        }
        if let Some(e) = problem() {
            ProblemLine { problem: e }
        }
        match p {
            None => rsx! { p { class: "coming", "Loading {display}…" } },
            Some(p) if p.items.is_empty() => rsx! { p { class: "coming", "No {display} tags match." } },
            Some(p) => rsx! {
                table { class: "tag-table",
                    thead { tr { th { "Tag" } th { "Prompts" } th { "Filter" } } }
                    tbody {
                        for ct in p.items.iter() {
                            {
                                let tag = ct.tag.clone();
                                let choice = d.choice(&tag);
                                let slug = format!("{}-{}", tag.namespace, tag.slug);
                                let set = move |c: Choice| {
                                    let tag = tag.clone();
                                    move |_| {
                                        let mut f = draft;
                                        f.write().set(&tag, c);
                                    }
                                };
                                rsx! {
                                    tr { key: "{slug}",
                                        th { scope: "row", "{ct.tag.display}" }
                                        td { "{ct.prompts}" }
                                        td {
                                            div { class: "segmented", role: "group", aria_label: "{ct.tag.display}",
                                                button {
                                                    id: "any-{slug}",
                                                    aria_pressed: if choice == Choice::Any { "true" } else { "false" },
                                                    onclick: set(Choice::Any),
                                                    "Any"
                                                }
                                                button {
                                                    id: "has-{slug}",
                                                    aria_pressed: if choice == Choice::Has { "true" } else { "false" },
                                                    onclick: set(Choice::Has),
                                                    "Has"
                                                }
                                                button {
                                                    id: "hide-{slug}",
                                                    aria_pressed: if choice == Choice::Hide { "true" } else { "false" },
                                                    onclick: set(Choice::Hide),
                                                    "Hide"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "pager",
                    span { class: "hint", "Showing {offset() + 1}–{shown_to} of {total}" }
                    if offset() > 0 {
                        button {
                            class: "secondary",
                            onclick: move |_| { offset.set(offset().saturating_sub(PER_PAGE)); load.call(()); },
                            "Previous"
                        }
                    }
                    if u64::from(shown_to) < total {
                        button {
                            class: "secondary",
                            onclick: move |_| { offset.set(offset() + PER_PAGE); load.call(()); },
                            "Next {PER_PAGE}"
                        }
                    }
                }
            },
        }
    }
}
