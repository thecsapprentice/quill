//! Server settings pages, for administrators: tag types and tags.

use dioxus::prelude::*;
use quill_proto::board::{AdminTag, TagNamespace, TagOrigin};

use crate::{
    AdminTagDeleteView, AdminTagTypesView, AdminTagView, AdminTagsView, Chrome, admin_tag_url,
    layout::{csrf, error_notice, tag_datalist},
    render,
};

const ALIAS_OPTIONS: &str = "alias-options";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    TagTypes,
    Tags,
}

fn tabs(current: Tab) -> Element {
    let mark = |tab: Tab| (tab == current).then_some("page");
    rsx! {
        nav { class: "tabs", "aria-label": "Server settings",
            a { href: "/admin/settings/tag-types", "aria-current": mark(Tab::TagTypes), "Tag types" }
            a { href: "/admin/settings/tags", "aria-current": mark(Tab::Tags), "Tags" }
        }
    }
}

/// What kind of tag this is, in plain words.
fn tag_kind(tag: &AdminTag) -> String {
    match (&tag.alias_of, tag.tag.origin) {
        (Some(target), _) => format!("Other name for {target}"),
        (None, TagOrigin::Server) => "Curated".to_owned(),
        (None, TagOrigin::User) => "Added by a writer".to_owned(),
    }
}

fn uses_text(uses: u64) -> String {
    match uses {
        0 => "No prompts".to_owned(),
        1 => "1 prompt".to_owned(),
        n => format!("{n} prompts"),
    }
}

fn namespace_options(namespaces: &[TagNamespace], selected: Option<&str>) -> Element {
    rsx! {
        for ns in namespaces.iter() {
            option { key: "{ns.name}", value: "{ns.name}", selected: selected == Some(ns.name.as_str()),
                "{ns.display}"
            }
        }
    }
}

/// The two switches of a tag type. Explanations show only on the "add" form;
/// the page introduction covers the existing types.
fn type_checkboxes(locked: bool, required: bool, explain: bool) -> Element {
    rsx! {
        div { class: "field",
            label { class: "inline-check",
                input { r#type: "checkbox", name: "locked", value: "yes", checked: locked }
                span {
                    "Curated tags only"
                    if explain {
                        span { class: "hint", "Writers can use only the tags listed here, and can't create new ones." }
                    }
                }
            }
        }
        div { class: "field",
            label { class: "inline-check",
                input { r#type: "checkbox", name: "required", value: "yes", checked: required }
                span {
                    "Required"
                    if explain {
                        span { class: "hint", "Every prompt needs at least one tag of this type when it's posted or edited. Existing prompts aren't changed until they're edited." }
                    }
                }
            }
        }
    }
}

// ---- Tag types -----------------------------------------------------------

pub fn admin_tag_types_page(chrome: &Chrome, view: AdminTagTypesView) -> String {
    let chrome_for_body = chrome.clone();
    render("Tag types", chrome, move || {
        let v = &view;
        rsx! {
            h1 { "Server settings" }
            {tabs(Tab::TagTypes)}
            p { class: "lede",
                "Tag types group the tags on prompts, like Genre or Content warning. The identifier appears in filters such as genre:horror and can't be changed."
            }
            p { class: "lede",
                "With curated tags only, writers can use just the tags listed here. A required type needs at least one tag on every prompt when it's posted or edited; existing prompts aren't changed until then."
            }
            {error_notice(v.error.as_ref())}
            for ns in v.namespaces.iter() {
                form { key: "{ns.name}", class: "form-card", method: "post",
                    action: "/admin/settings/tag-types/{ns.name}",
                    {csrf(&chrome_for_body)}
                    h2 { class: "form-title", "{ns.display} " code { "{ns.name}" } }
                    div { class: "field",
                        label { r#for: "display-{ns.name}", "Name shown on the board" }
                        input { r#type: "text", id: "display-{ns.name}", name: "display", value: "{ns.display}", required: true }
                    }
                    {type_checkboxes(ns.locked, ns.required, false)}
                    div { class: "actions",
                        button { class: "btn", r#type: "submit", "Save {ns.display}" }
                        a { href: "/admin/settings/tags?namespace={ns.name}", "See its tags" }
                    }
                }
            }
            h2 { "Add a tag type" }
            form { class: "form-card", method: "post", action: "/admin/settings/tag-types",
                {csrf(&chrome_for_body)}
                div { class: "filter-grid",
                    div { class: "field",
                        label { r#for: "new-name", "Identifier"
                            span { class: "hint", "Lowercase letters, digits and hyphens, like setting." }
                        }
                        input { r#type: "text", id: "new-name", name: "name", value: "{v.new.name}", required: true }
                    }
                    div { class: "field",
                        label { r#for: "new-display", "Name shown on the board"
                            span { class: "hint", "Like Setting." }
                        }
                        input { r#type: "text", id: "new-display", name: "display", value: "{v.new.display}", required: true }
                    }
                }
                {type_checkboxes(v.new.locked, v.new.required, true)}
                div { class: "actions",
                    button { class: "btn", r#type: "submit", "Add tag type" }
                }
            }
        }
    })
}

// ---- Tags ----------------------------------------------------------------

pub fn admin_tags_page(chrome: &Chrome, view: AdminTagsView) -> String {
    let chrome_for_body = chrome.clone();
    render("Tags", chrome, move || {
        let v = &view;
        let type_name = |name: &str| {
            v.namespaces
                .iter()
                .find(|n| n.name == name)
                .map_or_else(|| name.to_owned(), |n| n.display.clone())
        };
        rsx! {
            h1 { "Server settings" }
            {tabs(Tab::Tags)}
            p { class: "lede",
                "Curated tags are added here. Tags writers create while posting appear too, so you can rename, merge or delete them."
            }
            {error_notice(v.error.as_ref())}
            form { class: "form-card", method: "post", action: "/admin/settings/tags",
                {csrf(&chrome_for_body)}
                h2 { class: "form-title", "Add a curated tag" }
                div { class: "filter-grid",
                    div { class: "field",
                        label { r#for: "new-namespace", "Type" }
                        select { id: "new-namespace", name: "namespace", required: true,
                            {namespace_options(&v.namespaces, Some(v.new_namespace.as_str()))}
                        }
                    }
                    div { class: "field",
                        label { r#for: "new-tag", "Name"
                            span { class: "hint", "Adding a writer's tag here makes it curated." }
                        }
                        input { r#type: "text", id: "new-tag", name: "name", value: "{v.new_name}", required: true }
                    }
                }
                div { class: "actions",
                    button { class: "btn", r#type: "submit", "Add tag" }
                }
            }
            form { class: "form-card", method: "get", action: "/admin/settings/tags", role: "search",
                h2 { class: "form-title", "Find tags" }
                div { class: "filter-grid",
                    div { class: "field",
                        label { r#for: "filter-namespace", "Type" }
                        select { id: "filter-namespace", name: "namespace",
                            option { value: "", "All types" }
                            {namespace_options(&v.namespaces, v.namespace.as_deref())}
                        }
                    }
                    div { class: "field",
                        label { r#for: "filter-search", "Name contains" }
                        input { r#type: "search", id: "filter-search", name: "search", value: "{v.search}" }
                    }
                }
                div { class: "actions",
                    button { class: "btn btn-quiet", r#type: "submit", "Show tags" }
                }
            }
            if v.tags.is_empty() {
                div { class: "empty", "No tags match." }
            } else {
                div { class: "table-scroll",
                    table { class: "log",
                        thead {
                            tr { th { "Tag" } th { "Type" } th { "Kind" } th { "Used on" } th { span { class: "visually-hidden", "Actions" } } }
                        }
                        tbody {
                            for t in v.tags.iter() {
                                tr { key: "{t.tag.key()}",
                                    td {
                                        a { href: "{admin_tag_url(&t.tag)}", "{t.tag.display}" }
                                        br {}
                                        code { "{t.tag.key()}" }
                                    }
                                    td { "{type_name(&t.tag.namespace)}" }
                                    td { "{tag_kind(t)}" }
                                    td { "{uses_text(t.uses)}" }
                                    td { a { href: "{admin_tag_url(&t.tag)}/delete", "Delete" } }
                                }
                            }
                        }
                    }
                }
            }
            if v.truncated {
                p { class: "hint", "Showing the first {v.tags.len()} tags. Choose a type or search to narrow the list." }
            }
        }
    })
}

pub fn admin_tag_page(chrome: &Chrome, view: AdminTagView) -> String {
    let title = view.tag.tag.display.clone();
    let chrome_for_body = chrome.clone();
    render(title, chrome, move || {
        let v = &view;
        let t = &v.tag;
        let url = admin_tag_url(&t.tag);
        let key = t.tag.key();
        rsx! {
            p { a { href: "/admin/settings/tags?namespace={t.tag.namespace}", "All {v.namespace_display} tags" } }
            h1 { "{t.tag.display}" }
            p { class: "card-meta",
                code { "{key}" }
                span { "{tag_kind(t)}" }
                if t.alias_of.is_none() {
                    span { "{uses_text(t.uses)}" }
                }
            }
            {error_notice(v.error.as_ref())}
            form { class: "form-card", method: "post", action: "{url}",
                {csrf(&chrome_for_body)}
                h2 { class: "form-title", "Rename" }
                div { class: "field",
                    label { r#for: "rename", "Name shown on prompts"
                        span { class: "hint", "The identifier {key} stays the same, so links and filters keep working." }
                    }
                    input { r#type: "text", id: "rename", name: "display", value: "{v.rename}", required: true }
                }
                div { class: "actions",
                    button { class: "btn", r#type: "submit", "Save name" }
                }
            }
            if t.alias_of.is_none() {
                form { class: "form-card", method: "post", action: "{url}/merge",
                    {csrf(&chrome_for_body)}
                    h2 { class: "form-title", "Merge into another tag" }
                    div { class: "field",
                        label { r#for: "alias-target", "Tag to merge into"
                            span { class: "hint",
                                "Written as type:name, like genre:mystery. Prompts tagged {key} move to that tag, and {key} becomes another name for it. This can't be undone."
                            }
                        }
                        input { r#type: "text", id: "alias-target", name: "target", value: "{v.alias_target}",
                            required: true, list: ALIAS_OPTIONS }
                    }
                    div { class: "actions",
                        button { class: "btn btn-quiet", r#type: "submit", "Merge tag" }
                    }
                }
                {tag_datalist(ALIAS_OPTIONS, &v.tag_options)}
            }
            div { class: "form-card",
                h2 { class: "form-title", "Delete" }
                p { "Removes the tag from every prompt that has it." }
                div { class: "actions",
                    a { class: "btn btn-danger", href: "{url}/delete", "Delete tag" }
                }
            }
        }
    })
}

pub fn admin_tag_delete_page(chrome: &Chrome, view: AdminTagDeleteView) -> String {
    let title = format!("Delete {}", view.tag.tag.display);
    let chrome_for_body = chrome.clone();
    render(title, chrome, move || {
        let v = &view;
        let t = &v.tag;
        let url = admin_tag_url(&t.tag);
        let ns = &v.namespace;
        rsx! {
            h1 { "Delete {t.tag.display}?" }
            p { class: "card-meta", code { "{t.tag.key()}" } span { "{tag_kind(t)}" } }
            match (&t.alias_of, t.uses) {
                (Some(target), _) => rsx! {
                    p { "This is another name for {target}. Deleting it removes only that name; no prompts change." }
                },
                (None, 0) => rsx! { p { "No prompts use this tag." } },
                (None, n) => rsx! {
                    p { "It's on {uses_text(n).to_lowercase()}. Deleting removes it from them; the prompts stay on the board with their other tags." }
                },
            }
            if ns.required && t.alias_of.is_none() && t.uses > 0 {
                div { class: "warning",
                    p { "{ns.display} is a required type. Prompts left without a {ns.display} tag will need one the next time their authors edit them." }
                }
            }
            if !ns.locked {
                p { class: "hint", "Writers can still create this tag again by using it. To stop that, make {ns.display} curated-only." }
            }
            form { class: "form-card", method: "post", action: "{url}/delete",
                {csrf(&chrome_for_body)}
                div { class: "field",
                    label { class: "inline-check",
                        input { r#type: "checkbox", name: "confirm", value: "yes", required: true }
                        "Delete {t.tag.key()}. This can't be undone."
                    }
                }
                div { class: "actions",
                    button { class: "btn btn-danger", r#type: "submit", "Delete tag" }
                    a { href: "{url}", "Cancel" }
                }
            }
        }
    })
}
