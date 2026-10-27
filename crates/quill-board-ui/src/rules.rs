//! The server rules: the public page, accepting them, and the
//! administrators' editor in Server settings.

use dioxus::prelude::*;
use quill_proto::{
    limits::RULES_BODY_MAX_CHARS,
    rules::{Rules, RulesAcceptance},
};

use crate::{
    Chrome,
    admin::{Tab, tabs},
    layout::{csrf, error_notice},
    render, short_date,
};

pub const RULES_PATH: &str = "/rules";
pub const ACCEPT_RULES_PATH: &str = "/rules/accept";
pub const ADMIN_RULES_PATH: &str = "/admin/settings/rules";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesView {
    /// `None` until an administrator writes them.
    pub rules: Option<Rules>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminRulesView {
    /// What's published now, if anything.
    pub current: Option<Rules>,
    /// The editor's text: the current rules, or what was sent back.
    pub body: String,
    pub minor: bool,
    /// The editor's text rendered, after "Preview".
    pub preview_html: Option<String>,
    pub acceptance: RulesAcceptance,
    pub error: Option<String>,
}

/// The banner every page shows while the visitor must accept a change.
pub(crate) fn banner() -> Element {
    rsx! {
        aside { class: "announcement-banner sticky rules-banner", "aria-label": "Server rules",
            span { class: "banner-label", "Rules" }
            span {
                "The server rules have changed. "
                a { href: RULES_PATH, "Read and accept them" }
                " to keep posting, commenting and reporting."
            }
        }
    }
}

pub fn rules_page(chrome: &Chrome, view: RulesView) -> String {
    let chrome_for_body = chrome.clone();
    render("Server rules", chrome, move || {
        let v = &view;
        let c = &chrome_for_body;
        rsx! {
            h1 { "Server rules" }
            {error_notice(v.error.as_ref())}
            match &v.rules {
                None => rsx! {
                    p { class: "lede", "This server's administrators haven't written any rules yet." }
                },
                Some(r) => rsx! {
                    article { class: "card prompt-card",
                        p { class: "card-meta",
                            span { "Version {r.version}" }
                            span { "Updated {short_date(r.updated_at)}" }
                        }
                        div { class: "ruled", dangerous_inner_html: "{r.body_html}" }
                    }
                    if r.must_accept {
                        form { class: "form-card", method: "post", action: ACCEPT_RULES_PATH,
                            {csrf(c)}
                            input { r#type: "hidden", name: "version", value: "{r.version}" }
                            p { "These rules changed since you joined. Accept them to keep posting, commenting and reporting." }
                            div { class: "actions",
                                button { class: "btn", r#type: "submit", "I accept these rules" }
                            }
                        }
                    }
                },
            }
        }
    })
}

fn acceptance_label(a: RulesAcceptance) -> &'static str {
    match a {
        RulesAcceptance::None => "Never",
        RulesAcceptance::OnJoin => "When creating an account",
        RulesAcceptance::OnChange => "After each change",
        RulesAcceptance::Both => "Both",
    }
}

fn acceptance_hint(a: RulesAcceptance) -> &'static str {
    match a {
        RulesAcceptance::None => "The rules are there to read; nobody is asked to accept them.",
        RulesAcceptance::OnJoin => "The sign-up form asks people to accept the rules.",
        RulesAcceptance::OnChange => {
            "After a change that isn't a minor edit, accounts accept the rules before posting, commenting or reporting again. Turning this on asks accounts that haven't accepted the current rules to do so."
        }
        RulesAcceptance::Both => "On sign-up, and again after each change.",
    }
}

pub fn admin_rules_page(chrome: &Chrome, view: AdminRulesView) -> String {
    let chrome_for_body = chrome.clone();
    render("Rules", chrome, move || {
        let v = &view;
        let c = &chrome_for_body;
        rsx! {
            h1 { "Server settings" }
            {tabs(c, Tab::Rules)}
            p { class: "lede",
                "Everyone can read the rules at "
                a { href: RULES_PATH, "{RULES_PATH}" }
                ", linked at the bottom of every page and from the sign-up form. Each save is a new version; the admin log records who saved it."
            }
            {error_notice(v.error.as_ref())}
            form { class: "form-card", method: "post", action: ADMIN_RULES_PATH,
                {csrf(c)}
                h2 { class: "form-title",
                    match &v.current {
                        Some(r) => rsx! { "Edit the rules (now version {r.version})" },
                        None => rsx! { "Write the rules" },
                    }
                }
                div { class: "field",
                    label { r#for: "body", "Rules"
                        span { class: "hint", "Markdown works: headings, *italic*, **bold**, lists, quotes and links." }
                    }
                    textarea { id: "body", name: "body", required: true, class: "tall",
                        maxlength: "{RULES_BODY_MAX_CHARS}", "{v.body}" }
                }
                if v.current.is_some() {
                    div { class: "field",
                        label { class: "inline-check",
                            input { r#type: "checkbox", name: "minor", value: "on", checked: v.minor }
                            span {
                                "Minor edit"
                                span { class: "hint", "A typo or wording fix: accounts aren't asked to accept the rules again." }
                            }
                        }
                    }
                }
                div { class: "actions",
                    button { class: "btn", r#type: "submit", name: "action", value: "save", "Save rules" }
                    button { class: "btn btn-quiet", r#type: "submit", name: "action", value: "preview", "Preview" }
                }
            }
            if let Some(html) = &v.preview_html {
                section { class: "preview", "aria-label": "Preview",
                    h2 { "Preview" }
                    article { class: "card prompt-card",
                        div { class: "ruled", dangerous_inner_html: "{html}" }
                    }
                }
            }
            form { class: "form-card", method: "post", action: "/admin/settings/rules-acceptance",
                {csrf(c)}
                fieldset { class: "choices",
                    legend { class: "form-title", "When accounts accept the rules" }
                    for a in RulesAcceptance::ALL.iter() {
                        label { key: "{a}", class: "inline-check",
                            input { r#type: "radio", name: "acceptance", value: "{a}", checked: v.acceptance == *a }
                            span {
                                "{acceptance_label(*a)}"
                                span { class: "hint", "{acceptance_hint(*a)}" }
                            }
                        }
                    }
                }
                div { class: "actions",
                    button { class: "btn", r#type: "submit", "Save setting" }
                }
            }
        }
    })
}
