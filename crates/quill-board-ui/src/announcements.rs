//! Announcements: the board's banner, the public pages, and the
//! administrators' tab for posting and editing them.

use dioxus::prelude::*;
use quill_proto::{
    announcements::Announcement,
    limits::{ANNOUNCEMENT_BODY_MAX_CHARS, ANNOUNCEMENT_TITLE_MAX_CHARS},
};
use uuid::Uuid;

use crate::{
    Chrome,
    admin::{Tab, tabs},
    layout::{csrf, error_notice},
    render, short_date,
};

/// Where readers dismiss the banner (a form post; no JavaScript).
pub const DISMISS_PATH: &str = "/announcements/dismiss";

pub fn announcement_url(id: Uuid) -> String {
    format!("/announcements/{id}")
}

pub fn admin_announcement_url(id: Uuid) -> String {
    format!("/admin/settings/announcements/{id}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnouncementsView {
    pub announcements: Vec<Announcement>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminAnnouncementsView {
    pub announcements: Vec<Announcement>,
    pub next_cursor: Option<String>,
    /// The "post" form, kept when it's sent back with an error.
    pub title: String,
    pub body: String,
    pub comments_open: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminAnnouncementView {
    pub announcement: Announcement,
    /// The form's values, kept on error.
    pub title: String,
    pub body: String,
    pub error: Option<String>,
}

/// The one-line banner at the top of the board: the newest title, a link
/// to it, and a button that hides it until there's a newer one.
pub(crate) fn banner(a: &Announcement) -> Element {
    rsx! {
        aside { class: "announcement-banner", "aria-label": "Announcement",
            span { class: "banner-label", "Announcement" }
            a { href: "{announcement_url(a.id)}", "{a.title}" }
            form { method: "post", action: DISMISS_PATH,
                input { r#type: "hidden", name: "id", value: "{a.id}" }
                button { class: "link-button", r#type: "submit",
                    "aria-label": "Dismiss this announcement", "Dismiss" }
            }
        }
    }
}

fn meta(a: &Announcement) -> Element {
    rsx! {
        p { class: "card-meta",
            span { "From the administrators" }
            span { "{short_date(a.created_at)}" }
            if a.edited() {
                span { "Edited {short_date(a.updated_at)}" }
            }
        }
    }
}

fn comments_label(open: bool) -> &'static str {
    if open { "Open" } else { "Closed" }
}

// ---- Public pages ------------------------------------------------------------

pub fn announcements_page(chrome: &Chrome, view: AnnouncementsView) -> String {
    render("Announcements", chrome, move || {
        let v = &view;
        rsx! {
            h1 { "Announcements" }
            p { class: "lede", "News from this server's administrators, newest first." }
            if v.announcements.is_empty() {
                div { class: "empty", "No announcements yet." }
            }
            ul { class: "cards",
                for a in v.announcements.iter() {
                    li { class: "card", key: "{a.id}",
                        h2 { class: "card-title", a { href: "{announcement_url(a.id)}", "{a.title}" } }
                        {meta(a)}
                        div { class: "ruled excerpt", dangerous_inner_html: "{a.body_html}" }
                    }
                }
            }
            if let Some(cursor) = &v.next_cursor {
                p { class: "pager",
                    a { class: "btn btn-quiet", href: "/announcements?cursor={cursor}", "Older announcements" }
                }
            }
        }
    })
}

pub fn announcement_page(chrome: &Chrome, announcement: Announcement) -> String {
    let title = announcement.title.clone();
    let is_admin = chrome.viewer.as_ref().is_some_and(|v| v.is_admin);
    render(title, chrome, move || {
        let a = &announcement;
        rsx! {
            article { class: "card prompt-card",
                h1 { class: "card-title", "{a.title}" }
                {meta(a)}
                div { class: "ruled", dangerous_inner_html: "{a.body_html}" }
            }
            div { class: "prompt-actions",
                a { href: "/announcements", "All announcements" }
                if is_admin {
                    a { href: "{admin_announcement_url(a.id)}", "Edit" }
                }
            }
        }
    })
}

// ---- Administration ----------------------------------------------------------

fn title_and_body(title: &str, body: &str) -> Element {
    rsx! {
        div { class: "field",
            label { r#for: "title", "Title"
                span { class: "hint", "Shown on the board's banner." }
            }
            input { r#type: "text", id: "title", name: "title", value: "{title}", required: true,
                maxlength: "{ANNOUNCEMENT_TITLE_MAX_CHARS}" }
        }
        div { class: "field",
            label { r#for: "body", "Announcement"
                span { class: "hint", "Markdown works: *italic*, **bold**, lists, quotes and links." }
            }
            textarea { id: "body", name: "body", required: true,
                maxlength: "{ANNOUNCEMENT_BODY_MAX_CHARS}", "{body}" }
        }
    }
}

/// A button that opens or closes an announcement's comments.
fn comments_toggle(chrome: &Chrome, a: &Announcement) -> Element {
    let (value, label) = if a.comments_open {
        ("off", "Close comments")
    } else {
        ("on", "Open comments")
    };
    rsx! {
        form { method: "post", action: "{admin_announcement_url(a.id)}/comments",
            {csrf(chrome)}
            input { r#type: "hidden", name: "comments", value: "{value}" }
            button { class: "link-button", r#type: "submit",
                "aria-label": "{label} on “{a.title}”", "{label}" }
        }
    }
}

pub fn admin_announcements_page(chrome: &Chrome, view: AdminAnnouncementsView) -> String {
    let chrome_for_body = chrome.clone();
    render("Announcements", chrome, move || {
        let v = &view;
        let c = &chrome_for_body;
        rsx! {
            h1 { "Server settings" }
            {tabs(c, Tab::Announcements)}
            p { class: "lede",
                "The newest announcement's title shows as a banner at the top of the board, and every announcement is listed under Announcements. Readers see them as from “the administrators”; the admin log records who posted each."
            }
            {error_notice(v.error.as_ref())}
            form { class: "form-card", method: "post", action: "/admin/settings/announcements",
                {csrf(c)}
                h2 { class: "form-title", "Post an announcement" }
                {title_and_body(&v.title, &v.body)}
                div { class: "field",
                    label { class: "inline-check",
                        input { r#type: "checkbox", name: "comments", value: "on", checked: v.comments_open }
                        span {
                            "Allow comments"
                            span { class: "hint", "Comments aren't available yet; this decides whether they'll be open when they are. You can change it later." }
                        }
                    }
                }
                div { class: "actions",
                    button { class: "btn", r#type: "submit", "Post announcement" }
                }
            }
            if v.announcements.is_empty() {
                div { class: "empty", "No announcements yet." }
            } else {
                div { class: "table-scroll",
                    table { class: "log",
                        thead {
                            tr { th { "Title" } th { "Posted" } th { "Comments" } th { span { class: "visually-hidden", "Actions" } } }
                        }
                        tbody {
                            for a in v.announcements.iter() {
                                tr { key: "{a.id}",
                                    td { a { href: "{announcement_url(a.id)}", "{a.title}" } }
                                    td { "{short_date(a.created_at)}" }
                                    td { "{comments_label(a.comments_open)} " {comments_toggle(c, a)} }
                                    td { a { href: "{admin_announcement_url(a.id)}",
                                        "aria-label": "Edit “{a.title}”", "Edit" } }
                                }
                            }
                        }
                    }
                }
                if let Some(cursor) = &v.next_cursor {
                    p { class: "pager",
                        a { class: "btn btn-quiet", href: "/admin/settings/announcements?cursor={cursor}", "Older announcements" }
                    }
                }
            }
        }
    })
}

pub fn admin_announcement_page(chrome: &Chrome, view: AdminAnnouncementView) -> String {
    let chrome_for_body = chrome.clone();
    render("Edit announcement", chrome, move || {
        let v = &view;
        let a = &v.announcement;
        let c = &chrome_for_body;
        let url = admin_announcement_url(a.id);
        rsx! {
            h1 { "Edit announcement" }
            {error_notice(v.error.as_ref())}
            form { class: "form-card", method: "post", action: "{url}",
                {csrf(c)}
                {title_and_body(&v.title, &v.body)}
                div { class: "actions",
                    button { class: "btn", r#type: "submit", "Save changes" }
                    a { href: "/admin/settings/announcements", "Cancel" }
                }
            }
            section { class: "form-card",
                h2 { class: "form-title", "Comments" }
                p { "{comments_label(a.comments_open)}. " {comments_toggle(c, a)} }
            }
            details {
                summary { "Delete this announcement" }
                form { class: "form-card", method: "post", action: "{url}/delete",
                    {csrf(c)}
                    div { class: "field",
                        label { class: "inline-check",
                            input { r#type: "checkbox", name: "confirm", value: "yes", required: true }
                            "I want to delete this announcement. It will show nowhere, and this can't be undone."
                        }
                    }
                    div { class: "actions",
                        button { class: "btn btn-danger", r#type: "submit", "Delete announcement" }
                    }
                }
            }
        }
    })
}
