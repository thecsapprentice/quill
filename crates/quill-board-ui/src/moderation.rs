//! Automated moderation pages, for administrators: scripts, chains, common
//! data and dry runs. Everything works as plain forms, without JavaScript.

use dioxus::prelude::*;
use quill_proto::automod::{
    DryRunDecision, DryRunResult, DryRunStep, Hook, ModerationChain, ModerationData,
    ModerationDataContent, ModerationDataSummary, ModerationDataVersion, ModerationScript, OnError,
    ScriptTemplate,
};

use crate::{
    Chrome,
    admin::{Tab, byte_size, tabs},
    layout::{csrf, error_notice},
    render, short_date,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Scripts,
    Chains,
    Data,
    DryRun,
}

fn sections(current: Section) -> Element {
    let mark = |s: Section| (s == current).then_some("page");
    rsx! {
        nav { class: "tabs", "aria-label": "Moderation",
            a { href: "/admin/moderation/scripts", "aria-current": mark(Section::Scripts), "Scripts" }
            a { href: "/admin/moderation/chains", "aria-current": mark(Section::Chains), "Chains" }
            a { href: "/admin/moderation/data", "aria-current": mark(Section::Data), "Common data" }
            a { href: "/admin/moderation/dry-run", "aria-current": mark(Section::DryRun), "Dry run" }
        }
    }
}

fn top(chrome: &Chrome, section: Section) -> Element {
    rsx! {
        h1 { "Server settings" }
        {tabs(chrome, Tab::Moderation)}
        {sections(section)}
    }
}

pub fn hook_label(hook: Hook) -> &'static str {
    match hook {
        Hook::PromptCreated => "A prompt is posted",
        Hook::PromptEdited => "A prompt is edited",
        Hook::CommentCreated => "A comment is posted",
        Hook::TagRequested => "A new tag is used",
        Hook::PromptReported => "A prompt is reported",
    }
}

fn on_error_label(on_error: OnError) -> &'static str {
    match on_error {
        OnError::Continue => "Carry on with the next script",
        OnError::Reject => "Refuse the post (“couldn't be checked, try again later”)",
        OnError::Flag => "Let it through and file a report",
    }
}

fn mode_label(s: &ModerationScript) -> &'static str {
    match (s.enabled, s.log_only) {
        (false, _) => "Off",
        (true, true) => "Log only",
        (true, false) => "On",
    }
}

// ---- Scripts ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModerationScriptsView {
    pub scripts: Vec<ModerationScript>,
    pub templates: Vec<ScriptTemplate>,
    /// The upload form, kept on error.
    pub name: String,
    pub source: String,
    pub log_only: bool,
    pub error: Option<String>,
}

pub fn moderation_scripts_page(chrome: &Chrome, view: ModerationScriptsView) -> String {
    let chrome_for_body = chrome.clone();
    render("Moderation scripts", chrome, move || {
        let v = &view;
        let c = &chrome_for_body;
        rsx! {
            {top(c, Section::Scripts)}
            p { class: "lede",
                "Scripts check what writers post before it's saved, and can refuse it, flag it for you, or let it through. Put them in order in "
                a { href: "/admin/moderation/chains", "Chains" }
                ". "
                a { href: "{c.source_url}/blob/main/docs/automod.md", "How to write them" }
                "."
            }
            {error_notice(v.error.as_ref())}
            if v.scripts.is_empty() {
                div { class: "empty", "No scripts yet. Start from a template below, or upload your own." }
            } else {
                div { class: "table-scroll",
                    table { class: "log",
                        thead { tr { th { "Script" } th { "Runs" } th { "In chains" } th { "Reads" } } }
                        tbody {
                            for s in v.scripts.iter() {
                                tr { key: "{s.id}",
                                    td { a { href: "/admin/moderation/scripts/{s.id}", "{s.name}" } }
                                    td { "{mode_label(s)}" }
                                    td {
                                        if s.chains.is_empty() {
                                            span { class: "hint", "None" }
                                        }
                                        for (i, h) in s.chains.iter().enumerate() {
                                            if i > 0 { br {} }
                                            "{hook_label(*h)}"
                                        }
                                    }
                                    td {
                                        for d in s.data.iter() {
                                            a { key: "{d}", href: "/admin/moderation/data/{d}", code { "{d}" } }
                                            " "
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            h2 { "Start from a template" }
            p { class: "hint", "A copy becomes your own script: change it as you like. Lists it reads are created for you to fill in." }
            for t in v.templates.iter() {
                form { key: "{t.id}", class: "form-card", method: "post", action: "/admin/moderation/templates/{t.id}",
                    {csrf(c)}
                    h3 { class: "form-title", "{t.name}" }
                    p { "{t.summary}" }
                    details {
                        summary { "See the script" }
                        pre { class: "code", "{t.source}" }
                    }
                    div { class: "actions",
                        button { class: "btn btn-quiet", r#type: "submit", "Use this template" }
                    }
                }
            }
            form { class: "form-card", method: "post", action: "/admin/moderation/scripts",
                {csrf(c)}
                h2 { class: "form-title", "Upload a script" }
                div { class: "field",
                    label { r#for: "s-name", "Name" span { class: "hint", "Shown to administrators only, and in the log." } }
                    input { r#type: "text", id: "s-name", name: "name", value: "{v.name}", required: true }
                }
                div { class: "field",
                    label { r#for: "s-source", "Script" }
                    textarea { id: "s-source", class: "code", name: "source", spellcheck: "false", "{v.source}" }
                }
                div { class: "field",
                    label { class: "inline-check",
                        input { r#type: "checkbox", name: "log_only", value: "yes", checked: v.log_only }
                        span { "Log only" span { class: "hint", "It runs and logs what it would do, but does nothing. A good way to start." } }
                    }
                }
                div { class: "actions", button { class: "btn", r#type: "submit", "Upload" } }
            }
        }
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationScriptView {
    pub script: ModerationScript,
    /// The edit form, kept on error.
    pub name: String,
    pub source: String,
    pub error: Option<String>,
}

pub fn moderation_script_page(chrome: &Chrome, view: ModerationScriptView) -> String {
    let chrome_for_body = chrome.clone();
    let title = view.script.name.clone();
    render(&title, chrome, move || {
        let v = &view;
        let s = &v.script;
        let c = &chrome_for_body;
        rsx! {
            {top(c, Section::Scripts)}
            p { a { href: "/admin/moderation/scripts", "← All scripts" } }
            h2 { "{s.name}" }
            p { class: "card-meta",
                span { "{mode_label(s)}" }
                span { "Changed {short_date(s.updated_at)}" }
                if let Some(by) = &s.uploaded_by { span { "Uploaded by {by}" } }
            }
            if !s.chains.is_empty() {
                p { "Runs when: "
                    for (i, h) in s.chains.iter().enumerate() {
                        if i > 0 { "; " }
                        "{hook_label(*h).to_lowercase()}"
                    }
                    "."
                }
            }
            {error_notice(v.error.as_ref())}
            form { class: "form-card", method: "post", action: "/admin/moderation/scripts/{s.id}",
                {csrf(c)}
                div { class: "field",
                    label { r#for: "s-name", "Name" }
                    input { r#type: "text", id: "s-name", name: "name", value: "{v.name}", required: true }
                }
                div { class: "field",
                    label { r#for: "s-source", "Script" span { class: "hint", "Checked when you save: mistakes are shown with their line, and nothing changes until it's fixed." } }
                    textarea { id: "s-source", class: "code", name: "source", spellcheck: "false", "{v.source}" }
                }
                div { class: "actions",
                    button { class: "btn", r#type: "submit", "Save" }
                    a { class: "btn btn-quiet", href: "/admin/moderation/dry-run?script={s.id}", "Dry run" }
                }
            }
            form { class: "form-card", method: "post", action: "/admin/moderation/scripts/{s.id}/mode",
                {csrf(c)}
                h3 { class: "form-title", "Runs" }
                fieldset { class: "choices",
                    legend { class: "visually-hidden", "Runs" }
                    for (value, label, checked) in [
                        ("on", "On", s.enabled && !s.log_only),
                        ("log_only", "Log only: logs what it would do, does nothing", s.enabled && s.log_only),
                        ("off", "Off: stays in its chains but doesn't run", !s.enabled),
                    ] {
                        label { key: "{value}", class: "inline-check",
                            input { r#type: "radio", name: "mode", value: "{value}", checked }
                            "{label}"
                        }
                    }
                }
                div { class: "actions", button { class: "btn btn-quiet", r#type: "submit", "Change" } }
            }
            form { class: "form-card", method: "post", action: "/admin/moderation/scripts/{s.id}/delete",
                {csrf(c)}
                h3 { class: "form-title", "Remove" }
                label { class: "inline-check",
                    input { r#type: "checkbox", name: "confirm", value: "yes" }
                    "Remove this script, and take it out of every chain"
                }
                div { class: "actions", button { class: "btn btn-danger", r#type: "submit", "Remove script" } }
            }
        }
    })
}

// ---- Chains ----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModerationChainsView {
    pub chains: Vec<ModerationChain>,
    pub scripts: Vec<ModerationScript>,
    pub error: Option<String>,
}

pub fn moderation_chains_page(chrome: &Chrome, view: ModerationChainsView) -> String {
    let chrome_for_body = chrome.clone();
    render("Moderation chains", chrome, move || {
        let v = &view;
        let c = &chrome_for_body;
        rsx! {
            {top(c, Section::Chains)}
            p { class: "lede",
                "Each moment a writer posts something runs its chain: the scripts below, in order. The first script to refuse, flag or dismiss decides, and the rest don't run. If none does, it goes through. Administrators' own posts aren't checked."
            }
            {error_notice(v.error.as_ref())}
            if v.scripts.is_empty() {
                div { class: "empty", "Add a script first, in " a { href: "/admin/moderation/scripts", "Scripts" } "." }
            }
            for chain in v.chains.iter() {
                form { key: "{chain.hook}", class: "form-card", method: "post", action: "/admin/moderation/chains/{chain.hook}",
                    {csrf(c)}
                    h2 { class: "form-title", "{hook_label(chain.hook)}" span { class: "hint", " ({chain.hook})" } }
                    if !v.scripts.is_empty() {
                        p { class: "hint", "Number the scripts to run, in order. Leave a script blank to leave it out." }
                        div { class: "table-scroll",
                            table { class: "log",
                                thead { tr { th { "Order" } th { "Script" } th { "Runs" } } }
                                tbody {
                                    for s in v.scripts.iter() {
                                        tr { key: "{s.id}",
                                            td {
                                                input { class: "position", r#type: "number", min: "1", name: "pos_{s.id}",
                                                    "aria-label": "Order of {s.name}",
                                                    value: chain.steps.iter().position(|st| st.script_id == s.id).map(|i| (i + 1).to_string()).unwrap_or_default(),
                                                }
                                            }
                                            td { a { href: "/admin/moderation/scripts/{s.id}", "{s.name}" } }
                                            td { "{mode_label(s)}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    fieldset { class: "choices",
                        legend { class: "label", "When a script fails" span { class: "hint", " (a mistake that only shows on some posts, or taking too long; always logged)" } }
                        for o in OnError::ALL.iter() {
                            label { key: "{o}", class: "inline-check",
                                input { r#type: "radio", name: "on_error", value: "{o}", checked: chain.on_error == *o }
                                "{on_error_label(*o)}"
                            }
                        }
                    }
                    div { class: "actions", button { class: "btn", r#type: "submit", "Save chain" } }
                }
            }
        }
    })
}

// ---- Common data -----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModerationDataListView {
    pub entries: Vec<ModerationDataSummary>,
    /// The new-entry form, kept on error.
    pub name: String,
    pub description: String,
    pub content: String,
    pub error: Option<String>,
}

pub fn moderation_data_list_page(chrome: &Chrome, view: ModerationDataListView) -> String {
    let chrome_for_body = chrome.clone();
    render("Common data", chrome, move || {
        let v = &view;
        let c = &chrome_for_body;
        rsx! {
            {top(c, Section::Data)}
            p { class: "lede",
                "Lists and text kept once and read by any script, like blocked words or allowed sites. Change a list here and every script reading it follows. Every save is kept, so you can undo."
            }
            {error_notice(v.error.as_ref())}
            if v.entries.is_empty() {
                div { class: "empty", "Nothing yet." }
            } else {
                div { class: "table-scroll",
                    table { class: "log",
                        thead { tr { th { "Name" } th { "What it's for" } th { "Size" } th { "Read by" } } }
                        tbody {
                            for d in v.entries.iter() {
                                tr { key: "{d.name}",
                                    td { a { href: "/admin/moderation/data/{d.name}", code { "{d.name}" } } }
                                    td { "{d.description}" }
                                    td { "{byte_size(d.size)}" }
                                    td {
                                        if d.used_by.is_empty() { span { class: "hint", "No script" } }
                                        "{d.used_by.join(\", \")}"
                                    }
                                }
                            }
                        }
                    }
                }
            }
            form { class: "form-card", method: "post", action: "/admin/moderation/data",
                {csrf(c)}
                h2 { class: "form-title", "Add an entry" }
                div { class: "field",
                    label { r#for: "d-name", "Name" span { class: "hint", "Lowercase letters, digits and underscores, like blocked_words. Scripts read it by this name." } }
                    input { r#type: "text", id: "d-name", name: "name", value: "{v.name}", required: true, pattern: "[a-z0-9_]{{1,64}}" }
                }
                div { class: "field",
                    label { r#for: "d-desc", "What it's for" }
                    input { r#type: "text", id: "d-desc", name: "description", value: "{v.description}" }
                }
                div { class: "field",
                    label { r#for: "d-content", "Content" span { class: "hint", "For a list, one entry per line. Lines starting with # are notes." } }
                    textarea { id: "d-content", class: "code short", name: "content", spellcheck: "false", "{v.content}" }
                }
                div { class: "actions", button { class: "btn", r#type: "submit", "Add" } }
            }
        }
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationDataView {
    pub entry: ModerationData,
    pub history: Vec<ModerationDataVersion>,
    /// The edit form, kept on error or after a preview.
    pub description: String,
    pub content: String,
    pub base_version: i32,
    /// The text a preview ran on.
    pub preview_title: String,
    pub preview_body: String,
    /// A preview's results, chain by chain.
    pub preview: Option<Vec<(Hook, DryRunResult)>>,
    /// An old version being looked at.
    pub viewing: Option<ModerationDataContent>,
    pub error: Option<String>,
}

pub fn moderation_data_page(chrome: &Chrome, view: ModerationDataView) -> String {
    let chrome_for_body = chrome.clone();
    let title = view.entry.name.clone();
    render(&title, chrome, move || {
        let v = &view;
        let d = &v.entry;
        let c = &chrome_for_body;
        rsx! {
            {top(c, Section::Data)}
            p { a { href: "/admin/moderation/data", "← All common data" } }
            h2 { code { "{d.name}" } }
            p { class: "card-meta",
                span { "Version {d.version}" }
                span { "Saved {short_date(d.updated_at)}" }
                if let Some(by) = &d.updated_by { span { "by {by}" } }
            }
            if d.used_by.is_empty() {
                p { class: "hint", "No script reads this yet." }
            } else {
                p { "Read by: {d.used_by.join(\", \")}." }
            }
            {error_notice(v.error.as_ref())}
            if let Some(preview) = &v.preview {
                div { class: "form-card",
                    h3 { class: "form-title", "What would happen with your changes" }
                    p { class: "hint", "Not saved yet. Each chain ran on the text below with your version of this entry." }
                    if preview.is_empty() {
                        p { "No chain runs a script that reads this entry." }
                    }
                    for (hook, result) in preview.iter() {
                        h4 { key: "{hook}", "{hook_label(*hook)}" }
                        {dry_run_steps(&result.steps)}
                    }
                }
            }
            form { class: "form-card", method: "post", action: "/admin/moderation/data/{d.name}",
                {csrf(c)}
                input { r#type: "hidden", name: "base_version", value: "{v.base_version}" }
                div { class: "field",
                    label { r#for: "d-desc", "What it's for" }
                    input { r#type: "text", id: "d-desc", name: "description", value: "{v.description}" }
                }
                div { class: "field",
                    label { r#for: "d-content", "Content" span { class: "hint", "For a list, one entry per line. Lines starting with # are notes." } }
                    textarea { id: "d-content", class: "code", name: "content", spellcheck: "false", "{v.content}" }
                }
                details { open: v.preview.is_some(),
                    summary { "Try it before saving" }
                    p { class: "hint", "Runs every chain that reads this entry on this text, with your changes, without saving or doing anything." }
                    div { class: "field",
                        label { r#for: "p-title", "Title" }
                        input { r#type: "text", id: "p-title", name: "preview_title", value: "{v.preview_title}" }
                    }
                    div { class: "field",
                        label { r#for: "p-body", "Text" }
                        textarea { id: "p-body", class: "short", name: "preview_body", "{v.preview_body}" }
                    }
                    div { class: "actions",
                        button { class: "btn btn-quiet", r#type: "submit", name: "action", value: "preview", "Try it" }
                    }
                }
                div { class: "actions",
                    button { class: "btn", r#type: "submit", name: "action", value: "save", "Save as version {d.version + 1}" }
                }
            }
            if let Some(old) = &v.viewing {
                div { class: "form-card",
                    h3 { class: "form-title", "Version {old.version}" }
                    pre { class: "code", "{old.content}" }
                }
            }
            h3 { "History" }
            div { class: "table-scroll",
                table { class: "log",
                    thead { tr { th { "Version" } th { "Saved" } th { "By" } th { "Size" } th { span { class: "visually-hidden", "Actions" } } } }
                    tbody {
                        for h in v.history.iter() {
                            tr { key: "{h.version}",
                                td { "{h.version}" }
                                td { "{short_date(h.created_at)}" }
                                td { "{h.created_by.as_deref().unwrap_or(\"—\")}" }
                                td { "{byte_size(h.size)}" }
                                td {
                                    a { href: "/admin/moderation/data/{d.name}?version={h.version}", "View" }
                                    if h.version != d.version {
                                        form { method: "post", action: "/admin/moderation/data/{d.name}/restore",
                                            {csrf(c)}
                                            input { r#type: "hidden", name: "version", value: "{h.version}" }
                                            button { class: "link-button", r#type: "submit", "Go back to this version" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if d.used_by.is_empty() {
                form { class: "form-card", method: "post", action: "/admin/moderation/data/{d.name}/delete",
                    {csrf(c)}
                    h3 { class: "form-title", "Delete" }
                    label { class: "inline-check",
                        input { r#type: "checkbox", name: "confirm", value: "yes" }
                        "Delete this entry and all its versions"
                    }
                    div { class: "actions", button { class: "btn btn-danger", r#type: "submit", "Delete" } }
                }
            }
        }
    })
}

// ---- Dry runs --------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationDryRunView {
    pub scripts: Vec<ModerationScript>,
    pub hook: Hook,
    /// One script, or the whole chain.
    pub script: Option<uuid::Uuid>,
    /// An existing prompt's id or link, as typed.
    pub prompt: String,
    pub title: String,
    pub body: String,
    pub result: Option<DryRunResult>,
    pub error: Option<String>,
}

fn decision_label(d: DryRunDecision) -> &'static str {
    match d {
        DryRunDecision::Pass => "Nothing",
        DryRunDecision::Reject => "Refuse",
        DryRunDecision::Flag => "Flag",
        DryRunDecision::Dismiss => "Close the report",
        DryRunDecision::Error => "Failed",
    }
}

fn dry_run_steps(steps: &[DryRunStep]) -> Element {
    rsx! {
        if steps.is_empty() {
            p { class: "hint", "No scripts that are on." }
        } else {
            div { class: "table-scroll",
                table { class: "log",
                    thead { tr { th { "Script" } th { "Would" } th { "Details" } th { "Time" } } }
                    tbody {
                        for s in steps.iter() {
                            tr { key: "{s.script_id}",
                                td {
                                    "{s.name}"
                                    if s.log_only { span { class: "hint", " (log only)" } }
                                }
                                td {
                                    span { class: "decision-{s.decision}", "{decision_label(s.decision)}" }
                                    if s.decides { br {} span { class: "hint", "This decides." } }
                                }
                                td {
                                    if let Some(detail) = &s.detail { "{detail}" }
                                    if let Some(cat) = &s.category { span { class: "hint", " ({cat})" } }
                                    if !s.printed.is_empty() {
                                        pre { class: "code", "{s.printed.join(\"\\n\")}" }
                                    }
                                }
                                td { "{s.micros} µs" }
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn moderation_dry_run_page(chrome: &Chrome, view: ModerationDryRunView) -> String {
    let chrome_for_body = chrome.clone();
    render("Dry run", chrome, move || {
        let v = &view;
        let c = &chrome_for_body;
        rsx! {
            {top(c, Section::DryRun)}
            p { class: "lede",
                "Run a chain, or one script, on text you type or on a prompt already on the board. Every script runs, even after one decides, and nothing is done or logged."
            }
            {error_notice(v.error.as_ref())}
            form { class: "form-card", method: "post", action: "/admin/moderation/dry-run",
                {csrf(c)}
                div { class: "filter-grid",
                    div { class: "field",
                        label { r#for: "r-hook", "When" }
                        select { id: "r-hook", name: "hook",
                            for h in Hook::ALL.iter() {
                                option { key: "{h}", value: "{h}", selected: v.hook == *h, "{hook_label(*h)}" }
                            }
                        }
                    }
                    div { class: "field",
                        label { r#for: "r-script", "Run" }
                        select { id: "r-script", name: "script",
                            option { value: "", "The whole chain" }
                            for s in v.scripts.iter() {
                                option { key: "{s.id}", value: "{s.id}", selected: v.script == Some(s.id), "Only {s.name}" }
                            }
                        }
                    }
                }
                div { class: "field",
                    label { r#for: "r-prompt", "An existing prompt" span { class: "hint", "Its link or id. Its writer's history is used. Leave empty to use the text below." } }
                    input { r#type: "text", id: "r-prompt", name: "prompt", value: "{v.prompt}" }
                }
                div { class: "field",
                    label { r#for: "r-title", "Title" }
                    input { r#type: "text", id: "r-title", name: "title", value: "{v.title}" }
                }
                div { class: "field",
                    label { r#for: "r-body", "Text" span { class: "hint", "From a made-up writer with a clean record." } }
                    textarea { id: "r-body", class: "short", name: "body", "{v.body}" }
                }
                div { class: "actions", button { class: "btn", r#type: "submit", "Run" } }
            }
            if let Some(result) = &v.result {
                h2 { "Results" }
                {dry_run_steps(&result.steps)}
            }
        }
    })
}
