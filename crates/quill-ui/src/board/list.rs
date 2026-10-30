//! The board page (mockup D1; N1 and N4 when narrow): search, the filter
//! summary and the prompt list beside a reading pane.

use dioxus::prelude::*;
use quill_proto::board::Prompt;

use super::{
    MineState, View, filters::FilterBuilder, more, post::PostForm, reader::Reader, reload,
    use_board, use_tag_types,
};
use crate::{
    app::{ProblemLine, use_ui},
    board::reader::Chip,
};

/// How long typing in the search must pause before it searches.
const SEARCH_AFTER_MS: u64 = 300;
/// Within this many pixels of the end, scrolling loads the next page.
const LOAD_NEAR_PX: i32 = 300;

/// The Board section.
#[component]
pub fn BoardPage() -> Element {
    let (backend, _) = use_ui();
    let board = use_board();
    use_tag_types();
    // The first visit loads the first page; My prompts loads afresh on
    // each visit, since they may have changed on the Board.
    use_hook(move || {
        let b = board.peek();
        let again = b.is_mine() || !b.loaded;
        let busy = b.loading;
        drop(b);
        if again && !busy {
            reload(backend, board);
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
    let bm = backend.clone();
    let on_scroll = move |e: ScrollEvent| {
        let left = e.scroll_height() - e.client_height() - e.scroll_top() as i32;
        if left < LOAD_NEAR_PX {
            more(bm.clone(), board);
        }
    };
    let bm = backend.clone();
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
                header { class: "board-head",
                    h1 { if mine { "My prompts" } else { "Board" } }
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
                ul { class: "prompts", onscroll: on_scroll,
                    for p in items.iter() {
                        Row { key: "{p.id}", prompt: p.clone(), open: Some(p.id) == open }
                    }
                }
                div { class: "list-end", role: "status",
                    if loading {
                        "Loading prompts…"
                    } else if mine && loaded && items.is_empty() && !next && persona.is_some() {
                        "No prompts posted as this persona."
                    } else if mine && loaded && items.is_empty() && !next {
                        "You haven't posted a prompt yet."
                    } else if loaded && items.is_empty() && (count > 0 || searching) && !next {
                        "No prompts match. Try fewer filters, or another search."
                    } else if loaded && items.is_empty() && !next {
                        "No prompts on the board yet."
                    } else if next {
                        button {
                            class: "secondary",
                            onclick: move |_| more(bm.clone(), board),
                            "Show more prompts"
                        }
                    } else if loaded {
                        "That's every prompt."
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
