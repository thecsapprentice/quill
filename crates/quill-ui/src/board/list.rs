//! The board page (mockup D1; N1 and N4 when narrow): search, the filter
//! summary and the prompt list beside a reading pane.

use dioxus::html::geometry::PixelsVector2D;
use dioxus::prelude::*;
use quill_proto::board::Prompt;

use super::{
    MineState, View, check_new, filters::FilterBuilder, more, post::PostForm, reader::Reader,
    reload, use_board, use_board_epoch, use_tag_types,
};
use crate::{
    app::{ProblemLine, use_ui},
    board::reader::Chip,
};

/// How long typing in the search must pause before it searches.
const SEARCH_AFTER_MS: u64 = 300;
/// How often the Board looks for new prompts while it's in view.
const CHECK_EVERY_MS: u64 = 2 * 60 * 1000;

/// The Board section.
#[component]
pub fn BoardPage() -> Element {
    let (backend, _) = use_ui();
    let mut board = use_board();
    let epoch = use_board_epoch();
    use_tag_types();
    // The first visit loads the first page, and so does a visit after the
    // writer changed a prompt from the other list. My prompts loads afresh
    // on every visit; the Board keeps its place and looks for new prompts.
    use_hook(move || {
        let b = board.peek();
        let again = b.is_mine() || !b.loaded || b.seen != epoch.now();
        let busy = b.loading;
        drop(b);
        if again && !busy {
            board.write().seen = epoch.now();
            reload(backend, board);
        } else if !busy {
            check_new(backend, board);
        }
    });
    let view = board.read().view;
    match view {
        View::List => rsx! { BoardList {} },
        View::Filters => rsx! { FilterBuilder {} },
        View::Post(editing) => rsx! { PostForm { key: "{editing:?}", editing } },
    }
}

/// The My prompts section (mockup D6): the board's list and reading pane
/// over the login's own prompts, with a persona filter instead of search.
#[component]
pub fn MyPromptsPage() -> Element {
    let MineState(mine) = use_context::<MineState>();
    use_context_provider(|| mine);
    rsx! { BoardPage {} }
}

#[component]
fn BoardList() -> Element {
    let (backend, model) = use_ui();
    let mut board = use_board();
    let mut round = use_signal(|| 0u32);
    let b = board.read();
    let open = b.open;
    let mine = b.is_mine();
    let persona = b.persona;
    let items = b.shown();
    let loading = b.loading;
    let loaded = b.loaded;
    let next = b.next.is_some();
    let problem = b.problem.clone();
    let problem_none = problem.is_none();
    let filter = b.filter.clone();
    let said = b.said.clone();
    drop(b);
    let can_post = model.read().signed_in();
    let personas: Vec<(uuid::Uuid, String)> = model
        .read()
        .personas()
        .iter()
        .map(|p| (p.persona.id, p.persona.handle.clone()))
        .collect();

    let back = backend.clone();
    let on_search = use_callback(move |text: String| {
        board.write().filter.search = text;
        let mine = round() + 1;
        round.set(mine);
        let b = back.clone();
        spawn(async move {
            b.sleep(SEARCH_AFTER_MS).await;
            if round() == mine {
                reload(b, board);
            }
        });
    });
    // While the Board is in view, look for new prompts now and then.
    let bc = backend.clone();
    use_hook(move || {
        if board.peek().is_mine() {
            return;
        }
        spawn(async move {
            loop {
                bc.sleep(CHECK_EVERY_MS).await;
                check_new(bc.clone(), board);
            }
        });
    });
    let bm = backend.clone();
    let br = backend.clone();
    let fresh = board.read().fresh;
    // A new list starts at its top: the list's own scroll when wide, and the
    // page's (to the Board's header) when narrow.
    let mut list_el = use_signal(|| None::<std::rc::Rc<MountedData>>);
    let mut head_el = use_signal(|| None::<std::rc::Rc<MountedData>>);
    let mut seen_top = use_signal(|| board.peek().to_top);
    use_effect(move || {
        let top = board.read().to_top;
        if top == *seen_top.peek() {
            return;
        }
        seen_top.set(top);
        let (list, head) = (list_el.peek().clone(), head_el.peek().clone());
        spawn(async move {
            if let Some(l) = list {
                let _ = l
                    .scroll(PixelsVector2D::new(0.0, 0.0), ScrollBehavior::Instant)
                    .await;
            }
            if let Some(h) = head {
                let _ = h.scroll_to(ScrollBehavior::Instant).await;
            }
        });
    });
    let count = filter.count();
    let filters_label = if count > 0 {
        format!("Filters ({count})")
    } else {
        "Filters".to_owned()
    };
    let searching = !filter.search.trim().is_empty();
    let class = if open.is_some() {
        "board reading"
    } else {
        "board"
    };
    rsx! {
        div { class,
            section { class: "board-list", aria_label: "Prompts",
                header {
                    class: "board-head",
                    onmounted: move |e: MountedEvent| head_el.set(Some(e.data())),
                    h1 { if mine { "My prompts" } else { "Board" } }
                    button {
                        class: "quiet",
                        disabled: loading,
                        onclick: {
                            let b = br.clone();
                            move |_| reload(b.clone(), board)
                        },
                        "Refresh"
                    }
                    if can_post {
                        button {
                            class: "primary",
                            onclick: move |_| {
                                let mut b = board.write();
                                b.said = None;
                                b.view = View::Post(None);
                            },
                            "Post a prompt"
                        }
                    }
                }
                if mine {
                    div { class: "board-tools",
                        label { class: "sr-only", r#for: "mine-persona", "Persona" }
                        select {
                            id: "mine-persona",
                            onchange: move |e| board.write().persona = e.value().parse().ok(),
                            option { value: "", selected: persona.is_none(), "All my personas ({personas.len()})" }
                            for (id, handle) in personas.iter() {
                                option { key: "{id}", value: "{id}", selected: persona == Some(*id), "@{handle}" }
                            }
                        }
                    }
                } else {
                div { class: "board-tools",
                    input {
                        id: "board-search",
                        r#type: "search",
                        aria_label: "Search prompts",
                        placeholder: "Search prompts",
                        initial_value: "{filter.search}",
                        autocomplete: "off",
                        "writingsuggestions": "false",
                        spellcheck: "false",
                        oninput: move |e| on_search.call(e.value()),
                    }
                    button {
                        class: "secondary",
                        onclick: move |_| board.write().view = View::Filters,
                        "{filters_label}"
                    }
                }
                Summary {}
                }
                if let Some(text) = said {
                    p { class: "hint", role: "status", "{text}" }
                }
                if fresh > 0 && !loading {
                    div { class: "fresh", role: "status",
                        span { if fresh == 1 { "1 new prompt." } else { "{fresh} new prompts." } }
                        button {
                            class: "secondary",
                            onclick: {
                                let b = br.clone();
                                move |_| reload(b.clone(), board)
                            },
                            "Show them"
                        }
                    }
                }
                if let Some(p) = problem {
                    ProblemLine { problem: p }
                    div { class: "actions",
                        button {
                            class: "secondary",
                            onclick: {
                                let b = backend.clone();
                                move |_| reload(b.clone(), board)
                            },
                            "Try again"
                        }
                    }
                }
                // The end of the list lives inside its scrolling box, so it's
                // seen (and loads more) only once the list is scrolled there.
                // Keyed by the count, so a page too short to fill the view
                // loads the next.
                ul {
                    class: "prompts",
                    onmounted: move |e: MountedEvent| list_el.set(Some(e.data())),
                    for p in items.iter() {
                        Row { key: "{p.id}", prompt: p.clone(), open: Some(p.id) == open }
                    }
                    if next && !loading && problem_none {
                        for k in [items.len()] {
                            li {
                                key: "{k}",
                                class: "load-more",
                                aria_hidden: "true",
                                onvisible: {
                                    let bm = bm.clone();
                                    move |e: VisibleEvent| {
                                        if e.is_intersecting().unwrap_or(false) {
                                            more(bm.clone(), board);
                                        }
                                    }
                                },
                            }
                        }
                    }
                    if !items.is_empty() && (loading || (loaded && !next)) {
                        li { class: "list-end", role: "status",
                            if loading { "Loading more prompts…" } else { "That's every prompt." }
                        }
                    }
                }
                if items.is_empty() {
                    div { class: "list-end", role: "status",
                        if loading {
                            "Loading prompts…"
                        } else if mine && loaded && !next && persona.is_some() {
                            "No prompts posted as this persona."
                        } else if mine && loaded && !next {
                            "You haven't posted a prompt yet."
                        } else if loaded && (count > 0 || searching) && !next {
                            "No prompts match. Try fewer filters, or another search."
                        } else if loaded && !next {
                            "No prompts on the board yet."
                        }
                    }
                }
            }
            section { class: "board-reader", aria_label: "Prompt",
                if open.is_some() {
                    Reader { key: "{open:?}" }
                } else {
                    p { class: "coming", "Pick a prompt to read it here." }
                }
            }
        }
    }
}

/// The filter in a line or two: what it has, hides, and the basics.
#[component]
fn Summary() -> Element {
    let (backend, _) = use_ui();
    let mut board = use_board();
    let filter = board.read().filter.clone();
    if filter.count() == 0 {
        return rsx! {};
    }
    let remove = move |key: String| {
        {
            let mut b = board.write();
            b.filter.has.retain(|t| t.key() != key);
            b.filter.hides.retain(|t| t.key() != key);
        }
        reload(backend.clone(), board);
    };
    rsx! {
        div { class: "summary", aria_label: "Filter in use",
            if !filter.has.is_empty() {
                div { class: "chips",
                    span { class: "summary-label", "Has" }
                    for t in filter.has.iter() {
                        Chip { key: "{t.key()}", tag: t.clone(), on_remove: remove.clone() }
                    }
                }
            }
            if !filter.hides.is_empty() {
                div { class: "chips",
                    span { class: "summary-label", "Hides" }
                    for t in filter.hides.iter() {
                        Chip { key: "{t.key()}", tag: t.clone(), on_remove: remove.clone() }
                    }
                }
            }
            if let Some(basics) = filter.basics() {
                p { class: "hint", "{basics}" }
            }
            button { class: "quiet", onclick: move |_| board.write().view = super::View::Filters, "Edit" }
        }
    }
}

/// The first line of a prompt's text, without Markdown marks.
pub fn first_line(body: &str) -> String {
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let line = line.trim_start_matches(['#', '>', '-', '*', ' ']);
    line.replace("**", "")
        .replace('*', "")
        .replace('_', " ")
        .trim()
        .to_owned()
}

#[component]
fn Row(prompt: Prompt, open: bool) -> Element {
    let mut board = use_board();
    let id = prompt.id;
    let date = quill_board_ui::short_date(prompt.created_at);
    let line = first_line(&prompt.body);
    rsx! {
        li { class: if open { "row open" } else { "row" },
            button {
                class: "row-button",
                aria_current: if open { "true" } else { "false" },
                onclick: move |_| {
                    let mut b = board.write();
                    b.open = Some(id);
                    b.said = None;
                },
                span { class: "row-title", "{prompt.title}" }
                span { class: "row-meta",
                    "@{prompt.author.handle} · {quill_board_ui::maturity_label(prompt.maturity)} · {super::state_word(prompt.state)} · {date}"
                    if prompt.removed { " · removed" }
                }
                span { class: "row-line", "{line}" }
                if !prompt.tags.is_empty() {
                    span { class: "chips row-tags",
                        for t in prompt.tags.iter() {
                            Chip { key: "{t.key()}", tag: t.clone() }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_first_line_drops_markdown_marks() {
        assert_eq!(
            super::first_line("\n\n# **A** letter_arrives\nmore"),
            "A letter arrives"
        );
        assert_eq!(super::first_line(""), "");
    }
}
