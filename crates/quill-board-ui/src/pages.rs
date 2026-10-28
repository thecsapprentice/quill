//! Full pages. Each function returns a complete HTML document.

use dioxus::prelude::*;
use quill_proto::{
    board::{
        AdminAction, AdminLogEntry, LogActor, Maturity, Prompt, PromptState, Report, ReportKind,
        ReportReason, Resolution, Tag, TagNamespace, TagPolicy, TagStatus,
    },
    limits::{
        PROMPT_BODY_MAX_CHARS, PROMPT_TITLE_MAX_CHARS, REPORT_DETAILS_MAX_CHARS,
        STAFF_NAME_MAX_CHARS, USERNAME_MAX_CHARS,
    },
};
use uuid::Uuid;

use crate::{
    AdminLogEntryView, AdminLogView, AdminReportsView, AuthMode, AuthView, BoardFilters, BoardView,
    ChipKind, Chrome, Layout, MessageView, MyPromptsView, PersonasView, PromptFormView, PromptView,
    ReportView, SettingsView,
    admin::Tab,
    admin_log_entry_url, admin_log_url,
    chips::chip_classes,
    filters_url,
    layout::{csrf, error_notice, prompt_meta, tag_datalist, tag_list},
    maturity_label, prompt_url, reason_label, render, render_admin, render_with, short_date,
    short_datetime, state_label, tag_url,
};

const TAG_OPTIONS: &str = "tag-options";

pub(crate) fn maturity_options(selected: Option<Maturity>) -> Element {
    rsx! {
        for m in Maturity::ALL.iter() {
            option { key: "{m}", value: "{m}", selected: selected == Some(*m), "{maturity_label(*m)}" }
        }
    }
}

pub(crate) fn state_options(selected: Option<PromptState>) -> Element {
    rsx! {
        for s in PromptState::ALL.iter() {
            option { key: "{s}", value: "{s}", selected: selected == Some(*s), "{state_label(*s)}" }
        }
    }
}

// ---- Board ---------------------------------------------------------------

pub fn board_page(chrome: &Chrome, view: BoardView) -> String {
    let signed_in = chrome.viewer.is_some();
    render_with(Layout::Wide, "", chrome, move || {
        let v = &view;
        let f = &v.filters;
        let cursor = v.cursor.as_deref();
        let open_id = v.open.as_ref().map(|p| p.id);
        rsx! {
            for a in v.sticky_announcements.iter() {
                {crate::announcements::banner(a, false)}
            }
            if let Some(a) = &v.announcement {
                {crate::announcements::banner(a, true)}
            }
            div { class: if v.open.is_some() { "board-layout has-open" } else { "board-layout" },
                section { class: "board-list", "aria-label": "Prompts",
                    div { class: "board-head",
                        h1 { "Prompts" }
                        p { class: "lede",
                            "Newest first, rated {maturity_label(v.cap)} and below. "
                            if signed_in {
                                a { href: "/settings", "Change this in Settings." }
                            } else {
                                a { href: "/login", "Sign in" }
                                " to change this."
                            }
                        }
                        {board_search(v)}
                        {filter_summary(f)}
                        p { class: "all-filters",
                            a { class: "btn btn-quiet", href: "{filters_url(f)}", "Filter by tag" }
                        }
                    }
                    {error_notice(v.error.as_ref())}
                    if v.prompts.is_empty() && v.error.is_none() {
                        div { class: "empty",
                            if f.is_active() {
                                "No prompts match these filters. "
                                a { href: "/", "Clear filters" }
                            } else if signed_in {
                                "No prompts yet. "
                                a { href: "/new", "Post the first one." }
                            } else {
                                "No prompts yet."
                            }
                        }
                    }
                    ul { class: "rows",
                        for p in v.prompts.iter() {
                            {prompt_row(p, f.open_url(cursor, p.id), open_id == Some(p.id))}
                        }
                    }
                    if let Some(next) = &v.next_cursor {
                        p { class: "pager",
                            a { class: "btn btn-quiet", href: "{f.url(Some(next))}", "Older prompts" }
                        }
                    }
                }
                match &v.open {
                    Some(p) => rsx! { {reading_pane(p, f.url(cursor))} },
                    None => rsx! {
                        aside { class: "reading-pane empty-pane",
                            p { "Choose a prompt to read it here." }
                        }
                    },
                }
            }
            {tag_datalist(TAG_OPTIONS, &v.tag_options)}
        }
    })
}

/// The search box, and the full search and filter form folded away.
fn board_search(v: &BoardView) -> Element {
    let f = &v.filters;
    let count = f.count();
    rsx! {
        details { class: "filters", open: v.error.is_some(),
            summary {
                "Search and filter"
                if count > 0 {
                    " "
                    span { class: "count", "{count}" }
                }
            }
            form { method: "get", action: "/",
                div { class: "field",
                    label { r#for: "q", "Search" }
                    input { r#type: "search", id: "q", name: "q", value: "{f.q}" }
                }
                div { class: "filter-grid",
                    div { class: "field",
                        label { r#for: "include", "Must have tags"
                            span { class: "hint", "Comma-separated, like genre:horror" }
                        }
                        input { r#type: "text", id: "include", name: "include", value: "{f.include}", list: TAG_OPTIONS }
                    }
                    div { class: "field",
                        label { r#for: "exclude", "Hide tags"
                            span { class: "hint", "Like content-warning:gore" }
                        }
                        input { r#type: "text", id: "exclude", name: "exclude", value: "{f.exclude}", list: TAG_OPTIONS }
                    }
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
                }
                div { class: "actions",
                    button { class: "btn", r#type: "submit", "Apply filters" }
                    if f.is_active() {
                        a { href: "/", "Clear filters" }
                    }
                }
            }
        }
    }
}

/// The filters in force, in a box that never grows: a few tags of each
/// kind and "+N" for the rest, then the rating, status and search.
fn filter_summary(f: &BoardFilters) -> Element {
    if !f.is_active() {
        return rsx! {};
    }
    const SHOWN: usize = 3;
    let split = |list: &str| -> Vec<String> {
        list.split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let (has, hides) = (split(&f.include), split(&f.exclude));
    let mut rest = Vec::new();
    if let Some(m) = f.maturity {
        rest.push(format!("{} or under", maturity_label(m)));
    }
    if let Some(s) = f.state {
        rest.push(state_label(s).to_owned());
    }
    if !f.q.is_empty() {
        rest.push(format!("“{}”", f.q));
    }
    rsx! {
        div { class: "filter-summary", role: "status",
            p { class: "summary-tags",
                if !has.is_empty() {
                    b { "Has " }
                    for t in has.iter().take(SHOWN) {
                        span { key: "h-{t}", class: "chip", "{t}" }
                    }
                    if has.len() > SHOWN {
                        span { class: "chip", "+{has.len() - SHOWN}" }
                    }
                }
                if !hides.is_empty() {
                    b { " Hides " }
                    for t in hides.iter().take(SHOWN) {
                        span { key: "x-{t}", class: "chip chip-hide", "{t}" }
                    }
                    if hides.len() > SHOWN {
                        span { class: "chip chip-hide", "+{hides.len() - SHOWN}" }
                    }
                }
            }
            p { class: "summary-rest",
                span { "{rest.join(\" · \")}" }
                a { href: "/", "Clear" }
            }
        }
    }
}

/// One prompt in the board's list: its title opens it in the reading pane.
fn prompt_row(p: &Prompt, open_url: String, current: bool) -> Element {
    rsx! {
        li { key: "{p.id}", class: if current { "row current" } else { "row" },
            a { class: "row-title", href: "{open_url}",
                "aria-current": current.then_some("true"),
                "{p.title}"
            }
            {prompt_meta(p)}
            p { class: "row-excerpt", "{excerpt(&p.body)}" }
            if !p.tags.is_empty() {
                p { class: "row-tags",
                    for (i, t) in p.tags.iter().enumerate() {
                        span { key: "{t.key()}",
                            if i > 0 { " · " }
                            a { href: "{tag_url(t)}", "{t.display}" }
                        }
                    }
                }
            }
        }
    }
}

/// The start of a prompt's Markdown as plain text, for one line in the list.
fn excerpt(markdown: &str) -> String {
    const CHARS: usize = 160;
    let plain: String = markdown
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !matches!(c, '*' | '_' | '#' | '>' | '`' | '[' | ']'))
        .collect();
    match plain.char_indices().nth(CHARS) {
        Some((at, _)) => format!("{}…", plain[..at].trim_end()),
        None => plain,
    }
}

/// The open prompt beside the list. Phones and tablets show it instead of
/// the list, with a link back.
fn reading_pane(p: &Prompt, back: String) -> Element {
    let url = prompt_url(p.id);
    rsx! {
        article { class: "reading-pane card prompt-card",
            a { class: "back-link", href: "{back}", "← Back to the list" }
            h2 { class: "card-title", "{p.title}" }
            {prompt_meta(p)}
            div { class: "ruled", dangerous_inner_html: "{p.body_html}" }
            {tag_list(&p.tags)}
            p { class: "pane-actions",
                a { class: "btn btn-quiet", href: "{url}", "Open full page" }
                a { href: "{url}/report", "Report this prompt" }
            }
        }
    }
}

// ---- One prompt ----------------------------------------------------------

pub fn prompt_page(chrome: &Chrome, view: PromptView) -> String {
    let title = view.prompt.title.clone();
    let signed_in = chrome.viewer.is_some();
    let chrome_for_body = chrome.clone();
    render(title, chrome, move || {
        let p = &view.prompt;
        let url = prompt_url(p.id);
        if let Some(cap) = view.held_back {
            return rsx! {
                div { class: "warning", role: "alert",
                    p {
                        "“{p.title}” is rated {maturity_label(p.maturity)}, above your setting ({maturity_label(cap)})."
                    }
                    div { class: "actions",
                        a { class: "btn", href: "{url}?show=1", "Show this prompt" }
                        a { href: "/", "Back to the board" }
                        if signed_in {
                            a { href: "/settings", "Change your setting" }
                        }
                    }
                }
            };
        }
        rsx! {
            article { class: "card prompt-card",
                h1 { class: "card-title", "{p.title}" }
                {prompt_meta(p)}
                div { class: "ruled", dangerous_inner_html: "{p.body_html}" }
                {tag_list(&p.tags)}
            }
            div { class: "prompt-actions",
                if view.is_author && !p.removed {
                    a { href: "{url}/edit", "Edit" }
                    form { method: "post", action: "{url}/state",
                        {csrf(&chrome_for_body)}
                        label { class: "inline-check", r#for: "state", "Status" }
                        select { id: "state", name: "state", {state_options(Some(p.state))} }
                        button { class: "btn btn-quiet", r#type: "submit", "Update status" }
                    }
                }
                if !view.is_author {
                    a { href: "{url}/report", "Report this prompt" }
                }
            }
            if p.removed {
                p { class: "notice error",
                    "A server administrator removed this prompt from the board. Only you can see it, and it can't be edited."
                }
            }
            p { class: "aside",
                "Writing sessions happen in a Quill app, which isn't released yet. To share this prompt, copy its address."
            }
            if view.is_author {
                details {
                    summary { "Delete this prompt" }
                    form { class: "form-card", method: "post", action: "{url}/delete",
                        {csrf(&chrome_for_body)}
                        div { class: "field",
                            label { class: "inline-check",
                                input { r#type: "checkbox", name: "confirm", value: "yes", required: true }
                                "I want to delete this prompt. This can't be undone."
                            }
                        }
                        div { class: "actions",
                            button { class: "btn btn-danger", r#type: "submit", "Delete prompt" }
                        }
                    }
                }
            }
        }
    })
}

// ---- Sign in and register ------------------------------------------------

pub fn auth_page(chrome: &Chrome, view: AuthView) -> String {
    let title = match view.mode {
        AuthMode::SignIn => "Sign in",
        AuthMode::Register => "Create an account",
    };
    let chrome_has_rules = chrome.has_rules;
    render(title, chrome, move || {
        let v = &view;
        let register = v.mode == AuthMode::Register;
        let action = if register { "/register" } else { "/login" };
        rsx! {
            h1 { "{title}" }
            if register {
                p { class: "lede",
                    "Choose a private login name. Nobody else ever sees it: what people see are your personas, which you create next."
                }
            }
            {error_notice(v.error.as_ref())}
            if register && !v.registration_open {
                p { class: "notice error", "This server isn't accepting new accounts right now." }
            } else {
                form { class: "form-card", method: "post", action: "{action}",
                    div { class: "field",
                        label { r#for: "username", "Login name"
                            if register {
                                span { class: "hint", "Letters, digits and underscores, up to {USERNAME_MAX_CHARS} characters." }
                            }
                        }
                        input { r#type: "text", id: "username", name: "username", value: "{v.username}",
                            autocomplete: "username", required: true, maxlength: "{USERNAME_MAX_CHARS}",
                            autocapitalize: "none", spellcheck: "false" }
                    }
                    div { class: "field",
                        label { r#for: "password", "Password"
                            if register {
                                span { class: "hint", "At least 12 characters." }
                            }
                        }
                        input { r#type: "password", id: "password", name: "password", required: true,
                            autocomplete: if register { "new-password" } else { "current-password" } }
                    }
                    if register {
                        div { class: "field",
                            label { r#for: "confirm", "Repeat password" }
                            input { r#type: "password", id: "confirm", name: "confirm", required: true, autocomplete: "new-password" }
                        }
                        if let Some(version) = v.accept_rules {
                            div { class: "field",
                                input { r#type: "hidden", name: "rules_version", value: "{version}" }
                                label { class: "inline-check",
                                    input { r#type: "checkbox", name: "accept_rules", value: "on", required: true }
                                    span {
                                        "I accept the "
                                        a { href: crate::RULES_PATH, "server rules" }
                                    }
                                }
                            }
                        } else if chrome_has_rules {
                            p { class: "hint",
                                "Before you start, read the "
                                a { href: crate::RULES_PATH, "server rules" }
                                "."
                            }
                        }
                    }
                    div { class: "actions",
                        button { class: "btn", r#type: "submit", if register { "Create account" } else { "Sign in" } }
                    }
                }
            }
            if register {
                p { class: "aside",
                    "There is no email address and no password reset: if you forget your password, the account can't be recovered. "
                    a { href: "/login", "Already have an account? Sign in" }
                }
            } else if v.registration_open {
                p { class: "aside", a { href: "/register", "New here? Create an account" } }
            }
        }
    })
}

// ---- Posting and editing -------------------------------------------------

/// "a, b and c"
fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn tag_policy_hint(policy: TagPolicy, namespaces: &[TagNamespace]) -> String {
    let curated: Vec<String> = namespaces
        .iter()
        .filter(|n| n.locked)
        .map(|n| n.name.clone())
        .collect();
    let except = if curated.is_empty() {
        String::new()
    } else {
        format!(
            ", except in {}, which use the server's list",
            join_and(&curated)
        )
    };
    match policy {
        TagPolicy::Curated => "Only tags listed on this server can be used.".to_owned(),
        TagPolicy::Open => format!("New tags are created as you use them{except}."),
        TagPolicy::Approval => format!(
            "You can use new tags{except}. Others see a new tag once an administrator approves it."
        ),
    }
}

/// A tag type's count on the form against its limit, as in "1 of 1 to 2 ✓"
/// or "3 of up to 3 · full", with a class for its state. `None` for a type
/// with no limits.
fn limit_counter(n: &TagNamespace, count: u32) -> Option<(String, &'static str)> {
    if n.min == 0 && n.max.is_none() {
        return None;
    }
    let label = n.limit_label();
    Some(if count < n.min {
        (format!("{count} of {label}"), "limit-count short")
    } else if n.full(count) {
        (format!("{count} of {label} · full"), "limit-count full")
    } else if n.allows(count) {
        (format!("{count} of {label} ✓"), "limit-count ok")
    } else {
        (
            format!("{count} of {label} · too many"),
            "limit-count short",
        )
    })
}

/// The tag picker: chosen tags as chips, slots for types below their
/// minimum, a field to add tags, and "Browse tags" with each type's count
/// against its limit. No JavaScript: a chip is a checkbox (unticked chips
/// aren't sent), and Add or a browsed tag reloads the form, so the counts
/// are as of the last reload; the server's check is what counts.
fn tag_picker(v: &PromptFormView, action: &str) -> Element {
    // Add and browse jump back to the picker after reloading.
    let back_to_tags = format!("{action}#tags");
    let count = |n: &TagNamespace| {
        let c = v.chips.iter().filter(|c| c.tag.namespace == n.name).count();
        u32::try_from(c).unwrap_or(u32::MAX)
    };
    let missing: Vec<(&TagNamespace, u32)> = v
        .namespaces
        .iter()
        .filter_map(|n| {
            let have = count(n);
            (have < n.min).then(|| (n, n.min - have))
        })
        .collect();
    let chosen: Vec<String> = v.chips.iter().map(|c| c.tag.key()).collect();
    let mut groups: Vec<(&TagNamespace, Vec<&Tag>)> = v
        .namespaces
        .iter()
        .map(|n| {
            let tags: Vec<&Tag> = v
                .tag_options
                .iter()
                .filter(|t| t.namespace == n.name && !chosen.contains(&t.key()))
                .collect();
            (n, tags)
        })
        .filter(|(_, tags)| !tags.is_empty())
        .collect();
    // Types still short of their minimum first, matching the empty slots.
    groups.sort_by_key(|(n, _)| count(n) >= n.min);
    rsx! {
        fieldset { id: "tags", class: "tag-picker",
            legend { class: "label", "Tags" }
            span { class: "hint", "{tag_policy_hint(v.tag_policy, &v.namespaces)}" }
            div { class: "chips",
                for c in v.chips.iter() {
                    label { key: "{c.value}", class: "{chip_classes(&c.tag)} chip-choice",
                        input { r#type: "checkbox", name: "tag", value: "{c.value}", checked: true }
                        span { class: "tag-ns", "{c.tag.namespace}: " }
                        "{c.tag.display}"
                        match c.kind {
                            ChipKind::New => rsx! { em { " (new)" } },
                            ChipKind::NewNeedsApproval => rsx! { em { " (new, needs approval)" } },
                            ChipKind::Existing if c.tag.status == TagStatus::Pending => rsx! { em { " (awaiting approval)" } },
                            ChipKind::Existing => rsx! {},
                        }
                        span { class: "chip-x", "aria-hidden": "true" }
                    }
                }
                for (n, need) in missing.iter() {
                    span { key: "slot-{n.name}", class: "chip chip-slot",
                        "{n.display}: pick {need}"
                    }
                }
                if v.chips.is_empty() && missing.is_empty() {
                    span { class: "hint", "No tags yet." }
                }
            }
            if !v.chips.is_empty() {
                span { class: "hint", "Untick a tag to remove it." }
            }
            if !v.tag_errors.is_empty() {
                ul { class: "notice error tag-errors", role: "alert",
                    for e in v.tag_errors.iter() {
                        li { key: "{e}", "{e}" }
                    }
                }
            }
            {frequent_row(v, &back_to_tags, &count)}
            div { class: "add-tags",
                label { r#for: "add-tags", "Add tags"
                    span { class: "hint", "Type as type:name, like genre:horror or theme:found family. Separate several with commas." }
                }
                div { class: "add-row",
                    input { r#type: "text", id: "add-tags", name: "add", value: "{v.add_text}", list: TAG_OPTIONS,
                        autocomplete: "off" }
                    // The first submit button in the form, so Enter in any
                    // field adds tags rather than posting.
                    button { class: "btn btn-quiet", r#type: "submit", name: "op", value: "add",
                        formaction: "{back_to_tags}", formnovalidate: true, "Add" }
                }
            }
            if !groups.is_empty() {
                details { class: "browse",
                    summary { "Browse tags" }
                    for (n, tags) in groups.iter() {
                        div { key: "{n.name}", class: "browse-group",
                            h3 {
                                "{n.display}"
                                if let Some((text, class)) = limit_counter(n, count(n)) {
                                    " "
                                    span { class: "{class}", "{text}" }
                                }
                            }
                            div { class: "chips",
                                for tag in tags.iter().take(BROWSE_SHOWN) {
                                    {browse_chip(n, tag, &back_to_tags, n.full(count(n)))}
                                }
                            }
                            // Types with many tags show the first few; the
                            // rest fold away, so the panel stays short.
                            if tags.len() > BROWSE_SHOWN {
                                details { class: "browse-more",
                                    summary { "All {tags.len()} {n.display} tags" }
                                    div { class: "chips",
                                        for tag in tags.iter().skip(BROWSE_SHOWN) {
                                            {browse_chip(n, tag, &back_to_tags, n.full(count(n)))}
                                        }
                                    }
                                }
                            }
                            if n.full(count(n)) {
                                p { class: "hint", "Remove a {n.display} tag to pick a different one." }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// "You often use": the writer's own most used tags, one click each, greyed
/// like the browser's when their type is full. Only the writer sees it.
fn frequent_row(
    v: &PromptFormView,
    back_to_tags: &str,
    count: &dyn Fn(&TagNamespace) -> u32,
) -> Element {
    if v.frequent.is_empty() {
        return rsx! {};
    }
    rsx! {
        div { class: "frequent",
            p { class: "label",
                "You often use"
                if let Some(handle) = &v.frequent_for {
                    span { class: "hint", " for @{handle}" }
                }
            }
            div { class: "chips",
                for tag in v.frequent.iter() {
                    match v.namespaces.iter().find(|n| n.name == tag.namespace) {
                        Some(n) => rsx! { {browse_chip(n, tag, back_to_tags, n.full(count(n)))} },
                        None => rsx! {},
                    }
                }
            }
        }
    }
}

/// How many of a type's tags "Browse tags" shows before "All N".
const BROWSE_SHOWN: usize = 12;

/// A tag to add from "Browse tags": a submit button that reloads the form.
fn browse_chip(n: &TagNamespace, tag: &Tag, back_to_tags: &str, full: bool) -> Element {
    rsx! {
        button { key: "{tag.key()}", class: "{chip_classes(tag)} chip-add",
            r#type: "submit", name: "pick", value: "{tag.key()}",
            formaction: "{back_to_tags}", formnovalidate: true,
            disabled: full,
            "aria-label": "Add {n.display}: {tag.display}",
            "+ {tag.display}"
        }
    }
}

pub fn prompt_form_page(chrome: &Chrome, view: PromptFormView) -> String {
    let title = if view.editing.is_some() {
        "Edit prompt"
    } else {
        "Post a prompt"
    };
    let chrome_for_body = chrome.clone();
    render_with(Layout::Wide, title, chrome, move || {
        let v = &view;
        let action = v.editing.map_or_else(
            || "/new".to_owned(),
            |id| format!("{}/edit", prompt_url(id)),
        );
        let cancel = v.editing.map_or_else(|| "/".to_owned(), prompt_url);
        if v.editing.is_none() && v.personas.is_empty() {
            return rsx! {
                h1 { "{title}" }
                div { class: "empty",
                    p { "Prompts are posted under a persona: a public name that's separate from your login." }
                    a { class: "btn", href: "/personas", "Create a persona" }
                }
            };
        }
        rsx! {
            h1 { "{title}" }
            {error_notice(v.error.as_ref())}
            form { class: "post-form", method: "post", action: "{action}",
                // The form's first submit button, so Enter in any field adds
                // tags rather than posting; Add sits in the tag panel.
                button { class: "visually-hidden", r#type: "submit", name: "op", value: "add",
                    formaction: "{action}#tags", formnovalidate: true, tabindex: "-1",
                    "aria-hidden": "true", "Add tags" }
              div { class: "form-card post-main",
                {csrf(&chrome_for_body)}
                if v.editing.is_none() {
                    div { class: "field",
                        label { r#for: "persona", "Post as"
                            span { class: "hint", "The public name this prompt appears under." }
                        }
                        select { id: "persona", name: "persona", required: true,
                            for p in v.personas.iter() {
                                option { key: "{p.id}", value: "{p.id}", selected: v.persona == Some(p.id),
                                    "@{p.handle}"
                                }
                            }
                        }
                    }
                }
                div { class: "field",
                    label { r#for: "title", "Title" }
                    input { r#type: "text", id: "title", name: "title", value: "{v.title}", required: true,
                        maxlength: "{PROMPT_TITLE_MAX_CHARS}" }
                }
                div { class: "field",
                    label { r#for: "body", "Prompt"
                        span { class: "hint", "Markdown works: *italic*, **bold**, lists, quotes and links. Images appear as links." }
                    }
                    textarea { id: "body", name: "body", required: true, maxlength: "{PROMPT_BODY_MAX_CHARS}", "{v.body}" }
                }
                div { class: "filter-grid",
                    div { class: "field",
                        label { r#for: "maturity", "Maturity rating" }
                        select { id: "maturity", name: "maturity", {maturity_options(Some(v.maturity))} }
                    }
                    div { class: "field",
                        label { r#for: "state", "Status"
                            span { class: "hint", "Whether you're taking requests to write it together." }
                        }
                        select { id: "state", name: "state", {state_options(Some(v.state))} }
                    }
                }
                div { class: "actions",
                    button { class: "btn", r#type: "submit", name: "op", value: "save",
                        if v.editing.is_some() { "Save changes" } else { "Post prompt" }
                    }
                    button { class: "btn btn-quiet", r#type: "submit", name: "op", value: "preview",
                        formaction: "{action}#preview", formnovalidate: true, "Preview" }
                    a { href: "{cancel}", "Cancel" }
                }
              }
              div { class: "post-side",
                if let Some(html) = &v.preview_html {
                    section { id: "preview", class: "card prompt-card preview-card", "aria-label": "Preview",
                        p { class: "banner-label", "Preview" }
                        h2 { class: "card-title", "{v.title}" }
                        div { class: "ruled", dangerous_inner_html: "{html}" }
                    }
                }
                div { class: "form-card",
                    {tag_picker(v, &action)}
                }
              }
            }
            {tag_datalist(TAG_OPTIONS, &v.tag_options)}
        }
    })
}

// ---- Reporting -----------------------------------------------------------

pub fn report_page(chrome: &Chrome, view: ReportView) -> String {
    let chrome_for_body = chrome.clone();
    render("Report a prompt", chrome, move || {
        let v = &view;
        let p = &v.prompt;
        let url = prompt_url(p.id);
        rsx! {
            h1 { "Report a prompt" }
            p { class: "lede",
                "Reports go to this server's administrators. The author isn't told who reported their prompt."
            }
            div { class: "card",
                h2 { class: "card-title", a { href: "{url}", "{p.title}" } }
                {prompt_meta(p)}
            }
            {error_notice(v.error.as_ref())}
            form { class: "form-card", method: "post", action: "{url}/report",
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
                    a { href: "{url}", "Cancel" }
                }
            }
        }
    })
}

// ---- The visitor's own pages ---------------------------------------------

/// The address of My prompts showing closed prompts or not, for one
/// persona or all.
pub fn mine_url(closed: bool, persona: Option<Uuid>) -> String {
    let mut params = Vec::new();
    if closed {
        params.push("closed=1".to_owned());
    }
    if let Some(p) = persona {
        params.push(format!("persona={p}"));
    }
    if params.is_empty() {
        "/mine".to_owned()
    } else {
        format!("/mine?{}", params.join("&"))
    }
}

pub fn my_prompts_page(chrome: &Chrome, view: MyPromptsView) -> String {
    render_with(Layout::Wide, "My prompts", chrome, move || {
        let v = &view;
        rsx! {
            div { class: "mine-head",
                h1 { "My prompts" }
                if v.personas.len() > 1 {
                    form { class: "persona-pick", method: "get", action: "/mine",
                        if v.closed {
                            input { r#type: "hidden", name: "closed", value: "1" }
                        }
                        label { r#for: "persona", class: "visually-hidden", "Persona" }
                        select { id: "persona", name: "persona",
                            option { value: "", selected: v.persona.is_none(),
                                "All my personas ({v.personas.len()})" }
                            for p in v.personas.iter() {
                                option { key: "{p.id}", value: "{p.id}", selected: v.persona == Some(p.id),
                                    "@{p.handle}" }
                            }
                        }
                        button { class: "btn btn-quiet", r#type: "submit", "Show" }
                    }
                }
            }
            p { class: "lede", "Every prompt your personas posted, at every maturity rating." }
            nav { class: "tabs", "aria-label": "Show",
                a { href: "{mine_url(false, v.persona)}", "aria-current": (!v.closed).then_some("page"),
                    "On the board" }
                a { href: "{mine_url(true, v.persona)}", "aria-current": v.closed.then_some("page"),
                    "Closed" }
            }
            if v.prompts.is_empty() {
                div { class: "empty",
                    if v.closed {
                        "No closed prompts."
                    } else {
                        "Nothing on the board. "
                        a { href: "/new", "Post a prompt." }
                    }
                }
            }
            ul { class: "rows mine-rows",
                for p in v.prompts.iter() {
                    li { key: "{p.id}", class: "row mine-row",
                        div {
                            a { class: "row-title", href: "{prompt_url(p.id)}", "{p.title}" }
                            {prompt_meta(p)}
                        }
                        // A column of actions, with room for more per prompt.
                        p { class: "row-actions",
                            if !p.removed {
                                a { href: "{prompt_url(p.id)}/edit", "Edit" }
                            }
                        }
                    }
                }
            }
        }
    })
}

pub fn personas_page(chrome: &Chrome, view: PersonasView) -> String {
    let chrome_for_body = chrome.clone();
    render("Personas", chrome, move || {
        let v = &view;
        rsx! {
            h1 { "Personas" }
            p { class: "lede",
                "Personas are the public names your prompts appear under. Nobody else can see which personas belong to the same account."
            }
            {error_notice(v.error.as_ref())}
            if v.personas.is_empty() {
                div { class: "empty", "You don't have a persona yet. Create one to start posting." }
            } else {
                ul { class: "rows",
                    for p in v.personas.iter() {
                        li { key: "{p.id}",
                            span {
                                strong { "@{p.handle}" }
                                if let Some(name) = &p.display_name {
                                    " {name}"
                                }
                            }
                            details {
                                summary { "Delete" }
                                form { method: "post", action: "/personas/{p.id}/delete",
                                    {csrf(&chrome_for_body)}
                                    label { class: "inline-check",
                                        input { r#type: "checkbox", name: "confirm", value: "yes", required: true }
                                        "Also deletes every prompt posted as @{p.handle}."
                                    }
                                    button { class: "btn btn-danger", r#type: "submit", "Delete @{p.handle}" }
                                }
                            }
                        }
                    }
                }
            }
            h2 { "New persona" }
            form { class: "form-card", method: "post", action: "/personas",
                {csrf(&chrome_for_body)}
                div { class: "filter-grid",
                    div { class: "field",
                        label { r#for: "handle", "Handle"
                            span { class: "hint", "Letters, digits and underscores. Shown as @handle." }
                        }
                        input { r#type: "text", id: "handle", name: "handle", value: "{v.handle}", required: true,
                            maxlength: "{USERNAME_MAX_CHARS}", autocapitalize: "none", spellcheck: "false" }
                    }
                    div { class: "field",
                        label { r#for: "display_name", "Display name"
                            span { class: "hint", "Optional." }
                        }
                        input { r#type: "text", id: "display_name", name: "display_name", value: "{v.display_name}" }
                    }
                }
                div { class: "actions",
                    button { class: "btn", r#type: "submit", "Create persona" }
                }
            }
        }
    })
}

pub fn settings_page(chrome: &Chrome, view: SettingsView) -> String {
    let chrome_for_body = chrome.clone();
    render("Settings", chrome, move || {
        let s = &view.settings;
        rsx! {
            h1 { "Settings" }
            {error_notice(view.error.as_ref())}
            form { class: "form-card", method: "post", action: "/settings",
                {csrf(&chrome_for_body)}
                div { class: "field",
                    label { r#for: "maturity_max", "Show prompts rated up to"
                        span { class: "hint", "Applies to the board and searches. Your own prompts always show." }
                    }
                    select { id: "maturity_max", name: "maturity_max", {maturity_options(Some(s.maturity_max))} }
                }
                div { class: "field",
                    label { class: "inline-check",
                        input { r#type: "checkbox", name: "suggest_all_personas", value: "on",
                            checked: s.suggest_all_personas }
                        span {
                            "Suggest tags from all my personas"
                            span { class: "hint", "The post form's “You often use” row draws on the persona you're posting as. Tick this to use all of them. Only you see the suggestions." }
                        }
                    }
                }
                if s.is_admin {
                    div { class: "field",
                        label { r#for: "staff_name", "Staff name"
                            span { class: "hint",
                                "Optional. Comment on announcements under this name, marked as an administrator, so writers know which of you they're talking to. It's public; your login name stays private. Leave it empty to comment only as “Administrators”."
                            }
                        }
                        input { r#type: "text", id: "staff_name", name: "staff_name",
                            value: "{view.staff_name}", maxlength: "{STAFF_NAME_MAX_CHARS}" }
                    }
                }
                div { class: "actions",
                    button { class: "btn", r#type: "submit", "Save settings" }
                }
            }
            if s.is_admin {
                p { class: "aside", "This account is a server administrator." }
            }
        }
    })
}

// ---- Administration ------------------------------------------------------

/// What a report is about, in a few words, linked where possible.
fn report_subject(r: &Report) -> Element {
    match (r.kind, &r.prompt, &r.comment) {
        (ReportKind::Prompt, Some(p), _) => {
            rsx! { a { href: "{prompt_url(p.id)}?show=1", "{p.title}" } }
        }
        (ReportKind::Prompt, None, _) => rsx! { "Prompt deleted by its author" },
        (ReportKind::Comment, _, Some(c)) => rsx! {
            a { href: "{crate::comment_url(c.id)}", "Comment by {c.speaker()}" }
        },
        (ReportKind::Comment, _, None) => rsx! { "Comment deleted by its author" },
    }
}

fn report_status(r: &Report) -> Element {
    match (r.resolution, r.kind) {
        (Some(Resolution::Discard), _) => rsx! { span { "Discarded" } },
        (Some(Resolution::Delete), ReportKind::Prompt) => {
            rsx! { span { class: "removed", "Prompt removed" } }
        }
        (Some(Resolution::Delete), ReportKind::Comment) => {
            rsx! { span { class: "removed", "Comment removed" } }
        }
        (None, _) => rsx! { span { "Open" } },
    }
}

/// The reports page's address with report `id` open beside the table.
fn report_url(all: bool, id: Uuid) -> String {
    if all {
        format!("/admin/reports?all=1&open={id}")
    } else {
        format!("/admin/reports?open={id}")
    }
}

pub fn admin_reports_page(chrome: &Chrome, view: AdminReportsView) -> String {
    let chrome_for_body = chrome.clone();
    render_admin(Tab::Reports, "Reports", chrome, move || {
        let v = &view;
        // The report shown beside the table: the one asked for, else the
        // first, so the next report to decide is always one click away.
        let open = v
            .open
            .and_then(|id| v.reports.iter().find(|r| r.id == id))
            .or(v.reports.first());
        rsx! {
            h1 { "Reports" }
            p { class: "lede",
                if v.all {
                    "Every report, newest first. "
                    a { href: "/admin/reports", "Show open reports only." }
                } else {
                    "Open reports, oldest first. "
                    a { href: "/admin/reports?all=1", "Show all reports." }
                }
                " "
                a { href: "/admin/log", "See the admin log." }
            }
            if v.reports.is_empty() {
                div { class: "empty", if v.all { "No reports yet." } else { "No open reports." } }
            } else {
                div { class: "master-detail",
                    div { class: "table-scroll",
                        table { class: "report-table",
                            thead {
                                tr {
                                    th { "Reported" }
                                    th { "Reason" }
                                    th { "Received" }
                                    th { "Status" }
                                }
                            }
                            tbody {
                                for r in v.reports.iter() {
                                    tr { key: "{r.id}",
                                        class: if open.is_some_and(|o| o.id == r.id) { "selected" } else { "" },
                                        td {
                                            a { class: "row-open", href: "{report_url(v.all, r.id)}",
                                                "aria-current": open.is_some_and(|o| o.id == r.id).then_some("true"),
                                                match (&r.prompt, &r.comment) {
                                                    (Some(p), _) => rsx! { "{p.title}" },
                                                    (None, Some(c)) => rsx! { "Comment by {c.speaker()}" },
                                                    (None, None) if r.kind == ReportKind::Prompt => rsx! { "Prompt deleted by its author" },
                                                    (None, None) => rsx! { "Comment deleted by its author" },
                                                }
                                            }
                                        }
                                        td {
                                            "{reason_label(r.reason)}"
                                            if r.script.is_some() {
                                                " "
                                                span { class: "automated", "Automated" }
                                            }
                                        }
                                        td { "{short_date(r.created_at)}" }
                                        td { {report_status(r)} }
                                    }
                                }
                            }
                        }
                    }
                    if let Some(r) = open {
                        {report_detail(&chrome_for_body, r)}
                    }
                }
            }
        }
    })
}

/// One report in full, with the decision buttons while it's open.
fn report_detail(chrome: &Chrome, r: &Report) -> Element {
    rsx! {
        aside { class: if r.resolved_at.is_some() { "detail-panel report resolved" } else { "detail-panel report" },
            "aria-label": "Report",
            h2 { {report_subject(r)} }
            p { class: "card-meta",
                span { "{reason_label(r.reason)}" }
                if let Some(script) = &r.script {
                    span { class: "automated", "Automated: {script}" }
                }
                span { "Received {short_date(r.created_at)}" }
                {report_status(r)}
            }
            if let Some(c) = &r.comment {
                div { class: "comment-body", dangerous_inner_html: "{c.body_html}" }
            }
            if let Some(d) = &r.details {
                blockquote { "{d}" }
            }
            if r.resolved_at.is_none() {
                div { class: "decide",
                    form { method: "post", action: "/admin/reports/{r.id}/resolve",
                        {csrf(chrome)}
                        input { r#type: "hidden", name: "action", value: "discard" }
                        button { class: "btn btn-quiet", r#type: "submit", "Discard report" }
                    }
                    if r.prompt.as_ref().is_some_and(|p| !p.removed) {
                        form { method: "post", action: "/admin/reports/{r.id}/resolve",
                            {csrf(chrome)}
                            input { r#type: "hidden", name: "action", value: "delete" }
                            button { class: "btn btn-danger", r#type: "submit", "Remove prompt" }
                        }
                    }
                    if r.comment.as_ref().is_some_and(|c| !c.removed) {
                        form { method: "post", action: "/admin/reports/{r.id}/resolve",
                            {csrf(chrome)}
                            input { r#type: "hidden", name: "action", value: "delete" }
                            button { class: "btn btn-danger", r#type: "submit", "Remove comment" }
                        }
                    }
                }
                p { class: "hint", "Reporters are never shown. Removing tells the author the reason, never who reported it." }
            }
        }
    }
}

fn action_label(action: AdminAction) -> &'static str {
    match action {
        AdminAction::ReportReceived => "Report received",
        AdminAction::Discard => "Report discarded",
        AdminAction::DeletePrompt => "Prompt removed",
        AdminAction::CreateTagType => "Tag type added",
        AdminAction::UpdateTagType => "Tag type changed",
        AdminAction::CreateTag => "Tag added",
        AdminAction::RenameTag => "Tag renamed",
        AdminAction::AliasTag => "Tag merged",
        AdminAction::DeleteTag => "Tag deleted",
        AdminAction::ApproveTag => "Tag approved",
        AdminAction::RejectTag => "Tag rejected",
        AdminAction::HideTag => "Tag hidden",
        AdminAction::ShowTag => "Tag shown",
        AdminAction::ChangeSetting => "Setting changed",
        AdminAction::RecolorTag => "Tag colour changed",
        AdminAction::ReserveHandles => "Handles reserved",
        AdminAction::UnreserveHandle => "Handle no longer reserved",
        AdminAction::PostAnnouncement => "Announcement posted",
        AdminAction::EditAnnouncement => "Announcement edited",
        AdminAction::DeleteAnnouncement => "Announcement deleted",
        AdminAction::SetAnnouncementComments => "Announcement comments changed",
        AdminAction::SetAnnouncementSticky => "Announcement made sticky or not",
        AdminAction::DeleteComment => "Comment removed",
        AdminAction::UploadScript => "Moderation script added",
        AdminAction::ReplaceScript => "Moderation script replaced",
        AdminAction::RemoveScript => "Moderation script removed",
        AdminAction::SetScriptMode => "Moderation script switched",
        AdminAction::UpdateChain => "Moderation chain changed",
        AdminAction::SaveModerationData => "Moderation data saved",
        AdminAction::DeleteModerationData => "Moderation data deleted",
        AdminAction::AutomodReject => "Refused by a script",
        AdminAction::AutomodFlag => "Tag flagged by a script",
        AdminAction::AutomodDismiss => "Report closed by a script",
        AdminAction::AutomodError => "Moderation script failed",
        AdminAction::UpdateRules => "Server rules changed",
    }
}

fn log_subject(e: &AdminLogEntry) -> Element {
    if let Some(subject) = &e.subject {
        return rsx! { code { "{subject}" } };
    }
    if let Some(id) = e.comment_id {
        return rsx! { a { href: "{crate::comment_url(id)}", "View comment" } };
    }
    match e.prompt_id {
        Some(id) => rsx! { a { href: "{prompt_url(id)}?show=1", "View prompt" } },
        None => rsx! { "Prompt deleted" },
    }
}

fn log_detail(e: &AdminLogEntry) -> Element {
    if let Some(detail) = &e.detail {
        return rsx! { "{detail}" };
    }
    let reason = e.reason.map(reason_label).unwrap_or_default();
    let hash = e.prompt_hash.as_deref().unwrap_or_default();
    let short = hash.get(..12).unwrap_or(hash);
    rsx! {
        "{reason}"
        if !short.is_empty() {
            br {}
            span { class: "hint", "Fingerprint " code { "{short}" } }
        }
    }
}

/// Who made the entry: an administrator (a filter link), the server, a
/// report (whose writer isn't named, even here), or a moderation script.
fn log_actor(e: &AdminLogEntry) -> Element {
    if e.actor == LogActor::Automod {
        let name = e.script_name.as_deref().unwrap_or("a script");
        return rsx! { em { "Automated: " } "{name}" };
    }
    match (e.actor, e.admin_login.as_deref()) {
        (LogActor::Admin, Some(login)) => rsx! {
            a { href: "{admin_log_url(Some(login), None, None)}", title: "Everything {login} did", "{login}" }
        },
        (LogActor::Admin, None) => rsx! { "An administrator" },
        (LogActor::Server, _) => rsx! { em { "Server" } },
        (LogActor::Reporter, _) => rsx! { em { "A report" } },
        (LogActor::Automod, _) => rsx! { em { "A moderation script" } },
    }
}

/// The first few accounts an entry was against, each a filter link, and
/// how many more (a link to the entry's full list).
fn log_targets(e: &AdminLogEntry) -> Element {
    if e.targets.is_empty() {
        return rsx! { span { class: "hint", "—" } };
    }
    let total = usize::try_from(e.target_count).unwrap_or(usize::MAX);
    let more = total.saturating_sub(e.targets.len());
    rsx! {
        for (i, login) in e.targets.iter().enumerate() {
            span { key: "{login}",
                if i > 0 { ", " }
                a { href: "{admin_log_url(None, Some(login), None)}",
                    title: "Everything done to {login}", "{login}" }
            }
        }
        if more > 0 {
            " "
            a { class: "more", href: "{admin_log_entry_url(e.id, None)}",
                "and {more} more" }
        }
    }
}

pub fn admin_log_page(chrome: &Chrome, view: AdminLogView) -> String {
    render_admin(Tab::Log, "Admin log", chrome, move || {
        let v = &view;
        let filtered = v.admin.is_some() || v.account.is_some();
        rsx! {
            h1 { "Admin log" }
            p { class: "lede",
                "Every report received and every administrator action, newest first, with who acted and which accounts each decision was against. Only administrators see this; writers see their own notices, which never name an administrator. The log can't be edited. "
                a { href: "/admin/reports", "Back to reports" }
            }
            form { class: "form-card", method: "get", action: "/admin/log", role: "search",
                div { class: "filter-grid",
                    div { class: "field",
                        label { r#for: "log-admin", "Done by administrator"
                            span { class: "hint", "Login name." }
                        }
                        input { r#type: "text", id: "log-admin", name: "admin",
                            value: "{v.admin.as_deref().unwrap_or_default()}" }
                    }
                    div { class: "field",
                        label { r#for: "log-account", "Done to account"
                            span { class: "hint", "Login name." }
                        }
                        input { r#type: "text", id: "log-account", name: "account",
                            value: "{v.account.as_deref().unwrap_or_default()}" }
                    }
                }
                div { class: "actions",
                    button { class: "btn btn-quiet", r#type: "submit", "Filter" }
                    if filtered {
                        a { href: "/admin/log", "Show everything" }
                    }
                }
            }
            if v.entries.is_empty() {
                div { class: "empty", if filtered { "No entries match." } else { "Nothing logged yet." } }
            } else {
                div { class: "table-scroll",
                    table { class: "log",
                        thead {
                            tr {
                                th { "When" } th { "Who" } th { "Event" } th { "Subject" }
                                th { "Details" } th { "Accounts affected" }
                            }
                        }
                        tbody {
                            for e in v.entries.iter() {
                                tr { key: "{e.id}",
                                    td { class: "nowrap",
                                        a { href: "{admin_log_entry_url(e.id, None)}",
                                            title: "Entry {e.id} in full", "{short_datetime(e.at)}" }
                                    }
                                    td { {log_actor(e)} }
                                    td { "{action_label(e.action)}" }
                                    td { {log_subject(e)} }
                                    td { {log_detail(e)} }
                                    td { {log_targets(e)} }
                                }
                            }
                        }
                    }
                }
            }
            if let Some(older) = &v.older {
                p { class: "pager", a { class: "btn btn-quiet", href: "{older}", "Older entries" } }
            }
        }
    })
}

/// One log entry in full: every account it was against, a page at a time,
/// with the prompts the decision touched.
pub fn admin_log_entry_page(chrome: &Chrome, view: AdminLogEntryView) -> String {
    let title = format!("Admin log entry {}", view.detail.entry.id);
    render_admin(Tab::Log, title, chrome, move || {
        let v = &view;
        let e = &v.detail.entry;
        let count = e.target_count;
        let accounts = if count == 1 { "account" } else { "accounts" };
        rsx! {
            h1 { "{action_label(e.action)}" }
            p { class: "lede",
                "Entry {e.id} in the admin log. "
                a { href: "/admin/log", "Back to the log" }
            }
            dl { class: "entry-facts",
                dt { "When" } dd { "{short_datetime(e.at)}" }
                dt { "Who" } dd { {log_actor(e)} }
                dt { "Subject" } dd { {log_subject(e)} }
                dt { "Details" } dd { {log_detail(e)} }
            }
            h2 { "Accounts affected" }
            if count == 0 {
                p { class: "empty", "None. This was a server-wide change." }
            } else {
                p { class: "hint",
                    "{count} {accounts}, alphabetically. Each name links to everything done to that account."
                }
                if v.detail.targets.is_empty() {
                    div { class: "empty", "No more accounts." }
                } else {
                    div { class: "table-scroll",
                        table { class: "log",
                            thead { tr { th { "Account" } th { "Prompts affected" } } }
                            tbody {
                                for t in v.detail.targets.iter() {
                                    tr { key: "{t.login}",
                                        td { class: "nowrap",
                                            a { href: "{admin_log_url(None, Some(&t.login), None)}", "{t.login}" }
                                            if t.account_deleted {
                                                br {}
                                                span { class: "hint", "Account deleted" }
                                            }
                                        }
                                        td {
                                            if t.prompts.is_empty() {
                                                span { class: "hint",
                                                    if t.account_deleted { "Unknown" } else { "—" }
                                                }
                                            }
                                            if !t.prompts.is_empty() {
                                                ul { class: "touched",
                                                    for p in t.prompts.iter() {
                                                        li { key: "{p.id}",
                                                            if p.exists {
                                                                a { href: "{prompt_url(p.id)}?show=1", "{p.title}" }
                                                            } else {
                                                                "{p.title} "
                                                                span { class: "muted", "(since removed)" }
                                                            }
                                                            span { class: "muted", " by @{p.handle}" }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if v.detail.next.is_some() || v.paged {
                    p { class: "pager",
                        if v.paged {
                            a { class: "btn btn-quiet", href: "{admin_log_entry_url(e.id, None)}", "First accounts" }
                            " "
                        }
                        if let Some(next) = &v.detail.next {
                            a { class: "btn btn-quiet", href: "{admin_log_entry_url(e.id, Some(next))}", "Next accounts" }
                        }
                    }
                }
            }
        }
    })
}

// ---- More ------------------------------------------------------------------

/// The phone's "More" page: the links that don't fit its tab bar, grouped.
/// Wide screens have them all in the header and never link here.
pub fn more_page(chrome: &Chrome) -> String {
    let chrome_for_body = chrome.clone();
    render("More", chrome, move || {
        let c = &chrome_for_body;
        let admin = c.viewer.as_ref().is_some_and(|v| v.is_admin);
        let pending = c.viewer.as_ref().map_or(0, |v| v.pending_tags);
        rsx! {
            h1 { "More" }
            h2 { class: "list-group", "You" }
            ul { class: "link-list",
                li { a { href: "/personas", "Personas" } }
                li { a { href: "/settings", "Settings" } }
                li { a { href: "/announcements", "Announcements" } }
                if c.has_rules {
                    li { a { href: crate::RULES_PATH, "Server rules" } }
                }
            }
            if admin {
                h2 { class: "list-group admin", "Administration" }
                ul { class: "link-list",
                    li { a { href: "/admin/reports", "Reports" } }
                    li { a { href: "/admin/log", "Admin log" } }
                    li { a { href: "/admin/settings/announcements", "Announcements" } }
                    li { a { href: crate::ADMIN_RULES_PATH, "Rules" } }
                    li { a { href: "/admin/settings/tag-types", "Tag types" } }
                    li { a { href: "/admin/settings/tags", "Tags" } }
                    li {
                        a { href: "/admin/settings/tag-requests", "Tag requests" }
                        if pending > 0 {
                            " "
                            span { class: "count", "{pending}" }
                        }
                    }
                    li { a { href: "/admin/moderation/scripts", "Automated moderation" } }
                    li { a { href: "/admin/settings/handles", "Handles" } }
                    li { a { href: "/admin/settings/sessions", "Sessions" } }
                }
            }
            form { class: "form-card", method: "post", action: "/logout",
                {csrf(c)}
                button { class: "btn btn-quiet", r#type: "submit", "Sign out" }
            }
        }
    })
}

// ---- Messages and errors -------------------------------------------------

pub fn message_page(chrome: &Chrome, view: MessageView) -> String {
    let title = view.title.clone();
    render(title, chrome, move || {
        let v = &view;
        rsx! {
            h1 { "{v.title}" }
            p { "{v.text}" }
            if let Some((href, label)) = &v.link {
                p { a { href: "{href}", "{label}" } }
            }
        }
    })
}
