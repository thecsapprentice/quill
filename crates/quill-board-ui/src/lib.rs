//! Prompt board UI: Dioxus components rendered to HTML.
//!
//! The server renders these pages with no JavaScript at all; every action is
//! a plain link or HTML form. Future clients reuse the same components.
//!
//! This crate depends only on `quill-proto` and Dioxus. It must never depend
//! on any cryptographic or MLS crate, because the server links it.
//!
//! Safety: all text is escaped by Dioxus. The only raw HTML inserted is a
//! prompt's `body_html`, which the server has already sanitized.

mod layout;
mod pages;

use std::rc::Rc;

use dioxus::prelude::*;
use quill_proto::{
    api::Persona,
    board::{
        AccountSettings, AdminLogEntry, Maturity, Prompt, PromptState, Report, ReportReason, Tag,
        TagNamespace, TagPolicy,
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

pub use pages::*;

/// The board's stylesheet, served at [`STYLESHEET_PATH`].
pub const STYLESHEET: &str = include_str!("board.css");
pub const STYLESHEET_PATH: &str = "/static/board.css";

/// Form field carrying the per-session CSRF token.
pub const CSRF_FIELD: &str = "csrf";

/// Which navigation entry is current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Board,
    Post,
    Mine,
    Personas,
    Settings,
    Admin,
    SignIn,
    Other,
}

/// The signed-in visitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    /// Per-session token every form must echo back.
    pub csrf: String,
    pub is_admin: bool,
    pub maturity_max: Maturity,
}

/// What every page needs: site identity, the visitor and navigation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chrome {
    pub site_name: String,
    pub viewer: Option<Viewer>,
    /// Where users can get this server's source code (required by the AGPL).
    pub source_url: String,
    pub version: String,
    /// One-line message shown at the top of the page, such as "Prompt posted."
    pub notice: Option<String>,
    pub current: Nav,
}

/// Board search filters, as typed into the filter form.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoardFilters {
    pub include: String,
    pub exclude: String,
    pub q: String,
    pub maturity: Option<Maturity>,
    pub state: Option<PromptState>,
}

impl BoardFilters {
    pub fn is_active(&self) -> bool {
        !self.include.is_empty()
            || !self.exclude.is_empty()
            || !self.q.is_empty()
            || self.maturity.is_some()
            || self.state.is_some()
    }

    /// The board URL for these filters, optionally continuing from `cursor`.
    pub fn url(&self, cursor: Option<&str>) -> String {
        let mut s = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in [
            ("include", self.include.as_str()),
            ("exclude", self.exclude.as_str()),
            ("q", self.q.as_str()),
            ("maturity", self.maturity.map_or("", Maturity::as_str)),
            ("state", self.state.map_or("", PromptState::as_str)),
            ("cursor", cursor.unwrap_or("")),
        ] {
            if !value.is_empty() {
                s.append_pair(key, value);
            }
        }
        let query = s.finish();
        if query.is_empty() {
            "/".to_owned()
        } else {
            format!("/?{query}")
        }
    }
}

/// The board link that filters by one tag.
pub fn tag_url(tag: &Tag) -> String {
    BoardFilters {
        include: tag.key(),
        ..BoardFilters::default()
    }
    .url(None)
}

pub fn prompt_url(id: Uuid) -> String {
    format!("/p/{id}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardView {
    pub filters: BoardFilters,
    pub prompts: Vec<Prompt>,
    pub next_cursor: Option<String>,
    /// The visitor's maturity cap, explained above the results.
    pub cap: Maturity,
    pub namespaces: Vec<TagNamespace>,
    /// Suggestions for the tag fields.
    pub tag_options: Vec<Tag>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptView {
    pub prompt: Prompt,
    pub is_author: bool,
    /// The prompt is rated above the visitor's cap; show a warning first.
    pub held_back: Option<Maturity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    SignIn,
    Register,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthView {
    pub mode: AuthMode,
    pub username: String,
    pub error: Option<String>,
    pub registration_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptFormView {
    /// `None` when posting a new prompt.
    pub editing: Option<Uuid>,
    pub personas: Vec<Persona>,
    pub persona: Option<Uuid>,
    pub title: String,
    pub body: String,
    pub maturity: Maturity,
    /// Comma-separated `namespace:name`.
    pub tags: String,
    pub state: PromptState,
    pub error: Option<String>,
    pub tag_options: Vec<Tag>,
    pub tag_policy: TagPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportView {
    pub prompt: Prompt,
    pub reason: Option<ReportReason>,
    pub details: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonasView {
    pub personas: Vec<Persona>,
    pub handle: String,
    pub display_name: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsView {
    pub settings: AccountSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminReportsView {
    pub reports: Vec<Report>,
    pub all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminLogView {
    pub entries: Vec<AdminLogEntry>,
    /// Link to older entries.
    pub older: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageView {
    pub title: String,
    pub text: String,
    pub link: Option<(String, String)>,
}

// ---- Rendering -----------------------------------------------------------

#[derive(Clone)]
struct Doc {
    title: String,
    chrome: Chrome,
    body: Rc<dyn Fn() -> Element>,
}

fn document(doc: Doc) -> Element {
    let Doc {
        title,
        chrome,
        body,
    } = doc;
    let page_title = if title.is_empty() {
        chrome.site_name.clone()
    } else {
        format!("{title} – {}", chrome.site_name)
    };
    rsx! {
            head {
                meta { charset: "utf-8" }
                meta { name: "viewport", content: "width=device-width, initial-scale=1" }
                meta { name: "referrer", content: "same-origin" }
                title { "{page_title}" }
                link { rel: "stylesheet", href: "{STYLESHEET_PATH}?v={chrome.version}" }
            }
            body {
                {layout::header(&chrome)}
                main { class: "wrap",
                    if let Some(notice) = &chrome.notice {
                        p { class: "notice", role: "status", "{notice}" }
                    }
                    {body()}
                }
                {layout::footer(&chrome)}
            }
    }
}

/// Renders a complete HTML document.
pub(crate) fn render(
    title: impl Into<String>,
    chrome: &Chrome,
    body: impl Fn() -> Element + 'static,
) -> String {
    let mut dom = VirtualDom::new_with_props(
        document,
        Doc {
            title: title.into(),
            chrome: chrome.clone(),
            body: Rc::new(body),
        },
    );
    dom.rebuild_in_place();
    // Dioxus has no `html` element, so the root tag is written here.
    format!(
        "<!DOCTYPE html><html lang=\"en\">{}</html>",
        dioxus_ssr::render(&dom)
    )
}

// ---- Labels --------------------------------------------------------------

pub fn maturity_label(m: Maturity) -> &'static str {
    match m {
        Maturity::General => "General",
        Maturity::Teen => "Teen",
        Maturity::Mature => "Mature",
        Maturity::Explicit => "Explicit",
    }
}

pub fn state_label(s: PromptState) -> &'static str {
    match s {
        PromptState::Open => "Open to requests",
        PromptState::Busy => "Busy",
        PromptState::Closed => "Closed",
    }
}

pub fn reason_label(r: ReportReason) -> &'static str {
    match r {
        ReportReason::Illegal => "Illegal content",
        ReportReason::Spam => "Spam",
        ReportReason::Harassment => "Harassment",
        ReportReason::WrongMaturity => "Wrong maturity rating",
        ReportReason::Other => "Something else",
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A short UTC date such as "Oct 22, 2026".
pub fn short_date(at: OffsetDateTime) -> String {
    let month = MONTHS
        .get(usize::from(u8::from(at.month())) - 1)
        .copied()
        .unwrap_or("");
    format!("{month} {}, {}", at.day(), at.year())
}

#[cfg(test)]
mod tests;
