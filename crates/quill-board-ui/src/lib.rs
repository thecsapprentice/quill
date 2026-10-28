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

mod admin;
mod announcements;
pub mod chips;
mod layout;
mod moderation;
mod notices;
mod pages;
mod rules;

use std::rc::Rc;

use dioxus::prelude::*;
use quill_proto::{
    api::Persona,
    board::{
        AccountSettings, AdminLogDetail, AdminLogEntry, AdminTag, ChipColor, Maturity, Prompt,
        PromptState, Report, ReportReason, Tag, TagNamespace, TagPolicy,
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

pub use admin::*;
pub use announcements::*;
pub use moderation::*;
pub use notices::notices_page;
pub use pages::*;
pub use rules::*;

/// The board's stylesheet, served at [`STYLESHEET_PATH`].
pub const STYLESHEET: &str = include_str!("board.css");
pub const STYLESHEET_PATH: &str = "/static/board.css";

/// Form field carrying the per-session CSRF token.
pub const CSRF_FIELD: &str = "csrf";

/// Which navigation entry is current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Board,
    Announcements,
    Post,
    Mine,
    Personas,
    Settings,
    Notices,
    /// The report inbox and admin log.
    Admin,
    /// Server settings (administrators only).
    ServerSettings,
    /// The server rules page.
    Rules,
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
    /// Tag requests waiting for approval (administrators only, else 0).
    pub pending_tags: u64,
    /// The account's unread notices.
    pub unread_notices: u64,
    /// The server rules changed and the account must accept them before
    /// writing again.
    pub must_accept_rules: bool,
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
    /// Whether the server has rules, to link them in the footer.
    pub has_rules: bool,
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
    /// Sticky announcements, newest first: a banner each, not dismissible.
    pub sticky_announcements: Vec<quill_proto::announcements::Announcement>,
    /// The newest other announcement, unless the visitor dismissed it.
    pub announcement: Option<quill_proto::announcements::Announcement>,
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
    /// The rules version a new account accepts, when the server asks for
    /// that on joining. The form then has an "I accept" checkbox.
    pub accept_rules: Option<u32>,
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
    /// The chosen tags, as chips.
    pub chips: Vec<TagChip>,
    /// Text in the "add tags" field, kept when some of it couldn't be added.
    pub add_text: String,
    /// Why typed tags couldn't be added.
    pub tag_errors: Vec<String>,
    pub state: PromptState,
    pub error: Option<String>,
    /// Visible tags, for suggestions and "Browse tags".
    pub tag_options: Vec<Tag>,
    pub tag_policy: TagPolicy,
    /// Tag types, to show which are required or curated-only.
    pub namespaces: Vec<TagNamespace>,
}

/// A chosen tag on the prompt form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagChip {
    /// What the form sends back: `namespace:slug`, or `namespace:Display`
    /// for a tag that doesn't exist yet.
    pub value: String,
    /// For display. A new tag gets its type's colour.
    pub tag: Tag,
    pub kind: ChipKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipKind {
    /// A tag that exists.
    Existing,
    /// Created when the prompt is saved.
    New,
    /// Created when the prompt is saved, awaiting approval.
    NewNeedsApproval,
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
    /// The staff name field (administrators only), kept on error.
    pub staff_name: String,
    pub error: Option<String>,
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
    /// Filter: entries by this administrator.
    pub admin: Option<String>,
    /// Filter: entries against this account.
    pub account: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminLogEntryView {
    pub detail: AdminLogDetail,
    /// Showing a later page of accounts, so link back to the first.
    pub paged: bool,
}

/// One admin-log entry in full, its accounts after `after`.
pub fn admin_log_entry_url(id: i64, after: Option<&str>) -> String {
    match after {
        Some(a) => {
            let q = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("after", a)
                .finish();
            format!("/admin/log/{id}?{q}")
        }
        None => format!("/admin/log/{id}"),
    }
}

/// The admin log, filtered by administrator and/or affected account.
pub fn admin_log_url(admin: Option<&str>, account: Option<&str>, before: Option<i64>) -> String {
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    if let Some(a) = admin {
        q.append_pair("admin", a);
    }
    if let Some(a) = account {
        q.append_pair("account", a);
    }
    if let Some(b) = before {
        q.append_pair("before", &b.to_string());
    }
    let q = q.finish();
    if q.is_empty() {
        "/admin/log".to_owned()
    } else {
        format!("/admin/log?{q}")
    }
}

/// A date and time in UTC, such as "Oct 22, 2026 21:04 UTC".
pub fn short_datetime(at: OffsetDateTime) -> String {
    format!("{} {:02}:{:02} UTC", short_date(at), at.hour(), at.minute())
}

/// The "add a tag type" form's values, kept when it's sent back with an error.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewTagType {
    pub name: String,
    pub display: String,
    pub locked: bool,
    pub min: u32,
    pub max: Option<u32>,
    pub color: Option<ChipColor>,
}

/// New limits for a tag type that would leave prompts outside them, waiting
/// for the administrator to confirm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingLimits {
    pub namespace: String,
    pub min: u32,
    pub max: Option<u32>,
    /// Prompts on the board whose tags wouldn't meet them.
    pub outside: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminTagTypesView {
    /// The "new tags from writers" setting.
    pub new_tags: TagPolicy,
    pub namespaces: Vec<TagNamespace>,
    /// Per tag type, how many prompts on the board don't meet its limits.
    pub outside: Vec<(String, u64)>,
    pub new: NewTagType,
    pub pending: Option<PendingLimits>,
    pub error: Option<String>,
}

/// The prompts that don't meet a tag type's limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminOutsideLimitsView {
    pub namespace: TagNamespace,
    pub total: u64,
    /// The newest of them: id and title.
    pub prompts: Vec<(Uuid, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminTagsView {
    pub namespaces: Vec<TagNamespace>,
    pub tags: Vec<AdminTag>,
    /// Filter: tag type name.
    pub namespace: Option<String>,
    /// Filter: search text.
    pub search: String,
    /// More tags matched than are shown.
    pub truncated: bool,
    /// The "add a tag" form's values, kept on error.
    pub new_namespace: String,
    pub new_name: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminTagRequestsView {
    pub requests: Vec<AdminTag>,
    pub namespaces: Vec<TagNamespace>,
    pub new_tags: TagPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminTagView {
    pub tag: AdminTag,
    /// Display name of the tag's type.
    pub namespace_display: String,
    pub rename: String,
    pub alias_target: String,
    /// Suggestions for the alias target.
    pub tag_options: Vec<Tag>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminTagDeleteView {
    pub tag: AdminTag,
    pub namespace: TagNamespace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticesView {
    pub notices: Vec<quill_proto::notices::AccountNotice>,
    /// Link to older notices.
    pub older: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminHandlesView {
    pub list: quill_proto::handles::ReservedList,
    /// Filters.
    pub search: String,
    pub kind: Option<quill_proto::handles::ReservedKind>,
    pub source: Option<quill_proto::handles::ReservedSource>,
    /// The "check a handle" input and its result.
    pub test_input: String,
    pub test: Option<Result<quill_proto::handles::ReservedCheck, String>>,
    /// The "reserve" form, kept on error.
    pub add_text: String,
    pub add_kind: quill_proto::handles::ReservedKind,
    pub error: Option<String>,
}

/// Board link for a tag's admin page.
pub fn admin_tag_url(tag: &Tag) -> String {
    format!("/admin/settings/tags/{}/{}", tag.namespace, tag.slug)
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
                link { rel: "stylesheet", href: "{chips::TAG_COLORS_PATH}" }
            }
            body {
                {layout::header(&chrome)}
                main { class: "wrap",
                    if let Some(notice) = &chrome.notice {
                        p { class: "notice", role: "status", "{notice}" }
                    }
                    if chrome.viewer.as_ref().is_some_and(|v| v.must_accept_rules)
                        && chrome.current != Nav::Rules
                    {
                        {rules::banner()}
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

/// The admin "Sessions" settings tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminSessionsView {
    pub attachments: bool,
    pub max_attachment_bytes: u64,
    pub quota_bytes: u64,
    pub storage: quill_proto::sessions::ServerStorage,
    pub error: Option<String>,
}
