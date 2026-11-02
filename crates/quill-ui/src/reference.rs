//! The design reference (advanced mode): every token and component of
//! `docs/design/gui-design-language.md`, in every state, in light and dark
//! side by side. For hand checks; nothing here does anything.

use dioxus::prelude::*;

use crate::field::{Area, Field, Kind};

const COLOURS: [&str; 13] = [
    "paper",
    "surface",
    "ink",
    "muted",
    "line",
    "edge",
    "accent",
    "accent-ink",
    "accent-soft",
    "warn",
    "warn-soft",
    "bad",
    "bad-ink",
];
const SIZES: [(&str, &str); 6] = [
    ("text-xs", "Badges, counts and chip notes"),
    ("text-sm", "Hints, meta lines and labels"),
    ("text-base", "Body text and controls"),
    ("text-read", "Written text, in the book serif"),
    ("text-lg", "Section headings"),
    ("text-xl", "Page titles"),
];
const SPACES: [&str; 7] = ["1", "2", "3", "4", "5", "6", "7"];
const BUTTONS: [&str; 4] = ["primary", "secondary", "quiet", "danger"];
/// A state's label, and the class that shows it.
const STATES: [(&str, &str); 4] = [
    ("Rest", ""),
    ("Hover", " is-hover"),
    ("Focus", " is-focus"),
    ("Pressed", " is-active"),
];

#[component]
pub fn ReferencePage() -> Element {
    rsx! {
        h1 { "Design reference" }
        p { class: "lede",
            "Every part of the app's design language, in each of its states. The rules and their reasons are in docs/design/gui-design-language.md."
        }
        div { class: "ref-themes",
            Theme { theme: "light", name: "Light" }
            Theme { theme: "dark", name: "Dark" }
        }
    }
}

/// One theme's copy. Ids carry the theme, so they stay unique on the page.
#[component]
fn Theme(theme: &'static str, name: &'static str) -> Element {
    let id = |what: &str| format!("ref-{theme}-{what}");
    rsx! {
        section { class: "quill ref-theme", "data-theme": theme, aria_label: "{name}",
            h2 { "{name}" }

            h3 { "Colour" }
            div { class: "ref-swatches",
                for c in COLOURS {
                    div { key: "{c}", class: "ref-swatch",
                        span { class: "ref-chip", style: "background: var(--{c});" }
                        code { "--{c}" }
                    }
                }
            }

            h3 { "Type" }
            for (token, use_) in SIZES {
                p { key: "{token}", style: "font-size: var(--{token}); margin: 0 0 var(--space-1);",
                    if token == "text-read" {
                        span { style: "font-family: var(--write);", "{use_}" }
                    } else {
                        "{use_}"
                    }
                    span { class: "hint", "  --{token}" }
                }
            }

            h3 { "Spacing" }
            div { class: "ref-spaces",
                for s in SPACES {
                    div { key: "{s}", class: "ref-space",
                        span { style: "width: var(--space-{s});" }
                        code { "--space-{s}" }
                    }
                }
            }

            h3 { "Page header" }
            div { class: "page-head",
                h4 { class: "ref-title", "Board" }
                div { class: "head-actions",
                    button { class: "quiet", "Refresh" }
                    button { class: "primary", "Post a prompt" }
                }
            }

            h3 { "Buttons" }
            table { class: "ref-states",
                thead {
                    tr {
                        th { scope: "col", "" }
                        for (state, _) in STATES {
                            th { key: "{state}", scope: "col", "{state}" }
                        }
                        th { scope: "col", "Disabled" }
                        th { scope: "col", "Busy" }
                    }
                }
                tbody {
                    for kind in BUTTONS {
                        tr { key: "{kind}",
                            th { scope: "row", "{kind}" }
                            for (state, class) in STATES {
                                td { key: "{state}",
                                    button { class: "{kind}{class}", "Save" }
                                }
                            }
                            td { button { class: "{kind}", disabled: true, "Save" } }
                            td {
                                button { class: "{kind}", disabled: true, aria_busy: "true", "Saving…" }
                            }
                        }
                    }
                }
            }
            p { "Inside running text, a " button { class: "link", "link-style action" } "." }

            h3 { "Fields" }
            Field {
                label: "Title",
                id: id("title"),
                kind: Kind::Text,
                hint: "Up to 200 characters.",
                oninput: |_| {},
            }
            Field {
                label: "Title, typed past its limit",
                id: id("title-over"),
                kind: Kind::Text,
                initial: "A title that has run on for far too long",
                hint: "201 of 200 characters: 1 too many.",
                invalid: true,
                oninput: |_| {},
            }
            Area {
                label: "Prompt",
                id: id("body"),
                initial: "A letter arrives forty years late.",
                hint: "Markdown: *italic*, **bold**, lists and links. 33 of 20,000 characters.",
                oninput: |_| {},
            }
            div { class: "field-row",
                div { class: "field",
                    label { r#for: id("select"), "Maturity rating" }
                    select { id: id("select"),
                        option { "Teen" }
                        option { "General" }
                    }
                    p { class: "hint", "" }
                }
                div { class: "field",
                    label { r#for: id("disabled"), "Disabled" }
                    input { id: id("disabled"), disabled: true, initial_value: "Can't change this" }
                    p { class: "hint", "Says why nearby." }
                }
            }
            fieldset {
                legend { "Choices" }
                div { class: "choice",
                    input { r#type: "checkbox", id: id("check"), checked: true }
                    label { r#for: id("check"), "A checkbox, ticked" }
                }
                div { class: "choice",
                    input { r#type: "radio", id: id("radio-a"), name: id("radio"), checked: true }
                    label { r#for: id("radio-a"), "One radio, chosen" }
                }
                div { class: "choice",
                    input { r#type: "radio", id: id("radio-b"), name: id("radio") }
                    label { r#for: id("radio-b"), "Another radio" }
                }
            }
            div { class: "segmented", role: "group", aria_label: "A segmented choice",
                button { aria_pressed: "false", "Any" }
                button { aria_pressed: "true", "Has" }
                button { aria_pressed: "false", "Hide" }
            }

            h3 { "Surfaces" }
            div { class: "card ref-card",
                h4 { class: "ref-title", "A card" }
                p { class: "lede", "One question on its own, as in first run." }
                div { class: "actions",
                    button { class: "primary", "Continue" }
                    button { class: "quiet", "Not now" }
                }
            }
            ul { class: "list",
                li { class: "item",
                    header {
                        span { class: "name", "An item" }
                        span { class: "handle", "@ink_fox" }
                        span { class: "tag", "This computer" }
                    }
                    p { "One thing in a short list, with its actions." }
                    div { class: "actions", button { class: "secondary", "Rename" } button { class: "quiet", "Remove…" } }
                }
            }
            section { class: "panel", aria_label: "A panel",
                h4 { class: "ref-title", "A panel" }
                p { "A part of a page set apart." }
            }

            h3 { "Telling people things" }
            div { class: "nudge",
                p { "Something worth doing, with its button." }
                button { class: "primary", "Link this computer" }
            }
            div { class: "notice", "Something needs attention before going on." }
            p { class: "problem", "What went wrong, then what to do about it." }
            p { class: "hint", "What just happened: Prompt posted." }

            h3 { "Small things" }
            div { class: "chips",
                span { class: "chip", "genre: Horror" }
                span { class: "chip",
                    "theme: Lighthouses"
                    button { class: "chip-remove", aria_label: "Remove theme: Lighthouses", "×" }
                }
                span { class: "chip", "theme: Memory" span { class: "chip-note", " · new, needs approval" } }
                span { class: "slot", "Length: pick 1" }
                span { class: "tag", "Keys on another device" }
                span { class: "badge", "2" span { class: "sr-only", " unread" } }
            }

            h3 { "Empty, loading and done" }
            p { class: "coming", "No prompts on the board yet." }
            p { class: "coming", "Loading prompts…" }
            p { class: "list-end", "That's every prompt." }

            h3 { "Writing" }
            div { class: "markdown",
                p { "Every entry in the lighthouse log ends mid-sentence. Write the night it starts making sense, in the keeper's words or the sea's." }
            }
        }
    }
}
