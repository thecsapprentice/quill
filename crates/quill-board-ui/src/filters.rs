//! The board's filter builder: every tag of every type, however many, each
//! set to any, has or hide, without JavaScript.
//!
//! The whole page is one GET form. Hidden fields carry the filter so far;
//! radios cover the tags on screen (named in `shown`), and submit buttons
//! switch type, letter and page, so nothing chosen is lost. The server
//! merges them on every request. See `docs/plan.md`, "Board layout".

use dioxus::prelude::*;
use quill_proto::board::{Maturity, Tag, TagNamespace};

use crate::{
    BoardFilters, Chrome, Layout, maturity_label,
    pages::{maturity_options, state_options},
    render_with, state_label,
};

pub const FILTERS_PATH: &str = "/filters";

/// The panel for rating, status and search, as `type` names it.
pub const BASICS: &str = "basics";

/// How one tag is set in the filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagChoice {
    Any,
    Has,
    Hide,
}

impl TagChoice {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Has => "has",
            Self::Hide => "hide",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "any" => Some(Self::Any),
            "has" => Some(Self::Has),
            "hide" => Some(Self::Hide),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiltersView {
    /// The filter as it stands, with this request's choices merged in.
    pub filters: BoardFilters,
    pub cap: Maturity,
    pub namespaces: Vec<TagNamespace>,
    /// The tag type shown, or `None` for rating, status and search.
    pub current: Option<String>,
    /// Text the shown tags' names contain.
    pub search: String,
    /// The letter the shown tags' names start with.
    pub letter: Option<char>,
    /// A to Z rather than most used first.
    pub by_name: bool,
    pub page: u32,
    pub page_size: u32,
    /// The shown tags, each with how many prompts carry it.
    pub tags: Vec<(Tag, u64)>,
    /// How many tags matched, across pages.
    pub total: u64,
}

/// The filter builder, starting from the board's filters `f`.
pub fn filters_url(f: &BoardFilters) -> String {
    // The board's URL is "/" or "/?query"; the builder takes the same query.
    format!("{FILTERS_PATH}{}", &f.url(None)[1..])
}

/// The tag keys in a comma-separated filter list.
pub fn filter_keys(list: &str) -> Vec<String> {
    list.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

fn choice_of(f: &BoardFilters, key: &str) -> TagChoice {
    if filter_keys(&f.include).iter().any(|k| k == key) {
        TagChoice::Has
    } else if filter_keys(&f.exclude).iter().any(|k| k == key) {
        TagChoice::Hide
    } else {
        TagChoice::Any
    }
}

/// How many of a type's tags the filter names.
fn chosen_in(f: &BoardFilters, namespace: &str) -> usize {
    let prefix = format!("{namespace}:");
    filter_keys(&f.include)
        .iter()
        .chain(filter_keys(&f.exclude).iter())
        .filter(|k| k.starts_with(&prefix))
        .count()
}

const LETTERS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";

pub fn filters_page(chrome: &Chrome, view: FiltersView) -> String {
    render_with(Layout::Wide, "Filter prompts", chrome, move || {
        let v = &view;
        let f = &v.filters;
        let basics = v.current.is_none();
        let shown: Vec<String> = v.tags.iter().map(|(t, _)| t.key()).collect();
        let pages = v.total.div_ceil(u64::from(v.page_size.max(1)));
        let basics_count = usize::from(f.maturity.is_some())
            + usize::from(f.state.is_some())
            + usize::from(!f.q.is_empty());
        rsx! {
            p { class: "back-row", a { href: "{f.url(None)}", "← Back to the board" } }
            h1 { "Filter prompts" }
            p { class: "lede", "Pick a tag type, then set each tag to any, has or hide. Nothing changes on the board until you show the prompts." }
            form { class: "filter-builder", method: "get", action: FILTERS_PATH,
                // The filter so far. Tags on screen are named in `shown`,
                // and their radios replace what these say about them.
                input { r#type: "hidden", name: "include", value: "{f.include}" }
                input { r#type: "hidden", name: "exclude", value: "{f.exclude}" }
                input { r#type: "hidden", name: "shown", value: "{shown.join(\",\")}" }
                input { r#type: "hidden", name: "type", value: "{v.current.as_deref().unwrap_or(BASICS)}" }
                if !basics {
                    input { r#type: "hidden", name: "q", value: "{f.q}" }
                    input { r#type: "hidden", name: "maturity", value: "{f.maturity.map_or(\"\", |m| m.as_str())}" }
                    input { r#type: "hidden", name: "state", value: "{f.state.map_or(\"\", |s| s.as_str())}" }
                    input { r#type: "hidden", name: "page", value: "{v.page}" }
                    if let Some(l) = v.letter {
                        input { r#type: "hidden", name: "letter", value: "{l}" }
                    }
                }
                div { class: "filter-layout",
                    nav { class: "filter-types", "aria-label": "Tag types",
                        button { class: "type-btn", r#type: "submit", name: "to_type", value: BASICS,
                            "aria-current": basics.then_some("true"),
                            span { "Rating, status and search" }
                            if basics_count > 0 { span { class: "count", "{basics_count}" } }
                        }
                        for n in v.namespaces.iter() {
                            button { key: "{n.name}", class: "type-btn", r#type: "submit",
                                name: "to_type", value: "{n.name}",
                                "aria-current": (v.current.as_deref() == Some(n.name.as_str())).then_some("true"),
                                span { "{n.display}" }
                                if chosen_in(f, &n.name) > 0 {
                                    span { class: "count", "{chosen_in(f, &n.name)}" }
                                }
                            }
                        }
                    }
                    section { class: "filter-panel",
                        if basics {
                            {basics_panel(v)}
                        } else {
                            {tags_panel(v, pages)}
                        }
                    }
                    aside { class: "filter-chosen",
                        h2 { "Your filter" }
                        {chosen_list("Has", &filter_keys(&f.include))}
                        {chosen_list("Hides", &filter_keys(&f.exclude))}
                        if f.maturity.is_some() || f.state.is_some() || !f.q.is_empty() {
                            p { class: "chosen-rest",
                                if let Some(m) = f.maturity { span { "{maturity_label(m)} or under" } }
                                if let Some(s) = f.state { span { "{state_label(s)}" } }
                                if !f.q.is_empty() { span { "“{f.q}”" } }
                            }
                        }
                        if !f.is_active() {
                            p { class: "hint", "No filters yet: every prompt you can see." }
                        }
                        div { class: "actions",
                            button { class: "btn", r#type: "submit", name: "go", value: "apply", "Show prompts" }
                            a { href: FILTERS_PATH, "Clear all" }
                        }
                    }
                }
            }
        }
    })
}

fn chosen_list(label: &'static str, keys: &[String]) -> Element {
    if keys.is_empty() {
        return rsx! {};
    }
    rsx! {
        p { class: "chosen-group", b { "{label}" } }
        p { class: "chosen-tags",
            for k in keys.iter() {
                span { key: "{k}", class: if label == "Hides" { "chip chip-hide" } else { "chip" }, "{k}" }
            }
        }
    }
}

fn basics_panel(v: &FiltersView) -> Element {
    let f = &v.filters;
    rsx! {
        h2 { "Rating, status and search" }
        div { class: "field",
            label { r#for: "maturity", "Rated up to" }
            select { id: "maturity", name: "maturity",
                option { value: "", selected: f.maturity.is_none(), "My setting ({maturity_label(v.cap)})" }
                {maturity_options(f.maturity)}
            }
        }
        div { class: "field",
            label { r#for: "state", "Status" }
            select { id: "state", name: "state",
                option { value: "", selected: f.state.is_none(), "Any" }
                {state_options(f.state)}
            }
        }
        div { class: "field",
            label { r#for: "q", "Search titles and text" }
            input { r#type: "search", id: "q", name: "q", value: "{f.q}" }
        }
    }
}

fn tags_panel(v: &FiltersView, pages: u64) -> Element {
    let display = v
        .current
        .as_deref()
        .and_then(|c| v.namespaces.iter().find(|n| n.name == c))
        .map_or_else(
            || v.current.clone().unwrap_or_default(),
            |n| n.display.clone(),
        );
    rsx! {
        h2 { "{display} " span { class: "muted", "{v.total} tags" } }
        div { class: "tag-search",
            input { r#type: "search", name: "tq", value: "{v.search}", "aria-label": "Search {display}",
                placeholder: "Search {display}" }
            select { name: "order", "aria-label": "Order",
                option { value: "used", selected: !v.by_name, "Most used" }
                option { value: "az", selected: v.by_name, "A to Z" }
            }
            button { class: "btn btn-quiet", r#type: "submit", name: "find", value: "1", "Search" }
        }
        div { class: "letters", "aria-label": "Starting with",
            button { r#type: "submit", name: "to_letter", value: "",
                "aria-current": v.letter.is_none().then_some("true"), "All" }
            for l in LETTERS.chars() {
                button { key: "{l}", r#type: "submit", name: "to_letter", value: "{l}",
                    "aria-current": (v.letter == Some(l)).then_some("true"), "{l}" }
            }
        }
        if v.tags.is_empty() {
            p { class: "empty", "No tags here." }
        } else {
            ul { class: "tag-choices",
                for (t, uses) in v.tags.iter() {
                    {tag_choice(&v.filters, t, *uses)}
                }
            }
        }
        if pages > 1 {
            p { class: "pager",
                span { class: "muted", "Page {v.page + 1} of {pages}" }
                if v.page > 0 {
                    button { class: "btn btn-quiet", r#type: "submit", name: "to_page",
                        value: "{v.page - 1}", "Previous" }
                }
                if u64::from(v.page) + 1 < pages {
                    button { class: "btn btn-quiet", r#type: "submit", name: "to_page",
                        value: "{v.page + 1}", "Next" }
                }
            }
        }
    }
}

fn tag_choice(f: &BoardFilters, t: &Tag, uses: u64) -> Element {
    let key = t.key();
    let now = choice_of(f, &key);
    let prompts = if uses == 1 { "prompt" } else { "prompts" };
    rsx! {
        li { key: "{key}", class: "tag-choice",
            span { class: "tag-name", "{t.display}" }
            span { class: "muted uses", "{uses} {prompts}" }
            fieldset { class: "tri",
                legend { class: "visually-hidden", "{t.display}" }
                for c in [TagChoice::Any, TagChoice::Has, TagChoice::Hide] {
                    label { key: "{c.as_str()}", class: "tri-{c.as_str()}",
                        input { r#type: "radio", name: "tag:{key}", value: "{c.as_str()}", checked: now == c }
                        span {
                            match c {
                                TagChoice::Any => "Any",
                                TagChoice::Has => "Has",
                                TagChoice::Hide => "Hide",
                            }
                        }
                    }
                }
            }
        }
    }
}
