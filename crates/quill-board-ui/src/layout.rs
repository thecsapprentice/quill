//! Page chrome and pieces shared by several pages.

use dioxus::prelude::*;
use quill_proto::board::{Prompt, PromptState, Tag, TagStatus};

use crate::{
    CSRF_FIELD, Chrome, Nav, maturity_label, prompt_url, short_date, state_label, tag_url,
};

fn nav_link(chrome: &Chrome, nav: Nav, href: &'static str, label: &'static str) -> Element {
    let current = if chrome.current == nav {
        Some("page")
    } else {
        None
    };
    rsx! { a { href: "{href}", "aria-current": current, "{label}" } }
}

pub(crate) fn header(chrome: &Chrome) -> Element {
    rsx! {
        header { class: "site-header",
            div { class: "wrap",
                a { class: "site-name", href: "/", "{chrome.site_name}" }
                nav { class: "site-nav", "aria-label": "Main",
                    {nav_link(chrome, Nav::Board, "/", "Board")}
                    if let Some(viewer) = &chrome.viewer {
                        {nav_link(chrome, Nav::Post, "/new", "Post a prompt")}
                        {nav_link(chrome, Nav::Mine, "/mine", "My prompts")}
                        {nav_link(chrome, Nav::Personas, "/personas", "Personas")}
                        {nav_link(chrome, Nav::Settings, "/settings", "Settings")}
                        if viewer.is_admin {
                            {nav_link(chrome, Nav::Admin, "/admin/reports", "Reports")}
                            a { href: "/admin/settings",
                                "aria-current": (chrome.current == Nav::ServerSettings).then_some("page"),
                                "Server settings"
                                if viewer.pending_tags > 0 {
                                    " "
                                    span { class: "count", title: "Tag requests waiting", "{viewer.pending_tags}" }
                                }
                            }
                        }
                        form { method: "post", action: "/logout",
                            input { r#type: "hidden", name: CSRF_FIELD, value: "{viewer.csrf}" }
                            button { class: "link-button", r#type: "submit", "Sign out" }
                        }
                    } else {
                        {nav_link(chrome, Nav::SignIn, "/login", "Sign in")}
                    }
                }
            }
        }
    }
}

pub(crate) fn footer(chrome: &Chrome) -> Element {
    rsx! {
        footer { class: "site-footer",
            div { class: "wrap",
                p {
                    "Quill {chrome.version}, free software under the GNU AGPL. "
                    a { href: "{chrome.source_url}", "Source code" }
                }
                p { "This board works without JavaScript and loads nothing from other sites." }
            }
        }
    }
}

/// Hidden CSRF field for forms posted by a signed-in visitor.
pub(crate) fn csrf(chrome: &Chrome) -> Element {
    match &chrome.viewer {
        Some(v) => rsx! { input { r#type: "hidden", name: CSRF_FIELD, value: "{v.csrf}" } },
        None => rsx! {},
    }
}

pub(crate) fn state_badge(state: PromptState) -> Element {
    rsx! { span { class: "state state-{state}", "{state_label(state)}" } }
}

pub(crate) fn tag_list(tags: &[Tag]) -> Element {
    if tags.is_empty() {
        return rsx! {};
    }
    rsx! {
        ul { class: "tags", "aria-label": "Tags",
            for tag in tags.iter() {
                li { key: "{tag.key()}",
                    match tag.status {
                        TagStatus::Visible => rsx! {
                            a { href: "{tag_url(tag)}", title: "Show prompts tagged {tag.namespace}: {tag.display}",
                                span { class: "tag-ns", "{tag.namespace}: " }
                                "{tag.display}"
                            }
                        },
                        // Only its author and administrators get these.
                        TagStatus::Pending => rsx! {
                            span { class: "tag-quiet",
                                span { class: "tag-ns", "{tag.namespace}: " }
                                "{tag.display} "
                                em { "(awaiting approval)" }
                            }
                        },
                        TagStatus::Hidden => rsx! {
                            span { class: "tag-quiet",
                                span { class: "tag-ns", "{tag.namespace}: " }
                                "{tag.display} "
                                em { "(hidden)" }
                            }
                        },
                    }
                }
            }
        }
    }
}

pub(crate) fn prompt_meta(p: &Prompt) -> Element {
    let author = p.author.display_name.clone().map_or_else(
        || format!("@{}", p.author.handle),
        |name| format!("{name} (@{})", p.author.handle),
    );
    rsx! {
        p { class: "card-meta",
            span { "{author}" }
            span { class: "rating", title: "Maturity rating", "{maturity_label(p.maturity)}" }
            {state_badge(p.state)}
            span { "{short_date(p.created_at)}" }
            if p.removed {
                span { class: "removed", "Removed by a server administrator" }
            }
        }
    }
}

/// A prompt as an index card in a list, with a clipped excerpt.
pub(crate) fn prompt_card(p: &Prompt) -> Element {
    rsx! {
        li { class: "card", key: "{p.id}",
            h2 { class: "card-title", a { href: "{prompt_url(p.id)}", "{p.title}" } }
            {prompt_meta(p)}
            div { class: "ruled excerpt", dangerous_inner_html: "{p.body_html}" }
            {tag_list(&p.tags)}
        }
    }
}

pub(crate) fn card_list(prompts: &[Prompt]) -> Element {
    rsx! {
        ul { class: "cards",
            for p in prompts.iter() {
                {prompt_card(p)}
            }
        }
    }
}

/// `<datalist>` of tag suggestions; browsers offer them without JavaScript.
pub(crate) fn tag_datalist(id: &'static str, tags: &[Tag]) -> Element {
    rsx! {
        datalist { id: "{id}",
            for tag in tags.iter() {
                option { key: "{tag.key()}", value: "{tag.key()}", "{tag.display}" }
            }
        }
    }
}

pub(crate) fn error_notice(error: Option<&String>) -> Element {
    match error {
        Some(e) => rsx! { p { class: "notice error", role: "alert", "{e}" } },
        None => rsx! {},
    }
}
