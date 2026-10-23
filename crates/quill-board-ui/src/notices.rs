//! The account's notices page: administrators' actions against its prompts
//! and tags, in plain words.

use dioxus::prelude::*;
use quill_proto::notices::{AccountNotice, NoticeKind};

use crate::{Chrome, NoticesView, prompt_url, reason_label, render, short_date};

/// "genre: Horror", from the stored key and name.
fn tag_label(n: &AccountNotice) -> String {
    let key = n.fields.tag.as_deref().unwrap_or("a tag");
    match (key.split_once(':'), n.fields.tag_name.as_deref()) {
        (Some((ns, _)), Some(name)) => format!("{ns}: {name}"),
        _ => key.to_owned(),
    }
}

/// What happened, and what the list of prompts below it means.
fn sentence(n: &AccountNotice) -> (String, &'static str) {
    let tag = tag_label(n);
    match n.kind {
        NoticeKind::PromptRemoved => {
            let reason = n.fields.reason.map_or("a report", reason_label);
            (
                format!(
                    "An administrator removed your prompt from the board after a report ({reason})."
                ),
                "Only you can still see it, marked as removed:",
            )
        }
        NoticeKind::TagDeleted => (
            format!("An administrator deleted the tag {tag}."),
            "It was removed from your prompts:",
        ),
        NoticeKind::TagRejected => (
            format!("Your new tag {tag} wasn't approved."),
            "It was removed from your prompts:",
        ),
        NoticeKind::TagMerged => {
            let into = n.fields.into.as_deref().unwrap_or("another tag");
            (
                format!("An administrator merged the tag {tag} into {into}."),
                "These prompts carry the new tag now:",
            )
        }
        NoticeKind::TagHidden => (
            format!("An administrator hid the tag {tag}."),
            "It stays on these prompts, but only administrators see it:",
        ),
        NoticeKind::TagShown => (
            format!("The tag {tag} is visible again."),
            "Everyone can see it on these prompts:",
        ),
        NoticeKind::TagRenamed => {
            let from = n.fields.from.as_deref().unwrap_or("?");
            let to = n.fields.to.as_deref().unwrap_or("?");
            (
                format!("An administrator renamed a tag from {from} to {to}."),
                "It's on these prompts:",
            )
        }
        NoticeKind::TagApproved => (
            format!("Your new tag {tag} was approved."),
            "Everyone can see it now on these prompts:",
        ),
    }
}

pub fn notices_page(chrome: &Chrome, view: NoticesView) -> String {
    render("Notices", chrome, move || {
        let v = &view;
        rsx! {
            h1 { "Notices" }
            p { class: "lede",
                "What this server's administrators have done to your prompts and tags. Only you can see these. They never say who reported a prompt or which administrator acted."
            }
            if v.notices.is_empty() {
                div { class: "empty", "Nothing yet." }
            }
            for n in v.notices.iter() {
                {notice(n)}
            }
            if let Some(older) = &v.older {
                p { class: "pager", a { class: "btn btn-quiet", href: "{older}", "Older notices" } }
            }
        }
    })
}

fn notice(n: &AccountNotice) -> Element {
    let (what, list_intro) = sentence(n);
    rsx! {
        div { key: "{n.id}", class: if n.read { "report resolved" } else { "report" },
            p { class: "card-meta",
                span { "{short_date(n.at)}" }
                if !n.read { span { class: "new-notice", "New" } }
            }
            p { "{what}" }
            if !n.prompts.is_empty() {
                p { class: "hint", "{list_intro}" }
                ul { class: "notice-prompts",
                    for p in n.prompts.iter() {
                        li { key: "{p.id}",
                            a { href: "{prompt_url(p.id)}", "{p.title}" }
                            span { class: "hint", " as @{p.handle}" }
                        }
                    }
                }
            }
        }
    }
}
