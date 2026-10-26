//! Announcements: the board's banner, the public pages with their comments,
//! and the administrators' tab for posting and editing them.

use dioxus::prelude::*;
use quill_proto::{
    announcements::{Announcement, Comment, CommentVisibility},
    api::Persona,
    board::ReportReason,
    limits::{
        ANNOUNCEMENT_BODY_MAX_CHARS, ANNOUNCEMENT_TITLE_MAX_CHARS, COMMENT_BODY_MAX_CHARS,
        REPORT_DETAILS_MAX_CHARS,
    },
};
use uuid::Uuid;

use crate::{
    Chrome,
    admin::{Tab, tabs},
    layout::{csrf, error_notice},
    reason_label, render, short_date,
};

/// Where readers dismiss the banner (a form post; no JavaScript).
pub const DISMISS_PATH: &str = "/announcements/dismiss";

pub fn announcement_url(id: Uuid) -> String {
    format!("/announcements/{id}")
}

/// A comment's own address, which leads to it under its announcement.
pub fn comment_url(id: Uuid) -> String {
    format!("/comments/{id}")
}

pub fn admin_announcement_url(id: Uuid) -> String {
    format!("/admin/settings/announcements/{id}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnouncementsView {
    pub announcements: Vec<Announcement>,
    pub next_cursor: Option<String>,
}

/// One announcement and a page of its comments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnouncementView {
    pub announcement: Announcement,
    pub comments: Vec<Comment>,
    /// Link to the next page of comments.
    pub more_comments: Option<String>,
    /// The visitor's personas: to comment as, and to know their comments.
    pub personas: Vec<Persona>,
    /// The comment form, kept when it's sent back with an error.
    pub draft: String,
    pub persona: Option<Uuid>,
    pub error: Option<String>,
    /// The server shows comments to signed-in writers only, and the
    /// visitor isn't signed in: `comments` is empty.
    pub comments_hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentReportView {
    pub comment: Comment,
    pub reason: Option<ReportReason>,
    pub details: String,
    pub error: Option<String>,
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
    /// The server-wide setting: who can read comments.
    pub comment_visibility: CommentVisibility,
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

pub fn announcement_page(chrome: &Chrome, view: AnnouncementView) -> String {
    let title = view.announcement.title.clone();
    let is_admin = chrome.viewer.as_ref().is_some_and(|v| v.is_admin);
    let chrome_for_body = chrome.clone();
    render(title, chrome, move || {
        let v = &view;
        let a = &v.announcement;
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
            {comments_section(&chrome_for_body, v)}
        }
    })
}

// ---- Comments ----------------------------------------------------------------

fn author_label(c: &Comment) -> String {
    c.author.display_name.clone().map_or_else(
        || format!("@{}", c.author.handle),
        |name| format!("{name} (@{})", c.author.handle),
    )
}

fn comments_section(chrome: &Chrome, v: &AnnouncementView) -> Element {
    let a = &v.announcement;
    let viewer = chrome.viewer.as_ref();
    let is_admin = viewer.is_some_and(|v| v.is_admin);
    let mine = |c: &Comment| v.personas.iter().any(|p| p.id == c.author.id);
    if v.comments_hidden {
        return rsx! {
            section { class: "comments", "aria-labelledby": "comments-title",
                h2 { id: "comments-title", class: "form-title", "Comments" }
                p {
                    "On this server, comments are for signed-in writers. "
                    a { href: "/login", "Sign in" }
                    " to read them."
                }
            }
        };
    }
    rsx! {
        section { class: "comments", "aria-labelledby": "comments-title",
            h2 { id: "comments-title", class: "form-title", "Comments" }
            if v.comments.is_empty() {
                p { class: "hint", "No comments yet." }
            }
            ol { class: "comment-list",
                for c in v.comments.iter() {
                    li { key: "{c.id}", id: "c-{c.id}", class: "comment",
                        p { class: "card-meta",
                            span { "{author_label(c)}" }
                            span { "{short_date(c.created_at)}" }
                            if c.removed {
                                span { class: "removed", "Removed by a server administrator" }
                            }
                        }
                        div { class: "comment-body", dangerous_inner_html: "{c.body_html}" }
                        div { class: "comment-actions",
                            if mine(c) {
                                details {
                                    summary { "Delete" }
                                    form { method: "post", action: "{comment_url(c.id)}/delete",
                                        {csrf(chrome)}
                                        label { class: "inline-check",
                                            input { r#type: "checkbox", name: "confirm", value: "yes", required: true }
                                            "Delete this comment for good."
                                        }
                                        button { class: "btn btn-danger", r#type: "submit", "Delete comment" }
                                    }
                                }
                            } else if !c.removed {
                                a { href: "{comment_url(c.id)}/report", "Report" }
                            }
                            if is_admin && !c.removed {
                                {remove_form(chrome, c)}
                            }
                        }
                    }
                }
            }
            if let Some(more) = &v.more_comments {
                p { class: "pager", a { class: "btn btn-quiet", href: "{more}", "More comments" } }
            }
            if !a.comments_open {
                p { class: "notice", "Comments are closed." }
            } else if viewer.is_none() {
                p { a { href: "/login", "Sign in" } " to comment." }
            } else if v.personas.is_empty() {
                p { "Comments are posted under a persona. " a { href: "/personas", "Create one" } " to comment." }
            } else {
                {error_notice(v.error.as_ref())}
                form { class: "form-card", method: "post", action: "{announcement_url(a.id)}/comments",
                    {csrf(chrome)}
                    div { class: "field",
                        label { r#for: "persona", "Comment as" }
                        select { id: "persona", name: "persona", required: true,
                            for p in v.personas.iter() {
                                option { key: "{p.id}", value: "{p.id}", selected: v.persona == Some(p.id),
                                    "@{p.handle}"
                                }
                            }
                        }
                    }
                    div { class: "field",
                        label { r#for: "comment", "Comment"
                            span { class: "hint", "Markdown works. Comments can't be edited after posting, only deleted." }
                        }
                        textarea { class: "short", id: "comment", name: "body", required: true,
                            maxlength: "{COMMENT_BODY_MAX_CHARS}", "{v.draft}" }
                    }
                    div { class: "actions",
                        button { class: "btn", r#type: "submit", "Post comment" }
                    }
                }
            }
        }
    }
}

/// An administrator's direct removal, with the reason the author is told.
fn remove_form(chrome: &Chrome, c: &Comment) -> Element {
    rsx! {
        details {
            summary { "Remove" }
            form { method: "post", action: "/admin/comments/{c.id}/remove",
                {csrf(chrome)}
                label { r#for: "reason-{c.id}", "Reason" }
                select { id: "reason-{c.id}", name: "reason", required: true,
                    for r in ReportReason::ALL.iter() {
                        option { key: "{r}", value: "{r}", "{reason_label(*r)}" }
                    }
                }
                button { class: "btn btn-danger", r#type: "submit", "Remove comment" }
            }
        }
    }
}

pub fn comment_report_page(chrome: &Chrome, view: CommentReportView) -> String {
    let chrome_for_body = chrome.clone();
    render("Report a comment", chrome, move || {
        let v = &view;
        let c = &v.comment;
        let back = format!("{}#c-{}", announcement_url(c.announcement), c.id);
        rsx! {
            h1 { "Report a comment" }
            p { class: "lede",
                "Reports go to this server's administrators. The author isn't told who reported their comment."
            }
            div { class: "card",
                p { class: "card-meta",
                    span { "{author_label(c)}" }
                    span { "{short_date(c.created_at)}" }
                }
                div { class: "comment-body", dangerous_inner_html: "{c.body_html}" }
            }
            {error_notice(v.error.as_ref())}
            form { class: "form-card", method: "post", action: "{comment_url(c.id)}/report",
                {csrf(&chrome_for_body)}
                fieldset { class: "choices",
                    legend { class: "label", "What's wrong with it?" }
                    for r in ReportReason::ALL.iter() {
                        label { key: "{r}", class: "inline-check",
                            input { r#type: "radio", name: "reason", value: "{r}", required: true,
                                checked: v.reason == Some(*r) }
                            "{reason_label(*r)}"
                        }
                    }
                }
                div { class: "field",
                    label { r#for: "details", "Details"
                        span { class: "hint", "Optional. Up to {REPORT_DETAILS_MAX_CHARS} characters." }
                    }
                    textarea { class: "short", id: "details", name: "details",
                        maxlength: "{REPORT_DETAILS_MAX_CHARS}", "{v.details}" }
                }
                div { class: "actions",
                    button { class: "btn", r#type: "submit", "Send report" }
                    a { href: "{back}", "Cancel" }
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
            form { class: "form-card", method: "post", action: "/admin/settings/comment-visibility",
                {csrf(c)}
                fieldset { class: "choices",
                    legend { class: "form-title", "Who can read comments" }
                    label { class: "inline-check",
                        input { r#type: "radio", name: "visibility", value: "public",
                            checked: v.comment_visibility == CommentVisibility::Public }
                        span {
                            "Everyone"
                            span { class: "hint", "Anyone can read comments, signed in or not. Only signed-in writers can post them." }
                        }
                    }
                    label { class: "inline-check",
                        input { r#type: "radio", name: "visibility", value: "signed-in",
                            checked: v.comment_visibility == CommentVisibility::SignedIn }
                        span {
                            "Signed-in writers only"
                            span { class: "hint", "Visitors who aren't signed in see the announcements, but not their comments. Applies to every announcement." }
                        }
                    }
                }
                div { class: "actions",
                    button { class: "btn", r#type: "submit", "Save" }
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
