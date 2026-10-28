//! Page chrome and pieces shared by several pages.

use dioxus::prelude::*;
use quill_proto::board::{Prompt, PromptState, Tag, TagStatus};

use crate::{
    CSRF_FIELD, Chrome, Nav, chips::chip_classes, maturity_label, prompt_url, short_date,
    state_label, tag_url,
};

/// The small line icons of the phone's tab bar. Plain SVG markup, so the
/// CSP's ban on inline styles doesn't apply; hidden on wide screens.
#[derive(Clone, Copy)]
enum Icon {
    Board,
    Post,
    Mine,
    Notices,
    More,
    News,
    SignIn,
}

fn icon(which: Icon) -> Element {
    let path = match which {
        Icon::Board => "M4 6h16M4 12h16M4 18h10",
        Icon::Post => "M12 5v14M5 12h14",
        Icon::Mine => "M7 3h7l4 4v14H7zM14 3v4h4",
        Icon::Notices => "M6 16v-5a6 6 0 0 1 12 0v5l2 2H4zM10 20a2 2 0 0 0 4 0",
        Icon::More => "M5 12h.01M12 12h.01M19 12h.01",
        Icon::News => "M4 5h13v14H6a2 2 0 0 1-2-2zM17 9h3v8a2 2 0 0 1-2 2M8 9h5M8 13h5",
        Icon::SignIn => "M10 17l5-5-5-5M15 12H3M14 4h5v16h-5",
    };
    rsx! {
        svg { class: "nav-icon", "viewBox": "0 0 24 24", "aria-hidden": "true",
            "fill": "none", "stroke": "currentColor", "stroke-width": "2",
            "stroke-linecap": "round", "stroke-linejoin": "round",
            path { "d": "{path}" }
        }
    }
}

/// One link in the main nav. `tab` links also sit in the phone's bottom tab
/// bar, labelled `short` there; the rest are only on wide screens and the
/// More page.
fn nav_link(
    chrome: &Chrome,
    nav: Nav,
    href: &'static str,
    label: &'static str,
    tab: Option<(Icon, &'static str)>,
    (count, count_title): (u64, &'static str),
) -> Element {
    let current = (chrome.current == nav).then_some("page");
    match tab {
        Some((i, short)) => rsx! {
            a { class: "tab", href: "{href}", "aria-current": current,
                {icon(i)}
                span { class: "long", "{label}" }
                span { class: "short", "{short}" }
                if count > 0 {
                    span { class: "count", title: "{count_title}", "{count}" }
                }
            }
        },
        None => rsx! {
            a { href: "{href}", "aria-current": current,
                "{label}"
                if count > 0 {
                    " "
                    span { class: "count", title: "{count_title}", "{count}" }
                }
            }
        },
    }
}

/// The site header. Its one nav is a row of links on wide screens and a tab
/// bar fixed to the bottom on phones (`board.css`); there is never a second
/// copy. See `docs/plan.md`, "Board layout".
pub(crate) fn header(chrome: &Chrome) -> Element {
    rsx! {
        header { class: if chrome.viewer.is_some() { "site-header member" } else { "site-header" },
            div { class: "wrap",
                a { class: "site-name", href: "/", "{chrome.site_name}" }
                if let Some(viewer) = &chrome.viewer {
                    form { class: "sign-out", method: "post", action: "/logout",
                        input { r#type: "hidden", name: CSRF_FIELD, value: "{viewer.csrf}" }
                        button { class: "link-button", r#type: "submit", "Sign out" }
                    }
                }
                nav { class: "site-nav", "aria-label": "Main",
                    {nav_link(chrome, Nav::Board, "/", "Board", Some((Icon::Board, "Board")), (0, ""))}
                    if let Some(viewer) = &chrome.viewer {
                        {nav_link(chrome, Nav::Announcements, "/announcements", "Announcements", None, (0, ""))}
                        {nav_link(chrome, Nav::Post, "/new", "Post a prompt", Some((Icon::Post, "Post")), (0, ""))}
                        {nav_link(chrome, Nav::Mine, "/mine", "My prompts", Some((Icon::Mine, "Mine")), (0, ""))}
                        {nav_link(chrome, Nav::Personas, "/personas", "Personas", None, (0, ""))}
                        {nav_link(chrome, Nav::Settings, "/settings", "Settings", None, (0, ""))}
                        {nav_link(chrome, Nav::Notices, "/notices", "Notices",
                            Some((Icon::Notices, "Notices")), (viewer.unread_notices, "Unread notices"))}
                        if viewer.is_admin {
                            {nav_link(chrome, Nav::Admin, "/admin/reports", "Reports", None, (0, ""))}
                            {nav_link(chrome, Nav::ServerSettings, "/admin/settings", "Admin", None,
                                (viewer.pending_tags, "Tag requests waiting"))}
                        }
                        a { class: "tab more-link", href: "/more",
                            "aria-current": (chrome.current == Nav::More).then_some("page"),
                            {icon(Icon::More)}
                            span { class: "short", "More" }
                            if viewer.is_admin && viewer.pending_tags > 0 {
                                span { class: "count", "{viewer.pending_tags}" }
                            }
                        }
                    } else {
                        {nav_link(chrome, Nav::Announcements, "/announcements", "Announcements",
                            Some((Icon::News, "News")), (0, ""))}
                        {nav_link(chrome, Nav::SignIn, "/login", "Sign in", Some((Icon::SignIn, "Sign in")), (0, ""))}
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
                if chrome.has_rules {
                    p { a { href: crate::RULES_PATH, "Server rules" } }
                }
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
                            a { class: "{chip_classes(tag)}", href: "{tag_url(tag)}",
                                title: "Show prompts tagged {tag.namespace}: {tag.display}",
                                span { class: "tag-ns", "{tag.namespace}: " }
                                "{tag.display}"
                            }
                        },
                        // Only its author and administrators get these. Not
                        // links: filtering by them works for nobody else.
                        TagStatus::Pending => rsx! {
                            span { class: "{chip_classes(tag)}",
                                span { class: "tag-ns", "{tag.namespace}: " }
                                "{tag.display} "
                                em { "(awaiting approval)" }
                            }
                        },
                        TagStatus::Hidden => rsx! {
                            span { class: "{chip_classes(tag)}",
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
